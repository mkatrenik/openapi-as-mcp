//! End-to-end protocol test: drives the real binary over stdio.
//!
//! This is the test that catches an accidental `println!` — anything written to stdout that is not
//! JSON-RPC breaks initialization, which no unit test can see.

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{RECIPE_ID, failed_run, page, recipe};

fn args(value: serde_json::Value) -> rmcp::model::JsonObject {
    value.as_object().cloned().expect("object arguments")
}

/// Spawn the built binary in server mode, pointed at a mock gateway.
async fn spawn(gateway: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_recipe-run-debug-mcp"));
    command
        // The server is opt-in: without `serve` the binary prints help and exits.
        .arg("serve")
        .env("RRD_GATEWAY_BASE_URL", gateway)
        .env("RRD_ENV", "development")
        .env("RRD_LOG", "warn")
        // Keep the child off the developer's own ~/.config/recipe-run-debug/config.toml: a stored
        // token or gateway there would otherwise leak into this test.
        .env("RRD_CONFIG", "/nonexistent/recipe-run-debug/config.toml");

    ().serve(TokioChildProcess::new(command).expect("spawn server"))
        .await
        .expect("initialize over stdio")
}

#[tokio::test]
async fn initializes_and_lists_every_tool() {
    let gateway = MockServer::start().await;
    let client = spawn(&gateway.uri()).await;

    let tools = client.list_all_tools().await.expect("tools/list");
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort();

    assert_eq!(
        names,
        vec![
            "compare_runs",
            "describe_run_file",
            "diff_task_config",
            "explain_run_failure",
            "find_recipe_runs",
            "find_recipes",
            "get_recipe_config",
            "get_recipe_run",
            "get_run_log_links",
            "get_task_run_config",
            "list_run_files",
            "read_run_file",
            "search_run_file",
        ]
    );

    // Every tool must carry a description and an object input schema, or agents cannot use them.
    for tool in &tools {
        assert!(
            tool.description.as_ref().is_some_and(|d| d.len() > 20),
            "{} needs a usable description",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type").and_then(|t| t.as_str()),
            Some("object"),
            "{} input schema must be an object",
            tool.name
        );
    }

    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn run_lookup_returns_the_enveloped_summary() {
    let gateway = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/reciperun-aggregator/recipe-runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![failed_run()])))
        .mount(&gateway)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/recipe-store/recipes/{RECIPE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(recipe()))
        .mount(&gateway)
        .await;

    let client = spawn(&gateway.uri()).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("explain_run_failure")
                .with_arguments(args(serde_json::json!({ "recipe_run_id": "run-1" }))),
        )
        .await
        .expect("call explain_run_failure");

    assert_ne!(
        result.is_error,
        Some(true),
        "tool reported an error: {result:?}"
    );

    let structured = result
        .structured_content
        .as_ref()
        .expect("structured content");
    let data = &structured["data"];

    // The envelope must always name the environment, so prod/dev findings can't be confused.
    assert_eq!(structured["env"], "development");
    assert_eq!(structured["bucket"], "ca-gcp-agent-platform-artefacts");

    // Sorted by start time, so the first failure is `validate`, not merely the first in the array.
    assert_eq!(data["first_failed_task"]["name"], "validate");
    assert_eq!(data["failed_on_validation_agent"], true);
    assert_eq!(
        data["first_failed_task"]["attempts"][0]["error_type"],
        "ValidationError"
    );
    assert!(
        data["verdict"]
            .as_str()
            .unwrap()
            .contains("validation-agent"),
        "verdict was {:?}",
        data["verdict"]
    );

    // Links are derived, not fetched.
    assert!(
        data["log_links"]["grafana_logs_url"]
            .as_str()
            .unwrap()
            .contains("run-1")
    );
    assert_eq!(data["log_links"]["kibana_source_id"], "S:XYZ99");

    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn gateway_401_becomes_an_actionable_tool_error() {
    let gateway = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/reciperun-aggregator/recipe-runs"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "detail": "Authentication required"
        })))
        .mount(&gateway)
        .await;

    let client = spawn(&gateway.uri()).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("get_recipe_run")
                .with_arguments(args(serde_json::json!({ "recipe_run_id": "run-1" }))),
        )
        .await
        .expect("the call itself must succeed — auth failure is a tool-level error");

    assert_eq!(result.is_error, Some(true));
    let text = format!("{:?}", result.content);
    assert!(
        text.contains("RRD_TOKEN"),
        "the agent must be told how to fix it, got: {text}"
    );

    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn unresolvable_file_locator_is_rejected_before_any_request() {
    let gateway = MockServer::start().await;
    let client = spawn(&gateway.uri()).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("read_run_file")
                .with_arguments(args(serde_json::json!({ "task_name": "extract" }))),
        )
        .await
        .expect("call read_run_file");

    assert_eq!(result.is_error, Some(true));
    let text = format!("{:?}", result.content);
    assert!(text.contains("gcs_uri"), "got: {text}");

    // No gateway request should have been made for an unresolvable locator.
    assert!(
        gateway
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );

    client.cancel().await.expect("shutdown");
}
