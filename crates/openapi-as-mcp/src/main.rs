//! `openapi-as-mcp` — serve any OpenAPI 3.x document as an MCP tool set.
//!
//! The document-to-tools work lives in `openapi-core`; this binary is only the MCP front end over
//! it. To inspect or call the same tools from a shell, use `openapi-cli`.
//!
//! `serve` runs the MCP server. Its transport is stdio, which means **stdout belongs to the
//! JSON-RPC protocol**: all diagnostics go to stderr, and a stray `println!` corrupts the stream
//! (`tests/no_stdout.rs` guards this). A bare invocation prints help; the server must be asked for
//! explicitly, so an MCP client entry has to end in `… serve`.
//!
//! `serve --daemon` shares one server process between sessions — see `src/daemon.rs`.

// Unix sockets and `flock` have no drop-in Windows equivalent, so shared-daemon mode is Unix-only.
#[cfg(unix)]
mod daemon;
mod server;

use std::io::IsTerminal;
use std::process::ExitCode;
#[cfg(unix)]
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use openapi_core::config::ConfigArgs;
use openapi_core::spec;
use rmcp::{ServiceExt, transport::stdio};

use crate::server::OpenApiMcp;

#[derive(Debug, Parser)]
#[command(
    name = "openapi-as-mcp",
    version,
    about = "Serve any OpenAPI 3.x document as an MCP tool set",
    long_about = "Serve any OpenAPI 3.x document as an MCP tool set.\n\n`serve` runs the MCP \
                  server on stdio. To list, inspect or call the same tools from a shell, use \
                  `openapi-cli`.",
    // A bare invocation prints help rather than starting a server that waits silently on stdin:
    // an MCP client entry that forgets `serve` then fails loudly instead of hanging.
    arg_required_else_help = true
)]
struct Cli {
    #[command(flatten)]
    config: ConfigArgs,

    /// Log filter, in `tracing` directive form (`info`, `openapi_core=debug`). Diagnostics always
    /// go to stderr.
    #[arg(long, value_name = "FILTER", default_value = "info", global = true)]
    log: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the MCP server on stdio. This is how an MCP client launches the binary.
    Serve(ServeArgs),
}

#[derive(Debug, Default, Args)]
struct ServeArgs {
    /// Share one server process between every session with the same configuration: this process
    /// relays stdio to it, starting it first when it is not running.
    #[arg(long)]
    daemon: bool,

    /// With --daemon: seconds the shared server stays up with no sessions before exiting. 0 keeps
    /// it running until killed.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 300,
        requires = "daemon"
    )]
    idle_timeout: u64,

    /// Internal: run as the shared server itself. Added by the relay when it starts one.
    #[arg(long = "daemon-host", hide = true, requires = "daemon")]
    daemon_host: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let filter = tracing_subscriber::EnvFilter::try_new(&cli.log)
        .context("--log is not a valid tracing filter")?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // Colour only for a person watching: the shared daemon's stderr is a log file.
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter)
        .init();

    let Command::Serve(serve) = &cli.command;

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
