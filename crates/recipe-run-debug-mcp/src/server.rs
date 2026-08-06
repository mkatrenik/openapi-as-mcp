//! MCP surface: tool definitions and their wiring to the API clients.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::aggregator::{self, Aggregator, RunQuery};
use crate::api::gcs_reader::GatewayFileStore;
use crate::api::model::{FromWire, Recipe, RecipeRun, RecipeRunState, TaskRun};
use crate::api::recipe_store::RecipeStore;
use crate::config::Config;
use crate::diff;
use crate::domain::{links, paths, tasks};
use crate::files::{self, ReadMode};
use crate::http::{ApiError, HttpClient};

pub struct RecipeRunDebug {
    http: HttpClient,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl RecipeRunDebug {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            http: HttpClient::new(config)?,
            tool_router: Self::tool_router(),
        })
    }

    fn config(&self) -> &Config {
        self.http.config()
    }

    /// Every payload carries the environment it came from, so prod and dev findings can never be
    /// silently mixed up.
    fn ok(&self, payload: Value) -> Result<CallToolResult, ErrorData> {
        let envelope = json!({
            "env": self.config().env.to_string(),
            "bucket": self.config().bucket,
            "data": payload,
        });
        Ok(CallToolResult::structured(envelope))
    }

    /// Upstream failures are *tool-level* errors, not protocol errors: the agent should see the
    /// message (especially "the gateway wants a token") rather than an opaque internal error.
    fn upstream_err(err: ApiError) -> Result<CallToolResult, ErrorData> {
        tracing::warn!(error = %err, "tool call failed");
        Ok(CallToolResult::error(vec![ContentBlock::text(
            err.to_string(),
        )]))
    }
}

/// Wire values of `RecipeRunState`, used in validation messages.
const RUN_STATES: &[&str] = &["in_progress", "done", "skipped", "awaiting", "failed"];

