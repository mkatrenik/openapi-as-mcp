//! The MCP surface: one tool per OpenAPI operation.
//!
//! Unlike a hand-written server, the tool list is not known at compile time, so [`ServerHandler`]
//! is implemented directly instead of through the `#[tool]` macros: `list_tools` returns a vector
//! built at startup and `call_tool` dispatches by name.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, bail};
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

use crate::config::{ApiConfig, Config};
use crate::exec::{CallError, Executor};
use crate::spec::{Api, Operation};

/// One operation, plus which document it came from — the index into `executors`, which is what
/// decides where the call goes and with which credentials.
#[derive(Debug, Clone)]
struct Bound {
    operation: Operation,
    api: usize,
}

#[derive(Debug)]
pub struct OpenApiMcp {
    /// Parallel to the documents, in config order.
    executors: Vec<Executor>,
    tools: Vec<Tool>,
    operations: HashMap<String, Bound>,
    instructions: String,
}

impl OpenApiMcp {
    /// Builds the server from already-loaded documents, one per `config.apis` in the same order.
    /// Fails only for things that make the whole server useless: a document with no base URL to
    /// send its requests to, or no operations left after filtering.
    pub fn new(config: &Config, apis: Vec<Api>) -> anyhow::Result<Self> {
        assert_eq!(
            config.apis.len(),
            apis.len(),
            "one loaded document per configured api"
        );

        // Names are unique per document; two documents can still collide, so dedupe again here.
        let mut taken: BTreeMap<String, usize> = BTreeMap::new();
        let mut operations = HashMap::new();
        let mut tools = Vec::new();
        let mut executors = Vec::with_capacity(apis.len());
        let mut skipped = 0usize;

        for (index, (settings, api)) in config.apis.iter().zip(&apis).enumerate() {
            let base_url = base_url_for(settings, api)?;

            for operation in &api.operations {
                if !keep(settings, operation) {
                    skipped += 1;
                    continue;
                }
                let mut operation = operation.clone();
                operation.name = unique(prefixed(settings, &operation.name), &mut taken);
                tools.push(tool_for(&operation));
                operations.insert(
                    operation.name.clone(),
                    Bound {
                        operation,
                        api: index,
                    },
                );
            }

            tracing::info!(
                api = %settings.label,
                base_url = %base_url,
                read_only = settings.read_only,
                authenticated = !settings.headers.is_empty(),
                "serving document"
            );
            executors.push(Executor::new(settings, base_url)?);
        }

        if tools.is_empty() {
            bail!(
                "no operations to serve: {} found, all filtered out by read-only/include/exclude",
                skipped
            );
        }
        tracing::info!(
            tools = tools.len(),
            skipped,
            apis = apis.len(),
            "built tool set"
        );

        Ok(Self {
            instructions: instructions(config, &apis, &tools),
            executors,
            tools,
            operations,
        })
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn operation(&self, name: &str) -> Option<&Operation> {
        self.operations.get(name).map(|bound| &bound.operation)
    }

    /// Where one tool's requests go.
    pub fn base_url_of(&self, name: &str) -> Option<&str> {
        let bound = self.operations.get(name)?;
        Some(self.executors[bound.api].base_url())
    }

    /// Every base URL being served, in config order.
    pub fn base_urls(&self) -> Vec<&str> {
        self.executors.iter().map(Executor::base_url).collect()
    }

    /// Runs one tool. Shared by the MCP handler and the CLI so the two can never drift.
    pub async fn call(&self, name: &str, arguments: Map<String, Value>) -> CallToolResult {
        let Some(bound) = self.operations.get(name) else {
            let mut known: Vec<&str> = self.operations.keys().map(String::as_str).collect();
            known.sort_unstable();
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool {name:?}. Available: {}",
                known.join(", ")
            ))]);
        };

        match self.executors[bound.api]
            .call(&bound.operation, &arguments)
            .await
        {
            Ok(payload) => CallToolResult::structured(payload),
            // Everything the upstream API can do to us — a 404, a stale token, a bad argument —
            // is a *tool-level* error: the model should read the message and try again, not see
            // an opaque protocol failure.
            Err(err) => {
                tracing::warn!(tool = name, error = %err, "tool call failed");
                CallToolResult::error(vec![ContentBlock::text(describe_error(&err))])
            }
        }
    }
}

