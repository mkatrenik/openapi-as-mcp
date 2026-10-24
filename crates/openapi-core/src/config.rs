//! Configuration: which specs to serve, where to send each one's requests, and what to send with
//! them.
//!
//! Three sources, most specific first: a CLI flag, an `[[api]]` entry in the config file, then the
//! file's top-level defaults. Everything is configured by flags; the process environment is read
//! only for `${VAR}` expansion inside the config file.
//!
//! One document is one [`ApiConfig`]: its own base URL, headers, filters and prefix. That is the
//! point of the file — several specs in one server only works if each can be pointed at its own
//! host with its own token, which a flat flag set cannot express.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;
use serde::Deserialize;

use crate::spec::Source;

/// Default ceiling on a response body we will hand back to the model. Large enough for a page of
/// JSON, small enough that one accidental "list everything" call cannot fill a context window.
const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024;

const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Looked at, in order, when `--config` is not given. A missing file here is not an error: the
/// flags alone are still a complete configuration.
const DISCOVERED_CONFIG_PATHS: [&str; 2] = [
    "openapi-as-mcp.toml",
    ".config/openapi-as-mcp/config.toml", // relative to $HOME
];

#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Configuration")]
pub struct ConfigArgs {
    /// TOML config file. Defaults to ./openapi-as-mcp.toml, then
    /// ~/.config/openapi-as-mcp/config.toml.
    #[arg(long = "config", short = 'c', value_name = "PATH", global = true)]
    /// A `String` rather than a `PathBuf` because clap's path parser rejects an empty value, and
    /// `--config ""` is the explicit "no file, and do not go looking".
    pub config: Option<String>,

    /// OpenAPI document to serve: a file path or an http(s) URL. Repeatable. Added to whatever the
    /// config file declares.
    #[arg(
        long = "spec",
        short = 's',
        value_name = "PATH_OR_URL",
        value_delimiter = ',',
        // Not `required`: clap forbids that on a global argument, and the file may supply the
        // specs instead, so `resolve` reports the omission with a message covering both.
        global = true
    )]
    pub specs: Vec<String>,

    /// Serve only these `[[api]]` entries of the config file, by `name` — either the bare name
    /// or the `group/name` it reads as when the entry lives in a `[group.…]` table. Repeatable.
    #[arg(
        long = "api",
        value_name = "NAME",
        value_delimiter = ',',
        global = true
    )]
    pub api: Vec<String>,

    /// Serve only these `[group.…]` tables of the config file, by name. Repeatable. Without it
    /// every group is served, alongside any entry that belongs to none.
    #[arg(
        long = "group",
        value_name = "NAME",
        value_delimiter = ',',
        global = true
    )]
    pub groups: Vec<String>,

    /// Base URL for the requests. Defaults to the first `servers[].url` in each document.
    #[arg(long = "base-url", value_name = "URL", global = true)]
    pub base_url: Option<String>,

    /// Bearer token, sent as `Authorization: Bearer <token>`.
    #[arg(long, value_name = "TOKEN", global = true)]
    pub token: Option<String>,

    /// Shell command whose trimmed stdout becomes the bearer token, run again for every request.
    /// Use this instead of `--token` for a token that expires or lives in a secret manager, e.g.
    /// `gcloud auth print-identity-token` or `op read op://vault/item/token`. Mutually exclusive
    /// with `--token` at the same level.
    #[arg(long = "token-command", value_name = "COMMAND", global = true)]
    pub token_command: Option<String>,

    /// Extra request header, `Name: value`. Repeatable.
    #[arg(long = "header", short = 'H', value_name = "HEADER", global = true)]
    pub headers: Vec<String>,

    /// Expose only GET/HEAD/OPTIONS operations. Use against an API you do not want written to.
    #[arg(long = "read-only", global = true)]
    pub read_only: bool,

    /// Keep only operations whose tool name, method or path matches this regex. Repeatable.
    #[arg(long = "include", value_name = "REGEX", global = true)]
    pub include: Vec<String>,

    /// Drop operations whose tool name, method or path matches this regex. Applied after
    /// --include. Repeatable.
    #[arg(long = "exclude", value_name = "REGEX", global = true)]
    pub exclude: Vec<String>,

    /// Prefix every tool name with this, to keep two servers apart in one client.
    #[arg(long = "tool-prefix", value_name = "PREFIX", global = true)]
    pub tool_prefix: Option<String>,

    /// Per-request timeout in seconds.
    #[arg(long, value_name = "SECONDS", global = true)]
    pub timeout: Option<u64>,

    /// Ceiling on the response bytes one tool call may return.
    #[arg(long, value_name = "N", global = true)]
    pub max_response_bytes: Option<usize>,
}

/// One document and everything about how to call it.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// What to call this API in logs and errors: the `name` from the file, else the spec's path.
    pub label: String,
    pub spec: Source,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub read_only: bool,
    pub include: Vec<regex::Regex>,
    pub exclude: Vec<regex::Regex>,
    pub tool_prefix: Option<String>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
    /// When set, run this shell command before every request and send its trimmed stdout as
    /// `Authorization: Bearer <output>`, replacing any static token/`Authorization` header.
    pub token_command: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub apis: Vec<ApiConfig>,
    /// Timeout for fetching the documents themselves, as opposed to calling the APIs.
    pub timeout: Duration,
}

