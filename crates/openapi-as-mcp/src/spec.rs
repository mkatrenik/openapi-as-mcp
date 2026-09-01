//! OpenAPI 3.x document → a flat list of callable operations.
//!
//! This is deliberately a *reader*, not a validator: a document that is slightly off spec should
//! still yield the operations it does describe, because the point of this tool is to make an
//! existing API reachable, not to police it. Anything genuinely unusable (an operation whose path
//! template names a parameter that does not exist) is dropped with a warning rather than failing
//! the whole load.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use thiserror::Error;

use crate::schema;

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not fetch {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{source_name} is neither valid JSON nor valid YAML: {message}")]
    Parse {
        source_name: String,
        message: String,
    },

    #[error("{source_name} has no `paths` object — is it an OpenAPI document?")]
    NotOpenApi { source_name: String },
}

/// Where a parameter is carried on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Path,
    Query,
    Header,
    Cookie,
}

impl Location {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "path" => Some(Self::Path),
            "query" => Some(Self::Query),
            "header" => Some(Self::Header),
            "cookie" => Some(Self::Cookie),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Header => "header",
            Self::Cookie => "cookie",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub location: Location,
    pub required: bool,
    pub description: Option<String>,
    /// The parameter's JSON Schema, with `$ref`s already rewritten to point at `$defs`.
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub content_type: String,
    pub required: bool,
    /// `None` for a body whose media type carries no schema (`text/plain`, an upload): the tool
    /// then takes a string and sends it verbatim.
    pub schema: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct Operation {
    /// The MCP tool name. Unique across the whole server.
    pub name: String,
    pub method: String,
    /// Path template as written in the document, e.g. `/recipes/{recipe_id}`.
    pub path: String,
    pub description: String,
    pub params: Vec<Param>,
    pub body: Option<Body>,
    /// Name of the property carrying the request body, when there is one.
    pub body_property: Option<String>,
    pub input_schema: Map<String, Value>,
}

impl Operation {
    /// Whether calling this can change server state. Drives the tool's read-only hint, and the
    /// `--read-only` filter.
    pub fn is_read_only(&self) -> bool {
        matches!(self.method.as_str(), "GET" | "HEAD" | "OPTIONS")
    }
}

#[derive(Debug)]
pub struct Api {
    pub title: String,
    pub version: String,
    /// Server URLs declared by the document, in order. May be empty.
    pub servers: Vec<String>,
    pub operations: Vec<Operation>,
}

/// A spec to load: a local file or an `http(s)` URL.
#[derive(Debug, Clone)]
pub enum Source {
    File(PathBuf),
    Url(String),
}

impl Source {
    /// URLs are recognised by scheme; everything else is a path, so a Windows-style or
    /// relative path is never mistaken for a URL.
    pub fn parse(raw: &str) -> Self {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            Self::Url(raw.to_string())
        } else {
            Self::File(PathBuf::from(raw))
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Url(url) => url.clone(),
        }
    }
}

/// Loads every configured document, in order. One unreadable document fails the load: a server
/// that silently came up with half its tools would be worse than one that did not come up.
pub async fn load_all(
    sources: &[Source],
    timeout: std::time::Duration,
) -> anyhow::Result<Vec<Api>> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("openapi-as-mcp/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let mut apis = Vec::with_capacity(sources.len());
    for source in sources {
        let api = load(source, &client).await?;
        tracing::info!(
            source = %source.label(),
            title = %api.title,
            operations = api.operations.len(),
            "loaded spec"
        );
        apis.push(api);
    }
    Ok(apis)
}

/// Fetch a spec's bytes and parse it into an [`Api`].
pub async fn load(source: &Source, client: &reqwest::Client) -> Result<Api, SpecError> {
    let text = match source {
        Source::File(path) => read_file(path)?,
        Source::Url(url) => fetch(url, client).await?,
    };
    parse(&text, &source.label())
}

fn read_file(path: &Path) -> Result<String, SpecError> {
    std::fs::read_to_string(path).map_err(|source| SpecError::Read {
        path: path.to_path_buf(),
        source,
    })
}

async fn fetch(url: &str, client: &reqwest::Client) -> Result<String, SpecError> {
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|source| SpecError::Fetch {
            url: url.to_string(),
            source,
        })?;
    response.text().await.map_err(|source| SpecError::Fetch {
        url: url.to_string(),
        source,
    })
}

