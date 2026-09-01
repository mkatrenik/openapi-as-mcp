//! End-to-end protocol test: drives the real binary over stdio against a mock API.
//!
//! This is the test that catches an accidental `println!` — anything written to stdout that is not
//! JSON-RPC breaks initialization, which no unit test can see.

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn spec() -> String {
    format!(
        "{}/tests/fixtures/recipes.openapi.yaml",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn args(value: Value) -> rmcp::model::JsonObject {
    value.as_object().cloned().expect("object arguments")
}

/// Spawn the built binary in server mode, pointed at a mock API.
async fn spawn(base_url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openapi-as-mcp"));
    command
        // The server is opt-in: without `serve` the binary prints help and exits.
        .arg("serve")
        .env("OAM_SPEC", spec())
        .env("OAM_BASE_URL", base_url)
        .env("OAM_LOG", "warn")
        // Empty means "no config file, and do not go looking": a file in the developer's home
        // directory must not change what the test serves.
        .env("OAM_CONFIG", "")
        // Clear the rest of the OAM_* surface: a developer's own shell must not leak a token or a
        // filter into the test's tool set.
        .env_remove("OAM_TOKEN")
        .env_remove("OAM_HEADERS")
        .env_remove("OAM_READ_ONLY")
        .env_remove("OAM_INCLUDE")
        .env_remove("OAM_EXCLUDE")
        .env_remove("OAM_TOOL_PREFIX")
        .env_remove("OAM_API");

    ().serve(TokioChildProcess::new(command).expect("spawn server"))
        .await
        .expect("initialize over stdio")
}

#[tokio::test]
async fn initializes_and_lists_one_tool_per_operation() {
    let api = MockServer::start().await;
    let client = spawn(&api.uri()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["createRecipe", "deleteRecipe", "getRecipe", "listRecipes"]
    );

    for tool in &tools {
        assert!(
            tool.description.as_ref().is_some_and(|d| !d.is_empty()),
            "{} needs a description",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{} input schema must be an object",
            tool.name
        );
    }

    client.cancel().await.expect("shut down");
}

#[tokio::test]
async fn the_schema_reaches_the_client_with_refs_and_parameters_intact() {
    let api = MockServer::start().await;
    let client = spawn(&api.uri()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    let create = tools
        .iter()
        .find(|t| t.name == "createRecipe")
        .expect("createRecipe exists");

    let schema = Value::Object((*create.input_schema).clone());
    assert_eq!(schema["properties"]["body"]["$ref"], "#/$defs/Recipe");
    assert_eq!(
        schema["$defs"]["Recipe"]["properties"]["owner"]["$ref"],
        "#/$defs/User"
    );
    assert_eq!(
        schema["$defs"]["User"]["properties"]["email"]["type"],
        json!(["string", "null"]),
        "OpenAPI 3.0 nullable must reach the client as a JSON Schema type union"
    );
    assert_eq!(schema["required"], json!(["body"]));

    let get = tools
        .iter()
        .find(|t| t.name == "getRecipe")
        .expect("getRecipe exists");
    let schema = Value::Object((*get.input_schema).clone());
    assert_eq!(
        schema["required"],
        json!(["recipe_id"]),
        "an inherited path parameter is still required"
    );
    assert!(schema["properties"]["X-Trace"].is_object());

    client.cancel().await.expect("shut down");
}

#[tokio::test]
async fn a_tool_call_becomes_the_http_request_the_document_describes() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes"))
        .and(query_param("page", "2"))
        .and(query_param("state", "failed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
        .mount(&api)
        .await;

    let client = spawn(&api.uri()).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("listRecipes")
                .with_arguments(args(json!({"page": 2, "state": ["failed"]}))),
        )
        .await
        .expect("call_tool");

    assert_ne!(result.is_error, Some(true), "call should succeed");
    let payload = result.structured_content.expect("structured result");
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["json"]["items"], json!([]));

    client.cancel().await.expect("shut down");
}

#[tokio::test]
async fn a_request_body_is_sent_as_json() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/recipes"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({"name": "pep"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "r-1"})))
        .mount(&api)
        .await;

    let client = spawn(&api.uri()).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("createRecipe")
                .with_arguments(args(json!({"body": {"name": "pep"}}))),
        )
        .await
        .expect("call_tool");

    let payload = result.structured_content.expect("structured result");
    assert_eq!(payload["status"], 201);
    assert_eq!(payload["json"]["id"], "r-1");

    client.cancel().await.expect("shut down");
}

#[tokio::test]
async fn an_upstream_failure_is_a_tool_error_the_model_can_read() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such recipe"))
        .mount(&api)
        .await;

    let client = spawn(&api.uri()).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("getRecipe")
                .with_arguments(args(json!({"recipe_id": "missing"}))),
        )
        .await
        .expect("the protocol call itself succeeds");

    assert_eq!(result.is_error, Some(true));
    let text = result.content[0]
        .as_text()
        .expect("text block")
        .text
        .clone();
    assert!(text.contains("404"), "got: {text}");
    assert!(text.contains("no such recipe"), "got: {text}");

    client.cancel().await.expect("shut down");
}

#[tokio::test]
async fn read_only_mode_hides_the_write_operations() {
    let api = MockServer::start().await;
    let mut command = Command::new(env!("CARGO_BIN_EXE_openapi-as-mcp"));
    command
        .arg("serve")
        .arg("--read-only")
        .env("OAM_SPEC", spec())
        .env("OAM_BASE_URL", api.uri())
        .env("OAM_LOG", "warn")
        // Empty means "no config file, and do not go looking": a file in the developer's home
        // directory must not change what the test serves.
        .env("OAM_CONFIG", "");

    let client =
        ().serve(TokioChildProcess::new(command).expect("spawn"))
            .await
            .expect("initialize");

    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .expect("tools/list")
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["getRecipe", "listRecipes"]);

    client.cancel().await.expect("shut down");
}
