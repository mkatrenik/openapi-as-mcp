# openapi-tools

Cargo workspace holding the tools for working with openapi described apis. Each tool is its own
crate under [crates/](crates/), with shared dependency versions pinned once in the root
[Cargo.toml](Cargo.toml) (`[workspace.dependencies]`).

## Tools

| Crate                                           | Binary           | What it does                                                                                                                                                                   |
| ----------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [crates/openapi-as-mcp](crates/openapi-as-mcp/) | `openapi-as-mcp` | Serves any number of OpenAPI 3.x documents as one MCP tool set, one tool per operation, configured by flags or a TOML file — see its [README](crates/openapi-as-mcp/README.md) |
| [crates/openapi-cli](crates/openapi-cli/)       | `openapi-cli`    | Lists, inspects and calls the same tools from a shell, with the same flags and config file — see its [README](crates/openapi-cli/README.md)                                    |
| [crates/openapi-core](crates/openapi-core/)     | (library)        | What both binaries share: config, spec loading, schema building, the HTTP request path and the tool set itself                                                                 |

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
