//! CLI surface over the same tools the MCP server exposes.
//!
//! Every subcommand is a thin adapter: build the tool's parameter struct, call the tool method on
//! [`RecipeRunDebug`], print its structured payload. There is no second implementation of anything
//! — a behaviour difference between `recipe-run-debug-mcp runs` and the `find_recipe_runs` tool is
//! a bug. Subcommand names are the short human-facing ones; the tool name is kept as an alias so
//! the docs and the MCP surface stay interchangeable.
//!
//! Unlike the rest of `src/`, this module owns stdout: in CLI mode there is no JSON-RPC stream to
//! corrupt. `tests/no_stdout.rs` exempts this file for that reason.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use crate::config::{Config, Overrides};
use crate::files::ReadMode;
use crate::server::{
    CompareRunsParams, DescribeRunFileParams, FileLocator, FindRecipeRunsParams, FindRecipesParams,
    GetRecipeConfigParams, GetRecipeRunParams, ReadRunFileParams, RecipeRunDebug, RunFilesParams,
    SearchRunFileParams, TaskRunConfigParams,
};

const ENV_VALUES: [&str; 7] = [
    "local",
    "development",
    "dev",
    "staging",
    "stg",
    "production",
    "prod",
];

#[derive(Debug, Parser)]
#[command(
    name = "recipe-run-debug-mcp",
    version,
    about = "Debug ingestion-platform recipe runs, as an MCP server or from the shell",
    long_about = "Debug ingestion-platform recipe runs.\n\n`serve` runs the MCP server on stdio; \
                  every other subcommand runs one tool and prints its JSON result.",
    after_help = "Configuration precedence: flags, then RRD_* env vars, then the config file.",
    // A subcommand is required, and a bare invocation prints help rather than a one-line error:
    // it is what someone who just installed this types first.
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub config: ConfigArgs,

    #[command(flatten)]
    pub output: OutputArgs,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Whether this invocation is the stdio MCP server rather than a one-shot command.
    pub fn is_serve(&self) -> bool {
        matches!(self.command, Command::Serve)
    }
}

/// The config-file settings, as flags. Global so they may appear before or after the subcommand,
/// and given their own help heading so they do not interleave with a subcommand's own options.
#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Configuration")]
pub struct ConfigArgs {
    /// Config file to read instead of $RRD_CONFIG / the default location.
    #[arg(long = "config-file", value_name = "PATH", global = true)]
    config_file: Option<PathBuf>,

    /// Environment, which selects the artefact bucket and the log hosts.
    #[arg(long, value_name = "ENV", global = true, value_parser = ENV_VALUES)]
    env: Option<String>,

    /// API gateway base URL.
    #[arg(long = "gateway", value_name = "URL", global = true)]
    gateway_base_url: Option<String>,

    /// Priority endpoint variant; only affects production.
    #[arg(long, value_name = "P", global = true, value_parser = ["hp", "lp"])]
    priority: Option<String>,

    /// Override the per-environment artefact bucket.
    #[arg(long, value_name = "NAME", global = true)]
    bucket: Option<String>,

    /// Bearer token for the gateway.
    #[arg(long, value_name = "TOKEN", global = true)]
    token: Option<String>,

    /// Extra request header, `Name: value`. Repeatable; replaces RRD_EXTRA_HEADERS.
    #[arg(long = "header", short = 'H', value_name = "HEADER", global = true)]
    headers: Vec<String>,

    /// Ceiling on the bytes of file content one command may return.
    #[arg(long, value_name = "N", global = true)]
    max_file_bytes: Option<usize>,
}

impl ConfigArgs {
    pub fn overrides(&self) -> Overrides {
        Overrides {
            config_path: self.config_file.clone(),
            env: self.env.clone(),
            gateway_base_url: self.gateway_base_url.clone(),
            priority_endpoint: self.priority.clone(),
            bucket: self.bucket.clone(),
            token: self.token.clone(),
            // The env var's own format, so a malformed entry produces the same error either way.
            extra_headers: (!self.headers.is_empty()).then(|| self.headers.join(";")),
            max_file_bytes: self.max_file_bytes,
        }
    }
}