/// Errors get an extra sentence where the fix is not obvious from the message alone.
fn describe_error(err: &CallError) -> String {
    let hint = match err {
        CallError::Upstream { status, .. }
            if *status == reqwest::StatusCode::UNAUTHORIZED
                || *status == reqwest::StatusCode::FORBIDDEN =>
        {
            Some(
                "The API rejected the credentials. Restart the server with --token <TOKEN>, or \
                 fix this API's `token` in the config file.",
            )
        }
        CallError::Transport { .. } => Some(
            "Check the base URL: it must be reachable from this machine, and it is where this \
             tool's calls are sent.",
        ),
        _ => None,
    };

    match hint {
        Some(hint) => format!("{err}\n\n{hint}"),
        None => err.to_string(),
    }
}

fn keep(config: &ApiConfig, operation: &Operation) -> bool {
    if config.read_only && !operation.is_read_only() {
        return false;
    }
    // Each candidate is matched on its own rather than as one joined string, so an anchored
    // pattern means what it looks like: `exclude = ['^/admin']` filters by path, `'^delete'` by
    // tool name, and neither has to know where the other sits in a haystack.
    let candidates = [
        operation.name.as_str(),
        operation.method.as_str(),
        operation.path.as_str(),
    ];
    let matches = |patterns: &[regex::Regex]| {
        patterns
            .iter()
            .any(|re| candidates.iter().any(|candidate| re.is_match(candidate)))
    };

    if !config.include.is_empty() && !matches(&config.include) {
        return false;
    }
    !matches(&config.exclude)
}

fn prefixed(config: &ApiConfig, name: &str) -> String {
    match &config.tool_prefix {
        Some(prefix) => format!("{prefix}{name}"),
        None => name.to_string(),
    }
}

fn unique(name: String, taken: &mut BTreeMap<String, usize>) -> String {
    match taken.get_mut(&name) {
        None => {
            taken.insert(name.clone(), 1);
            name
        }
        Some(count) => {
            *count += 1;
            format!("{name}_{count}")
        }
    }
}

fn tool_for(operation: &Operation) -> Tool {
    let read_only = operation.is_read_only();
    Tool::new(
        operation.name.clone(),
        operation.description.clone(),
        Arc::new(operation.input_schema.clone()),
    )
    .annotate(
        ToolAnnotations::new()
            .read_only(read_only)
            // DELETE removes something; PUT and PATCH overwrite in place. POST is the additive
            // one, so it is the only write method that is not flagged destructive.
            .destructive(matches!(
                operation.method.as_str(),
                "DELETE" | "PUT" | "PATCH"
            ))
            .idempotent(matches!(
                operation.method.as_str(),
                "GET" | "HEAD" | "OPTIONS" | "PUT" | "DELETE"
            ))
            .open_world(true),
    )
}

/// A document's base URL: its own setting first, then what the document declares. Named in the
/// error, because with several documents "no base URL" has to say *which*.
fn base_url_for(settings: &ApiConfig, api: &Api) -> anyhow::Result<String> {
    settings
        .base_url
        .clone()
        .or_else(|| api.servers.first().cloned())
        .with_context(|| {
            format!(
                "no base URL for {}: the document declares no `servers`, so set `base_url` for it \
                 in the config file (or pass --base-url)",
                settings.label
            )
        })
}

/// Shown to the client once, at initialize. Says what this server is a front for, because
/// "getRecipe" on its own does not tell an agent which API it is talking to.
fn instructions(config: &Config, apis: &[Api], tools: &[Tool]) -> String {
    let mut text = String::from(
        "Each tool is one operation of an OpenAPI document, called over HTTP. Parameters are \
         flat: path, query, header and cookie parameters are all top-level arguments, and a \
         request body is the `body` argument. Results carry the HTTP status alongside the \
         response.\n\nServing:\n",
    );
    for (settings, api) in config.apis.iter().zip(apis) {
        let base_url = settings.base_url.as_deref().unwrap_or_else(|| {
            api.servers
                .first()
                .map(String::as_str)
                .expect("a document without a base URL fails at startup")
        });
        text.push_str(&format!("- {} {} at {base_url}\n", api.title, api.version));
    }
    text.push_str(&format!("\n{} tools.", tools.len()));
    text
}

