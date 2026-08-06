//! Configuration, from a TOML file and the environment.
//!
//! A stdio server has no interactive surface, so settings come from two places: a persistent
//! `~/.config/recipe-run-debug/config.toml` for the things an operator sets once (gateway URL,
//! environment), and env vars, which override the file so a single MCP client entry can point the
//! same install at another environment. Port of the UI's `apps/ui/src/config.ts` (bucket names,
//! Grafana/Kibana hosts, `-hp` endpoint selection).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "no gateway base URL: set RRD_GATEWAY_BASE_URL, or `gateway_base_url` in {} \
         (e.g. https://api-gateway-service.k8s.euw1.dev.gcp.ivxs.uk)",
        config_path_hint()
    )]
    MissingGatewayUrl,
    #[error("{key} must be one of local|development|staging|production, got {value:?}")]
    UnknownEnv { key: &'static str, value: String },
    #[error("{key} must be hp or lp, got {value:?}")]
    UnknownPriority { key: &'static str, value: String },
    #[error("RRD_EXTRA_HEADERS entry {0:?} is not in `Name: value` form")]
    MalformedHeader(String),
    #[error("cannot read {}: {source}", path.display())]
    UnreadableFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not valid TOML: {source}", path.display())]
    MalformedFile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Local,
    Development,
    Staging,
    Production,
}

impl Env {
    /// `key` names where the value came from (env var or file field) so the error is actionable.
    fn parse(raw: &str, key: &'static str) -> Result<Self, ConfigError> {
        match raw {
            "local" => Ok(Self::Local),
            "development" | "dev" => Ok(Self::Development),
            "staging" | "stg" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            other => Err(ConfigError::UnknownEnv {
                key,
                value: other.to_string(),
            }),
        }
    }

    /// Artefact bucket per environment — mirrors `BUCKET` in the UI's config.ts.
    pub fn bucket(self) -> &'static str {
        match self {
            Self::Local | Self::Development => "ca-gcp-agent-platform-artefacts",
            Self::Staging => "ca-gcp-agent-platform-artefacts-stg",
            Self::Production => "ca-gcp-agent-platform-artefacts-prod",
        }
    }

    /// Grafana only distinguishes development vs production.
    pub fn grafana_env(self) -> &'static str {
        match self {
            Self::Production => "production",
            _ => "development",
        }
    }

    /// Kibana only distinguishes staging vs production.
    pub fn kibana_env(self) -> &'static str {
        match self {
            Self::Production => "production",
            _ => "staging",
        }
    }
}

impl fmt::Display for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Local => "local",
            Self::Development => "development",
            Self::Staging => "staging",
            Self::Production => "production",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// High priority — the aggregator/trigger `-hp` endpoint variants.
    Hp,
    Lp,
}

impl Priority {
    fn parse(raw: &str, key: &'static str) -> Result<Self, ConfigError> {
        match raw {
            "hp" => Ok(Self::Hp),
            "lp" => Ok(Self::Lp),
            other => Err(ConfigError::UnknownPriority {
                key,
                value: other.to_string(),
            }),
        }
    }
}

/// The on-disk half of the configuration: `~/.config/recipe-run-debug/config.toml`.
///
/// Unknown keys are rejected rather than ignored — a typo in a file you edit once and forget is
/// otherwise indistinguishable from a setting that does not work.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    env: Option<String>,
    gateway_base_url: Option<String>,
    priority_endpoint: Option<String>,
    bucket_override: Option<String>,
    token: Option<String>,
    max_file_bytes: Option<usize>,
    /// Sent verbatim on every request. `token` is the shorthand for `Authorization`.
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl FileConfig {
    /// Where the file is looked for: `$RRD_CONFIG`, else `$XDG_CONFIG_HOME` or `~/.config`.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(explicit) = non_empty("RRD_CONFIG") {
            return Some(PathBuf::from(explicit));
        }
        let base = non_empty("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| non_empty("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("recipe-run-debug").join("config.toml"))
    }

    /// Reads `path`. A missing file is not an error — the env vars alone are a valid setup.
    fn load(path: &Path) -> Result<Option<Self>, ConfigError> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ConfigError::UnreadableFile {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        toml::from_str(&raw)
            .map(Some)
            .map_err(|source| ConfigError::MalformedFile {
                path: path.to_path_buf(),
                source,
            })
    }
}

