//! `openapi-cli` — list, inspect and call the operations of any OpenAPI 3.x document from a shell.
//!
//! `list`, `schema` and `call` exist so a spec can be checked before an agent is pointed at it —
//! "which tools does this produce, what does one take, does it actually work" is three commands
//! rather than an MCP client session. The tools are built by `openapi-core`, the same crate behind
//! `openapi-as-mcp`, so what this prints is exactly what the MCP server serves and `call` goes
//! through the same request path.
//!
//! Unlike the MCP server, this binary owns stdout: the result is the output.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use openapi_core::config::ConfigArgs;
use openapi_core::spec;
use openapi_core::toolset::ToolSet;
use serde_json::{Map, Value};

#[derive(Debug, Parser)]
#[command(
    name = "openapi-cli",
    version,
    about = "List, inspect and call the operations of any OpenAPI 3.x document",
    // A bare invocation prints help rather than a one-line error: it is what someone who just
    // installed this types first.
    arg_required_else_help = true
)]
struct Cli {
    #[command(flatten)]
    config: ConfigArgs,

    #[command(flatten)]
    output: OutputArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Output")]
struct OutputArgs {
    /// Print single-line JSON instead of indented, for piping into jq.
    #[arg(long, global = true)]
    compact: bool,

    /// Log filter, in `tracing` directive form (`info`, `openapi_core=debug`). Diagnostics
    /// always go to stderr.
    #[arg(long, value_name = "FILTER", default_value = "error", global = true)]
    log: String,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List the tools the document produces, one entry per tool.
    List,

    /// Print one tool's input schema.
    Schema {
        /// Tool name, as shown by `list`.
        tool: String,
    },

    /// Call one tool and print the result.
    Call {
        /// Tool name, as shown by `list`.
        tool: String,

        /// Argument as `name=value`. The value is parsed as JSON when it parses, so
        /// `page=2` is a number and `name=pep` is the string "pep". Repeatable.
        #[arg(long = "arg", short = 'a', value_name = "NAME=VALUE")]
        args: Vec<String>,

        /// All arguments at once, as a JSON object. Merged under any --arg.
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    // The default is quiet: the result is the output, and a failed call already prints its
    // message to stderr — logging it too would just print everything twice.
    let filter = tracing_subscriber::EnvFilter::try_new(&cli.output.log)
        .context("--log is not a valid tracing filter")?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter)
        .init();

    run(cli).await
}

/// Runs one command and prints its result.
async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let config = cli.config.resolve().context("invalid configuration")?;
    let apis = spec::load_all(&config.specs(), config.timeout).await?;
    let tools = ToolSet::new(&config, apis)?;
    let compact = cli.output.compact;

    match cli.command {
        Command::List => emit(&tools.describe(), compact),

        Command::Schema { tool } => {
            let operation = tools
                .operation(&tool)
                .with_context(|| format!("no tool named {tool:?}; try `list`"))?;
            emit(&Value::Object(operation.input_schema.clone()), compact)
        }

        Command::Call { tool, args, json } => {
            let arguments = arguments(&args, json.as_deref())?;
            match tools.call(&tool, arguments).await {
                Ok(payload) => emit(&payload, compact),
                // A failed call is the *expected* failure mode here (a stale token, a 404), so it
                // goes to stderr as plain text with a non-zero exit rather than as JSON on stdout.
                Err(message) => {
                    eprintln!("{message}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
    }
}

/// Merges `--json` and `--arg`, with `--arg` winning so a single value can be overridden on top
/// of a stored object.
fn arguments(args: &[String], raw_json: Option<&str>) -> anyhow::Result<Map<String, Value>> {
    let mut arguments = match raw_json {
        Some(text) => serde_json::from_str::<Value>(text)
            .context("--json is not valid JSON")?
            .as_object()
            .cloned()
            .context("--json must be a JSON object")?,
        None => Map::new(),
    };

    for arg in args {
        let (name, value) = arg
            .split_once('=')
            .with_context(|| format!("argument {arg:?} is not in `name=value` form"))?;
        // Bare words are the common case and are not valid JSON, so a parse failure means "this
        // is a string", not "this is malformed".
        let parsed = serde_json::from_str::<Value>(value)
            .unwrap_or_else(|_| Value::String(value.to_string()));
        arguments.insert(name.to_string(), parsed);
    }

    Ok(arguments)
}

fn emit(payload: &Value, compact: bool) -> anyhow::Result<ExitCode> {
    let rendered = if compact {
        serde_json::to_string(payload)
    } else {
        serde_json::to_string_pretty(payload)
    }
    .context("serializing the result")?;

    let mut out = std::io::stdout().lock();
    writeln!(out, "{rendered}").context("writing to stdout")?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arg_values_are_json_when_they_parse_and_strings_otherwise() {
        let arguments = arguments(
            &[
                "page=2".into(),
                "name=pep".into(),
                "state=[\"failed\"]".into(),
                "verbose=true".into(),
            ],
            None,
        )
        .expect("parses");

        assert_eq!(arguments["page"], json!(2));
        assert_eq!(arguments["name"], json!("pep"));
        assert_eq!(arguments["state"], json!(["failed"]));
        assert_eq!(arguments["verbose"], json!(true));
    }

    #[test]
    fn arg_overrides_json() {
        let arguments =
            arguments(&["page=3".into()], Some(r#"{"page": 1, "size": 10}"#)).expect("parses");
        assert_eq!(arguments["page"], json!(3));
        assert_eq!(arguments["size"], json!(10));
    }

    #[test]
    fn a_value_containing_an_equals_sign_survives() {
        let arguments = arguments(&["filter=a=b".into()], None).expect("parses");
        assert_eq!(arguments["filter"], json!("a=b"));
    }

    #[test]
    fn a_json_array_is_rejected_as_the_argument_object() {
        let err = arguments(&[], Some("[1]")).expect_err("rejected");
        assert!(err.to_string().contains("object"), "got: {err}");
    }

    #[test]
    fn an_argument_without_an_equals_sign_names_itself() {
        let err = arguments(&["page".into()], None).expect_err("rejected");
        assert!(err.to_string().contains("page"), "got: {err}");
    }
}