impl Config {
    pub fn specs(&self) -> Vec<Source> {
        self.apis.iter().map(|api| api.spec.clone()).collect()
    }
}

/// The config file, and — with `api` empty — one `[[api]]` entry inside it. The two levels take
/// the same keys, which is what makes "set it once at the top, override it per API" work.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// `[[api]]` only: names the entry, for `--api` and for log lines.
    name: Option<String>,

    /// A path or URL, or a list of them. At the top level each entry becomes its own API.
    #[serde(alias = "specs")]
    spec: Option<Specs>,

    #[serde(alias = "base-url")]
    base_url: Option<String>,
    token: Option<String>,
    #[serde(alias = "token-command")]
    token_command: Option<String>,
    headers: Option<Headers>,
    #[serde(alias = "read-only")]
    read_only: Option<bool>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    #[serde(alias = "tool-prefix")]
    tool_prefix: Option<String>,
    timeout: Option<u64>,
    #[serde(alias = "max-response-bytes")]
    max_response_bytes: Option<usize>,

    #[serde(default, alias = "apis")]
    api: Vec<Settings>,

    /// Top level only: named sets of entries, each with its own defaults. `[group.prod]` holds
    /// what every `[[group.prod.api]]` inherits, and `--group prod` serves that set alone.
    #[serde(default, alias = "groups")]
    group: BTreeMap<String, Settings>,
}

/// One `[[api]]` entry paired with the level it inherits from, before the flags have their say.
struct Candidate<'a> {
    /// The `[group.…]` table it came from, if any. Part of the label, and what `--group` filters.
    group: Option<String>,
    /// That table's own keys, which sit between the entry and the file's top-level defaults.
    group_defaults: Option<&'a Settings>,
    entry: Settings,
}

impl Candidate<'_> {
    /// How the entry is named on the command line and in logs: `prod/recipes` inside a group, so
    /// that two groups may reuse one name, and the bare name outside one.
    fn label(&self) -> Option<String> {
        let name = self.entry.name.as_ref()?;
        Some(match &self.group {
            Some(group) => format!("{group}/{name}"),
            None => name.clone(),
        })
    }

    /// `--api` accepts either spelling, so `--api recipes` still reaches `prod/recipes`.
    fn matches(&self, wanted: &str) -> bool {
        self.label().is_some_and(|label| label == wanted)
            || self.entry.name.as_deref() == Some(wanted)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Specs {
    One(String),
    Many(Vec<String>),
}

impl Specs {
    fn to_vec(&self) -> Vec<String> {
        match self {
            Self::One(one) => vec![one.clone()],
            Self::Many(many) => many.clone(),
        }
    }
}

/// `headers = { "X-Api-Key" = "…" }` or `headers = ["X-Api-Key: …"]`, whichever reads better at
/// the call site.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Headers {
    Map(BTreeMap<String, String>),
    List(Vec<String>),
}

impl Headers {
    fn parse(&self) -> anyhow::Result<Vec<(String, String)>> {
        match self {
            Self::Map(map) => Ok(map
                .iter()
                .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
                .collect()),
            Self::List(list) => list
                .iter()
                .filter(|raw| !raw.trim().is_empty())
                .map(|raw| parse_header(raw))
                .collect(),
        }
    }
}

impl Settings {
    /// Expands `${VAR}` and `${VAR:-fallback}` in every value a secret or a host can hide in.
    /// Only file-sourced values go through this — a shell has already done it for a flag.
    ///
    /// `token_command` is deliberately left untouched: it is handed to a shell of its own at
    /// request time, which expands its own `$VAR`/`${VAR}` references, so running our expander
    /// over it first would consume syntax the command needed for itself.
    fn expand_env(&mut self) -> anyhow::Result<()> {
        for text in [&mut self.base_url, &mut self.token].into_iter().flatten() {
            *text = expand_env(text)?;
        }
        if let Some(spec) = &mut self.spec {
            *spec = match spec {
                Specs::One(one) => Specs::One(expand_env(one)?),
                Specs::Many(many) => Specs::Many(
                    many.iter()
                        .map(|one| expand_env(one))
                        .collect::<anyhow::Result<_>>()?,
                ),
            };
        }
        if let Some(headers) = &mut self.headers {
            *headers = match headers {
                Headers::Map(map) => Headers::Map(
                    map.iter()
                        .map(|(name, value)| Ok((name.clone(), expand_env(value)?)))
                        .collect::<anyhow::Result<_>>()?,
                ),
                Headers::List(list) => Headers::List(
                    list.iter()
                        .map(|raw| expand_env(raw))
                        .collect::<anyhow::Result<_>>()?,
                ),
            };
        }
        for entry in &mut self.api {
            entry.expand_env()?;
        }
        for group in self.group.values_mut() {
            group.expand_env()?;
        }
        Ok(())
    }

