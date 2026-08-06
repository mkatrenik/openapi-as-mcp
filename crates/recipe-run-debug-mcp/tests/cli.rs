//! End-to-end CLI test: runs the real binary as a one-shot command against a mock gateway.
//!
//! What this guards that the protocol test cannot: that a subcommand does not start the stdio
//! server, that stdout carries exactly the JSON result, that tool-level failures leave stdout empty
//! and exit non-zero, and that a config flag outranks the matching env var.

use std::process::Output;

use serde_json::Value;
use tokio::process::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{RECIPE_ID, failed_run, page, recipe};

/// Run the binary with `args`, pointed at `gateway`. `RRD_ENV` is deliberately set to staging so
/// tests can prove `--env` overrides it.
async fn run(gateway: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_recipe-run-debug-mcp"))
        .args(args)
        .env("RRD_GATEWAY_BASE_URL", gateway)
        .env("RRD_ENV", "staging")
        .env("RRD_LOG", "error")
        // Keep the child off the developer's own config file, whose token or gateway would
        // otherwise leak into this test.
        .env("RRD_CONFIG", "/nonexistent/recipe-run-debug/config.toml")
        .output()
        .await
        .expect("spawn the binary")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {:?}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

async fn gateway_with_failed_run() -> MockServer {
    let gateway = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/reciperun-aggregator/recipe-runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![failed_run()])))
        .mount(&gateway)
        .await;

    // Production selects the `-hp` aggregator variant, so the `--env production` test needs it too.
    Mock::given(method("GET"))
        .and(path("/api/reciperun-aggregator-hp/recipe-runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![failed_run()])))
        .mount(&gateway)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/api/recipe-store/recipes/{RECIPE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(recipe()))
        .mount(&gateway)
        .await;

    gateway
}

#[tokio::test]
async fn a_subcommand_prints_the_tool_envelope_and_exits_zero() {
    let gateway = gateway_with_failed_run().await;
    let output = run(&gateway.uri(), &["why", "run-1"]).await;

    assert!(
        output.status.success(),
        "exit {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let payload = stdout_json(&output);
    // The same envelope the MCP tool returns — one implementation, two front ends.
    assert_eq!(payload["env"], "staging");
    assert_eq!(payload["data"]["first_failed_task"]["name"], "validate");
    assert_eq!(payload["data"]["failed_on_validation_agent"], true);

    // Indented by default; --compact is the pipe-friendly form.
    assert!(String::from_utf8_lossy(&output.stdout).contains("\n  \""));
    let compact = run(&gateway.uri(), &["why", "run-1", "--compact"]).await;
    assert_eq!(
        String::from_utf8_lossy(&compact.stdout)
            .trim()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn a_config_flag_outranks_the_env_var() {
    let gateway = gateway_with_failed_run().await;
    // RRD_ENV=staging in the environment, --env production on the command line.
    let output = run(&gateway.uri(), &["links", "run-1", "--env", "production"]).await;

    assert!(output.status.success(), "{:?}", output.status.code());
    let payload = stdout_json(&output);
    assert_eq!(payload["env"], "production");
    assert_eq!(payload["bucket"], "ca-gcp-agent-platform-artefacts-prod");
}

#[tokio::test]
async fn a_tool_level_failure_goes_to_stderr_with_a_non_zero_exit() {
    let gateway = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/reciperun-aggregator/recipe-runs"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "detail": "Not authenticated"
        })))
        .mount(&gateway)
        .await;

    let output = run(&gateway.uri(), &["run", "run-1"]).await;

    assert!(!output.status.success(), "a 401 must not exit zero");
    assert!(
        output.stdout.is_empty(),
        "nothing may reach stdout on failure: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("RRD_TOKEN"), "stderr was {stderr:?}");
}

#[tokio::test]
async fn a_missing_config_flag_target_is_an_error() {
    let gateway = MockServer::start().await;
    let output = run(
        &gateway.uri(),
        &["--config-file", "/nonexistent/rrd/nope.toml", "recipes"],
    )
    .await;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nope.toml"), "stderr was {stderr:?}");
}

/// The server is opt-in. A bare invocation must not sit there holding stdin open waiting for
/// JSON-RPC — it must print help and exit, so `serve` is the only way into server mode.
#[tokio::test]
async fn a_bare_invocation_prints_help_instead_of_starting_the_server() {
    let gateway = MockServer::start().await;
    let output = run(&gateway.uri(), &[]).await;

    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing subcommand is a usage error"
    );
    assert!(
        output.stdout.is_empty(),
        "usage output belongs on stderr, so stdout stays parseable"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage:"), "stderr was {stderr:?}");
    assert!(
        stderr.contains("serve"),
        "help must name the server subcommand"
    );
}

/// Tool names remain usable as aliases, so anything documented for the MCP surface works verbatim.
#[tokio::test]
async fn tool_names_work_as_aliases() {
    let gateway = gateway_with_failed_run().await;
    let output = run(&gateway.uri(), &["explain-run-failure", "run-1"]).await;

    assert!(output.status.success(), "{:?}", output.status.code());
    assert_eq!(
        stdout_json(&output)["data"]["first_failed_task"]["name"],
        "validate"
    );
}