impl ServerHandler for OpenApiMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        info.instructions = Some(self.instructions.clone());
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // The whole set fits in one page: even a large spec is a few hundred tools, and paging
        // would only give clients a chance to stop halfway through.
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = request.arguments.unwrap_or_default();
        Ok(self.call(&request.name, arguments).await.into())
    }
}

/// A one-line-per-tool listing, for `openapi-as-mcp list`.
pub fn describe_tools(server: &OpenApiMcp) -> Value {
    let tools: Vec<Value> = server
        .tools()
        .iter()
        .map(|tool| {
            let operation = server.operation(&tool.name).expect("tool has an operation");
            json!({
                "name": tool.name,
                "base_url": server.base_url_of(&tool.name).unwrap_or(""),
                "method": operation.method,
                "path": operation.path,
                "read_only": operation.is_read_only(),
                "summary": tool.description.as_deref().unwrap_or("")
                    .lines().next().unwrap_or(""),
            })
        })
        .collect();

    json!({
        "base_urls": server.base_urls(),
        "count": tools.len(),
        "tools": tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigArgs;
    use crate::config::test_support::config_from;
    use crate::spec::parse;

    const SPEC: &str = r#"{
      "openapi": "3.0.0",
      "info": {"title": "Recipes", "version": "1"},
      "servers": [{"url": "https://api.example.com/v1"}],
      "paths": {
        "/recipes": {"get": {"operationId": "listRecipes"},
                     "post": {"operationId": "createRecipe"}},
        "/recipes/{id}": {
          "delete": {"operationId": "deleteRecipe", "parameters": [
            {"name": "id", "in": "path", "schema": {"type": "string"}}]},
          "put": {"operationId": "replaceRecipe", "parameters": [
            {"name": "id", "in": "path", "schema": {"type": "string"}}]}
        },
        "/admin/purge": {"post": {"operationId": "purge"}}
      }
    }"#;

    fn server(args: ConfigArgs) -> anyhow::Result<OpenApiMcp> {
        let config = ConfigArgs {
            specs: vec!["unused.json".into()],
            ..args
        }
        .resolve()?;
        OpenApiMcp::new(&config, vec![parse(SPEC, "test").expect("parses")])
    }

    fn base_url(server: &OpenApiMcp) -> String {
        server.base_urls().join(", ")
    }

    fn names(server: &OpenApiMcp) -> Vec<String> {
        let mut names: Vec<String> = server.tools().iter().map(|t| t.name.to_string()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_base_url_falls_back_to_the_documents_first_server() {
        let server = server(ConfigArgs::default()).expect("builds");
        assert_eq!(base_url(&server), "https://api.example.com/v1");
    }

    #[test]
    fn an_explicit_base_url_wins_over_the_document() {
        let server = server(ConfigArgs {
            base_url: Some("http://localhost:8080".into()),
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(base_url(&server), "http://localhost:8080");
    }

    #[test]
    fn a_document_without_servers_and_without_a_base_url_is_a_startup_error() {
        let config = ConfigArgs {
            specs: vec!["unused.json".into()],
            ..ConfigArgs::default()
        }
        .resolve()
        .expect("resolves");
        let api = parse(r#"{"paths": {"/a": {"get": {}}}}"#, "test").expect("parses");

        let err = OpenApiMcp::new(&config, vec![api]).expect_err("no base URL");
        let message = format!("{err:#}");
        assert!(message.contains("base_url"), "got: {message}");
        assert!(
            message.contains("unused.json"),
            "it must name the document: {message}"
        );
    }

    #[test]
    fn read_only_keeps_only_safe_methods() {
        let server = server(ConfigArgs {
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(names(&server), vec!["listRecipes"]);
    }

    #[test]
    fn exclude_matches_the_path_as_well_as_the_name() {
        let server = server(ConfigArgs {
            exclude: vec!["^/admin".into()],
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert!(!names(&server).contains(&"purge".to_string()));
        assert!(names(&server).contains(&"listRecipes".to_string()));
    }

    #[test]
    fn include_is_applied_before_exclude() {
        let server = server(ConfigArgs {
            include: vec!["Recipe".into()],
            exclude: vec!["delete".into()],
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(
            names(&server),
            vec!["createRecipe", "listRecipes", "replaceRecipe"]
        );
    }

    #[test]
    fn filtering_everything_out_is_a_startup_error_rather_than_an_empty_server() {
        let err = server(ConfigArgs {
            include: vec!["nothing-matches-this".into()],
            ..ConfigArgs::default()
        })
        .expect_err("empty tool set");
        assert!(err.to_string().contains("no operations"), "got: {err}");
    }

    #[test]
    fn a_prefix_is_applied_to_every_tool_name() {
        let server = server(ConfigArgs {
            tool_prefix: Some("recipes_".into()),
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(names(&server), vec!["recipes_listRecipes"]);
    }

    #[test]
    fn names_colliding_across_two_documents_are_disambiguated() {
        let config = ConfigArgs {
            specs: vec!["one.json".into(), "two.json".into()],
            ..ConfigArgs::default()
        }
        .resolve()
        .expect("resolves");
        let one = parse(SPEC, "one").expect("parses");
        let two = parse(SPEC, "two").expect("parses");

        let server = OpenApiMcp::new(&config, vec![one, two]).expect("builds");
        let names = names(&server);
        assert!(names.contains(&"listRecipes".to_string()));
        assert!(names.contains(&"listRecipes_2".to_string()));
        assert_eq!(server.tools().len(), 10);
    }

    #[test]
    fn each_document_keeps_its_own_base_url_and_credentials() {
        let file = r#"
            [[api]]
            name = "one"
            spec = "one.json"
            base_url = "https://one.example.com"
            token = "t-one"
            read_only = true

            [[api]]
            name = "two"
            spec = "two.json"
            base_url = "https://two.example.com/"
            headers = { "X-Api-Key" = "k-two" }
            tool_prefix = "two_"
        "#;
        let config = config_from(file);

        assert_eq!(config.apis[0].headers[0].1, "Bearer t-one");
        assert_eq!(
            config.apis[1].headers[0],
            ("X-Api-Key".into(), "k-two".into())
        );
        assert!(config.apis[0].read_only, "per-api read_only applies");
        assert!(
            !config.apis[1].read_only,
            "and does not leak to the next api"
        );

        let server = OpenApiMcp::new(
            &config,
            vec![
                parse(SPEC, "one").expect("parses"),
                parse(SPEC, "two").expect("parses"),
            ],
        )
        .expect("builds");

        assert_eq!(
            server.base_urls(),
            vec!["https://one.example.com", "https://two.example.com"]
        );
        assert_eq!(
            server.base_url_of("listRecipes"),
            Some("https://one.example.com")
        );
        assert_eq!(
            server.base_url_of("two_listRecipes"),
            Some("https://two.example.com"),
            "the second document is reached through its prefix, with no name collision"
        );
        // read_only kept one tool from the first document; the second contributes all five.
        assert_eq!(server.tools().len(), 6);
    }

    #[test]
    fn annotations_describe_what_each_method_does() {
        let server = server(ConfigArgs::default()).expect("builds");
        let annotations = |name: &str| {
            server
                .get_tool(name)
                .expect("tool exists")
                .annotations
                .expect("annotated")
        };

        assert_eq!(annotations("listRecipes").read_only_hint, Some(true));
        assert_eq!(annotations("deleteRecipe").read_only_hint, Some(false));
        assert_eq!(annotations("deleteRecipe").destructive_hint, Some(true));
        assert_eq!(
            annotations("createRecipe").destructive_hint,
            Some(false),
            "POST is additive"
        );
        assert_eq!(annotations("replaceRecipe").idempotent_hint, Some(true));
    }

    #[tokio::test]
    async fn calling_an_unknown_tool_lists_the_real_ones() {
        let server = server(ConfigArgs::default()).expect("builds");
        let result = server.call("nope", Map::new()).await;

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().expect("text").text.clone();
        assert!(text.contains("listRecipes"), "got: {text}");
    }

    #[test]
    fn the_listing_carries_the_request_line_for_every_tool() {
        let server = server(ConfigArgs {
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        let listing = describe_tools(&server);

        assert_eq!(listing["count"], 1);
        assert_eq!(listing["tools"][0]["method"], "GET");
        assert_eq!(listing["tools"][0]["path"], "/recipes");
        assert_eq!(
            listing["tools"][0]["base_url"], "https://api.example.com/v1",
            "each tool says where its calls go"
        );
    }
}
