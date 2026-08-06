//! `recipe-run-debug-mcp` — MCP server *and* CLI for debugging ingestion-platform recipe runs.
//!
//! `serve` runs the MCP server. Its transport is stdio, which means **stdout belongs to the
//! JSON-RPC protocol**: all diagnostics go to stderr, and a stray `println!` corrupts the stream.
//! Every other subcommand is an ordinary CLI run — one tool, its JSON printed to stdout — see
//! `src/cli.rs`, the only module allowed to write there. A bare invocation prints help; the server
//! must be asked for explicitly, so an MCP client entry has to end in `… serve`.

mod api;
mod cli;
mod config;
mod diff;
mod domain;
mod files;
mod fixtures;
mod http;
mod server;

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};

use crate::cli::Cli;
use crate::config::Config;
use crate::server::RecipeRunDebug;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    // In CLI mode the default is quiet: the result is the output, and a failed call already prints
    // its message to stderr — logging it at `warn` too would just print everything twice. RRD_LOG
    // still turns logging back on.
    let default_level = if cli.is_serve() { "info" } else { "error" };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RRD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level)),
        )
        .init();

    if !cli.is_serve() {
        return cli::run(cli).await;
    }

    let config = Config::load(&cli.config.overrides()).context("invalid configuration")?;
    let config_file = config
        .source_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    tracing::info!(
        env = %config.env,
        bucket = %config.bucket,
        gateway = %config.gateway_base_url,
        authenticated = config.auth.is_configured(),
        config_file = %config_file,
        "starting recipe-run-debug-mcp"
    );

    let service = RecipeRunDebug::new(config)?
        .serve(stdio())
        .await
        .context("failed to start MCP server on stdio")?;

    service.waiting().await?;
    Ok(ExitCode::SUCCESS)
}
