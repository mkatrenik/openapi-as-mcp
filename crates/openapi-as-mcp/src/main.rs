//! `openapi-as-mcp` — turn any OpenAPI 3.x document into an MCP tool set.
//!
//! Point it at a spec and a base URL and every operation becomes a tool, named after its
//! `operationId`, with a JSON Schema built from its parameters and request body. Nothing about the
//! API is compiled in, so a new endpoint appears the moment the document does.
//!
//! `serve` runs the MCP server. Its transport is stdio, which means **stdout belongs to the
//! JSON-RPC protocol**: all diagnostics go to stderr, and a stray `println!` corrupts the stream.
//! Every other subcommand is an ordinary CLI run — see `src/cli.rs`, the only module allowed to
//! write there. A bare invocation prints help; the server must be asked for explicitly, so an MCP
//! client entry has to end in `… serve`.

mod cli;
mod config;
mod exec;
mod schema;
mod server;
mod spec;

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};

use crate::cli::Cli;
use crate::server::OpenApiMcp;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    // In CLI mode the default is quiet: the result is the output, and a failed call already prints
    // its message to stderr — logging it too would just print everything twice. OAM_LOG still
    // turns logging back on.
    let default_level = if cli.is_serve() { "info" } else { "error" };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("OAM_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level)),
        )
        .init();

    if !cli.is_serve() {
        return cli::run(cli).await;
    }

    let config = cli.config.resolve().context("invalid configuration")?;
    let apis = spec::load_all(&config.specs, config.timeout).await?;
    let server = OpenApiMcp::new(&config, apis)?;

    tracing::info!(
        base_url = %server.base_url(),
        tools = server.tools().len(),
        authenticated = !config.headers.is_empty(),
        read_only = config.read_only,
        "starting openapi-as-mcp"
    );

    let service = server
        .serve(stdio())
        .await
        .context("failed to start MCP server on stdio")?;

    service.waiting().await?;
    Ok(ExitCode::SUCCESS)
}
