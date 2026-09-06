//! The CLI half of the binary, driven as a subprocess.
//!
//! `list`, `schema` and `call` are how a spec gets checked before an agent is pointed at it, so
//! they are worth testing the same way a person uses them: run the binary, read stdout.

use std::process::Output;

use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::matchers::{header, method, path, query_param};
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
        .args(["--log", "error"])
        // An empty `--config` means "no config file, and do not go looking": a file in the
        // developer's home directory must not change what these tests serve.
        .args(["--config", ""])
        .output()
        .await
        .expect("binary runs")
}

/// Runs the binary against a written config file, with no `--spec` of its own.
async fn run_with_config(config: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openapi-as-mcp"))
        .args(args)
        .arg("--config")
        .arg(config)
        .args(["--log", "error"])
        .output()
        .await
        .expect("binary runs")
}

/// A config file in a fresh temp directory, removed when the returned guard drops.
struct ConfigFile(std::path::PathBuf);

impl ConfigFile {
    fn write(name: &str, body: &str) -> Self {
        let path = std::env::temp_dir().join(format!("oam-cli-{}-{name}.toml", std::process::id()));
        std::fs::write(&path, body).expect("writes the config");
        Self(path)
    }
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
        .output()
        .await
        .expect("binary runs");

    assert!(!output.status.success());
    let help = String::from_utf8_lossy(&output.stderr);
    assert!(help.contains("serve"), "help must mention serve: {help}");
}

#[tokio::test]
async fn a_config_file_serves_two_documents_each_against_its_own_host() {
    // Two mock APIs standing in for two services: the point is that one server can call both, and
    // that each call goes to the right one with the right credentials.
    let one = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes"))
        .and(header("Authorization", "Bearer token-one"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"from": "one"})))
        .mount(&one)
        .await;

    let two = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/recipes"))
        .and(header("X-Api-Key", "key-two"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"from": "two"})))
        .mount(&two)
        .await;

    let config = ConfigFile::write(
        "two-apis",
        &format!(
            r#"
read_only = true

[[api]]
name = "one"
spec = "{spec}"
base_url = "{one}"
token = "token-one"

[[api]]
name = "two"
spec = "{spec}"
base_url = "{two}"
tool_prefix = "two_"
headers = {{ "X-Api-Key" = "key-two" }}
"#,
            spec = spec(),
            one = one.uri(),
            two = two.uri(),
        ),
    );

    let listing = stdout_json(&run_with_config(&config.0, &["list"]).await);
    // The fixture has two read-only operations, so the shared `read_only = true` leaves two tools
    // per document rather than all four.
    assert_eq!(listing["count"], 4, "got: {listing}");
    assert_eq!(
        listing["base_urls"],
        json!([one.uri(), two.uri()]),
        "got: {listing}"
    );

    let first = stdout_json(&run_with_config(&config.0, &["call", "listRecipes"]).await);
    assert_eq!(first["json"]["from"], "one");

    let second = stdout_json(&run_with_config(&config.0, &["call", "two_listRecipes"]).await);
    assert_eq!(
        second["json"]["from"], "two",
        "the prefixed tool is sent to the second host with its own header"
    );

    // --api narrows the same file to one document.
    let narrowed = stdout_json(&run_with_config(&config.0, &["list", "--api", "two"]).await);
    assert_eq!(narrowed["base_urls"], json!([two.uri()]));
}

#[tokio::test]
async fn a_config_file_that_does_not_exist_fails_loudly() {
    let output = run_with_config(std::path::Path::new("/nonexistent/oam.toml"), &["list"]).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"), "got: {stderr}");
}
