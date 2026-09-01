//! CLI surface over the same tool set the MCP server exposes.
//!
//! `list`, `schema` and `call` exist so a spec can be checked from a shell before an agent is
//! pointed at it — "which tools does this produce, what does one take, does it actually work" is
//! three commands rather than an MCP client session. `call` goes through
//! [`OpenApiMcp::call`](crate::server::OpenApiMcp::call), so there is no second request path.
//!
//! Unlike the rest of `src/`, this module owns stdout: in CLI mode there is no JSON-RPC stream to
//! corrupt. `tests/no_stdout.rs` exempts this file for that reason.

use std::io::Write;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use serde_json::{Map, Value, json};

use crate::config::ConfigArgs;
use crate::server::{OpenApiMcp, describe_tools};
use crate::spec;

#[derive(Debug, Parser)]
#[command(
    name = "openapi-as-mcp",
    version,
    about = "Serve any OpenAPI 3.x document as an MCP tool set",
    long_about = "Serve any OpenAPI 3.x document as an MCP tool set.\n\n`serve` runs the MCP \
                  server on stdio; the other subcommands inspect and exercise the same tools \
                  from a shell.",
    after_help = "Every flag has an OAM_* environment fallback, for MCP client entries that can \
                  only set `env`.",
    // A bare invocation prints help rather than a one-line error: it is what someone who just
    // installed this types first.
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub config: ConfigArgs,

    #[command(flatten)]
    pub output: OutputArgs,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Whether this invocation is the stdio MCP server rather than a one-shot command.
    pub fn is_serve(&self) -> bool {
        matches!(self.command, Command::Serve)
    }
}

#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Output")]
pub struct OutputArgs {
    /// Print single-line JSON instead of indented, for piping into jq.
    #[arg(long, global = true)]
    compact: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the MCP server on stdio. This is how an MCP client launches the binary.
    Serve,

    /// List the tools the document produces, one line of JSON each.
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

/// Runs one command and prints its result. `Serve` never reaches here — `main` handles it.
pub async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let config = cli.config.resolve().context("invalid configuration")?;
    let apis = spec::load_all(&config.specs, config.timeout).await?;
    let server = OpenApiMcp::new(&config, apis)?;
    let compact = cli.output.compact;

    match cli.command {
        Command::Serve => unreachable!("serve is dispatched by main"),

        Command::List => emit(&describe_tools(&server), compact),

        Command::Schema { tool } => {
            let operation = server
                .operation(&tool)
                .with_context(|| format!("no tool named {tool:?}; try `list`"))?;
            emit(&Value::Object(operation.input_schema.clone()), compact)
        }

        Command::Call { tool, args, json } => {
            let arguments = arguments(&args, json.as_deref())?;
            let result = server.call(&tool, arguments).await;

            // A failed call is the *expected* failure mode here (a stale token, a 404), so it goes
            // to stderr as plain text with a non-zero exit rather than as JSON on stdout.
            if result.is_error == Some(true) {
                let text: Vec<&str> = result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text())
                    .map(|block| block.text.as_str())
                    .collect();
                eprintln!("{}", text.join("\n"));
                return Ok(ExitCode::FAILURE);
            }

            match &result.structured_content {
                Some(payload) => emit(payload, compact),
                None => emit(&json!({"result": "ok"}), compact),
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