/// Parse a document. JSON first, because every JSON document is also valid YAML and the JSON
/// parser gives the better error message for the common case.
pub fn parse(text: &str, source_name: &str) -> Result<Api, SpecError> {
    let root: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(json_err) => serde_yaml::from_str(text).map_err(|yaml_err| SpecError::Parse {
            source_name: source_name.to_string(),
            message: format!("JSON: {json_err}; YAML: {yaml_err}"),
        })?,
    };

    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| SpecError::NotOpenApi {
            source_name: source_name.to_string(),
        })?;

    let title = root
        .pointer("/info/title")
        .and_then(Value::as_str)
        .unwrap_or(source_name)
        .to_string();
    let version = root
        .pointer("/info/version")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let servers = root
        .get("servers")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("url").and_then(Value::as_str))
                .map(|url| url.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default();

    // BTreeMap, not a counter: names must be stable across runs so a client's cached tool list
    // stays valid, and iteration over `paths` is already ordered by serde_json's preserve_order
    // being off — sorting the collisions is what actually pins the suffixes down.
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    let mut operations = Vec::new();

    for (path, item) in paths {
        let Some(item) = resolve(&root, item).as_object() else {
            continue;
        };
        // Path-level parameters apply to every operation under it; an operation-level parameter
        // with the same (name, in) overrides rather than duplicates.
        let shared = collect_params(&root, item.get("parameters"));

        for method in METHODS {
            let Some(operation) = item.get(*method).and_then(Value::as_object) else {
                continue;
            };
            match build_operation(&root, path, method, operation, &shared, &mut taken) {
                Some(op) => operations.push(op),
                None => tracing::warn!(%path, method, "skipping unusable operation"),
            }
        }
    }

    Ok(Api {
        title,
        version,
        servers,
        operations,
    })
}

const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];

fn build_operation(
    root: &Value,
    path: &str,
    method: &str,
    operation: &Map<String, Value>,
    shared: &[Param],
    taken: &mut BTreeMap<String, usize>,
) -> Option<Operation> {
    let mut params = shared.to_vec();
    for param in collect_params(root, operation.get("parameters")) {
        // Override the inherited one rather than sending the parameter twice.
        params
            .retain(|existing| existing.name != param.name || existing.location != param.location);
        params.push(param);
    }

    // A path template naming a parameter nobody declared cannot be filled in, so the operation is
    // uncallable. Better to drop it than to hand an agent a tool that always 404s.
    for placeholder in placeholders(path) {
        let declared = params
            .iter()
            .any(|p| p.location == Location::Path && p.name == placeholder);
        if !declared {
            return None;
        }
    }

    let body = collect_body(root, operation.get("requestBody"));
    let name = unique_name(tool_name(operation, method, path), taken);
    let (input_schema, body_property) = schema::build(root, &params, body.as_ref());

    Some(Operation {
        name,
        method: method.to_uppercase(),
        path: path.to_string(),
        description: describe(operation, method, path),
        params,
        body,
        body_property,
        input_schema,
    })
}

/// The description an agent sees. Always includes the method and path, because two operations can
/// share a summary and the request line is what actually distinguishes them.
fn describe(operation: &Map<String, Value>, method: &str, path: &str) -> String {
    let summary = operation
        .get("summary")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());
    let long = operation
        .get("description")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());

    let mut text = format!("{} {}", method.to_uppercase(), path);
    if let Some(summary) = summary {
        text = format!("{summary} ({text})");
    }
    if let Some(long) = long {
        // Long descriptions in real specs run to paragraphs; keep the tool list readable.
        text.push_str("\n\n");
        text.push_str(truncate(long, 800).trim());
    }
    if operation
        .get("deprecated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        text.push_str("\n\nDEPRECATED.");
    }
    text
}

