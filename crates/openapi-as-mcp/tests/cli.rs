//! The CLI half of the binary, driven as a subprocess.
//!
//! `list`, `schema` and `call` are how a spec gets checked before an agent is pointed at it, so
//! they are worth testing the same way a person uses them: run the binary, read stdout.

use std::process::Output;

use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn spec() -> String {
    format!(
        "{}/tests/fixtures/recipes.openapi.yaml",
        env!("CARGO_MANIFEST_DIR")
    )
}

async fn run(base_url: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openapi-as-mcp"))
        .args(args)
        .arg("--spec")
        .arg(spec())
        .arg("--base-url")
        .arg(base_url)
        .env("OAM_LOG", "error")
        .output()
        .await
        .expect("binary runs")
}

fn stdout_json(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&text).unwrap_or_else(|err| {
        panic!(
            "stdout is not JSON ({err}):\nstdout: {text}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[tokio::test]
async fn list_prints_every_tool_with_its_request_line() {
    let output = run("http://localhost:1", &["list"]).await;
    assert!(output.status.success());

    let listing = stdout_json(&output);
    assert_eq!(listing["count"], 4);
    let delete = listing["tools"]
        .as_array()
        .expect("array")
        .iter()
        .find(|tool| tool["name"] == "deleteRecipe")
        .expect("deleteRecipe listed");
    assert_eq!(delete["method"], "DELETE");
    assert_eq!(delete["path"], "/recipes/{recipe_id}");
    assert_eq!(delete["read_only"], false);
}

#[tokio::test]
async fn schema_prints_the_input_schema_of_one_tool() {
    let output = run("http://localhost:1", &["schema", "listRecipes"]).await;
    assert!(output.status.success());

    let schema = stdout_json(&output);
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["page"]["minimum"], 1);
    assert_eq!(schema["additionalProperties"], false);
}

#[tokio::test]
async fn schema_for_an_unknown_tool_fails_and_points_at_list() {
    let output = run("http://localhost:1", &["schema", "nope"]).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("list"), "got: {stderr}");
}

#[tokio::test]
async fn call_sends_the_request_and_prints_the_response() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes"))
        .and(query_param("page", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [1]})))
        .mount(&api)
        .await;

    let output = run(&api.uri(), &["call", "listRecipes", "-a", "page=3"]).await;
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload = stdout_json(&output);
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["json"]["items"], json!([1]));
}

#[tokio::test]
async fn a_failed_call_exits_non_zero_and_writes_to_stderr() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes/x"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&api)
        .await;

    let output = run(&api.uri(), &["call", "getRecipe", "-a", "recipe_id=x"]).await;

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "a failure must not print a result to stdout"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("boom"), "got: {stderr}");
}

#[tokio::test]
async fn a_bare_invocation_prints_help_rather_than_starting_the_server() {
    let output = Command::new(env!("CARGO_BIN_EXE_openapi-as-mcp"))
        .env("OAM_LOG", "error")
        .output()
        .await
        .expect("binary runs");

    assert!(!output.status.success());
    let help = String::from_utf8_lossy(&output.stderr);
    assert!(help.contains("serve"), "help must mention serve: {help}");
}