    /// Headers this level contributes, its `token` first so a more specific `Authorization`
    /// header still wins the dedupe in [`ConfigArgs::api_config`].
    fn header_list(&self) -> anyhow::Result<Vec<(String, String)>> {
        let mut headers = Vec::new();
        if let Some(token) = self.token.as_ref().filter(|t| !t.trim().is_empty()) {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        if let Some(declared) = &self.headers {
            headers.extend(declared.parse()?);
        }
        Ok(headers)
    }
}

/// A single level (a flag pair, or one `Settings`) may pick at most one way to authenticate.
fn reject_conflicting_token(
    token: Option<&String>,
    token_command: Option<&String>,
    whom: &str,
) -> anyhow::Result<()> {
    let set = |value: Option<&String>| value.is_some_and(|v| !v.trim().is_empty());
    if set(token) && set(token_command) {
        bail!("{whom}: `token` and `token_command` cannot both be set");
    }
    Ok(())
}

impl ConfigArgs {
    pub fn resolve(&self) -> anyhow::Result<Config> {
        let (path, defaults) = self.load_file()?;
        let source = |what: &str| match &path {
            Some(path) => format!("{what} in {}", path.display()),
            None => what.to_string(),
        };

        reject_conflicting_token(
            self.token.as_ref(),
            self.token_command.as_ref(),
            "--token / --token-command",
        )?;

        // The top-level defaults first, then each group, so the tool order matches how the file
        // reads — an ungrouped entry before any group's, whatever the group is called.
        let mut levels: Vec<(Option<String>, &Settings)> = vec![(None, &defaults)];
        levels.extend(
            defaults
                .group
                .iter()
                .map(|(name, group)| (Some(name.clone()), group)),
        );

        let mut candidates: Vec<Candidate> = Vec::new();
        for (group, level) in levels {
            let whose = |what: &str| match &group {
                Some(name) => source(&format!("{what} of group {name:?}")),
                None => source(what),
            };

            if group.is_some() && !level.group.is_empty() {
                bail!(
                    "{}",
                    whose("nested `[group]` tables are not supported; the defaults")
                );
            }
            reject_conflicting_token(
                level.token.as_ref(),
                level.token_command.as_ref(),
                &match &group {
                    Some(_) => whose("the defaults"),
                    None => source("the top-level config"),
                },
            )?;

            for entry in &level.api {
                let named = || {
                    entry
                        .name
                        .as_deref()
                        .unwrap_or("an [[api]] entry")
                        .to_string()
                };
                if !entry.api.is_empty() {
                    bail!(
                        "{}: `[[api]]` entries cannot contain their own `[[api]]`",
                        source(&named())
                    );
                }
                if !entry.group.is_empty() {
                    bail!(
                        "{}: `[[api]]` entries cannot contain a `[group]` table",
                        source(&named())
                    );
                }
                reject_conflicting_token(
                    entry.token.as_ref(),
                    entry.token_command.as_ref(),
                    &source(&named()),
                )?;
                candidates.push(Candidate {
                    group: group.clone(),
                    group_defaults: group.as_ref().map(|_| level),
                    entry: entry.clone(),
                });
            }

            // A bare `spec`/`specs` at a level is shorthand for entries carrying nothing else.
            for spec in level.spec.iter().flat_map(Specs::to_vec) {
                candidates.push(Candidate {
                    group: group.clone(),
                    group_defaults: group.as_ref().map(|_| level),
                    entry: Settings {
                        spec: Some(Specs::One(spec)),
                        ..Settings::default()
                    },
                });
            }
        }

        // `--group` narrows to whole sets, `--api` to individual entries; a `--spec` given on the
        // command line is an explicit ask and is never filtered out, so it is appended afterwards.
        if !self.groups.is_empty() {
            let known: Vec<&String> = defaults.group.keys().collect();
            if let Some(wanted) = self
                .groups
                .iter()
                .find(|wanted| !defaults.group.contains_key(*wanted))
            {
                bail!(
                    "--group {wanted:?} matches no [group] table; {}",
                    if known.is_empty() {
                        source("no groups are declared")
                    } else {
                        format!(
                            "known groups: {}",
                            known
                                .iter()
                                .map(|name| name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                );
            }
            candidates
                .retain(|candidate| matches!(&candidate.group, Some(g) if self.groups.contains(g)));
        }

        if !self.api.is_empty() {
            let known: Vec<String> = candidates.iter().filter_map(Candidate::label).collect();
            candidates.retain(|candidate| self.api.iter().any(|wanted| candidate.matches(wanted)));
            if candidates.is_empty() {
                bail!(
                    "--api {:?} matches no [[api]] entry; {}",
                    self.api.join(", "),
                    if known.is_empty() {
                        source("no named [[api]] entries")
                    } else {
                        format!("known names: {}", known.join(", "))
                    }
                );
            }
        }

        for spec in &self.specs {
            candidates.push(Candidate {
                group: None,
                group_defaults: None,
                entry: Settings {
                    spec: Some(Specs::One(spec.clone())),
                    ..Settings::default()
                },
            });
        }

        if candidates.is_empty() {
            bail!(
                "no OpenAPI document given; pass --spec <path-or-url>, or declare [[api]] \
                 entries in a config file (--config)"
            );
        }

        let mut apis = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            apis.push(self.api_config(candidate, &defaults)?);
        }

        Ok(Config {
            apis,
            timeout: Duration::from_secs(
                self.timeout
                    .or(defaults.timeout)
                    .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            ),
        })
    }

    /// Reads the config file, if there is one. An explicitly named file must exist; a discovered
    /// one need not.
    fn load_file(&self) -> anyhow::Result<(Option<PathBuf>, Settings)> {
        let path = match &self.config {
            // `--config ""` is "no file, and do not go looking", which is how a test or a
            // locked-down MCP entry keeps a stray file out of the configuration.
            Some(path) if path.trim().is_empty() => None,
            Some(path) => {
                let path = PathBuf::from(path);
                if !path.exists() {
                    bail!("config file {} does not exist", path.display());
                }
                Some(path)
            }
            None => discover(),
        };

        let Some(path) = path else {
            return Ok((None, Settings::default()));
        };

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let mut settings: Settings = toml::from_str(&text)
            .with_context(|| format!("{} is not a valid openapi-as-mcp config", path.display()))?;
        settings
            .expand_env()
            .with_context(|| format!("in {}", path.display()))?;

        let grouped: usize = settings.group.values().map(|group| group.api.len()).sum();
        tracing::info!(
            config = %path.display(),
            apis = settings.api.len() + grouped,
            groups = settings.group.len(),
            "loaded config file"
        );
        Ok((Some(path), settings))
    }

    /// Merges one entry with its group's defaults, the file's defaults and the flags. A flag wins
    /// when it was given at all: `Some`, a non-empty list, or `--read-only`, which can only ever
    /// restrict. Below the flags it is the most specific level that sets a key: the entry, then
    /// its `[group.…]` table, then the file's top level.
    fn api_config(&self, candidate: &Candidate, defaults: &Settings) -> anyhow::Result<ApiConfig> {
        let entry = &candidate.entry;
        let empty = Settings::default();
        let group = candidate.group_defaults.unwrap_or(&empty);
        let spec = entry
            .spec
            .as_ref()
            .map(Specs::to_vec)
            .unwrap_or_default()
            .first()
            .cloned()
            .with_context(|| {
                format!(
                    "[[api]] {} has no `spec`",
                    entry.name.as_deref().unwrap_or("entry")
                )
            })?;

        // Least specific first, so a later duplicate of the same header name wins.
        let mut headers = defaults.header_list()?;
        headers.extend(group.header_list()?);
        headers.extend(entry.header_list()?);
        if let Some(token) = self.token.as_ref().filter(|t| !t.trim().is_empty()) {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        for raw in self
            .headers
            .iter()
            .map(String::as_str)
            .filter(|raw| !raw.trim().is_empty())
        {
            headers.push(parse_header(raw)?);
        }

        // Whichever of `token`/`token_command` the most specific level sets wins outright, the
        // same way it would for `base_url`: an [[api]] entry's plain `token` overrides a
        // `token_command` declared at the top level, and vice versa.
        fn non_empty(value: Option<&String>) -> bool {
            value.is_some_and(|v| !v.trim().is_empty())
        }
        let token_command = [
            (self.token_command.as_ref(), self.token.as_ref()),
            (entry.token_command.as_ref(), entry.token.as_ref()),
            (group.token_command.as_ref(), group.token.as_ref()),
            (defaults.token_command.as_ref(), defaults.token.as_ref()),
        ]
        .into_iter()
        .find(|(command, token)| non_empty(*command) || non_empty(*token))
        .and_then(|(command, _)| command.cloned());

        let mut headers = dedupe_headers(headers);
        if token_command.is_some() {
            // Resolved fresh on every call instead; a stale static one must not also go out.
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        }

        let picked =
            |flag: Option<&String>,
             entry: Option<&String>,
             group: Option<&String>,
             default: Option<&String>| flag.or(entry).or(group).or(default).cloned();
        let patterns = |flag: &[String],
                        entry: &Option<Vec<String>>,
                        group: &Option<Vec<String>>,
                        default: &Option<Vec<String>>| {
            if !flag.is_empty() {
                flag.to_vec()
            } else {
                entry
                    .clone()
                    .or_else(|| group.clone())
                    .or_else(|| default.clone())
                    .unwrap_or_default()
            }
        };

        Ok(ApiConfig {
            label: candidate.label().unwrap_or_else(|| match &candidate.group {
                Some(group) => format!("{group}/{spec}"),
                None => spec.clone(),
            }),
            spec: Source::parse(&spec),
            // A base URL with a trailing slash and a path starting with one would produce `//`,
            // which some gateways route differently from the intended path.
            base_url: picked(
                self.base_url.as_ref(),
                entry.base_url.as_ref(),
                group.base_url.as_ref(),
                defaults.base_url.as_ref(),
            )
            .map(|url| url.trim_end_matches('/').to_string()),
            headers,
            read_only: self.read_only
                || entry
                    .read_only
                    .or(group.read_only)
                    .or(defaults.read_only)
                    .unwrap_or(false),
            include: compile(
                &patterns(
                    &self.include,
                    &entry.include,
                    &group.include,
                    &defaults.include,
                ),
                "include",
            )?,
            exclude: compile(
                &patterns(
                    &self.exclude,
                    &entry.exclude,
                    &group.exclude,
                    &defaults.exclude,
                ),
                "exclude",
            )?,
            tool_prefix: picked(
                self.tool_prefix.as_ref(),
                entry.tool_prefix.as_ref(),
                group.tool_prefix.as_ref(),
                defaults.tool_prefix.as_ref(),
            ),
            timeout: Duration::from_secs(
                self.timeout
                    .or(entry.timeout)
                    .or(group.timeout)
                    .or(defaults.timeout)
                    .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            ),
            max_response_bytes: self
                .max_response_bytes
                .or(entry.max_response_bytes)
                .or(group.max_response_bytes)
                .or(defaults.max_response_bytes)
                .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES),
            token_command,
        })
    }
}

#[cfg(test)]
impl ConfigArgs {
    /// Defaults without the `--spec` requirement, for tests that only need the request settings.
    pub fn resolve_for_test(mut self) -> Config {
        if self.specs.is_empty() {
            self.specs = vec!["unused.json".into()];
        }
        self.resolve().expect("default config resolves")
    }
}

fn discover() -> Option<PathBuf> {
    // A unit test must not pick up the developer's own config; the integration tests pass
    // `--config ""` for the same reason.
    if cfg!(test) {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    DISCOVERED_CONFIG_PATHS
        .iter()
        .filter_map(|candidate| {
            let path = Path::new(candidate);
            if path.components().count() > 1 {
                Some(home.as_ref()?.join(path))
            } else {
                Some(path.to_path_buf())
            }
        })
        .find(|path| path.is_file())
}

/// Keeps the last header of each name, case-insensitively, in first-seen order. Two
/// `Authorization` headers would be sent as two, and the API would reject the request.
fn dedupe_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        match kept
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            Some(slot) => slot.1 = value,
            None => kept.push((name, value)),
        }
    }
    kept
}

/// `${VAR}`, or `${VAR:-fallback}`. `$${` is a literal `${`, for the rare value that contains one.
fn expand_env(text: &str) -> anyhow::Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find("${") {
        if rest[..at].ends_with('$') {
            out.push_str(&rest[..at - 1]);
            out.push_str("${");
            rest = &rest[at + 2..];
            continue;
        }
        out.push_str(&rest[..at]);
        let body = &rest[at + 2..];
        let end = body
            .find('}')
            .with_context(|| format!("unterminated ${{…}} in {text:?}"))?;
        let (name, fallback) = match body[..end].split_once(":-") {
            Some((name, fallback)) => (name.trim(), Some(fallback)),
            None => (body[..end].trim(), None),
        };

        match std::env::var(name) {
            Ok(value) => out.push_str(&value),
            Err(_) => match fallback {
                Some(fallback) => out.push_str(fallback),
                None => bail!(
                    "environment variable {name} is referenced but not set (use \
                     ${{{name}:-fallback}} to make it optional)"
                ),
            },
        }
        rest = &body[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}

fn compile(patterns: &[String], what: &str) -> anyhow::Result<Vec<regex::Regex>> {
    patterns
        .iter()
        .map(|pattern| {
            regex::Regex::new(pattern)
                .with_context(|| format!("{what} {pattern:?} is not a valid regex"))
        })
        .collect()
}

fn parse_header(raw: &str) -> anyhow::Result<(String, String)> {
    let (name, value) = raw
        .split_once(':')
        .with_context(|| format!("header {raw:?} is not in `Name: value` form"))?;
    let name = name.trim();
    if name.is_empty() {
        bail!("header {raw:?} has an empty name");
    }
    Ok((name.to_string(), value.trim().to_string()))
}

/// Building a [`Config`] from TOML text, for tests in this crate that need several APIs.
#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Writes `toml` to a temporary file and resolves it as if it had been passed to `--config`.
    pub fn config_from(toml: &str) -> Config {
        try_config_from(toml).expect("config resolves")
    }

    pub fn try_config_from(toml: &str) -> anyhow::Result<Config> {
        try_config_from_args(toml, ConfigArgs::default())
    }

    /// The same, with flags on top — for the precedence the file alone cannot express.
    pub fn try_config_from_args(toml: &str, args: ConfigArgs) -> anyhow::Result<Config> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "openapi-as-mcp-{}-{}.toml",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, toml).expect("writes the temp config");
        let resolved = ConfigArgs {
            config: Some(path.display().to_string()),
            ..args
        }
        .resolve();
        let _ = std::fs::remove_file(&path);
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{config_from, try_config_from, try_config_from_args};
    use super::*;