// ---------------------------------------------------------------------------
// Tool inputs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindRecipesParams {
    /// Substring to match against recipe names. Omit to list everything.
    #[serde(default)]
    pub name_query: Option<String>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub size: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindRecipeRunsParams {
    #[serde(default)]
    pub recipe_id: Option<String>,
    /// Exact recipe name; resolved to a recipe id before querying.
    #[serde(default)]
    pub recipe_name: Option<String>,
    #[serde(default)]
    pub recipe_run_id: Option<String>,
    #[serde(default)]
    pub composite_run_id: Option<String>,
    /// Run states to include: in_progress, done, skipped, awaiting, failed.
    #[serde(default)]
    pub state: Option<Vec<String>>,
    /// Server-side task predicate filter, as in the UI's "Recipe Task Predicate" field.
    #[serde(default)]
    pub task_predicate: Option<String>,
    /// Overlay name; applied client-side by comparing task-name sets, as the UI does.
    #[serde(default)]
    pub overlay: Option<String>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub size: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRecipeRunParams {
    /// The `recipe_run_id` (not the row `id`).
    pub recipe_run_id: String,
}

/// Locates one artefact, either directly by URI or by (run, task, filename).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FileLocator {
    /// A `gs://bucket/path` URI, as returned by the other tools.
    #[serde(default)]
    pub gcs_uri: Option<String>,
    #[serde(default)]
    pub recipe_run_id: Option<String>,
    #[serde(default)]
    pub task_name: Option<String>,
    /// Output file name, or "config" / the config file name for the as-executed config.
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadRunFileParams {
    #[serde(flatten)]
    pub locator: FileLocator,
    #[serde(default)]
    pub mode: Option<ReadMode>,
    /// Line offset, used with mode="slice".
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub max_lines: Option<usize>,
    #[serde(default)]
    pub limit_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRunFileParams {
    #[serde(flatten)]
    pub locator: FileLocator,
    /// Rust regex, matched per line.
    pub pattern: String,
    #[serde(default)]
    pub max_matches: Option<usize>,
    #[serde(default)]
    pub context_lines: Option<usize>,
    #[serde(default)]
    pub case_insensitive: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DescribeRunFileParams {
    #[serde(flatten)]
    pub locator: FileLocator,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunFilesParams {
    pub recipe_run_id: String,
    /// Restrict to a single task.
    #[serde(default)]
    pub task_name: Option<String>,
    /// Probe each derived path to see whether it really exists. Costs one request per file.
    #[serde(default)]
    pub check_existence: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskRunConfigParams {
    pub recipe_run_id: String,
    pub task_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareRunsParams {
    pub recipe_run_id_a: String,
    pub recipe_run_id_b: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRecipeConfigParams {
    #[serde(default)]
    pub recipe_id: Option<String>,
    #[serde(default)]
    pub recipe_name: Option<String>,
    /// Restrict the output to a single task's config.
    #[serde(default)]
    pub task_name: Option<String>,
    /// Restrict the task list to the tasks of one overlay.
    #[serde(default)]
    pub overlay: Option<String>,
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router]
impl RecipeRunDebug {
    #[tool(
        description = "Find recipes by name. Returns recipe ids, descriptions, the extracted \
                       source id, task names and overlay names."
    )]
    pub(crate) async fn find_recipes(
        &self,
        Parameters(params): Parameters<FindRecipesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = RecipeStore::new(&self.http);
        let page = params.page.unwrap_or(1);
        let size = params.size.unwrap_or(20).min(100);

        match store
            .list_recipes(params.name_query.as_deref(), page, size)
            .await
        {
            Ok(result) => {
                let recipes: Vec<Value> = result.items.iter().map(recipe_summary).collect();
                self.ok(json!({
                    "recipes": recipes,
                    "total": result.total,
                    "page": page,
                    "size": size,
                }))
            }
            Err(err) => Self::upstream_err(err),
        }
    }

    #[tool(
        description = "Find recipe runs, mirroring the Recipe Runs UI filters. Returns compact \
                       summaries including the failing task names — call get_recipe_run for detail."
    )]
    pub(crate) async fn find_recipe_runs(
        &self,
        Parameters(params): Parameters<FindRecipeRunsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = RecipeStore::new(&self.http);
        let aggregator = Aggregator::new(&self.http);

        // Validate the two constrained filters before spending a request, so the agent gets a
        // usable message instead of a 422 from the service.
        if let Some(states) = &params.state {
            let unknown: Vec<&str> = states
                .iter()
                .filter(|s| RecipeRunState::from_wire(s).is_none())
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "unknown run state(s) {unknown:?}; valid values are: {}",
                    RUN_STATES.join(", ")
                ))]));
            }
        }

        if let Some(predicate) = &params.task_predicate
            && !aggregator::task_predicate_is_valid(predicate)
        {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "task_predicate {predicate:?} does not match the required `<task-name>:<state>` \
                 form; state must be one of: {}",
                aggregator::TASK_PREDICATE_STATES.join(", ")
            ))]));
        }

        // An overlay filter needs the recipe, because overlay membership is derived, not stored.
        // Otherwise the aggregator filters by `recipe_name` itself and no lookup is needed.
        let recipe = if params.overlay.is_some() {
            match resolve_recipe(
                &store,
                params.recipe_id.as_deref(),
                params.recipe_name.as_deref(),
            )
            .await
            {
                Ok(recipe) => recipe,
                Err(Some(err)) => return Self::upstream_err(err),
                Err(None) => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        "an overlay filter requires recipe_id or recipe_name, because overlay \
                         membership is derived from the recipe's task sets",
                    )]));
                }
            }
        } else {
            None
        };

        let query = RunQuery {
            recipe_id: params.recipe_id.clone(),
            recipe_name: params.recipe_name.clone(),
            recipe_run_id: params.recipe_run_id.clone(),
            composite_run_id: params.composite_run_id.clone(),
            states: params.state.clone().unwrap_or_default(),
            task_predicate: params.task_predicate.clone(),
            page: params.page,
            size: params.size.map(|s| s.min(50)),
        };

        let result = match aggregator.list_runs(&query).await {
            Ok(result) => result,
            Err(err) => return Self::upstream_err(err),
        };

        let mut runs = Vec::new();
        let mut filtered_out = 0usize;
        for run in &result.items {
            let overlay = recipe.as_ref().and_then(|r| tasks::overlay_name(r, run));
            if let Some(wanted) = &params.overlay
                && overlay.as_deref() != Some(wanted.as_str())
            {
                filtered_out += 1;
                continue;
            }
            runs.push(run_summary(run, overlay));
        }

        self.ok(json!({
            "runs": runs,
            "total_before_overlay_filter": result.total,
            "pages": result.pages,
            "overlay_filtered_out_on_this_page": filtered_out,
        }))
    }

    #[tool(
        description = "Full detail for one recipe run: tasks in execution order, every attempt \
                       with its reason and error_type, and the derived artefact paths."
    )]
    pub(crate) async fn get_recipe_run(
        &self,
        Parameters(params): Parameters<GetRecipeRunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let aggregator = Aggregator::new(&self.http);
        let run = match aggregator.get_run(&params.recipe_run_id).await {
            Ok(run) => run,
            Err(err) => return Self::upstream_err(err),
        };

        // The recipe is best-effort: without it, output-file names cannot be derived, but the
        // task/attempt detail is still worth returning.
        let recipe = RecipeStore::new(&self.http)
            .get_recipe(&run.recipe_id)
            .await
            .inspect_err(|err| tracing::warn!(error = %err, "recipe lookup failed"))
            .ok();

        let sorted = tasks::sort_tasks(&run.task_runs);
        let bucket = &self.config().bucket;

        let task_details: Vec<Value> = sorted
            .iter()
            .map(|task| {
                let mut files = vec![paths::config_file(bucket, task)];
                if let Some(recipe) = &recipe {
                    for name in paths::expected_output_names(recipe, task) {
                        files.push(paths::output_file(bucket, task, &run.recipe_run_id, &name));
                    }
                }
                task_detail(task, files)
            })
            .collect();

        self.ok(json!({
            "run": run_summary(&run, recipe.as_ref().and_then(|r| tasks::overlay_name(r, &run))),
            "recipe_available": recipe.is_some(),
            "failed_on_validation_agent": tasks::is_failed_on_validation(&sorted),
            "first_failed_task": tasks::first_failed(&sorted).map(|t| t.name.clone()),
            "tasks": task_details,
            "note": "artefact paths are derived from backend naming convention and are not \
                     verified to exist; use list_run_files to check",
        }))
    }

    #[tool(
        description = "The recipe's current stored config, optionally narrowed to one task or \
                       overlay. For what actually ran, use get_task_run_config instead."
    )]
    pub(crate) async fn get_recipe_config(
        &self,
        Parameters(params): Parameters<GetRecipeConfigParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = RecipeStore::new(&self.http);
        let recipe = match resolve_recipe(
            &store,
            params.recipe_id.as_deref(),
            params.recipe_name.as_deref(),
        )
        .await
        {
            Ok(Some(recipe)) => recipe,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "provide recipe_id or recipe_name",
                )]));
            }
            Err(Some(err)) => return Self::upstream_err(err),
            Err(None) => unreachable!("resolve_recipe returns Err(None) only without inputs"),
        };

        let overlay_task_names: Option<Vec<String>> = params.overlay.as_ref().map(|name| {
            recipe
                .tasks_overlays
                .get(name)
                .map(|o| o.tasks.iter().map(|t| t.name.clone()).collect())
                .unwrap_or_default()
        });

        let tasks: Vec<Value> = recipe
            .tasks
            .iter()
            .filter(|t| {
                params
                    .task_name
                    .as_ref()
                    .is_none_or(|wanted| &t.name == wanted)
            })
            .filter(|t| {
                overlay_task_names
                    .as_ref()
                    .is_none_or(|names| names.contains(&t.name))
            })
            .map(|t| {
                json!({
                    "task_id": t.id.to_string(),
                    "name": t.name,
                    "config": t.config,
                })
            })
            .collect();

        self.ok(json!({
            "recipe_id": recipe.id.to_string(),
            "name": recipe.name,
            "description": recipe.description,
            "source_id": tasks::source_id(&recipe),
            "overlays": recipe.tasks_overlays.keys().collect::<Vec<_>>(),
            "overlay_filter": params.overlay,
            "tasks": tasks,
        }))
    }
    #[tool(
        description = "List the artefact files for a run: the as-executed config per task plus the \
                       output files derived from the recipe config. Set check_existence=true to \
                       verify each path actually exists in the bucket."
    )]
    pub(crate) async fn list_run_files(
        &self,
        Parameters(params): Parameters<RunFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let run = match Aggregator::new(&self.http)
            .get_run(&params.recipe_run_id)
            .await
        {
            Ok(run) => run,
            Err(err) => return Self::upstream_err(err),
        };

        let recipe = RecipeStore::new(&self.http)
            .get_recipe(&run.recipe_id)
            .await;
        let recipe = match recipe {
            Ok(recipe) => recipe,
            Err(err) => return Self::upstream_err(err),
        };

        let bucket = &self.config().bucket;
        let store = GatewayFileStore::new(&self.http);
        let check = params.check_existence.unwrap_or(false);

        let mut tasks_out = Vec::new();
        for task in tasks::sort_tasks(&run.task_runs) {
            if let Some(wanted) = &params.task_name
                && &task.name != wanted
            {
                continue;
            }

            let mut entries = vec![paths::config_file(bucket, &task)];
            for name in paths::expected_output_names(&recipe, &task) {
                entries.push(paths::output_file(bucket, &task, &run.recipe_run_id, &name));
            }

            let mut files = Vec::new();
            for entry in entries {
                let exists = if check {
                    match store.exists(&entry.path).await {
                        Ok(exists) => Some(exists),
                        Err(err) => {
                            tracing::warn!(error = %err, path = %entry.path, "existence probe failed");
                            None
                        }
                    }
                } else {
                    None
                };

                let mut value = serde_json::to_value(&entry).unwrap_or(Value::Null);
                value["exists"] = json!(exists);
                files.push(value);
            }

            tasks_out.push(json!({
                "task_name": task.name,
                "task_state": task.state,
                "is_failed": task.is_failed(),
                "files": files,
            }));
        }

        if tasks_out.is_empty() && params.task_name.is_some() {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "run {} has no task named {:?}",
                params.recipe_run_id,
                params.task_name.unwrap_or_default()
            ))]));
        }

        self.ok(json!({
            "recipe_run_id": run.recipe_run_id,
            "existence_checked": check,
            "tasks": tasks_out,
            "note": if check {
                "exists=false on an expected output usually means the task failed before writing it"
            } else {
                "paths are derived from backend naming convention; pass check_existence=true to verify"
            },
        }))
    }

    #[tool(
        description = "Read a window of an artefact file. Defaults to the first lines; use \
                       mode=tail/slice with offset to page. Never returns the whole file unbounded."
    )]
    pub(crate) async fn read_run_file(
        &self,
        Parameters(params): Parameters<ReadRunFileParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let resolved = match self.resolve_locator(&params.locator).await {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };

        let content = match GatewayFileStore::new(&self.http)
            .read_all(&resolved.path)
            .await
        {
            Ok(content) => content,
            Err(err) => return Self::upstream_err(err),
        };

        let max_bytes = params
            .limit_bytes
            .unwrap_or(self.config().max_file_bytes)
            .min(self.config().max_file_bytes);

        let mut payload = files::slice(
            &content,
            &resolved.name,
            params.mode.unwrap_or_default(),
            params.offset.unwrap_or(0),
            params.max_lines.unwrap_or(200),
            max_bytes,
        );
        payload["gcs_uri"] = json!(resolved.gcs_uri);
        payload["name"] = json!(resolved.name);

        self.ok(payload)
    }

    #[tool(
        description = "Regex-search an artefact file server-side and return only matching lines \
                       with line numbers. Use this instead of reading large files."
    )]
    pub(crate) async fn search_run_file(
        &self,
        Parameters(params): Parameters<SearchRunFileParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let resolved = match self.resolve_locator(&params.locator).await {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };

        let content = match GatewayFileStore::new(&self.http)
            .read_all(&resolved.path)
            .await
        {
            Ok(content) => content,
            Err(err) => return Self::upstream_err(err),
        };

        match files::search(
            &content,
            &params.pattern,
            params.max_matches.unwrap_or(20).min(200),
            params.context_lines.unwrap_or(0).min(10),
            params.case_insensitive.unwrap_or(false),
        ) {
            Ok(mut payload) => {
                payload["gcs_uri"] = json!(resolved.gcs_uri);
                self.ok(payload)
            }
            Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
        }
    }

    #[tool(
        description = "Cheap structural summary of an artefact file: size, line/record count, \
                       detected format, headers or top-level keys. The right first look."
    )]
    pub(crate) async fn describe_run_file(
        &self,
        Parameters(params): Parameters<DescribeRunFileParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let resolved = match self.resolve_locator(&params.locator).await {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };

        let content = match GatewayFileStore::new(&self.http)
            .read_all(&resolved.path)
            .await
        {
            Ok(content) => content,
            Err(err) => return Self::upstream_err(err),
        };

        let mut payload = files::describe(&content, &resolved.name);
        payload["gcs_uri"] = json!(resolved.gcs_uri);
        self.ok(payload)
    }

    #[tool(
        description = "The config a task actually ran with, read from the run's own config \
                       artefact. This is ground truth; the recipe's stored config may have \
                       changed since the run."
    )]
    pub(crate) async fn get_task_run_config(
        &self,
        Parameters(params): Parameters<TaskRunConfigParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .as_executed_config(&params.recipe_run_id, &params.task_name)
            .await
        {
            Ok((task, config, raw_len)) => self.ok(json!({
                "recipe_run_id": params.recipe_run_id,
                "task_name": task.name,
                "agent": task.agent(),
                "source": paths::config_file(&self.config().bucket, &task),
                "bytes": raw_len,
                "config": config,
            })),
            Err(result) => result,
        }
    }

    #[tool(
        description = "Diff the config a task actually ran with against the recipe's current \
                       stored config. Explains 'it worked last week'."
    )]
    pub(crate) async fn diff_task_config(
        &self,
        Parameters(params): Parameters<TaskRunConfigParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (task, as_executed, _) = match self
            .as_executed_config(&params.recipe_run_id, &params.task_name)
            .await
        {
            Ok(found) => found,
            Err(result) => return result,
        };

        let run = match Aggregator::new(&self.http)
            .get_run(&params.recipe_run_id)
            .await
        {
            Ok(run) => run,
            Err(err) => return Self::upstream_err(err),
        };
        let recipe = match RecipeStore::new(&self.http)
            .get_recipe(&run.recipe_id)
            .await
        {
            Ok(recipe) => recipe,
            Err(err) => return Self::upstream_err(err),
        };

        let current = recipe
            .tasks
            .iter()
            .find(|t| t.id.to_string() == task.task_id)
            .map(|t| Value::Object(t.config.clone()))
            .unwrap_or(Value::Null);

        // The as-executed artefact wraps the task config in its own envelope in some agents, so
        // compare the task-config subtree when it is present, and the whole document otherwise.
        let left = as_executed
            .get("config")
            .cloned()
            .unwrap_or_else(|| as_executed.clone());

        let changes = diff::diff(&left, &current);

        self.ok(json!({
            "recipe_run_id": params.recipe_run_id,
            "task_name": task.name,
            "left": "as_executed (from the run's config artefact)",
            "right": "current (from recipe-store)",
            "identical": changes.is_empty(),
            "change_count": changes.len(),
            "changes": changes,
            "caveat": "the as-executed artefact layout varies per agent; if every field shows as \
                       changed, inspect it with read_run_file before trusting this diff",
        }))
    }

    #[tool(
        description = "Triage a run in one call: the first failing task, every attempt's reason \
                       and error_type, whether it failed on the validation agent, its files, and \
                       Grafana/Kibana links."
    )]
    pub(crate) async fn explain_run_failure(
        &self,
        Parameters(params): Parameters<GetRecipeRunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let run = match Aggregator::new(&self.http)
            .get_run(&params.recipe_run_id)
            .await
        {
            Ok(run) => run,
            Err(err) => return Self::upstream_err(err),
        };

        let recipe = RecipeStore::new(&self.http)
            .get_recipe(&run.recipe_id)
            .await
            .inspect_err(|err| tracing::warn!(error = %err, "recipe lookup failed"))
            .ok();

        let sorted = tasks::sort_tasks(&run.task_runs);
        let bucket = &self.config().bucket;

        let failing = tasks::first_failed(&sorted).cloned();
        let failing_detail = match &failing {
            Some(task) => {
                let mut files = vec![paths::config_file(bucket, task)];
                if let Some(recipe) = &recipe {
                    for name in paths::expected_output_names(recipe, task) {
                        files.push(paths::output_file(bucket, task, &run.recipe_run_id, &name));
                    }
                }
                task_detail(task, files)
            }
            None => Value::Null,
        };

        // Tasks that never started are the downstream fallout, not the cause — call that out.
        let not_started: Vec<&str> = sorted
            .iter()
            .filter(|t| t.started_at_utc.is_none())
            .map(|t| t.name.as_str())
            .collect();

        self.ok(json!({
            "run": run_summary(&run, recipe.as_ref().and_then(|r| tasks::overlay_name(r, &run))),
            "verdict": match (run.is_failed(), &failing) {
                (true, Some(task)) => format!(
                    "run failed; first failure was task {:?} on agent {}",
                    task.name, task.agent_name
                ),
                (true, None) => {
                    "run is marked failed but no task run is in a failed state".to_string()
                }
                (false, _) => format!("run is {}, not failed", run.state),
            },
            "failed_on_validation_agent": tasks::is_failed_on_validation(&sorted),
            "first_failed_task": failing_detail,
            "tasks_never_started": not_started,
            "log_links": self.log_links(&run, recipe.as_ref()),
            "next_steps": [
                "search_run_file with the error string on the failing task's output or config",
                "get_task_run_config for the exact config that ran",
                "compare_runs against the last successful run of this recipe",
            ],
        }))
    }

    #[tool(description = "Grafana Loki and Kibana deep links for a run, scoped to its time range.")]
    pub(crate) async fn get_run_log_links(
        &self,
        Parameters(params): Parameters<GetRecipeRunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let run = match Aggregator::new(&self.http)
            .get_run(&params.recipe_run_id)
            .await
        {
            Ok(run) => run,
            Err(err) => return Self::upstream_err(err),
        };
        let recipe = RecipeStore::new(&self.http)
            .get_recipe(&run.recipe_id)
            .await
            .ok();
        self.ok(self.log_links(&run, recipe.as_ref()))
    }

    #[tool(
        description = "Compare two runs task by task: state, timing, and which expected outputs \
                       each produced. Use against the last good run to isolate what changed."
    )]
    pub(crate) async fn compare_runs(
        &self,
        Parameters(params): Parameters<CompareRunsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let aggregator = Aggregator::new(&self.http);
        let (a, b) = match (
            aggregator.get_run(&params.recipe_run_id_a).await,
            aggregator.get_run(&params.recipe_run_id_b).await,
        ) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(err), _) | (_, Err(err)) => return Self::upstream_err(err),
        };

        let sorted_a = tasks::sort_tasks(&a.task_runs);
        let sorted_b = tasks::sort_tasks(&b.task_runs);

        let mut names: Vec<String> = sorted_a
            .iter()
            .chain(sorted_b.iter())
            .map(|t| t.name.clone())
            .collect();
        names.sort();
        names.dedup();

        let task_deltas: Vec<Value> = names
            .iter()
            .map(|name| {
                let ta = sorted_a.iter().find(|t| &t.name == name);
                let tb = sorted_b.iter().find(|t| &t.name == name);
                json!({
                    "task_name": name,
                    "state_a": ta.map(|t| t.state.clone()),
                    "state_b": tb.map(|t| t.state.clone()),
                    "same_state": match (ta, tb) {
                        (Some(x), Some(y)) => Some(x.state == y.state),
                        _ => None,
                    },
                    "agent_a": ta.map(|t| t.agent()),
                    "agent_b": tb.map(|t| t.agent()),
                    "duration_seconds_a": ta.and_then(task_duration),
                    "duration_seconds_b": tb.and_then(task_duration),
                    "only_in": match (ta, tb) {
                        (Some(_), None) => Some("a"),
                        (None, Some(_)) => Some("b"),
                        _ => None,
                    },
                })
            })
            .collect();

        let differing: Vec<&Value> = task_deltas
            .iter()
            .filter(|d| d["same_state"] != json!(true))
            .collect();

        self.ok(json!({
            "a": run_summary(&a, None),
            "b": run_summary(&b, None),
            "same_recipe": a.recipe_id == b.recipe_id,
            "recipe_version_a": a.recipe_version,
            "recipe_version_b": b.recipe_version,
            "differing_task_count": differing.len(),
            "differing_tasks": differing,
            "all_tasks": task_deltas,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RecipeRunDebug {}

// ---------------------------------------------------------------------------
// Shared tool internals
// ---------------------------------------------------------------------------

/// A locator resolved to a concrete artefact.
struct ResolvedFile {
    name: String,
    path: String,
    gcs_uri: String,
}

impl RecipeRunDebug {
    /// Resolve a `FileLocator` to a bucket-less path. Returns the tool-level error result directly
    /// when the locator cannot be resolved, so callers can `?`-style bail.
    async fn resolve_locator(
        &self,
        locator: &FileLocator,
    ) -> Result<ResolvedFile, Result<CallToolResult, ErrorData>> {
        if let Some(uri) = &locator.gcs_uri {
            let Some((bucket, path)) = paths::parse_gcs_uri(uri) else {
                return Err(Ok(CallToolResult::error(vec![ContentBlock::text(
                    format!("{uri:?} is not a gs://bucket/path URI"),
                )])));
            };
            if bucket != self.config().bucket {
                return Err(Ok(CallToolResult::error(vec![ContentBlock::text(
                    format!(
                        "this server is configured for bucket {:?} but the URI names {bucket:?}; \
                     restart with RRD_ENV/RRD_BUCKET_OVERRIDE matching the environment you mean",
                        self.config().bucket
                    ),
                )])));
            }
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            return Ok(ResolvedFile {
                name,
                gcs_uri: uri.clone(),
                path,
            });
        }

        let (Some(run_id), Some(task_name), Some(filename)) = (
            locator.recipe_run_id.as_ref(),
            locator.task_name.as_ref(),
            locator.filename.as_ref(),
        ) else {
            return Err(Ok(CallToolResult::error(vec![ContentBlock::text(
                "provide either gcs_uri, or all of recipe_run_id + task_name + filename",
            )])));
        };

        let run = match Aggregator::new(&self.http).get_run(run_id).await {
            Ok(run) => run,
            Err(err) => return Err(Self::upstream_err(err)),
        };

        let Some(task) = run.task_runs.iter().find(|t| &t.name == task_name) else {
            let available: Vec<&str> = run.task_runs.iter().map(|t| t.name.as_str()).collect();
            return Err(Ok(CallToolResult::error(vec![ContentBlock::text(
                format!(
                    "run {run_id} has no task named {task_name:?}; tasks are: {}",
                    available.join(", ")
                ),
            )])));
        };

        let bucket = &self.config().bucket;
        let file = if filename == "config" || filename == "config.yaml" {
            paths::config_file(bucket, task)
        } else {
            paths::output_file(bucket, task, &run.recipe_run_id, filename)
        };

        Ok(ResolvedFile {
            name: file.name,
            path: file.path,
            gcs_uri: file.gcs_uri,
        })
    }

    /// Fetch and parse a task's as-executed config artefact (YAML, occasionally JSON).
    async fn as_executed_config(
        &self,
        recipe_run_id: &str,
        task_name: &str,
    ) -> Result<(TaskRun, Value, usize), Result<CallToolResult, ErrorData>> {
        let run = match Aggregator::new(&self.http).get_run(recipe_run_id).await {
            Ok(run) => run,
            Err(err) => return Err(Self::upstream_err(err)),
        };

        let Some(task) = run.task_runs.iter().find(|t| t.name == task_name).cloned() else {
            let available: Vec<&str> = run.task_runs.iter().map(|t| t.name.as_str()).collect();
            return Err(Ok(CallToolResult::error(vec![ContentBlock::text(
                format!(
                    "run {recipe_run_id} has no task named {task_name:?}; tasks are: {}",
                    available.join(", ")
                ),
            )])));
        };

        let raw = match GatewayFileStore::new(&self.http)
            .read_all(&task.config_file_path)
            .await
        {
            Ok(raw) => raw,
            Err(err) => return Err(Self::upstream_err(err)),
        };

        let parsed = parse_config_document(&raw).map_err(|message| {
            Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "could not parse {} as YAML or JSON: {message}",
                task.config_file_path
            ))]))
        })?;

        let len = raw.len();
        Ok((task, parsed, len))
    }

    fn log_links(&self, run: &RecipeRun, recipe: Option<&Recipe>) -> Value {
        let env = self.config().env;
        // Runs always carry a start time (required, non-nullable in the spec); an in-flight run has
        // no end time yet, so scope the window to "now" in that case.
        let started = run.started_at_utc;
        let ended = run.ended_at_utc.unwrap_or_else(chrono::Utc::now);
        let source = recipe.and_then(tasks::source_id);

        json!({
            "grafana_logs_url": links::grafana_logs_url(env, &run.recipe_run_id, started, ended),
            "grafana_window_end_is_now": run.ended_at_utc.is_none(),
            "kibana_source_id": source,
            "kibana_discover_url": source.as_deref().map(|id| links::kibana_discover_url(env, id)),
            "kibana_note": source.is_none().then_some(
                "no source id in the recipe description, so no Kibana link is available"
            ),
        })
    }
}

