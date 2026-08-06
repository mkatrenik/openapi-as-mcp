//! Shared HTTP client for the api-gateway.
//!
//! Errors are typed so that "the gateway wants a token" can never be mistaken for "that run does
//! not exist" — the Phase 0 spike showed every unauthenticated call returns 401.

use std::time::Duration;

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::config::Config;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(
        "the api-gateway requires authentication (HTTP {status}). Set RRD_TOKEN=<bearer token> \
         or RRD_EXTRA_HEADERS=\"Authorization: Bearer …\" and restart the MCP server."
    )]
    AuthRequired { status: StatusCode },

    #[error("{what} not found")]
    NotFound { what: String },

    #[error("gateway returned HTTP {status} for {url}: {body}")]
    Upstream {
        status: StatusCode,
        url: String,
        body: String,
    },

    #[error("could not reach the gateway at {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("gateway response for {url} did not match the expected schema: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    config: Config,
}

impl HttpClient {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("recipe-run-debug-mcp/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The auth seam: every outgoing request passes through here.
    fn authed(&self, builder: RequestBuilder) -> RequestBuilder {
        let mut builder = builder;
        for (name, value) in self.config.auth.headers() {
            builder = builder.header(name, value);
        }
        builder
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        let text = self.get_text(url, query).await?;
        serde_json::from_str(&text).map_err(|source| ApiError::Decode {
            url: url.to_string(),
            source,
        })
    }

    pub async fn get_text(&self, url: &str, query: &[(&str, String)]) -> Result<String, ApiError> {
        let request = self.authed(self.client.get(url).query(query));

        tracing::debug!(%url, ?query, "gateway request");

        let response = request.send().await.map_err(|source| ApiError::Transport {
            url: url.to_string(),
            source,
        })?;

        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::AuthRequired { status });
        }

        let body = response
            .text()
            .await
            .map_err(|source| ApiError::Transport {
                url: url.to_string(),
                source,
            })?;

        if !status.is_success() {
            return Err(ApiError::Upstream {
                status,
                url: url.to_string(),
                body: truncate(&body, 2_000),
            });
        }

        Ok(body)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… (truncated)", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Auth, Env, Priority};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(base: &str) -> HttpClient {
        HttpClient::new(Config {
            env: Env::Development,
            bucket: "bucket".into(),
            gateway_base_url: base.trim_end_matches('/').to_string(),
            priority: Priority::Hp,
            auth: Auth::default(),
            max_file_bytes: 1024,
            source_path: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn unauthorized_maps_to_auth_required() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/thing"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "detail": "Authentication required"
            })))
            .mount(&server)
            .await;

        let err = client_for(&server.uri())
            .get_text(&format!("{}/thing", server.uri()), &[])
            .await
            .expect_err("should fail");

        assert!(matches!(err, ApiError::AuthRequired { .. }));
        assert!(
            err.to_string().contains("RRD_TOKEN"),
            "message must tell the operator how to fix it, got: {err}"
        );
    }

    #[tokio::test]
    async fn server_error_keeps_body_for_diagnosis() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/thing"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let err = client_for(&server.uri())
            .get_text(&format!("{}/thing", server.uri()), &[])
            .await
            .expect_err("should fail");

        assert!(err.to_string().contains("boom"), "got: {err}");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "héllo wörld";
        let out = truncate(s, 2);
        assert!(out.starts_with('h'), "got {out}");
        assert!(out.contains("truncated"));
    }
}
