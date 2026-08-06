//! Artefact file access.
//!
//! v1 reads through the api-gateway's `GET /gcs/file/{path}` (bucket-less path; the gateway
//! resolves the bucket). That endpoint has no range support, so the whole object is pulled into
//! *this process* — but only the requested slice ever reaches the agent, which is the point.
//!
//! `FileStore` is a trait so a direct-GCS backend (ADC, real ranged reads, real prefix listing)
//! can be added without touching the tools. That backend is deferred: the local ADC credentials
//! could not be verified during the spike (`gcloud auth login` was required).

use crate::http::{ApiError, HttpClient};

pub struct GatewayFileStore<'a> {
    http: &'a HttpClient,
}

impl<'a> GatewayFileStore<'a> {
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Fetch a whole artefact as text. `path` is bucket-less, as the gateway expects.
    pub async fn read_all(&self, path: &str) -> Result<String, ApiError> {
        let url = self.http.config().gcs_file_url(path);
        match self.http.get_text(&url, &[]).await {
            Err(ApiError::Upstream { status, .. }) if status.as_u16() == 404 => {
                Err(ApiError::NotFound {
                    what: format!("file {path}"),
                })
            }
            other => other,
        }
    }

    /// Cheap-ish existence probe. Without range support this still costs a request, so callers
    /// should probe only the handful of files a task is expected to have produced.
    pub async fn exists(&self, path: &str) -> Result<bool, ApiError> {
        match self.read_all(path).await {
            Ok(_) => Ok(true),
            Err(ApiError::NotFound { .. }) => Ok(false),
            Err(other) => Err(other),
        }
    }
}