    fn args() -> ConfigArgs {
        ConfigArgs {
            specs: vec!["spec.json".into()],
            ..ConfigArgs::default()
        }
    }

    fn only(args: ConfigArgs) -> ApiConfig {
        args.resolve().expect("resolves").apis.remove(0)
    }

    #[test]
    fn a_token_becomes_a_bearer_header() {
        let api = only(ConfigArgs {
            token: Some("abc".into()),
            ..args()
        });
        assert_eq!(
            api.headers,
            vec![("Authorization".to_string(), "Bearer abc".to_string())]
        );
    }

    #[test]
    fn a_token_command_is_carried_through_and_drops_no_static_header() {
        let api = only(ConfigArgs {
            token_command: Some("echo abc".into()),
            ..args()
        });
        assert_eq!(api.token_command.as_deref(), Some("echo abc"));
        assert!(
            api.headers.is_empty(),
            "no static Authorization header when a command supplies it"
        );
    }

    #[test]
    fn a_token_and_a_token_command_at_the_same_level_are_rejected() {
        let err = ConfigArgs {
            token: Some("abc".into()),
            token_command: Some("echo abc".into()),
            ..args()
        }
        .resolve()
        .expect_err("rejected");
        assert!(err.to_string().contains("token_command"), "got: {err}");
    }

