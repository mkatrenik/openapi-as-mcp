# openapi-cli

List, inspect and call the operations of any OpenAPI 3.x document from a shell. It builds the same
tool set `openapi-mcp` serves (both sit on `openapi-core`), takes the same flags and reads the
same config file, so it is how a spec gets checked before an agent is pointed at it.

```bash
cargo install --path crates/openapi-cli

openapi-cli list --spec ./openapi.yaml --base-url https://api.example.com
openapi-cli schema listRecipes --spec ./openapi.yaml
openapi-cli call listRecipes -a page=2 -a 'state=["failed"]' --spec ./openapi.yaml
```

`list` prints every tool with its request line. `-a name=value` parses the value as JSON when it
parses (`page=2` is a number, `name=pep` is the string), so quoting is only needed for arrays and
objects; `--json '{...}'` passes the whole argument object at once. A failed call prints its
message to stderr and exits non-zero. `--compact` prints single-line JSON for `jq`.

Configuration — `--config`, `./openapi-tools.toml`, `~/.config/openapi-tools/config.toml`, `--api`,
`--group` — is documented in the [openapi-mcp README](../openapi-mcp/README.md).