/// The as-executed config artefact is YAML by convention; accept JSON too, since YAML parsers
/// accept JSON but the error messages are clearer when tried explicitly.
fn parse_config_document(raw: &str) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Ok(value);
    }
    serde_yaml::from_str::<Value>(raw).map_err(|e| e.to_string())
}

fn task_duration(task: &TaskRun) -> Option<i64> {
    let start = task.started_at_utc?;
    let end = task.ended_at_utc?;
    Some((end - start).num_seconds())
}

// ---------------------------------------------------------------------------
// Shaping helpers
// ---------------------------------------------------------------------------

/// Resolve a recipe from an id or a name. `Err(None)` means "no identifier supplied".
async fn resolve_recipe(
    store: &RecipeStore<'_>,
    recipe_id: Option<&str>,
    recipe_name: Option<&str>,
) -> Result<Option<Recipe>, Option<ApiError>> {
    if let Some(id) = recipe_id {
        return store.get_recipe(id).await.map(Some).map_err(Some);
    }
    if let Some(name) = recipe_name {
        return store.get_recipe_by_name(name).await.map(Some).map_err(Some);
    }
    Err(None)
}

fn recipe_summary(recipe: &Recipe) -> Value {
    json!({
        "recipe_id": recipe.id.to_string(),
        "name": recipe.name,
        "description": recipe.description,
        "source_id": tasks::source_id(recipe),
        "task_names": recipe.tasks.iter().map(|t| &t.name).collect::<Vec<_>>(),
        "overlay_names": recipe.tasks_overlays.keys().collect::<Vec<_>>(),
    })
}

