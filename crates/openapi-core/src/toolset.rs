//! The tool set: one tool per OpenAPI operation, across every configured document.
//!
//! This is the part both front ends share — `openapi-mcp` serves it over MCP, `openapi-cli`
//! drives it from a shell — so naming, filtering and the request path cannot drift between them.
//! It deliberately knows nothing about MCP: a front end turns [`ToolSet::operations`] into
//! whatever its protocol calls a tool.

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};

use anyhow::{Context, bail};
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
pub struct ToolSet {
    /// Parallel to the documents, in config order.
    executors: Vec<Executor>,
    /// Tool names in serving order: config order, then document order. `operations` is the
    /// lookup; this keeps listings stable.
    order: Vec<String>,
    operations: HashMap<String, Bound>,
    instructions: String,
}

impl ToolSet {
    /// Builds the tool set from already-loaded documents, one per `config.apis` in the same
    /// order. Fails only for things that make the whole set useless: a document with no base URL
    /// to send its requests to, or no operations left after filtering.
    pub fn new(config: &Config, apis: Vec<Api>) -> anyhow::Result<Self> {
        assert_eq!(
            config.apis.len(),
            apis.len(),
            "one loaded document per configured api"
        );

        // Names are unique per document; two documents can still collide, so dedupe again here.
        let mut taken: BTreeMap<String, usize> = BTreeMap::new();
        let mut operations = HashMap::new();
        let mut order = Vec::new();
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
                operation.name = capped(unique(prefixed(settings, &operation.name), &mut taken));
                order.push(operation.name.clone());
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
                authenticated = !settings.headers.is_empty() || settings.token_command.is_some(),
                "serving document"
            );
            executors.push(Executor::new(settings, base_url)?);
        }

        if order.is_empty() {
            bail!(
                "no operations to serve: {} found, all filtered out by read-only/include/exclude",
                skipped
            );
        }
        tracing::info!(
            tools = order.len(),
            skipped,
            apis = apis.len(),
            "built tool set"
        );

        Ok(Self {
            instructions: instructions(config, &apis, order.len()),
            executors,
            order,
            operations,
        })
    }

    /// Every tool's operation, in serving order. `Operation::name` is the final tool name, with
    /// prefixes, collision suffixes and length capping already applied.
    pub fn operations(&self) -> impl Iterator<Item = &Operation> {
        self.order
            .iter()
            .map(|name| &self.operations[name].operation)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
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

    /// A paragraph saying what this tool set is a front for, because "getRecipe" on its own does
    /// not tell an agent which API it is talking to. The MCP server sends it at initialize.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Runs one tool and returns the response payload.
    ///
    /// `Err` is a ready-to-show message, not a typed error: everything that can go wrong here —
    /// an unknown name, a 404, a stale token — is something the caller (a model or a person)
    /// fixes by reading it, so both front ends just display it.
    pub async fn call(&self, name: &str, arguments: Map<String, Value>) -> Result<Value, String> {
        let Some(bound) = self.operations.get(name) else {
            let mut known: Vec<&str> = self.order.iter().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "unknown tool {name:?}. Available: {}",
                known.join(", ")
            ));
        };

        self.executors[bound.api]
            .call(&bound.operation, &arguments)
            .await
            .map_err(|err| {
                tracing::warn!(tool = name, error = %err, "tool call failed");
                describe_error(&err)
            })
    }

    /// A one-entry-per-tool listing, for `openapi-cli list`.
    pub fn describe(&self) -> Value {
        let tools: Vec<Value> = self
            .operations()
            .map(|operation| {
                json!({
                    "name": operation.name,
                    "base_url": self.base_url_of(&operation.name).unwrap_or(""),
                    "method": operation.method,
                    "path": operation.path,
                    "read_only": operation.is_read_only(),
                    "summary": operation.description.lines().next().unwrap_or(""),
                })
            })
            .collect();

        json!({
            "base_urls": self.base_urls(),
            "count": tools.len(),
            "tools": tools,
        })
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
                "The API rejected the credentials. Pass --token <TOKEN> (restarting the server if \
                 this is MCP), or fix this API's `token` in the config file.",
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

/// The longest tool name served. MCP itself allows 128 characters, but clients namespace tools
/// before handing them to a model — Claude Code sends `mcp__<server>__<tool>`, and the Anthropic
/// API rejects names over 64 characters. 38 leaves room for a server name of up to 19 characters.
const MAX_TOOL_NAME_LEN: usize = 38;

/// Shortens a name over [`MAX_TOOL_NAME_LEN`] to a readable head plus a hash of the full name, so
/// two long names that share a head still come out distinct. Names are ASCII (see
/// `spec::sanitize`), so byte slicing cannot split a character.
fn capped(name: String) -> String {
    if name.len() <= MAX_TOOL_NAME_LEN {
        return name;
    }
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    // 6 hex digits of hash plus the `_` separator.
    let hash = format!("{:06x}", hasher.finish() & 0xff_ffff);
    let head = name[..MAX_TOOL_NAME_LEN - hash.len() - 1].trim_end_matches(['_', '-']);
    format!("{head}_{hash}")
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

fn instructions(config: &Config, apis: &[Api], tool_count: usize) -> String {
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
    text.push_str(&format!("\n{tool_count} tools."));
    text
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

    fn tool_set(args: ConfigArgs) -> anyhow::Result<ToolSet> {
        let config = ConfigArgs {
            specs: vec!["unused.json".into()],
            ..args
        }
        .resolve()?;
        ToolSet::new(&config, vec![parse(SPEC, "test").expect("parses")])
    }

    fn base_url(tools: &ToolSet) -> String {
        tools.base_urls().join(", ")
    }

    fn names(tools: &ToolSet) -> Vec<String> {
        let mut names: Vec<String> = tools.operations().map(|op| op.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_base_url_falls_back_to_the_documents_first_server() {
        let tools = tool_set(ConfigArgs::default()).expect("builds");
        assert_eq!(base_url(&tools), "https://api.example.com/v1");
    }

    #[test]
    fn an_explicit_base_url_wins_over_the_document() {
        let tools = tool_set(ConfigArgs {
            base_url: Some("http://localhost:8080".into()),
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(base_url(&tools), "http://localhost:8080");
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

        let err = ToolSet::new(&config, vec![api]).expect_err("no base URL");
        let message = format!("{err:#}");
        assert!(message.contains("base_url"), "got: {message}");
        assert!(
            message.contains("unused.json"),
            "it must name the document: {message}"
        );
    }

    #[test]
    fn read_only_keeps_only_safe_methods() {
        let tools = tool_set(ConfigArgs {
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(names(&tools), vec!["listRecipes"]);
    }

    #[test]
    fn exclude_matches_the_path_as_well_as_the_name() {
        let tools = tool_set(ConfigArgs {
            exclude: vec!["^/admin".into()],
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert!(!names(&tools).contains(&"purge".to_string()));
        assert!(names(&tools).contains(&"listRecipes".to_string()));
    }

    #[test]
    fn include_is_applied_before_exclude() {
        let tools = tool_set(ConfigArgs {
            include: vec!["Recipe".into()],
            exclude: vec!["delete".into()],
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(
            names(&tools),
            vec!["createRecipe", "listRecipes", "replaceRecipe"]
        );
    }

    #[test]
    fn filtering_everything_out_is_a_startup_error_rather_than_an_empty_set() {
        let err = tool_set(ConfigArgs {
            include: vec!["nothing-matches-this".into()],
            ..ConfigArgs::default()
        })
        .expect_err("empty tool set");
        assert!(err.to_string().contains("no operations"), "got: {err}");
    }

    #[test]
    fn a_prefix_is_applied_to_every_tool_name() {
        let tools = tool_set(ConfigArgs {
            tool_prefix: Some("recipes_".into()),
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        assert_eq!(names(&tools), vec!["recipes_listRecipes"]);
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

        let tools = ToolSet::new(&config, vec![one, two]).expect("builds");
        let names = names(&tools);
        assert!(names.contains(&"listRecipes".to_string()));
        assert!(names.contains(&"listRecipes_2".to_string()));
        assert_eq!(tools.len(), 10);
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

        let tools = ToolSet::new(
            &config,
            vec![
                parse(SPEC, "one").expect("parses"),
                parse(SPEC, "two").expect("parses"),
            ],
        )
        .expect("builds");

        assert_eq!(
            tools.base_urls(),
            vec!["https://one.example.com", "https://two.example.com"]
        );
        assert_eq!(
            tools.base_url_of("listRecipes"),
            Some("https://one.example.com")
        );
        assert_eq!(
            tools.base_url_of("two_listRecipes"),
            Some("https://two.example.com"),
            "the second document is reached through its prefix, with no name collision"
        );
        // read_only kept one tool from the first document; the second contributes all five.
        assert_eq!(tools.len(), 6);
    }

    #[test]
    fn long_names_are_capped_and_stay_distinct() {
        let short = "prod_list_monitors".to_string();
        assert_eq!(capped(short.clone()), short);

        let one = capped("prod_get_monitoring_provider_monitoring_providers_id_get".into());
        let two = capped("prod_get_monitoring_provider_monitoring_providers_id_put".into());
        assert_eq!(one.len(), MAX_TOOL_NAME_LEN, "got: {one}");
        assert!(
            one.starts_with("prod_get_monitoring_provider"),
            "got: {one}"
        );
        assert_ne!(one, two, "a shared head must not merge two tools");
    }

    #[tokio::test]
    async fn calling_an_unknown_tool_lists_the_real_ones() {
        let tools = tool_set(ConfigArgs::default()).expect("builds");
        let err = tools.call("nope", Map::new()).await.expect_err("unknown");
        assert!(err.contains("listRecipes"), "got: {err}");
    }

    #[test]
    fn the_listing_carries_the_request_line_for_every_tool() {
        let tools = tool_set(ConfigArgs {
            read_only: true,
            ..ConfigArgs::default()
        })
        .expect("builds");
        let listing = tools.describe();

        assert_eq!(listing["count"], 1);
        assert_eq!(listing["tools"][0]["method"], "GET");
        assert_eq!(listing["tools"][0]["path"], "/recipes");
        assert_eq!(
            listing["tools"][0]["base_url"], "https://api.example.com/v1",
            "each tool says where its calls go"
        );
    }
}
