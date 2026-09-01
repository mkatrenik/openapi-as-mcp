# openapi-as-mcp

Turn any OpenAPI 3.x document into an MCP tool set. Point it at a spec and a base URL; every
operation in the document becomes a tool, named after its `operationId`, with a JSON Schema built
from its parameters and request body. Nothing about the API is compiled in, so a new endpoint shows
up the moment the document does.

Use it when an API is worth handing to an agent but not worth a hand-written MCP server. When the
tools need to *do* something the API does not — derive a path, join two calls, explain a failure —
write a real server instead; [recipe-run-debug-mcp](../recipe-run-debug-mcp/) is that.

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

`serve` runs the server on stdio. Everything has an `OAM_*` environment fallback, for clients that
can only set `env`:

```json
{
  "mcpServers": {
    "recipes": {
      "command": "openapi-as-mcp",
      "args": ["serve"],
      "env": {
        "OAM_SPEC": "/path/to/openapi.yaml",
        "OAM_BASE_URL": "https://api.example.com",
        "OAM_TOKEN": "…"
      }
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

| Flag | Env | |
| --- | --- | --- |
| `--spec`, `-s` | `OAM_SPEC` | document to serve; a path or an `http(s)` URL. Repeatable |
| `--base-url` | `OAM_BASE_URL` | where requests go; defaults to the document's first `servers[].url` |
| `--token` | `OAM_TOKEN` | sent as `Authorization: Bearer …` |
| `--header`, `-H` | `OAM_HEADERS` | extra header, `Name: value`. The env form takes `A: 1;B: 2` |
| `--read-only` | `OAM_READ_ONLY` | expose only GET/HEAD/OPTIONS |
| `--include` / `--exclude` | `OAM_INCLUDE` / `OAM_EXCLUDE` | regexes, matched against the tool name, the method and the path independently |
| `--tool-prefix` | `OAM_TOOL_PREFIX` | keeps two of these apart in one client |
| `--timeout` | `OAM_TIMEOUT` | per-request, in seconds (default 60) |
| `--max-response-bytes` | `OAM_MAX_RESPONSE_BYTES` | ceiling on one response (default 256 KiB) |
| `OAM_LOG` | | log filter; diagnostics always go to stderr |

Several `--spec`s are served together. Names are deduplicated across documents, so a collision
becomes `listRecipes_2` rather than a lost tool.

Startup fails, rather than serving something useless, when there is no base URL to send requests to
or when the filters leave no operations at all.

## Development

```bash
cargo test -p openapi-as-mcp
```

`tests/protocol.rs` and `tests/cli.rs` drive the built binary against a mock API — the protocol
test is the one that catches a stray `println!`, which would corrupt the stdio JSON-RPC stream.
`tests/no_stdout.rs` guards the same thing statically.
