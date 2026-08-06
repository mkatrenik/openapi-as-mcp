# recipe-run-debug-mcp

An MCP server — and a CLI over the same tools — for debugging ingestion-platform recipe runs. It
gives an agent what the Recipe Runs page in `ingestion-platform-ui` gives a human — find runs, read
task/agent state and failure reasons, resolve the config that produced them, inspect the artefacts
the agents wrote — plus the things a UI can't do: server-side grep over multi-megabyte artefacts,
existence checks on expected outputs, and run-to-run comparison.

Rust, stdio transport, read-only. One binary: `serve` runs the MCP server, every other subcommand
runs a single tool and prints its JSON. A bare invocation prints help.

## Quick start

```bash
# From anywhere in the workspace; the binary lands in the workspace-root target/ directory.
cargo build --release -p recipe-run-debug-mcp
BIN=$(cargo metadata --format-version 1 --no-deps | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/recipe-run-debug-mcp

export RRD_GATEWAY_BASE_URL=https://api-gateway-service.k8s.euw1.dev.gcp.ivxs.uk
export RRD_ENV=development
export RRD_TOKEN=<bearer token>        # required: the gateway rejects anonymous requests

claude mcp add recipe-run-debug \
  --env RRD_GATEWAY_BASE_URL=$RRD_GATEWAY_BASE_URL \
  --env RRD_ENV=$RRD_ENV \
  --env RRD_TOKEN=$RRD_TOKEN \
  -- "$BIN" serve
```

The trailing `serve` is required — without it the binary prints help and exits rather than holding
stdin open.

## CLI

The same install is a shell tool. Every subcommand runs exactly one MCP tool and prints its result —
the same `{env, bucket, data}` envelope — so anything you learn from one surface applies to the other.

```bash
rrd=$BIN                      # or: make install, then ~/.local/bin/recipe-run-debug-mcp

$rrd recipes pep              # find recipes matching "pep"
$rrd runs pep-recipe --state failed --size 5
$rrd why run-1                # triage: first failing task, attempts, reasons, log links
$rrd files run-1 --check      # which expected outputs are actually missing
$rrd search 'ERROR' --run run-1 --task validate --file entities.json -C 2
$rrd read --uri gs://ca-gcp-agent-platform-artefacts/agents/x/run-1/out.jsonl --mode tail
$rrd diff-config run-1 validate
$rrd compare run-1 run-2 --compact | jq '.data.differing_tasks'
```

| Command | Tool | |
| --- | --- | --- |
| `recipes [NAME_QUERY]` | `find_recipes` | find recipes by name |
| `runs [RECIPE_NAME]` | `find_recipe_runs` | the Recipe Runs UI filters |
| `run <RUN_ID>` | `get_recipe_run` | full detail for one run |
| `config [RECIPE_NAME]` | `get_recipe_config` | the recipe's current stored config |
| `files <RUN_ID>` | `list_run_files` | artefact files, `--check` to verify they exist |
| `stat` | `describe_run_file` | size, format, keys — the right first look |
| `search <PATTERN>` | `search_run_file` | regex, matching lines only |
| `read` | `read_run_file` | a bounded window: `--mode head\|tail\|slice` |
| `task-config <RUN_ID> <TASK>` | `get_task_run_config` | the config that actually ran |
| `diff-config <RUN_ID> <TASK>` | `diff_task_config` | as-executed vs current |
| `why <RUN_ID>` | `explain_run_failure` | triage in one call |
| `links <RUN_ID>` | `get_run_log_links` | Grafana and Kibana deep links |
| `compare <RUN_A> <RUN_B>` | `compare_runs` | two runs, task by task |
| `serve` | — | the MCP server on stdio; how an MCP client launches the binary |

Each tool name is also accepted as an alias (`$rrd explain-run-failure run-1`). `stat`, `search` and
`read` locate a file either with `--uri gs://…` or with `--run` + `--task` + `--file`; pass
`--file config` for the as-executed config.

Output is indented JSON on stdout, or one line with `--compact` for `jq`. Failures — a stale token,
an unknown run, a bad regex — print to stderr and exit non-zero, leaving stdout empty. Logging is
quiet in CLI mode; `RRD_LOG=info` turns it back on.

### Getting a token

There is no token lifecycle in this server by design. The UI obtains its bearer token through an
interactive OAuth redirect, which a headless server cannot do, so you supply one:

1. Open the UI, authenticate, and copy the `access_token` your browser holds.
2. Set `RRD_TOKEN` to it (or `RRD_EXTRA_HEADERS="Authorization: Bearer …"`).

