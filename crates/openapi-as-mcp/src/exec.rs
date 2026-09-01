//! Turning a validated argument object into an HTTP request, and the response into a tool result.

use reqwest::{Client, Method, StatusCode};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::config::ApiConfig;
use crate::spec::{Location, Operation, placeholders};

#[derive(Debug, Error)]
pub enum CallError {
    #[error("{0}")]
    InvalidArgument(String),

    #[error("could not reach {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{method} {url} returned HTTP {status}: {body}")]
    Upstream {
        method: String,
        url: String,
        status: StatusCode,
        body: String,
    },
}

#[derive(Debug)]
pub struct Executor {
    client: Client,
    base_url: String,
    headers: Vec<(String, String)>,
    max_response_bytes: usize,
}

impl Executor {
    /// One executor per document: the base URL, credentials and limits all come from that
    /// document's own [`ApiConfig`], so two specs in one server never share a token.
    pub fn new(api: &ApiConfig, base_url: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(api.timeout)
            .user_agent(concat!("openapi-as-mcp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            headers: api.headers.clone(),
            max_response_bytes: api.max_response_bytes,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn call(
        &self,
        operation: &Operation,
        arguments: &Map<String, Value>,
    ) -> Result<Value, CallError> {
        reject_unknown(operation, arguments)?;

        let url = format!("{}{}", self.base_url, self.fill_path(operation, arguments)?);
        let method = Method::from_bytes(operation.method.as_bytes()).map_err(|_| {
            CallError::InvalidArgument(format!("unsupported HTTP method {}", operation.method))
        })?;

        let mut request = self.client.request(method, &url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }

        let mut cookies = Vec::new();
        for param in &operation.params {
            let Some(value) = present(arguments, &param.name) else {
                if param.required && param.location != Location::Path {
                    return Err(CallError::InvalidArgument(format!(
                        "missing required {} parameter {:?}",
                        param.location.as_str(),
                        param.name
                    )));
                }
                continue;
            };

            match param.location {
                // Already substituted into the path.
                Location::Path => {}
                Location::Query => {
                    let pairs: Vec<(&str, String)> = expand(value)
                        .into_iter()
                        .map(|rendered| (param.name.as_str(), rendered))
                        .collect();
                    request = request.query(&pairs);
                }
                Location::Header => {
                    request = request.header(&param.name, render(value));
                }
                Location::Cookie => cookies.push(format!("{}={}", param.name, render(value))),
            }
        }
        if !cookies.is_empty() {
            request = request.header("Cookie", cookies.join("; "));
        }

        if let Some(property) = &operation.body_property
            && let Some(value) = present(arguments, property)
        {
            let content_type = operation
                .body
                .as_ref()
                .map(|body| body.content_type.as_str())
                .unwrap_or("application/json");
            request = if content_type.contains("json") {
                request.header("Content-Type", content_type).json(value)
            } else {
                // A non-JSON media type takes the string through untouched; anything else would be
                // guessing at an encoding the document did not describe.
                request
                    .header("Content-Type", content_type)
                    .body(render(value))
            };
        }

        tracing::debug!(%url, method = %operation.method, tool = %operation.name, "request");

        let response = request
            .send()
            .await
            .map_err(|source| CallError::Transport {
                url: url.clone(),
                source,
            })?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let raw = response
            .text()
            .await
            .map_err(|source| CallError::Transport {
                url: url.clone(),
                source,
            })?;
        let (body, truncated) = truncate(&raw, self.max_response_bytes);

        if !status.is_success() {
            return Err(CallError::Upstream {
                method: operation.method.clone(),
                url,
                status,
                body,
            });
        }

        let mut payload = json!({
            "status": status.as_u16(),
            "url": url,
            "content_type": content_type,
        });
        let payload_object = payload.as_object_mut().expect("built as an object");

        // Parse JSON when we can: a structured payload is far more useful to a model than the
        // same bytes as one long string. A truncated body is never valid JSON, so it stays text.
        match (truncated, serde_json::from_str::<Value>(&body)) {
            (false, Ok(value)) => {
                payload_object.insert("json".into(), value);
            }
            _ => {
                payload_object.insert("text".into(), Value::String(body));
                if truncated {
                    payload_object.insert("truncated".into(), Value::Bool(true));
                }
            }
        }

        Ok(payload)
    }

    /// Substitutes `{name}` placeholders. Path parameters are required, so a missing one is an
    /// argument error rather than a request to a literal `/recipes/{id}`.
    fn fill_path(
        &self,
        operation: &Operation,
        arguments: &Map<String, Value>,
    ) -> Result<String, CallError> {
        let mut path = operation.path.clone();
        for name in placeholders(&operation.path) {
            let value = present(arguments, &name).ok_or_else(|| {
                CallError::InvalidArgument(format!("missing required path parameter {name:?}"))
            })?;
            let rendered = render(value);
            if rendered.is_empty() {
                return Err(CallError::InvalidArgument(format!(
                    "path parameter {name:?} is empty, which would change the request path"
                )));
            }
            path = path.replace(&format!("{{{name}}}"), &urlencoding::encode(&rendered));
        }
        Ok(path)
    }
}

/// The schema is closed, so an argument nobody declared is a mistake worth naming — usually a
/// near-miss on a real parameter, which the model can fix if it is told.
fn reject_unknown(operation: &Operation, arguments: &Map<String, Value>) -> Result<(), CallError> {
    let Some(properties) = operation
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
    else {
        return Ok(());
    };

    let unknown: Vec<&str> = arguments
        .keys()
        .filter(|key| !properties.contains_key(*key))
        .map(String::as_str)
        .collect();

    if unknown.is_empty() {
        return Ok(());
    }

    let mut known: Vec<&str> = properties.keys().map(String::as_str).collect();
    known.sort_unstable();
    Err(CallError::InvalidArgument(format!(
        "unknown argument(s) {}; {} accepts {}",
        unknown.join(", "),
        operation.name,
        if known.is_empty() {
            "no arguments".to_string()
        } else {
            known.join(", ")
        }
    )))
}

/// An explicit `null` means "not supplied": clients emit it for optional fields, and sending
/// `?page=null` upstream is never what was meant.
fn present<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    arguments.get(name).filter(|value| !value.is_null())
}

/// One value as it goes on the wire. Strings pass through unquoted — `?state=failed`, not
/// `?state="failed"`.
fn render(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// An array query parameter becomes a repeated key (`?state=a&state=b`), which is the default
/// `form`/`explode: true` serialisation and what every framework we target expects.
fn expand(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().map(render).collect(),
        other => vec![render(other)],
    }
}

fn truncate(body: &str, max: usize) -> (String, bool) {
    if body.len() <= max {
        return (body.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    (body[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigArgs;
    use crate::spec::{Api, parse};
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SPEC: &str = r#"{
      "openapi": "3.0.0",
      "info": {"title": "T", "version": "1"},
      "paths": {
        "/recipes/{id}": {
          "get": {
            "operationId": "getRecipe",
            "parameters": [
              {"name": "id", "in": "path", "schema": {"type": "string"}},
              {"name": "state", "in": "query", "schema": {"type": "array",
                "items": {"type": "string"}}},
              {"name": "page", "in": "query", "schema": {"type": "integer"}},
              {"name": "X-Trace", "in": "header", "schema": {"type": "string"}},
              {"name": "sid", "in": "cookie", "schema": {"type": "string"}}
            ]
          }
        },
        "/recipes": {
          "post": {
            "operationId": "createRecipe",
            "requestBody": {"required": true, "content": {"application/json": {
              "schema": {"type": "object"}
            }}}
          }
        },
        "/raw": {
          "post": {
            "operationId": "postRaw",
            "requestBody": {"content": {"text/plain": {}}}
          }
        }
      }
    }"#;

    fn api() -> Api {
        parse(SPEC, "test").expect("parses")
    }

    fn operation(api: &Api, name: &str) -> Operation {
        api.operations
            .iter()
            .find(|op| op.name == name)
            .expect("operation exists")
            .clone()
    }

    fn api_config() -> ApiConfig {
        ConfigArgs::default().resolve_for_test().apis.remove(0)
    }

    fn executor(base: &str) -> Executor {
        Executor::new(&api_config(), base.to_string()).expect("client builds")
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object")
    }

    #[tokio::test]
    async fn every_parameter_location_reaches_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recipes/pep%20list"))
            .and(query_param("page", "2"))
            .and(header("X-Trace", "t-1"))
            .and(header("Cookie", "sid=s-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;

        let api = api();
        let payload = executor(&server.uri())
            .call(
                &operation(&api, "getRecipe"),
                &args(json!({
                    "id": "pep list",
                    "page": 2,
                    "X-Trace": "t-1",
                    "sid": "s-1"
                })),
            )
            .await
            .expect("call succeeds");

        assert_eq!(payload["status"], 200);
        assert_eq!(payload["json"]["ok"], true, "JSON responses are parsed");
    }

    #[tokio::test]
    async fn an_array_query_parameter_repeats_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recipes/x"))
            .and(query_param("state", "failed"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let api = api();
        let payload = executor(&server.uri())
            .call(
                &operation(&api, "getRecipe"),
                &args(json!({"id": "x", "state": ["failed", "done"]})),
            )
            .await
            .expect("call succeeds");
        assert_eq!(payload["json"], json!([]));
    }

    #[tokio::test]
    async fn a_json_body_is_sent_as_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/recipes"))
            .and(body_json(json!({"name": "pep"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "1"})))
            .mount(&server)
            .await;

        let api = api();
        let payload = executor(&server.uri())
            .call(
                &operation(&api, "createRecipe"),
                &args(json!({"body": {"name": "pep"}})),
            )
            .await
            .expect("call succeeds");
        assert_eq!(payload["status"], 201);
    }

    #[tokio::test]
    async fn a_non_json_body_is_sent_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/raw"))
            .and(header("Content-Type", "text/plain"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let api = api();
        let payload = executor(&server.uri())
            .call(&operation(&api, "postRaw"), &args(json!({"body": "a,b,c"})))
            .await
            .expect("call succeeds");
        assert_eq!(payload["text"], "ok", "a non-JSON response stays text");
    }

    #[tokio::test]
    async fn an_error_status_keeps_the_body_for_diagnosis() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recipes/x"))
            .respond_with(ResponseTemplate::new(404).set_body_string("no such recipe"))
            .mount(&server)
            .await;

        let api = api();
        let err = executor(&server.uri())
            .call(&operation(&api, "getRecipe"), &args(json!({"id": "x"})))
            .await
            .expect_err("404 is a failure");
        let message = err.to_string();
        assert!(message.contains("404"), "got: {message}");
        assert!(message.contains("no such recipe"), "got: {message}");
    }

    #[tokio::test]
    async fn an_oversized_response_is_truncated_and_says_so() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(5_000)))
            .mount(&server)
            .await;

        let mut api_config = api_config();
        api_config.max_response_bytes = 100;
        let executor = Executor::new(&api_config, server.uri()).expect("client builds");

        let api = api();
        let payload = executor
            .call(&operation(&api, "getRecipe"), &args(json!({"id": "x"})))
            .await
            .expect("call succeeds");
        assert_eq!(payload["truncated"], true);
        assert_eq!(payload["text"].as_str().unwrap().len(), 100);
    }

    #[tokio::test]
    async fn a_missing_path_parameter_is_an_argument_error() {
        let api = api();
        let err = executor("http://127.0.0.1:1")
            .call(&operation(&api, "getRecipe"), &args(json!({})))
            .await
            .expect_err("rejected before any request");
        assert!(err.to_string().contains("\"id\""), "got: {err}");
    }

    #[tokio::test]
    async fn an_unknown_argument_is_rejected_with_the_accepted_names() {
        let api = api();
        let err = executor("http://127.0.0.1:1")
            .call(
                &operation(&api, "getRecipe"),
                &args(json!({"id": "x", "pge": 2})),
            )
            .await
            .expect_err("rejected");
        let message = err.to_string();
        assert!(message.contains("pge"), "names the offender: {message}");
        assert!(message.contains("page"), "and the real one: {message}");
    }

    #[tokio::test]
    async fn an_explicit_null_is_treated_as_absent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recipes/x"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let api = api();
        // wiremock's mock has no query matcher, so a stray `?page=null` would still match; the
        // assertion that matters is that a null optional does not become a header value error.
        let payload = executor(&server.uri())
            .call(
                &operation(&api, "getRecipe"),
                &args(json!({"id": "x", "page": null, "X-Trace": null})),
            )
            .await
            .expect("call succeeds");
        assert_eq!(payload["status"], 200);
    }
}