    #[test]
    fn an_entrys_own_token_overrides_a_shared_token_command() {
        let config = config_from(
            r#"
            token_command = "echo shared"

            [[api]]
            name = "own-token"
            spec = "a.json"
            token = "own"
        "#,
        );
        let api = &config.apis[0];
        assert_eq!(
            api.token_command, None,
            "the entry's own token wins outright"
        );
        assert_eq!(
            api.headers,
            vec![("Authorization".to_string(), "Bearer own".to_string())]
        );
    }

    #[test]
    fn an_entrys_own_token_command_overrides_a_shared_token() {
        let config = config_from(
            r#"
            token = "shared"

            [[api]]
            name = "own-command"
            spec = "a.json"
            token_command = "echo own"
        "#,
        );
        let api = &config.apis[0];
        assert_eq!(api.token_command.as_deref(), Some("echo own"));
        assert!(
            api.headers.is_empty(),
            "the shared static token must not also be sent"
        );
    }

    #[test]
    fn headers_come_from_the_repeated_flag() {
        let api = only(ConfigArgs {
            headers: vec!["X-A: 1".into(), "X-B: 2".into(), "X-C: 3".into()],
            ..args()
        });
        let names: Vec<&str> = api.headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["X-A", "X-B", "X-C"]);
        assert_eq!(api.headers[1].1, "2", "the value is trimmed");
    }

    #[test]
    fn a_malformed_header_is_rejected_with_the_offending_text() {
        let err = ConfigArgs {
            headers: vec!["nope".into()],
            ..args()
        }
        .resolve()
        .expect_err("rejected");
        assert!(err.to_string().contains("nope"), "got: {err}");
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_is_dropped() {
        let api = only(ConfigArgs {
            base_url: Some("https://api.example.com/v1/".into()),
            ..args()
        });
        assert_eq!(api.base_url.as_deref(), Some("https://api.example.com/v1"));
    }

    #[test]
    fn a_bad_filter_regex_names_the_setting() {
        let err = ConfigArgs {
            exclude: vec!["[".into()],
            ..args()
        }
        .resolve()
        .expect_err("rejected");
        assert!(err.to_string().contains("exclude"), "got: {err}");
    }

    #[test]
    fn a_url_spec_is_told_apart_from_a_path() {
        let config = ConfigArgs {
            specs: vec!["https://x/openapi.json".into(), "./local.yaml".into()],
            ..args()
        }
        .resolve()
        .expect("resolves");
        assert!(matches!(config.apis[0].spec, Source::Url(_)));
        assert!(matches!(config.apis[1].spec, Source::File(_)));
    }

    #[test]
    fn with_no_spec_anywhere_the_message_names_every_way_to_give_one() {
        let err = ConfigArgs::default().resolve().expect_err("rejected");
        let message = err.to_string();
        assert!(message.contains("--spec"), "got: {message}");
        assert!(message.contains("[[api]]"), "got: {message}");
    }

    #[test]
    fn top_level_settings_are_defaults_that_each_api_can_override() {
        let config = config_from(
            r#"
            base_url = "https://shared.example.com"
            token = "shared"
            timeout = 5

            [[api]]
            name = "shared-everything"
            spec = "a.json"

            [[api]]
            name = "own-host"
            spec = "b.json"
            base_url = "https://own.example.com"
            token = "own"
            timeout = 9
        "#,
        );

        assert_eq!(config.apis.len(), 2);
        assert_eq!(
            config.apis[0].base_url.as_deref(),
            Some("https://shared.example.com")
        );
        assert_eq!(config.apis[0].headers[0].1, "Bearer shared");
        assert_eq!(config.apis[0].timeout, Duration::from_secs(5));

        assert_eq!(
            config.apis[1].base_url.as_deref(),
            Some("https://own.example.com")
        );
        assert_eq!(
            config.apis[1].headers,
            vec![("Authorization".to_string(), "Bearer own".to_string())],
            "the api's own token replaces the shared one rather than being sent alongside it"
        );
        assert_eq!(config.apis[1].timeout, Duration::from_secs(9));
    }

    #[test]
    fn a_top_level_spec_list_becomes_one_api_each() {
        let config = config_from(
            r#"
            specs = ["a.json", "https://b.example.com/openapi.json"]
            base_url = "https://shared.example.com"
        "#,
        );
        assert_eq!(config.apis.len(), 2);
        assert!(config.apis.iter().all(|api| api.base_url.is_some()));
    }

    #[test]
    fn a_flag_overrides_the_file() {
        let config = ConfigArgs {
            base_url: Some("http://localhost:8080".into()),
            read_only: true,
            ..ConfigArgs::default()
        };
        // Resolved by hand rather than through `config_from`, which takes no flags.
        let path = std::env::temp_dir().join(format!("oam-flag-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "base_url = \"https://from-file\"\n[[api]]\nspec = \"a.json\"\n",
        )
        .expect("writes");
        let resolved = ConfigArgs {
            config: Some(path.display().to_string()),
            ..config
        }
        .resolve()
        .expect("resolves");
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            resolved.apis[0].base_url.as_deref(),
            Some("http://localhost:8080")
        );
        assert!(resolved.apis[0].read_only, "--read-only can only restrict");
    }

    #[test]
    fn the_api_flag_selects_entries_by_name() {
        let path = std::env::temp_dir().join(format!("oam-select-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[api]]\nname = \"one\"\nspec = \"a.json\"\n\n[[api]]\nname = \"two\"\nspec = \"b.json\"\n",
        )
        .expect("writes");
        let resolve = |wanted: &[&str]| {
            ConfigArgs {
                config: Some(path.display().to_string()),
                api: wanted.iter().map(|s| s.to_string()).collect(),
                ..ConfigArgs::default()
            }
            .resolve()
        };

        let config = resolve(&["two"]).expect("resolves");
        assert_eq!(config.apis.len(), 1);
        assert_eq!(config.apis[0].label, "two");

        let err = resolve(&["three"]).expect_err("rejected");
        let message = err.to_string();
        let _ = std::fs::remove_file(&path);
        assert!(
            message.contains("one, two"),
            "it lists the real names: {message}"
        );
    }

    #[test]
    fn an_env_reference_is_expanded_and_a_missing_one_is_an_error() {
        unsafe { std::env::set_var("CONFIG_EXPANSION_TEST_TOKEN", "s3cret") };
        let config = config_from(
            r#"
            [[api]]
            spec = "a.json"
            base_url = "https://x.example.com"
            token = "${CONFIG_EXPANSION_TEST_TOKEN}"
            headers = { "X-Env" = "${CONFIG_EXPANSION_TEST_MISSING:-fallback}" }
        "#,
        );
        assert_eq!(config.apis[0].headers[0].1, "Bearer s3cret");
        assert_eq!(config.apis[0].headers[1].1, "fallback");

        let err = try_config_from(
            "[[api]]\nspec = \"a.json\"\ntoken = \"${CONFIG_EXPANSION_TEST_DEFINITELY_MISSING}\"\n",
        )
        .expect_err("rejected");
        assert!(
            format!("{err:#}").contains("CONFIG_EXPANSION_TEST_DEFINITELY_MISSING"),
            "got: {err:#}"
        );
    }

    #[test]
    fn a_misspelled_key_is_rejected_rather_than_silently_ignored() {
        let err = try_config_from("[[api]]\nspec = \"a.json\"\nbase_ur = \"typo\"\n")
            .expect_err("rejected");
        assert!(format!("{err:#}").contains("base_ur"), "got: {err:#}");
    }

    #[test]
    fn an_api_entry_without_a_spec_names_itself() {
        let err = try_config_from("[[api]]\nname = \"nospec\"\nbase_url = \"https://x\"\n")
            .expect_err("rejected");
        assert!(format!("{err:#}").contains("nospec"), "got: {err:#}");
    }

    #[test]
    fn a_flag_spec_is_served_alongside_the_files_apis() {
        let path = std::env::temp_dir().join(format!("oam-both-{}.toml", std::process::id()));
        std::fs::write(&path, "[[api]]\nname = \"from-file\"\nspec = \"a.json\"\n")
            .expect("writes");
        let config = ConfigArgs {
            config: Some(path.display().to_string()),
            specs: vec!["from-flag.json".into()],
            ..ConfigArgs::default()
        }
        .resolve()
        .expect("resolves");
        let _ = std::fs::remove_file(&path);

        let labels: Vec<&str> = config.apis.iter().map(|api| api.label.as_str()).collect();
        assert_eq!(labels, vec!["from-file", "from-flag.json"]);
    }

    #[test]
    fn an_explicit_config_that_does_not_exist_is_an_error() {
        let err = ConfigArgs {
            config: Some("/nonexistent/openapi-as-mcp.toml".into()),
            specs: vec!["a.json".into()],
            ..ConfigArgs::default()
        }
        .resolve()
        .expect_err("rejected");
        assert!(err.to_string().contains("does not exist"), "got: {err}");
    }

    #[test]
    fn two_authorization_headers_collapse_to_the_most_specific_one() {
        assert_eq!(
            dedupe_headers(vec![
                ("Authorization".into(), "Bearer shared".into()),
                ("authorization".into(), "Bearer own".into()),
                ("X-Keep".into(), "1".into()),
            ]),
            vec![
                ("Authorization".to_string(), "Bearer own".to_string()),
                ("X-Keep".to_string(), "1".to_string()),
            ]
        );
    }

    /// A config with the two environments the groups exist for, one name reused across both.
    const GROUPED: &str = r#"
        timeout = 30

        [[api]]
        name = "shared"
        spec = "shared.json"

        [group.prod]
        base_url = "https://prod.internal"
        tool_prefix = "prod_"

        [[group.prod.api]]
        name = "recipes"
        spec = "prod-recipes.json"

        [[group.prod.api]]
        name = "billing"
        spec = "prod-billing.json"
        base_url = "https://billing.prod.internal"

        [group.stg]
        base_url = "https://stg.internal"

        [[group.stg.api]]
        name = "recipes"
        spec = "stg-recipes.json"
    "#;

    fn labels(config: &Config) -> Vec<String> {
        config.apis.iter().map(|api| api.label.clone()).collect()
    }

    #[test]
    fn every_group_is_served_together_and_labelled_by_group() {
        let config = config_from(GROUPED);
        assert_eq!(
            labels(&config),
            ["shared", "prod/recipes", "prod/billing", "stg/recipes"],
            "ungrouped entries first, then each group; a reused name stays distinct"
        );
    }

    #[test]
    fn a_group_supplies_defaults_between_the_entry_and_the_file() {
        let config = config_from(GROUPED);
        let api = |label: &str| {
            config
                .apis
                .iter()
                .find(|api| api.label == label)
                .unwrap_or_else(|| panic!("{label} is served"))
        };

        assert_eq!(
            api("prod/recipes").base_url.as_deref(),
            Some("https://prod.internal"),
            "the group's base_url reaches an entry that sets none"
        );
        assert_eq!(
            api("prod/billing").base_url.as_deref(),
            Some("https://billing.prod.internal"),
            "the entry still wins over its group"
        );
        assert_eq!(api("prod/recipes").tool_prefix.as_deref(), Some("prod_"));
        assert_eq!(api("stg/recipes").tool_prefix, None, "groups do not leak");
        assert_eq!(api("shared").base_url, None, "nor reach an ungrouped entry");
        assert_eq!(
            api("prod/recipes").timeout,
            Duration::from_secs(30),
            "the file's top-level defaults still apply inside a group"
        );
    }

    #[test]
    fn group_narrows_the_run_to_whole_sets() {
        let config = try_config_from_args(
            GROUPED,
            ConfigArgs {
                groups: vec!["prod".into()],
                ..ConfigArgs::default()
            },
        )
        .expect("resolves");
        assert_eq!(
            labels(&config),
            ["prod/recipes", "prod/billing"],
            "--group drops the other groups and the ungrouped entries"
        );
    }

    #[test]
    fn an_unknown_group_lists_the_ones_there_are() {
        let err = try_config_from_args(
            GROUPED,
            ConfigArgs {
                groups: vec!["dev".into()],
                ..ConfigArgs::default()
            },
        )
        .expect_err("rejected");
        assert!(err.to_string().contains("prod, stg"), "got: {err}");
    }

    #[test]
    fn api_selects_a_grouped_entry_by_either_spelling() {
        let qualified = try_config_from_args(
            GROUPED,
            ConfigArgs {
                api: vec!["prod/recipes".into()],
                ..ConfigArgs::default()
            },
        )
        .expect("resolves");
        assert_eq!(labels(&qualified), ["prod/recipes"]);

        let bare = try_config_from_args(
            GROUPED,
            ConfigArgs {
                api: vec!["recipes".into()],
                ..ConfigArgs::default()
            },
        )
        .expect("resolves");
        assert_eq!(
            labels(&bare),
            ["prod/recipes", "stg/recipes"],
            "the bare name reaches the entry of that name in every group"
        );
    }

    #[test]
    fn a_group_may_not_contain_another_group() {
        let err = try_config_from(
            r#"
            [group.prod]
            spec = "a.json"

            [group.prod.group.inner]
            spec = "b.json"
        "#,
        )
        .expect_err("rejected");
        assert!(err.to_string().contains("nested"), "got: {err}");
    }

    #[test]
    fn a_groups_token_command_is_inherited_and_overridable() {
        let config = config_from(
            r#"
            [group.prod]
            token_command = "echo group"

            [[group.prod.api]]
            name = "inherits"
            spec = "a.json"

            [[group.prod.api]]
            name = "own"
            spec = "b.json"
            token = "own"
        "#,
        );
        assert_eq!(config.apis[0].token_command.as_deref(), Some("echo group"));
        assert_eq!(config.apis[1].token_command, None);
        assert_eq!(
            config.apis[1].headers,
            vec![("Authorization".to_string(), "Bearer own".to_string())]
        );
    }
}