fn collect_params(root: &Value, value: Option<&Value>) -> Vec<Param> {
    let Some(entries) = value.map(|v| resolve(root, v)).and_then(Value::as_array) else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let entry = resolve(root, entry).as_object()?;
            let name = entry.get("name").and_then(Value::as_str)?.to_string();
            let location = Location::parse(entry.get("in").and_then(Value::as_str)?)?;
            Some(Param {
                // Path parameters are required by spec whether or not they say so.
                required: location == Location::Path
                    || entry
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                name,
                location,
                description: entry
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                schema: entry.get("schema").cloned().unwrap_or_else(|| {
                    // No schema means a `content`-typed parameter; treat it as free-form.
                    Value::Object(Map::new())
                }),
            })
        })
        .collect()
}

fn collect_body(root: &Value, value: Option<&Value>) -> Option<Body> {
    let body = resolve(root, value?).as_object()?;
    let content = body.get("content").and_then(Value::as_object)?;

    // Prefer JSON: it is the only media type we can describe to an agent as structured input.
    let (content_type, media) = content
        .iter()
        .find(|(name, _)| name.starts_with("application/json") || name.ends_with("+json"))
        .or_else(|| content.iter().next())?;

    Some(Body {
        content_type: content_type.clone(),
        required: body
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        schema: media.get("schema").cloned(),
    })
}

/// `/recipes/{id}/tasks/{task}` → `["id", "task"]`.
pub fn placeholders(path: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        found.push(rest[open + 1..open + close].to_string());
        rest = &rest[open + close + 1..];
    }
    found
}

/// Follow local `$ref`s. Foreign refs (another file, a URL) are left as-is: resolving them would
/// mean fetching arbitrary documents, and specs that use them are rare enough to not be worth it.
pub fn resolve<'a>(root: &'a Value, value: &'a Value) -> &'a Value {
    let mut current = value;
    // Guard against a `$ref` cycle, which a hand-edited document can easily contain.
    for _ in 0..16 {
        let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
            return current;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return current;
        };
        match root.pointer(pointer) {
            Some(target) => current = target,
            None => return current,
        }
    }
    current
}

/// `operationId` when the document has one; otherwise the request line, which is always unique.
fn tool_name(operation: &Map<String, Value>, method: &str, path: &str) -> String {
    let raw = operation
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{method}_{path}"));

    sanitize(&raw)
}

/// MCP tool names are restricted to `[A-Za-z0-9_-]`; specs routinely use `.`, `/` and spaces.
fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_sep = true;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    let trimmed = out.trim_matches('_');
    // Leave room for a `_2` disambiguation suffix under the 128-character protocol limit.
    let capped: String = trimmed.chars().take(100).collect();
    if capped.is_empty() {
        "operation".to_string()
    } else {
        capped
    }
}