fn run_summary(run: &RecipeRun, overlay: Option<String>) -> Value {
    let sorted = tasks::sort_tasks(&run.task_runs);
    let failed: Vec<&str> = sorted
        .iter()
        .filter(|t| t.is_failed())
        .map(|t| t.name.as_str())
        .collect();

    json!({
        "recipe_run_id": run.recipe_run_id,
        "composite_run_id": run.composite_run_id,
        "is_simple_run": run.is_simple(),
        "previous_recipe_run_id": run.previous_recipe_run_id,
        "recipe_id": run.recipe_id,
        "recipe_name": run.recipe_name,
        "recipe_version": run.recipe_version,
        "state": run.state,
        "state_recognised": run.known_state().is_some(),
        "started_at_utc": run.started_at_utc,
        "ended_at_utc": run.ended_at_utc,
        "duration_seconds": duration_seconds(run),
        "overlay": overlay,
        "task_count": run.task_runs.len(),
        "failed_task_names": failed,
        "task_names_in_order": sorted.iter().map(|t| &t.name).collect::<Vec<_>>(),
    })
}

fn task_detail(task: &TaskRun, files: Vec<paths::ArtefactFile>) -> Value {
    let attempts: Vec<Value> = task
        .task_run_executions
        .iter()
        .map(|e| {
            json!({
                "execution_state": e.execution_state,
                "reason": e.reason,
                "error_type": e.error_type,
                "started_at_utc": e.started_at_utc,
                "ended_at_utc": e.ended_at_utc,
            })
        })
        .collect();

    json!({
        "name": task.name,
        "task_id": task.task_id,
        "agent": task.agent(),
        "agent_name": task.agent_name,
        "state": task.state,
        // A state the vendored spec doesn't list means either a backend change or a stale spec —
        // worth surfacing rather than silently treating as "not failed".
        "state_recognised": task.known_state().is_some(),
        "is_failed": task.is_failed(),
        "started_at_utc": task.started_at_utc,
        "ended_at_utc": task.ended_at_utc,
        "attempts": attempts,
        "data_files_path": task.data_files_path,
        "files": files,
    })
}

/// `None` while a run is still in flight — it has a start time but no end time.
fn duration_seconds(run: &RecipeRun) -> Option<i64> {
    let end = run.ended_at_utc?;
    Some((end - run.started_at_utc).num_seconds())
}