/// The auth seam. v1 has no token lifecycle by design: whatever headers the operator supplies are
/// passed through verbatim. Adding real auth means adding a variant here and nothing else.
#[derive(Debug, Clone, Default)]
pub struct Auth {
    headers: BTreeMap<String, String>,
}

impl Auth {
    /// File headers first, then the env ones on top: an `Authorization` from `RRD_TOKEN` replaces
    /// the stored token rather than being merged with it.
    fn resolve(
        file: &FileConfig,
        get: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut headers: BTreeMap<String, String> = file
            .headers
            .iter()
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();

        // Convenience: a bare token becomes a bearer header. Still no refresh, no storage.
        for token in [file.token.as_deref().map(str::to_string), get("RRD_TOKEN")]
            .into_iter()
            .flatten()
            .filter(|t| !t.trim().is_empty())
        {
            let token = token.trim();
            let token = token.strip_prefix("Bearer ").unwrap_or(token);
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }

        if let Some(raw) = get("RRD_EXTRA_HEADERS") {
            for entry in raw.split(';').filter(|e| !e.trim().is_empty()) {
                let (name, value) = entry
                    .split_once(':')
                    .ok_or_else(|| ConfigError::MalformedHeader(entry.to_string()))?;
                headers.insert(name.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(Self { headers })
    }

    pub fn headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Whether any auth was supplied — reported at startup so a wall of 401s is self-explanatory.
    pub fn is_configured(&self) -> bool {
        !self.headers.is_empty()
    }
}

/// Settings supplied on the command line. They sit *above* the environment in the precedence
/// chain — flag, then env var, then file, then default — and are keyed by the env var each flag
/// mirrors so that parsing and precedence stay in one place.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// Config file to read instead of the default location. Unlike the default, a path named
    /// explicitly here must exist.
    pub config_path: Option<PathBuf>,
    pub env: Option<String>,
    pub gateway_base_url: Option<String>,
    pub priority_endpoint: Option<String>,
    pub bucket: Option<String>,
    pub token: Option<String>,
    /// `Name: value` entries joined with `;`, in the `RRD_EXTRA_HEADERS` format. Replaces that
    /// variable rather than merging with it.
    pub extra_headers: Option<String>,
    pub max_file_bytes: Option<usize>,
}

impl Overrides {
    fn lookup(&self, key: &str) -> Option<String> {
        match key {
            "RRD_ENV" => self.env.clone(),
            "RRD_GATEWAY_BASE_URL" => self.gateway_base_url.clone(),
            "RRD_PRIORITY_ENDPOINT" => self.priority_endpoint.clone(),
            "RRD_BUCKET_OVERRIDE" => self.bucket.clone(),
            "RRD_TOKEN" => self.token.clone(),
            "RRD_EXTRA_HEADERS" => self.extra_headers.clone(),
            "RRD_MAX_FILE_BYTES" => self.max_file_bytes.map(|n| n.to_string()),
            _ => None,
        }
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    }

    /// The lookup `resolve` sees: an override if there is one, else `base` (the process env).
    fn layered<'a>(
        &'a self,
        base: &'a dyn Fn(&str) -> Option<String>,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| self.lookup(key).or_else(|| base(key))
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub env: Env,
    pub bucket: String,
    pub gateway_base_url: String,
    pub priority: Priority,
    pub auth: Auth,
    /// Hard ceiling on bytes of file content returned by a single tool call.
    pub max_file_bytes: usize,
    /// The config file that contributed, if one was found — logged at startup so it is obvious
    /// which file the running server actually read.
    pub source_path: Option<PathBuf>,
}

impl Config {
    /// Loads the config file (if any), layers the environment on top, then the command line on top
    /// of that. Pass `&Overrides::default()` for the environment-only behaviour.
    pub fn load(overrides: &Overrides) -> Result<Self, ConfigError> {
        let explicit = overrides.config_path.is_some();
        let path = overrides
            .config_path
            .clone()
            .or_else(FileConfig::default_path);

        let file = match path.as_deref() {
            Some(path) => match FileConfig::load(path)? {
                Some(file) => Some(file),
                // A missing default file is a normal setup; a missing `--config-file` is a typo.
                None if explicit => {
                    return Err(ConfigError::UnreadableFile {
                        path: path.to_path_buf(),
                        source: std::io::Error::from(std::io::ErrorKind::NotFound),
                    });
                }
                None => None,
            },
            None => None,
        };
        let source_path = file.is_some().then_some(path).flatten();

        Self::resolve(
            file.unwrap_or_default(),
            source_path,
            &overrides.layered(&non_empty),
        )
    }

    /// The precedence rule in one place: env var, else file, else default. `get` is the env lookup,
    /// injected so this is testable without mutating the process environment.
    fn resolve(
        file: FileConfig,
        source_path: Option<PathBuf>,
        get: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let env = match pick(get, "RRD_ENV", &file.env, "`env` in the config file") {
            Some((raw, key)) => Env::parse(&raw, key)?,
            None => Env::Development,
        };

        let gateway_base_url = pick(
            get,
            "RRD_GATEWAY_BASE_URL",
            &file.gateway_base_url,
            "`gateway_base_url` in the config file",
        )
        .ok_or(ConfigError::MissingGatewayUrl)?
        .0
        .trim_end_matches('/')
        .to_string();

        let priority = match pick(
            get,
            "RRD_PRIORITY_ENDPOINT",
            &file.priority_endpoint,
            "`priority_endpoint` in the config file",
        ) {
            Some((raw, key)) => Priority::parse(&raw, key)?,
            None => Priority::Hp,
        };

        let bucket = pick(
            get,
            "RRD_BUCKET_OVERRIDE",
            &file.bucket_override,
            "`bucket_override` in the config file",
        )
        .map(|(raw, _)| raw)
        .unwrap_or_else(|| env.bucket().to_string());

        let max_file_bytes = get("RRD_MAX_FILE_BYTES")
            .and_then(|v| v.parse().ok())
            .or(file.max_file_bytes)
            .unwrap_or(256 * 1024);

        let auth = Auth::resolve(&file, get)?;

        Ok(Self {
            env,
            bucket,
            gateway_base_url,
            priority,
            auth,
            max_file_bytes,
            source_path,
        })
    }

    /// Prefix for the reciperun-aggregator service, including the `-hp` variant selection the UI
    /// performs in `Platform.resolvePriorityEndpoint`. Non-production environments have no `-hp`.
    pub fn aggregator_prefix(&self) -> String {
        self.priority_prefix("/api/reciperun-aggregator")
    }

    pub fn recipe_store_prefix(&self) -> String {
        format!("{}/api/recipe-store", self.gateway_base_url)
    }

    fn priority_prefix(&self, path: &str) -> String {
        let suffix = if self.env == Env::Production && self.priority == Priority::Hp {
            "-hp"
        } else {
            ""
        };
        format!("{}{path}{suffix}", self.gateway_base_url)
    }

    /// `GET /gcs/file/{path}` on the gateway. The gateway resolves the bucket itself, so `path`
    /// must be bucket-less.
    pub fn gcs_file_url(&self, bucketless_path: &str) -> String {
        format!(
            "{}/gcs/file/{}",
            self.gateway_base_url,
            urlencoding::encode(bucketless_path)
        )
    }
}

fn non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Env var wins over file value; the returned key names whichever source supplied it, so a parse
/// error can point at the thing the operator has to edit.
fn pick(
    get: &impl Fn(&str) -> Option<String>,
    env_key: &'static str,
    file_value: &Option<String>,
    file_key: &'static str,
) -> Option<(String, &'static str)> {
    if let Some(value) = get(env_key) {
        return Some((value, env_key));
    }
    file_value
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(|v| (v, file_key))
}

/// For the "you have not configured a gateway" error: name the file the operator would create.
fn config_path_hint() -> String {
    FileConfig::default_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/recipe-run-debug/config.toml".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(env: Env, priority: Priority) -> Config {
        Config {
            env,
            bucket: env.bucket().to_string(),
            gateway_base_url: "https://gw.example".to_string(),
            priority,
            auth: Auth::default(),
            max_file_bytes: 1024,
            source_path: None,
        }
    }

    /// Resolve against a file body plus an explicit env map — no process env mutation, so these
    /// tests stay independent of each other and of the machine they run on.
    fn resolve(toml_body: &str, env: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let file: FileConfig = toml::from_str(toml_body).expect("test fixture parses");
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::resolve(file, None, &move |key: &str| env.get(key).cloned())
    }

    #[test]
    fn buckets_match_the_ui() {
        assert_eq!(Env::Development.bucket(), "ca-gcp-agent-platform-artefacts");
        assert_eq!(Env::Local.bucket(), "ca-gcp-agent-platform-artefacts");
        assert_eq!(Env::Staging.bucket(), "ca-gcp-agent-platform-artefacts-stg");
        assert_eq!(
            Env::Production.bucket(),
            "ca-gcp-agent-platform-artefacts-prod"
        );
    }

    #[test]
    fn hp_suffix_applies_only_in_production() {
        assert_eq!(
            config(Env::Production, Priority::Hp).aggregator_prefix(),
            "https://gw.example/api/reciperun-aggregator-hp"
        );
        assert_eq!(
            config(Env::Production, Priority::Lp).aggregator_prefix(),
            "https://gw.example/api/reciperun-aggregator"
        );
        assert_eq!(
            config(Env::Development, Priority::Hp).aggregator_prefix(),
            "https://gw.example/api/reciperun-aggregator"
        );
    }

    #[test]
    fn gcs_path_is_url_encoded() {
        let cfg = config(Env::Development, Priority::Hp);
        assert_eq!(
            cfg.gcs_file_url("agents/foo/bar/config.yaml"),
            "https://gw.example/gcs/file/agents%2Ffoo%2Fbar%2Fconfig.yaml"
        );
    }

    #[test]
    fn env_aliases_parse() {
        assert_eq!(Env::parse("dev", "RRD_ENV").unwrap(), Env::Development);
        assert_eq!(Env::parse("prod", "RRD_ENV").unwrap(), Env::Production);
        assert!(Env::parse("nope", "RRD_ENV").is_err());
    }

    #[test]
    fn the_config_file_alone_is_enough() {
        let cfg = resolve(
            r#"
            env = "staging"
            gateway_base_url = "https://gw.example/"
            priority_endpoint = "lp"
            max_file_bytes = 42
            token = "abc"
            "#,
            &[],
        )
        .unwrap();

        assert_eq!(cfg.env, Env::Staging);
        // The trailing slash is trimmed here as it is for the env var.
        assert_eq!(cfg.gateway_base_url, "https://gw.example");
        assert_eq!(cfg.priority, Priority::Lp);
        assert_eq!(cfg.bucket, Env::Staging.bucket());
        assert_eq!(cfg.max_file_bytes, 42);
        assert_eq!(
            cfg.auth.headers().collect::<Vec<_>>(),
            [("Authorization", "Bearer abc")]
        );
    }

    #[test]
    fn env_vars_override_the_file() {
        let cfg = resolve(
            r#"
            env = "production"
            gateway_base_url = "https://stored.example"
            priority_endpoint = "hp"
            bucket_override = "stored-bucket"
            max_file_bytes = 42
            token = "stored"
            "#,
            &[
                ("RRD_ENV", "dev"),
                ("RRD_GATEWAY_BASE_URL", "https://override.example"),
                ("RRD_PRIORITY_ENDPOINT", "lp"),
                ("RRD_BUCKET_OVERRIDE", "override-bucket"),
                ("RRD_MAX_FILE_BYTES", "7"),
                ("RRD_TOKEN", "override"),
            ],
        )
        .unwrap();

        assert_eq!(cfg.env, Env::Development);
        assert_eq!(cfg.gateway_base_url, "https://override.example");
        assert_eq!(cfg.priority, Priority::Lp);
        assert_eq!(cfg.bucket, "override-bucket");
        assert_eq!(cfg.max_file_bytes, 7);
        assert_eq!(
            cfg.auth.headers().collect::<Vec<_>>(),
            [("Authorization", "Bearer override")]
        );
    }

    /// As `resolve`, with CLI overrides layered on top — the same composition `Config::load` uses.
    fn resolve_with(
        overrides: &Overrides,
        toml_body: &str,
        env: &[(&str, &str)],
    ) -> Result<Config, ConfigError> {
        let file: FileConfig = toml::from_str(toml_body).expect("test fixture parses");
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let base = move |key: &str| env.get(key).cloned();
        Config::resolve(file, None, &overrides.layered(&base))
    }

    #[test]
    fn cli_overrides_beat_env_vars_and_the_file() {
        let overrides = Overrides {
            env: Some("staging".to_string()),
            gateway_base_url: Some("https://flag.example/".to_string()),
            priority_endpoint: Some("lp".to_string()),
            bucket: Some("flag-bucket".to_string()),
            token: Some("flag-token".to_string()),
            max_file_bytes: Some(11),
            ..Overrides::default()
        };

        let cfg = resolve_with(
            &overrides,
            r#"
            env = "local"
            gateway_base_url = "https://stored.example"
            token = "stored"
            "#,
            &[
                ("RRD_ENV", "prod"),
                ("RRD_GATEWAY_BASE_URL", "https://env.example"),
                ("RRD_PRIORITY_ENDPOINT", "hp"),
                ("RRD_BUCKET_OVERRIDE", "env-bucket"),
                ("RRD_TOKEN", "env-token"),
                ("RRD_MAX_FILE_BYTES", "7"),
            ],
        )
        .unwrap();

        assert_eq!(cfg.env, Env::Staging);
        assert_eq!(cfg.gateway_base_url, "https://flag.example");
        assert_eq!(cfg.priority, Priority::Lp);
        assert_eq!(cfg.bucket, "flag-bucket");
        assert_eq!(cfg.max_file_bytes, 11);
        assert_eq!(
            cfg.auth.headers().collect::<Vec<_>>(),
            [("Authorization", "Bearer flag-token")]
        );
    }

    #[test]
    fn unset_overrides_fall_through_to_the_environment() {
        let cfg = resolve_with(
            &Overrides::default(),
            "",
            &[("RRD_GATEWAY_BASE_URL", "https://env.example")],
        )
        .unwrap();

        assert_eq!(cfg.gateway_base_url, "https://env.example");
        assert_eq!(cfg.env, Env::Development);
    }

    #[test]
    fn header_flags_replace_the_env_variable() {
        let overrides = Overrides {
            extra_headers: Some("X-Flag: 1;X-Both: flag".to_string()),
            ..Overrides::default()
        };

        let cfg = resolve_with(
            &overrides,
            r#"
            gateway_base_url = "https://gw.example"

            [headers]
            "X-From-File" = "1"
            "#,
            &[("RRD_EXTRA_HEADERS", "X-Both: env; X-From-Env: 2")],
        )
        .unwrap();

        assert_eq!(
            cfg.auth.headers().collect::<Vec<_>>(),
            [("X-Both", "flag"), ("X-Flag", "1"), ("X-From-File", "1")]
        );
    }

    #[test]
    fn file_headers_merge_with_env_headers() {
        let cfg = resolve(
            r#"
            gateway_base_url = "https://gw.example"

            [headers]
            "X-From-File" = "1"
            "X-Both" = "file"
            "#,
            &[("RRD_EXTRA_HEADERS", "X-Both: env; X-From-Env: 2")],
        )
        .unwrap();

        assert_eq!(
            cfg.auth.headers().collect::<Vec<_>>(),
            [("X-Both", "env"), ("X-From-Env", "2"), ("X-From-File", "1")]
        );
    }

    #[test]
    fn a_missing_gateway_url_is_still_an_error() {
        assert!(matches!(
            resolve("env = \"dev\"", &[]),
            Err(ConfigError::MissingGatewayUrl)
        ));
    }

    #[test]
    fn a_bad_file_value_names_the_file_field() {
        let err = resolve("env = \"nope\"", &[]).unwrap_err().to_string();
        assert!(err.contains("`env` in the config file"), "{err}");
    }

    #[test]
    fn unknown_file_keys_are_rejected() {
        let err = toml::from_str::<FileConfig>("gatewy_base_url = \"typo\"").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = std::env::temp_dir().join("recipe-run-debug-does-not-exist/config.toml");
        assert!(FileConfig::load(&path).unwrap().is_none());
    }

    #[test]
    fn a_malformed_file_names_the_path() {
        let path = std::env::temp_dir().join("rrd-malformed-config.toml");
        std::fs::write(&path, "this is not toml").unwrap();
        let err = FileConfig::load(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("is not valid TOML"), "{err}");
        assert!(err.contains("rrd-malformed-config.toml"), "{err}");
    }
}
