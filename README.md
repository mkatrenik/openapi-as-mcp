# ingestion-tools

Cargo workspace holding the tools for working with the ingestion platform. Each tool is its own
binary crate under [crates/](crates/), with shared dependency versions pinned once in the root
[Cargo.toml](Cargo.toml) (`[workspace.dependencies]`).

## Tools

| Crate | Binary | What it does |
| --- | --- | --- |
| [crates/openapi-as-mcp](crates/openapi-as-mcp/) | `openapi-as-mcp` | Serves any number of OpenAPI 3.x documents as one MCP tool set, one tool per operation, configured by flags or a TOML file — see its [README](crates/openapi-as-mcp/README.md) |

## Working in the workspace

```bash
cargo build --workspace
cargo test --workspace
cargo run -p openapi-as-mcp -- --help   # run one tool
cargo test -p openapi-as-mcp            # test one tool
```

Commands that a tool documents relative to its own directory (for example
`./specs/refresh.sh`) still run from that crate's directory.

## Adding a tool

```bash
cargo new --bin crates/<name>
```

`members = ["crates/*"]` picks it up with no root edit. Take dependencies from the workspace —
`serde = { workspace = true }` — and add any new shared version to `[workspace.dependencies]`
rather than to the crate, so tools cannot drift onto different versions of the same library.
