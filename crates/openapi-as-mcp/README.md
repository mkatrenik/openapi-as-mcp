# openapi-as-mcp

Turn any OpenAPI 3.x document into an MCP tool set. Point it at a spec and a base URL; every
operation in the document becomes a tool, named after its `operationId`, with a JSON Schema built
from its parameters and request body. Nothing about the API is compiled in, so a new endpoint shows
up the moment the document does.

Use it when an API is worth handing to an agent but not worth a hand-written MCP server. When the
tools need to *do* something the API does not — derive a path, join two calls, explain a failure —
write a real server instead.

## Quick start

```bash
cargo run -p openapi-as-mcp -- list \
  --spec https://api.example.com/openapi.json \
  --base-url https://api.example.com
```

`list` prints one line of JSON per tool. Then look at one, and call it:

```bash
openapi-as-mcp schema listRecipes --spec ./openapi.yaml
openapi-as-mcp call listRecipes -a page=2 -a 'state=["failed"]' --spec ./openapi.yaml
```

`-a name=value` parses the value as JSON when it parses (`page=2` is a number, `name=pep` is the
string), so quoting is only needed for arrays and objects. `--json '{...}'` passes the whole
argument object at once.

## As an MCP server

`serve` runs the server on stdio. Everything is configured by flags, so a client entry is one
`args` list:

```json
{
  "mcpServers": {
    "recipes": {
      "command": "openapi-as-mcp",
      "args": [
        "serve",
        "--spec", "/path/to/openapi.yaml",
        "--base-url", "https://api.example.com",
        "--token", "…"
      ]
    }
  }
}
```

The subcommand is not optional: a bare invocation prints help, so an entry that forgets `serve`
fails loudly instead of hanging.

## What the mapping looks like

| OpenAPI | MCP |
| --- | --- |
| operation | one tool, named after `operationId` (falling back to `get_recipes_latest`) |
| `summary` / `description` | the tool description, always prefixed with `GET /recipes/{id}` |
| path, query, header, cookie parameters | flat top-level arguments |
| request body | the `body` argument |
| `#/components/schemas/X` | `#/$defs/X`, with the transitive closure copied into `$defs` |
| `nullable: true`, boolean `exclusiveMinimum` | their JSON Schema 2020-12 equivalents |
| GET/HEAD/OPTIONS | `readOnlyHint`; DELETE/PUT/PATCH get `destructiveHint` |

Parameters are flat on purpose: an agent filling in `{"recipe_id": "…", "page": 2}` does not care
which of those rides in the path and which in the query string. The input schema is closed, so a
misspelled argument comes back as an error naming the real ones rather than being dropped from the
request.

A successful call returns the HTTP status, the request URL, and the response — parsed as `json`
when it is JSON, otherwise as `text`. A non-2xx response, a timeout or a bad argument is a
*tool-level* error carrying the upstream message, so the model can read it and try again.

## Options

| Flag | |
| --- | --- |
| `--config`, `-c` | TOML config file; `""` means "no file, and do not go looking" |
| `--spec`, `-s` | document to serve; a path or an `http(s)` URL. Repeatable |
| `--api` | serve only these `[[api]]` entries of the config file, by name |
| `--base-url` | where requests go; defaults to the document's first `servers[].url` |
| `--token` | sent as `Authorization: Bearer …` |
| `--header`, `-H` | extra header, `Name: value`. Repeatable |
| `--read-only` | expose only GET/HEAD/OPTIONS |
| `--include` / `--exclude` | regexes, matched against the tool name, the method and the path independently |
| `--tool-prefix` | keeps two of these apart in one client |
| `--timeout` | per-request, in seconds (default 60) |
| `--max-response-bytes` | ceiling on one response (default 256 KiB) |
| `--log` | log filter; diagnostics always go to stderr |

Startup fails, rather than serving something useless, when a document has no base URL to send its
requests to or when the filters leave no operations at all.

## Several documents at once

Every `--spec` is served together in one tool set, and names are deduplicated across documents, so
a collision becomes `listRecipes_2` rather than a lost tool. Flags apply to all of them, which is
fine when the documents share a host and a token.

When they do not, describe them in a config file instead: each `[[api]]` gets its own base URL,
credentials, filters and prefix. `--config <path>`, or `./openapi-as-mcp.toml` and
`~/.config/openapi-as-mcp/config.toml`, which are picked up automatically.

```toml
# Defaults for every api below.
timeout = 30
exclude = ["^/internal"]

[[api]]
name = "recipes"
spec = "./recipes.openapi.yaml"
base_url = "https://recipes.internal"
token = "${RECIPES_TOKEN}"
read_only = true

[[api]]
name = "billing"
spec = "https://billing.internal/openapi.json"
base_url = "https://billing.internal"
tool_prefix = "billing_"
headers = { "X-Api-Key" = "${BILLING_KEY}" }
include = ["^/invoices"]
```

See [example.config.toml](example.config.toml) for the annotated version. The keys are the flag
names: `spec` (or `specs`, a list, at the top level), `base_url`, `token`, `headers`, `read_only`,
`include`, `exclude`, `tool_prefix`, `timeout`, `max_response_bytes` — kebab-case spellings work
too. A misspelled key is an error rather than a silently ignored line.

`${VAR}` and `${VAR:-fallback}` are expanded in `spec`, `base_url`, `token` and header values, so
a token stays in the environment and the file stays committable.

Precedence, most specific first: a CLI flag, the `[[api]]` entry, the file's top-level defaults. A `--spec` on the command line is served *in addition to* the file's entries,
never instead of them, and `--api recipes` narrows the run to the entries you name:

```bash
openapi-as-mcp list --api recipes          # one document out of the file
openapi-as-mcp serve --api recipes --api billing
```

An MCP client entry can then be one line of arguments:

```json
{
  "mcpServers": {
    "apis": {
      "command": "openapi-as-mcp",
      "args": ["serve", "--config", "/path/to/openapi-as-mcp.toml"],
      "env": { "RECIPES_TOKEN": "…", "BILLING_KEY": "…" }
    }
  }
}
```

## Development

```bash
cargo test -p openapi-as-mcp
```

`tests/protocol.rs` and `tests/cli.rs` drive the built binary against a mock API — the protocol
test is the one that catches a stray `println!`, which would corrupt the stdio JSON-RPC stream.
`tests/no_stdout.rs` guards the same thing statically.
