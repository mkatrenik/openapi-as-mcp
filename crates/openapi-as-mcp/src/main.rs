//! `openapi-as-mcp` — turn any OpenAPI 3.x document into an MCP tool set.
//!
//! Point it at a spec and a base URL and every operation becomes a tool, named after its
//! `operationId`, with a JSON Schema built from its parameters and request body. Nothing about the
//! API is compiled in, so a new endpoint appears the moment the document does.
//!
//! Several documents can be served at once, each with its own base URL, credentials and filters —
//! see `src/config.rs` for the TOML file that describes them.
//!
//! `serve` runs the MCP server. Its transport is stdio, which means **stdout belongs to the
//! JSON-RPC protocol**: all diagnostics go to stderr, and a stray `println!` corrupts the stream.
//! Every other subcommand is an ordinary CLI run — see `src/cli.rs`, the only module allowed to
//! write there. A bare invocation prints help; the server must be asked for explicitly, so an MCP
//! client entry has to end in `… serve`.
//!
//! `serve --daemon` shares one server process between sessions — see `src/daemon.rs`.

mod cli;
mod config;
// Unix sockets and `flock` have no drop-in Windows equivalent, so shared-daemon mode is Unix-only.
#[cfg(unix)]
mod daemon;
mod exec;
mod schema;
mod server;
mod spec;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};

use crate::cli::{Cli, Command};
use crate::server::OpenApiMcp;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    // In CLI mode the default is quiet: the result is the output, and a failed call already prints
    // its message to stderr — logging it too would just print everything twice. `--log` turns
    // logging back on.
    let default_level = if cli.is_serve() { "info" } else { "error" };
    let filter = match cli.output.log.as_deref() {
        Some(directives) => tracing_subscriber::EnvFilter::try_new(directives)
            .context("--log is not a valid tracing filter")?,
        None => tracing_subscriber::EnvFilter::new(default_level),
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // Colour only for a person watching: the shared daemon's stderr is a log file.
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter)
        .init();

    let Command::Serve(serve) = &cli.command else {
        return cli::run(cli).await;
    };

    let config = cli.config.resolve().context("invalid configuration")?;
    #[cfg(unix)]
    {
        if serve.daemon_host {
            return daemon::host(config, Duration::from_secs(serve.idle_timeout)).await;
        }
        if serve.daemon {
            return daemon::relay(&config).await;
        }
    }
    #[cfg(not(unix))]
    if serve.daemon {
        anyhow::bail!("--daemon is only supported on Unix");
    }

    let apis = spec::load_all(&config.specs(), config.timeout).await?;
    let server = OpenApiMcp::new(&config, apis)?;

    tracing::info!(
        documents = config.apis.len(),
        base_urls = ?server.base_urls(),
        tools = server.tools().len(),
        "starting openapi-as-mcp"
    );

    let service = server
        .serve(stdio())
        .await
        .context("failed to start MCP server on stdio")?;

    service.waiting().await?;
    Ok(ExitCode::SUCCESS)
}
