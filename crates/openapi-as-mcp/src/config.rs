//! Configuration: which specs to serve, where to send the requests, and what to send with them.
//!
//! Everything is a flag with an `OAM_*` environment fallback, so the same invocation works from a
//! shell and from an MCP client entry that can only set `env`.

use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;

use crate::spec::Source;

/// Default ceiling on a response body we will hand back to the model. Large enough for a page of
/// JSON, small enough that one accidental "list everything" call cannot fill a context window.
const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Configuration")]
pub struct ConfigArgs {
    /// OpenAPI document to serve: a file path or an http(s) URL. Repeatable.
    #[arg(
        long = "spec",
        short = 's',
        value_name = "PATH_OR_URL",
        env = "OAM_SPEC",
        value_delimiter = ',',
        // Not `required`: clap forbids that on a global argument, and `resolve` reports the
        // omission with a message that also names the environment variable.
        global = true
    )]
    pub specs: Vec<String>,

    /// Base URL for the requests. Defaults to the first `servers[].url` in the document.
    #[arg(
        long = "base-url",
        value_name = "URL",
        env = "OAM_BASE_URL",
        global = true
    )]
    pub base_url: Option<String>,

    /// Bearer token, sent as `Authorization: Bearer <token>`.
    #[arg(long, value_name = "TOKEN", env = "OAM_TOKEN", global = true)]
    pub token: Option<String>,

    /// Extra request header, `Name: value`. Repeatable.
    #[arg(long = "header", short = 'H', value_name = "HEADER", global = true)]
    pub headers: Vec<String>,

    /// Same as --header, as one `Name: value;Name: value` string, for MCP client `env` blocks.
    #[arg(
        long = "headers",
        value_name = "HEADERS",
        env = "OAM_HEADERS",
        global = true
    )]
    pub headers_env: Option<String>,

    /// Expose only GET/HEAD/OPTIONS operations. Use against an API you do not want written to.
    #[arg(long = "read-only", env = "OAM_READ_ONLY", global = true)]
    pub read_only: bool,

    /// Keep only operations whose tool name, method or path matches this regex. Repeatable.
    #[arg(
        long = "include",
        value_name = "REGEX",
        env = "OAM_INCLUDE",
        global = true
    )]
    pub include: Vec<String>,

    /// Drop operations whose tool name, method or path matches this regex. Applied after
    /// --include. Repeatable.
    #[arg(
        long = "exclude",
        value_name = "REGEX",
        env = "OAM_EXCLUDE",
        global = true
    )]
    pub exclude: Vec<String>,

    /// Prefix every tool name with this, to keep two servers apart in one client.
    #[arg(
        long = "tool-prefix",
        value_name = "PREFIX",
        env = "OAM_TOOL_PREFIX",
        global = true
    )]
    pub tool_prefix: Option<String>,

    /// Per-request timeout in seconds.
    #[arg(long, value_name = "SECONDS", env = "OAM_TIMEOUT", global = true)]
    pub timeout: Option<u64>,

    /// Ceiling on the response bytes one tool call may return.
    #[arg(long, value_name = "N", env = "OAM_MAX_RESPONSE_BYTES", global = true)]
    pub max_response_bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub specs: Vec<Source>,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub read_only: bool,
    pub include: Vec<regex::Regex>,
    pub exclude: Vec<regex::Regex>,
    pub tool_prefix: Option<String>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

impl ConfigArgs {
    pub fn resolve(&self) -> anyhow::Result<Config> {
        if self.specs.is_empty() {
            bail!("no OpenAPI document given; pass --spec <path-or-url> or set OAM_SPEC");
        }

        let mut headers = Vec::new();
        if let Some(token) = self.token.as_ref().filter(|t| !t.trim().is_empty()) {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        for raw in self
            .headers
            .iter()
            .map(String::as_str)
            .chain(self.headers_env.iter().flat_map(|s| s.split(';')))
            .filter(|raw| !raw.trim().is_empty())
        {
            headers.push(parse_header(raw)?);
        }

        Ok(Config {
            specs: self.specs.iter().map(|raw| Source::parse(raw)).collect(),
            // A base URL with a trailing slash and a path starting with one would produce `//`,
            // which some gateways route differently from the intended path.
            base_url: self
                .base_url
                .as_ref()
                .map(|url| url.trim_end_matches('/').to_string()),
            headers,
            read_only: self.read_only,
            include: compile(&self.include, "--include")?,
            exclude: compile(&self.exclude, "--exclude")?,
            tool_prefix: self.tool_prefix.clone(),
            timeout: Duration::from_secs(self.timeout.unwrap_or(60)),
            max_response_bytes: self
                .max_response_bytes
                .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES),
        })
    }
}

#[cfg(test)]
impl ConfigArgs {
    /// Defaults without the `--spec` requirement, for tests that only need the request settings.
    pub fn resolve_for_test(mut self) -> Config {
        self.specs = vec!["unused.json".into()];
        self.resolve().expect("default config resolves")
    }
}

fn compile(patterns: &[String], flag: &str) -> anyhow::Result<Vec<regex::Regex>> {
    patterns
        .iter()
        .map(|pattern| {
            regex::Regex::new(pattern)
                .with_context(|| format!("{flag} {pattern:?} is not a valid regex"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> ConfigArgs {
        ConfigArgs {
            specs: vec!["spec.json".into()],
            ..ConfigArgs::default()
        }
    }

    #[test]
    fn a_token_becomes_a_bearer_header() {
        let config = ConfigArgs {
            token: Some("abc".into()),
            ..args()
        }
        .resolve()
        .expect("resolves");
        assert_eq!(
            config.headers,
            vec![("Authorization".to_string(), "Bearer abc".to_string())]
        );
    }

    #[test]
    fn headers_come_from_both_the_flag_and_the_env_string() {
        let config = ConfigArgs {
            headers: vec!["X-A: 1".into()],
            headers_env: Some("X-B: 2;X-C: 3".into()),
            ..args()
        }
        .resolve()
        .expect("resolves");
        let names: Vec<&str> = config.headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["X-A", "X-B", "X-C"]);
        assert_eq!(config.headers[1].1, "2", "the value is trimmed");
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
        let config = ConfigArgs {
            base_url: Some("https://api.example.com/v1/".into()),
            ..args()
        }
        .resolve()
        .expect("resolves");
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
    }

    #[test]
    fn a_bad_filter_regex_names_the_flag() {
        let err = ConfigArgs {
            exclude: vec!["[".into()],
            ..args()
        }
        .resolve()
        .expect_err("rejected");
        assert!(err.to_string().contains("--exclude"), "got: {err}");
    }

    #[test]
    fn a_url_spec_is_told_apart_from_a_path() {
        let config = ConfigArgs {
            specs: vec!["https://x/openapi.json".into(), "./local.yaml".into()],
            ..args()
        }
        .resolve()
        .expect("resolves");
        assert!(matches!(config.specs[0], Source::Url(_)));
        assert!(matches!(config.specs[1], Source::File(_)));
    }
}