Tokens expire. When one does, every tool returns a message naming `RRD_TOKEN` — that is a stale
token, not a missing run. Restart the server with a fresh value. Wiring in a service account or a
refresh flow means implementing one seam in `src/http.rs`; nothing else changes.

## Configuration

Settings come from a config file, the environment, and — in CLI mode — flags. **Precedence is flag,
then env var, then file, then default**, so the file holds what you set once, an MCP client entry can
point the same install elsewhere, and a single command can point somewhere else again.

### `~/.config/recipe-run-debug/config.toml`

Optional; a missing file is fine. Location is `$RRD_CONFIG` if set, else
`$XDG_CONFIG_HOME/recipe-run-debug/config.toml`, else `~/.config/recipe-run-debug/config.toml`. The
path actually read is logged at startup as `config_file`. Unknown keys are rejected rather than
ignored, so a typo fails loudly instead of silently doing nothing.

```toml
gateway_base_url = "https://api-gateway-service.k8s.euw1.dev.gcp.ivxs.uk"
env = "development"          # local|development|staging|production
token = "<bearer token>"     # sent as `Authorization: Bearer …`

# priority_endpoint = "hp"   # hp|lp; only affects production
# bucket_override = "…"      # override the per-env artefact bucket
# max_file_bytes = 262144    # ceiling on file content returned by one call

# Sent verbatim on every request; `token` is the shorthand for Authorization.
[headers]
"X-Debug-User" = "me"
```

With the file in place, registering the server needs no `--env` flags:

```bash
claude mcp add recipe-run-debug -- "$BIN" serve
```

Headers from the file and from `RRD_EXTRA_HEADERS` are merged; on a name collision the env value
wins, and `RRD_TOKEN` replaces a `token` from the file.

### Environment

