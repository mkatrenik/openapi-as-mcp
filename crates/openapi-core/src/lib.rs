//! The engine behind `openapi-mcp` and `openapi-cli`: turn OpenAPI 3.x documents into a set
//! of callable tools.
//!
//! Point it at a spec and a base URL and every operation becomes a tool, named after its
//! `operationId`, with a JSON Schema built from its parameters and request body. Nothing about the
//! API is compiled in, so a new endpoint appears the moment the document does.
//!
//! Several documents can be served at once, each with its own base URL, credentials and filters —
//! see [`config`] for the TOML file that describes them. [`toolset::ToolSet`] is the entry point
//! both front ends build on.
//!
//! Nothing in this crate writes to stdout: under `openapi-mcp serve` stdout is the JSON-RPC
//! stream. Diagnostics go through `tracing`, and the binary decides where they land.

pub mod config;
pub mod exec;
pub mod schema;
pub mod spec;
pub mod toolset;