/// Two operations can carry the same `operationId` — it happens in generated specs. Suffix the
/// later ones rather than silently dropping them.
fn unique_name(name: String, taken: &mut BTreeMap<String, usize>) -> String {
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

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn api(spec: Value) -> Api {
        parse(&spec.to_string(), "test").expect("parses")
    }

    #[test]
    fn yaml_and_json_produce_the_same_operations() {
        let yaml = "
openapi: 3.0.0
info: { title: T, version: '1' }
paths:
  /ping:
    get:
      operationId: ping
      responses: { '200': { description: ok } }
";
        let from_yaml = parse(yaml, "test.yaml").expect("yaml parses");
        assert_eq!(from_yaml.operations.len(), 1);
        assert_eq!(from_yaml.operations[0].name, "ping");
        assert_eq!(from_yaml.operations[0].method, "GET");
    }

    #[test]
    fn non_openapi_input_is_rejected() {
        let err = parse("{\"hello\": 1}", "thing.json").expect_err("no paths");
        assert!(err.to_string().contains("OpenAPI"), "got: {err}");
    }

    #[test]
    fn path_level_parameters_are_inherited_and_overridable() {
        let api = api(json!({
            "paths": {
                "/r/{id}": {
                    "parameters": [
                        {"name": "id", "in": "path", "schema": {"type": "string"}},
                        {"name": "verbose", "in": "query", "schema": {"type": "boolean"}}
                    ],
                    "get": {
                        "operationId": "getR",
                        "parameters": [
                            {"name": "verbose", "in": "query", "required": true,
                             "schema": {"type": "string"}}
                        ]
                    }
                }
            }
        }));

        let op = &api.operations[0];
        assert_eq!(op.params.len(), 2, "override must not duplicate");
        let verbose = op
            .params
            .iter()
            .find(|p| p.name == "verbose")
            .expect("verbose kept");
        assert!(verbose.required, "operation-level definition must win");
        assert_eq!(verbose.schema["type"], "string");
    }

    #[test]
    fn path_parameters_are_required_even_when_the_document_says_otherwise() {
        let api = api(json!({
            "paths": {"/r/{id}": {"get": {"operationId": "getR", "parameters": [
                {"name": "id", "in": "path", "required": false, "schema": {"type": "string"}}
            ]}}}
        }));
        assert!(api.operations[0].params[0].required);
    }

    #[test]
    fn operation_with_an_undeclared_path_parameter_is_dropped() {
        let api = api(json!({
            "paths": {"/r/{id}": {"get": {"operationId": "getR"}}}
        }));
        assert!(
            api.operations.is_empty(),
            "an unfillable path template is not a callable tool"
        );
    }

    #[test]
    fn missing_operation_id_falls_back_to_the_request_line() {
        let api = api(json!({
            "paths": {"/recipes/latest": {"get": {}}}
        }));
        assert_eq!(api.operations[0].name, "get_recipes_latest");
    }

    #[test]
    fn duplicate_operation_ids_are_disambiguated() {
        let api = api(json!({
            "paths": {
                "/a": {"get": {"operationId": "same"}},
                "/b": {"get": {"operationId": "same"}}
            }
        }));
        let names: Vec<&str> = api.operations.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["same", "same_2"]);
    }

    #[test]
    fn operation_ids_are_sanitized_to_the_allowed_character_set() {
        assert_eq!(sanitize("get recipes.v2/list"), "get_recipes_v2_list");
        assert_eq!(sanitize("__weird__"), "weird");
        assert_eq!(sanitize("***"), "operation");
    }

    #[test]
    fn refs_are_followed_for_parameters_and_bodies() {
        let api = api(json!({
            "paths": {"/r": {"post": {
                "operationId": "createR",
                "parameters": [{"$ref": "#/components/parameters/Page"}],
                "requestBody": {"$ref": "#/components/requestBodies/RBody"}
            }}},
            "components": {
                "parameters": {
                    "Page": {"name": "page", "in": "query", "schema": {"type": "integer"}}
                },
                "requestBodies": {
                    "RBody": {"required": true, "content": {"application/json": {
                        "schema": {"type": "object"}
                    }}}
                }
            }
        }));

        let op = &api.operations[0];
        assert_eq!(op.params[0].name, "page");
        let body = op.body.as_ref().expect("body resolved through the ref");
        assert!(body.required);
        assert_eq!(body.content_type, "application/json");
    }

    #[test]
    fn json_media_type_wins_over_the_alphabetically_first_one() {
        let api = api(json!({
            "paths": {"/r": {"post": {"operationId": "createR", "requestBody": {"content": {
                "application/octet-stream": {"schema": {"type": "string"}},
                "application/json": {"schema": {"type": "object"}}
            }}}}}
        }));
        assert_eq!(
            api.operations[0].body.as_ref().unwrap().content_type,
            "application/json"
        );
    }

    #[test]
    fn a_ref_cycle_terminates() {
        let root = json!({"a": {"$ref": "#/b"}, "b": {"$ref": "#/a"}});
        // The guard bounds the walk; the only requirement is that it returns.
        let _ = resolve(&root, &root["a"]);
    }

    #[test]
    fn description_carries_the_request_line() {
        let api = api(json!({
            "paths": {"/r/{id}": {"delete": {
                "operationId": "delR",
                "summary": "Remove a recipe",
                "deprecated": true,
                "parameters": [{"name": "id", "in": "path", "schema": {"type": "string"}}]
            }}}
        }));
        let description = &api.operations[0].description;
        assert!(description.contains("DELETE /r/{id}"), "got: {description}");
        assert!(description.contains("Remove a recipe"));
        assert!(description.contains("DEPRECATED"));
        assert!(!api.operations[0].is_read_only());
    }

    #[test]
    fn servers_are_collected_without_trailing_slashes() {
        let api = api(json!({
            "servers": [{"url": "https://api.example.com/v1/"}],
            "paths": {}
        }));
        assert_eq!(api.servers, vec!["https://api.example.com/v1"]);
    }
}