| Variable | Required | Default | Meaning |
| --- | --- | --- | --- |
| `RRD_CONFIG` | no | see above | path to the config file |
| `RRD_GATEWAY_BASE_URL` | yes¹ | — | api-gateway origin (the UI's `API_GATEWAY_API_BASE_URL`) |
| `RRD_ENV` | no | `development` | `local`\|`development`\|`staging`\|`production` — selects artefact bucket and Grafana/Kibana hosts |
| `RRD_TOKEN` | in practice | — | bearer token, sent as `Authorization: Bearer …` |
| `RRD_EXTRA_HEADERS` | no | — | `Name: value` pairs, `;`-separated |
| `RRD_BUCKET_OVERRIDE` | no | per-env | override the artefact bucket |
| `RRD_PRIORITY_ENDPOINT` | no | `hp` | `hp`\|`lp`; only affects production, mirroring the UI |
| `RRD_MAX_FILE_BYTES` | no | `262144` | hard ceiling on file content returned by one call |
| `RRD_LOG` | no | `info` (`error` in CLI mode) | `tracing` filter; **stderr only** |

¹ unless `gateway_base_url` is in the config file — every setting above except `RRD_CONFIG` and
`RRD_LOG` has a file equivalent.

### Flags

Every setting above has a flag, usable before or after the subcommand and listed under
`Configuration` in `--help`:

| Flag | Overrides |
| --- | --- |
| `--config-file PATH` | `RRD_CONFIG`; unlike the default location, a path named here must exist |
| `--env ENV` | `RRD_ENV` (aliases `dev`, `stg`, `prod`) |
| `--gateway URL` | `RRD_GATEWAY_BASE_URL` |
| `--token TOKEN` | `RRD_TOKEN` |
| `-H, --header 'Name: value'` | `RRD_EXTRA_HEADERS` — repeatable, and *replaces* the variable rather than merging with it |
| `--bucket NAME` | `RRD_BUCKET_OVERRIDE` |
| `--priority hp\|lp` | `RRD_PRIORITY_ENDPOINT` |
| `--max-file-bytes N` | `RRD_MAX_FILE_BYTES` |

They apply to `serve` too, so an MCP client entry can use flags instead of `--env` wiring.

`RRD_ENV` decides which bucket paths and log links you get. It is echoed in the `env` and `bucket`
fields of every response so an agent cannot mistake a dev finding for a prod one.

## Tools

**Discovery** — `find_recipes`, `find_recipe_runs`, `get_recipe_run`

**Config** — `get_recipe_config` (current stored config), `get_task_run_config` (the config that
*actually ran*, read from the run's own artefact), `diff_task_config` (the two, diffed)

**Files** — `list_run_files`, `describe_run_file`, `search_run_file`, `read_run_file`

**Diagnosis** — `explain_run_failure`, `get_run_log_links`, `compare_runs`

### Suggested flow for a failed run

```
explain_run_failure { recipe_run_id }          → first failing task, attempts, reasons, log links
list_run_files      { recipe_run_id, check_existence: true }
                                               → which expected outputs are actually missing
search_run_file     { recipe_run_id, task_name, filename, pattern: "ERROR" }
                                               → matching lines only, never the whole artefact
get_task_run_config { recipe_run_id, task_name }
compare_runs        { recipe_run_id_a, recipe_run_id_b }
```

Reach for `describe_run_file` before `read_run_file`: it reports format, size, line/record count and
headers or top-level keys without pulling content into the conversation.

## API models are generated from the OpenAPI specs

`src/api/generated.rs` is produced by [typify](https://github.com/oxidecomputer/typify) from the
specs vendored in `specs/`. Do not edit it; adapters live in `src/api/model.rs`.

```bash
./specs/refresh.sh dev     # re-fetch specs (dev|stg|prod) and regenerate
cargo run --example generate_models   # regenerate from the vendored specs only
```

The specs are served **unauthenticated over https** at each service's `/openapi.json` — note the
UI's `generate-schemas.sh` uses `http://` for dev, which does not answer.

Two deliberate divergences from the specs, both applied in `examples/generate_models.rs`:

- **`additionalProperties: true` is forced on every object.** The aggregator spec declares
  `additionalProperties: false`, which typify honours with `#[serde(deny_unknown_fields)]` — one new
  backend field would make every run unparseable. Strictness protects a producer; we are a consumer
  that has to keep working when the platform is odd. Cost: unknown fields are ignored, not surfaced.
- **State fields are generated as `String`, not enums.** `TaskRun.state`, `RecipeRun.state`,
  `publication_state`, `execution_state` and `Recipe.status` keep their raw value so an added enum
  member degrades instead of failing. The enums are still generated, and
  `model::FromWire::from_wire` parses into them on demand — tool output reports
  `state_recognised: false` when a value isn't in the vendored spec.

`tests/generated_is_current.rs` re-runs the generator and fails if the committed file differs, so a
hand edit or a forgotten regeneration can't slip through.

## Design notes worth knowing before you trust output

- **Artefact paths are derived, not authoritative.** Output paths are built as
  `{bucket}/{data_files_path}/{recipe_run_id}/{filename}`, with filenames taken from the recipe
  task's `config.output`. That is replicated backend convention — the UI says as much in a comment.
  `list_run_files` with `check_existence: true` verifies against the bucket; everything else is a
  hypothesis, and says so in a `note` field.
- **As-executed beats current.** A recipe's stored config can change after a run. `get_recipe_config`
  reads the current config; `get_task_run_config` reads the run's own config artefact. When they
  disagree, `diff_task_config` shows where.
- **File reads happen through the gateway**, which has no range support, so an artefact is pulled
  into this process in full — but only a bounded, labelled slice reaches the agent. Truncation
  always states what was cut and how to continue. A direct-GCS backend with real ranged reads and
  prefix listing is the intended next step (`src/api/gcs_reader.rs` is the seam).
- **Read-only.** Resume/re-trigger is deliberately absent; it mutates production pipelines.

## Development

```bash
cargo test          # 71 tests: unit + protocol-over-stdio + CLI end-to-end + stdout and codegen guards
cargo clippy --all-targets
cargo fmt
```

Five things the test suite deliberately protects:

- `tests/no_stdout.rs` fails if any source file writes to stdout. On stdio transport stdout is the
  JSON-RPC channel, and a stray `println!` breaks every session in a way that looks like a client bug.
  `src/cli.rs` is the one exemption: in CLI mode there is no protocol stream, and the server path
  never reaches that module.
- `tests/protocol.rs` spawns the real binary and speaks MCP to it, so schema and initialization
  regressions surface at build time.
- `tests/cli.rs` spawns the same binary as a one-shot command against a mock gateway: a subcommand
  must not start the server, stdout must carry only the JSON result, a tool failure must exit
  non-zero with an empty stdout, a bare invocation must print help rather than start the server,
  and a flag must outrank the matching env var.
- `tests/generated_is_current.rs` fails if `src/api/generated.rs` doesn't match the vendored specs.
- The ports of the UI's derivations (`src/domain/`) carry the UI's own test cases. If the two
  implementations drift, a test goes red rather than an agent quietly reading the wrong path.

### Relationship to the UI

The reference implementation lives in `ingestion-platform-ui`:
`apps/ui/src/modules/recipes/detail/recipe-runs/`. `src/domain/paths.rs`, `tasks.rs` and `links.rs`
are ports of `tasks/utils.ts`, `Files.tsx`, `RecipeDetailStore` and `grafanaUrl.ts`; each Rust module
names its TypeScript counterpart. Change one, check the other.