#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Output")]
pub struct OutputArgs {
    /// Print single-line JSON instead of indented, for piping into jq.
    #[arg(long, global = true)]
    compact: bool,
}

/// Locates one artefact, either by URI or by (run, task, filename).
#[derive(Debug, Args)]
pub struct LocatorArgs {
    /// A gs://bucket/path URI, as printed by the other commands.
    #[arg(long = "uri", alias = "gcs-uri", value_name = "GCS_URI")]
    gcs_uri: Option<String>,

    /// The run the artefact belongs to; use with --task and --file.
    #[arg(long = "run", alias = "recipe-run-id", value_name = "RUN_ID")]
    recipe_run_id: Option<String>,

    /// The task within that run.
    #[arg(long = "task", alias = "task-name", value_name = "TASK")]
    task_name: Option<String>,

    /// Output file name, or "config" for the as-executed config.
    #[arg(long = "file", alias = "filename", value_name = "NAME")]
    filename: Option<String>,
}

impl From<LocatorArgs> for FileLocator {
    fn from(args: LocatorArgs) -> Self {
        Self {
            gcs_uri: args.gcs_uri,
            recipe_run_id: args.recipe_run_id,
            task_name: args.task_name,
            filename: args.filename,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the MCP server on stdio. This is how an MCP client launches the binary.
    Serve,

    /// Find recipes by name. [tool: find_recipes]
    #[command(visible_alias = "find-recipes")]
    Recipes {
        /// Substring matched against recipe names; omit to list everything.
        name_query: Option<String>,
        /// 1-based page number.
        #[arg(long)]
        page: Option<u32>,
        /// Results per page; capped at 100.
        #[arg(long)]
        size: Option<u32>,
    },

    /// Find recipe runs, mirroring the Recipe Runs UI filters. [tool: find_recipe_runs]
    #[command(visible_alias = "find-recipe-runs")]
    Runs {
        /// Exact recipe name; resolved to a recipe id before querying.
        recipe_name: Option<String>,
        /// Recipe id, if you have it instead of the name.
        #[arg(long)]
        recipe_id: Option<String>,
        /// Narrow to a single run.
        #[arg(long)]
        recipe_run_id: Option<String>,
        /// Narrow to the runs of one composite run.
        #[arg(long)]
        composite_run_id: Option<String>,
        /// Run state to include. Repeatable: in_progress|done|skipped|awaiting|failed
        #[arg(long = "state", value_name = "STATE")]
        state: Vec<String>,
        /// Server-side task predicate, as in the UI's "Recipe Task Predicate" field.
        #[arg(long)]
        task_predicate: Option<String>,
        /// Overlay name; needs a recipe, because overlay membership is derived.
        #[arg(long)]
        overlay: Option<String>,
        /// 1-based page number.
        #[arg(long)]
        page: Option<u32>,
        /// Results per page; capped at 50.
        #[arg(long)]
        size: Option<u32>,
    },

    /// Full detail for one run: tasks in order, every attempt, derived paths. [tool: get_recipe_run]
    #[command(visible_alias = "get-recipe-run")]
    Run {
        /// The recipe_run_id (not the row id).
        recipe_run_id: String,
    },

    /// The recipe's current stored config. [tool: get_recipe_config]
    #[command(visible_alias = "get-recipe-config")]
    Config {
        /// Recipe name; or pass --recipe-id.
        recipe_name: Option<String>,
        /// Recipe id, if you have it instead of the name.
        #[arg(long)]
        recipe_id: Option<String>,
        /// Restrict the output to a single task's config.
        #[arg(long = "task", alias = "task-name", value_name = "TASK")]
        task_name: Option<String>,
        /// Restrict the task list to the tasks of one overlay.
        #[arg(long)]
        overlay: Option<String>,
    },

    /// List a run's artefact files. [tool: list_run_files]
    #[command(visible_alias = "list-run-files")]
    Files {
        recipe_run_id: String,
        /// Restrict to a single task.
        #[arg(long = "task", alias = "task-name", value_name = "TASK")]
        task_name: Option<String>,
        /// Probe each derived path to see whether it exists. One request per file.
        #[arg(long = "check", alias = "check-existence")]
        check_existence: bool,
    },

    /// Read a window of an artefact file. [tool: read_run_file]
    #[command(visible_alias = "read-run-file")]
    Read {
        #[command(flatten)]
        locator: LocatorArgs,
        /// Which window to read; defaults to head.
        #[arg(long, value_enum, value_name = "MODE")]
        mode: Option<ReadMode>,
        /// Line offset, used with --mode slice.
        #[arg(long)]
        offset: Option<usize>,
        /// Lines to return; defaults to 200.
        #[arg(long, value_name = "N")]
        max_lines: Option<usize>,
        /// Byte ceiling for this call, clamped to --max-file-bytes.
        #[arg(long, value_name = "N")]
        limit_bytes: Option<usize>,
    },

    /// Regex-search an artefact file and print only matching lines. [tool: search_run_file]
    #[command(visible_alias = "search-run-file")]
    Search {
        /// Rust regex, matched per line.
        pattern: String,
        #[command(flatten)]
        locator: LocatorArgs,
        /// Matches to return; defaults to 20, capped at 200.
        #[arg(long, value_name = "N")]
        max_matches: Option<usize>,
        /// Lines of context around each match; capped at 10.
        #[arg(long = "context", short = 'C', value_name = "N")]
        context_lines: Option<usize>,
        /// Match case-insensitively.
        #[arg(long = "ignore-case", short = 'i')]
        case_insensitive: bool,
    },

    /// Structural summary of an artefact file: size, format, keys. [tool: describe_run_file]
    #[command(visible_alias = "describe-run-file")]
    Stat {
        #[command(flatten)]
        locator: LocatorArgs,
    },

    /// The config a task actually ran with. [tool: get_task_run_config]
    #[command(visible_alias = "get-task-run-config")]
    TaskConfig {
        recipe_run_id: String,
        task_name: String,
    },

    /// Diff a task's as-executed config against the recipe's current one. [tool: diff_task_config]
    #[command(visible_alias = "diff-task-config")]
    DiffConfig {
        recipe_run_id: String,
        task_name: String,
    },

    /// Triage a run in one call. [tool: explain_run_failure]
    #[command(visible_alias = "explain-run-failure")]
    Why { recipe_run_id: String },

    /// Grafana and Kibana deep links for a run. [tool: get_run_log_links]
    #[command(visible_alias = "get-run-log-links")]
    Links { recipe_run_id: String },

    /// Compare two runs task by task. [tool: compare_runs]
    #[command(visible_alias = "compare-runs")]
    Compare {
        recipe_run_id_a: String,
        recipe_run_id_b: String,
    },
}

/// Runs one command and prints its result. `Serve` never reaches here — `main` handles it.
pub async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let config = Config::load(&cli.config.overrides()).context("invalid configuration")?;
    let server = RecipeRunDebug::new(config)?;

    let result = match cli.command {
        Command::Serve => unreachable!("serve is dispatched by main"),

        Command::Recipes {
            name_query,
            page,
            size,
        } => {
            server
                .find_recipes(Parameters(FindRecipesParams {
                    name_query,
                    page,
                    size,
                }))
                .await
        }

        Command::Runs {
            recipe_name,
            recipe_id,
            recipe_run_id,
            composite_run_id,
            state,
            task_predicate,
            overlay,
            page,
            size,
        } => {
            server
                .find_recipe_runs(Parameters(FindRecipeRunsParams {
                    recipe_id,
                    recipe_name,
                    recipe_run_id,
                    composite_run_id,
                    state: (!state.is_empty()).then_some(state),
                    task_predicate,
                    overlay,
                    page,
                    size,
                }))
                .await
        }

        Command::Run { recipe_run_id } => {
            server
                .get_recipe_run(Parameters(GetRecipeRunParams { recipe_run_id }))
                .await
        }

        Command::Config {
            recipe_name,
            recipe_id,
            task_name,
            overlay,
        } => {
            server
                .get_recipe_config(Parameters(GetRecipeConfigParams {
                    recipe_id,
                    recipe_name,
                    task_name,
                    overlay,
                }))
                .await
        }

        Command::Files {
            recipe_run_id,
            task_name,
            check_existence,
        } => {
            server
                .list_run_files(Parameters(RunFilesParams {
                    recipe_run_id,
                    task_name,
                    check_existence: Some(check_existence),
                }))
                .await
        }

        Command::Read {
            locator,
            mode,
            offset,
            max_lines,
            limit_bytes,
        } => {
            server
                .read_run_file(Parameters(ReadRunFileParams {
                    locator: locator.into(),
                    mode,
                    offset,
                    max_lines,
                    limit_bytes,
                }))
                .await
        }

        Command::Search {
            pattern,
            locator,
            max_matches,
            context_lines,
            case_insensitive,
        } => {
            server
                .search_run_file(Parameters(SearchRunFileParams {
                    locator: locator.into(),
                    pattern,
                    max_matches,
                    context_lines,
                    case_insensitive: Some(case_insensitive),
                }))
                .await
        }

        Command::Stat { locator } => {
            server
                .describe_run_file(Parameters(DescribeRunFileParams {
                    locator: locator.into(),
                }))
                .await
        }

        Command::TaskConfig {
            recipe_run_id,
            task_name,
        } => {
            server
                .get_task_run_config(Parameters(TaskRunConfigParams {
                    recipe_run_id,
                    task_name,
                }))
                .await
        }

        Command::DiffConfig {
            recipe_run_id,
            task_name,
        } => {
            server
                .diff_task_config(Parameters(TaskRunConfigParams {
                    recipe_run_id,
                    task_name,
                }))
                .await
        }

        Command::Why { recipe_run_id } => {
            server
                .explain_run_failure(Parameters(GetRecipeRunParams { recipe_run_id }))
                .await
        }

        Command::Links { recipe_run_id } => {
            server
                .get_run_log_links(Parameters(GetRecipeRunParams { recipe_run_id }))
                .await
        }

        Command::Compare {
            recipe_run_id_a,
            recipe_run_id_b,
        } => {
            server
                .compare_runs(Parameters(CompareRunsParams {
                    recipe_run_id_a,
                    recipe_run_id_b,
                }))
                .await
        }
    };

    // A protocol-level error cannot happen for a direct call, but map it rather than panic.
    let result = result.map_err(|err| anyhow::anyhow!("{err}"))?;
    emit(&result, cli.output.compact)
}

/// Tool-level errors (a stale token, an unknown run, a bad regex) are the *expected* failure mode,
/// so they go to stderr as plain text with a non-zero exit rather than as JSON.
fn emit(result: &CallToolResult, compact: bool) -> anyhow::Result<ExitCode> {
    let text = || {
        result
            .content
            .iter()
            .filter_map(|block| block.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };

    if result.is_error == Some(true) {
        eprintln!("{}", text());
        return Ok(ExitCode::FAILURE);
    }

    match &result.structured_content {
        Some(payload) => {
            let rendered = if compact {
                serde_json::to_string(payload)
            } else {
                serde_json::to_string_pretty(payload)
            }
            .context("serializing the result")?;
            let mut out = std::io::stdout().lock();
            writeln!(out, "{rendered}").context("writing to stdout")?;
        }
        None => println!("{}", text()),
    }

    Ok(ExitCode::SUCCESS)
}
