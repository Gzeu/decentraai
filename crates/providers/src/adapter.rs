//! Provider adapter layer.
//!
//! Reuses the existing backend-neutral [`decentraai_inference_adapter`] which
//! targets any OpenAI-compatible HTTP surface (`/v1/chat/completions`).
//! Provider adapters handle:
//! - **test_connection** — does the credential authenticate?
//! - **discover_models** — list available models from the upstream provider.
//! - **complete / stream** — execute through a configured backend.
//! - **error classification** — map HTTP status + body to
//!   [`ProviderErrorClass`](crate::ProviderErrorClass).

use async_trait::async_trait;
use decentraai_inference_adapter::{
    BackendConfig, BackendRequest, InferenceBackend, OpenAiCompatibleBackend, TokenStream,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use crate::{ProviderErrorClass, ProviderKind};

/// Model metadata as reported by an upstream provider.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProviderModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub created_by: Option<String>,
    #[serde(default)]
    pub context_window: u32,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    #[serde(default)]
    pub accessible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt_per_1m: Option<f64>,
    pub completion_per_1m: Option<f64>,
}

/// The adapter abstraction every provider must implement.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Test whether a credential authenticates successfully.
    async fn test_connection(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Result<(u64, usize), ProviderConnError>;
    /// Discover available models from the provider.
    async fn discover_models(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<ProviderModelInfo>, ProviderConnError>;
}

// ─── OpenAICompatibleProvider ─────────────────────────────────────────

/// Generic adapter targeting any OpenAI-compatible `/v1/*` surface.
pub struct OpenAICompatibleProvider {
    client: reqwest::Client,
    _kind: ProviderKind,
}

impl OpenAICompatibleProvider {
    pub fn new(kind: ProviderKind) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .expect("reqwest Client build failed"),
            _kind: kind,
        }
    }

    fn auth_url(&self, base_url: &str, _api_key: &str, path: &str) -> String {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn classify_error(status: StatusCode, body: &str) -> ProviderErrorClass {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorClass::Auth,
            StatusCode::TOO_MANY_REQUESTS => ProviderErrorClass::RateLimited,
            StatusCode::PAYMENT_REQUIRED => ProviderErrorClass::QuotaExhausted,
            StatusCode::NOT_FOUND => ProviderErrorClass::ModelUnavailable,
            _ if status.is_server_error() => ProviderErrorClass::Upstream,
            _ if status.is_client_error() && status != StatusCode::BAD_REQUEST => {
                ProviderErrorClass::Policy
            }
            StatusCode::BAD_REQUEST => {
                if body.contains("unsupported")
                    || body.contains("not found")
                    || body.contains("invalid_model")
                {
                    ProviderErrorClass::ModelUnavailable
                } else {
                    ProviderErrorClass::Protocol
                }
            }
            _ => ProviderErrorClass::Unknown,
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAICompatibleProvider {
    async fn test_connection(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Result<(u64, usize), ProviderConnError> {
        let start = std::time::Instant::now();
        let url = self.auth_url(base_url, api_key, "v1/models");
        let resp = self.client.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderConnError::Timeout(format!("connection timed out to {base_url}"))
            } else if e.is_connect() {
                ProviderConnError::Network(format!("cannot reach {base_url}: {e}"))
            } else {
                ProviderConnError::Transport(e.to_string())
            }
        })?;
        let latency_ms = start.elapsed().as_millis() as u64;
        let http_status = resp.status();
        if !http_status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let err_class = Self::classify_error(http_status, &body);
            return Err(match err_class {
                ProviderErrorClass::Auth => ProviderConnError::InvalidCredentials(body),
                other => ProviderConnError::HttpError {
                    status: http_status.as_u16(),
                    body,
                    error_class: other,
                },
            });
        }
        // Clone needed fields before resp is consumed by parse_model_list_response.
        let models_json = resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ProviderConnError::Protocol(format!("failed to parse /v1/models: {e}")))?;
        let items = models_json
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let mut models = Vec::new();
        for item in items {
            let id = item["id"].as_str().unwrap_or_default().to_string();
            let name = item["name"].as_str().map(String::from);
            let created_by = item
                .get("created_by")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    let owner = item.get("owner");
                    owner.and_then(|v| v.as_str())
                })
                .map(String::from);
            let ctx = item["context_length"]
                .as_u64()
                .or_else(|| item["context_window"].as_u64())
                .unwrap_or(0) as u32;
            models.push(ProviderModelInfo {
                id,
                name,
                created_by,
                context_window: ctx,
                pricing: None,
                accessible: true,
            });
        }
        Ok((latency_ms, models.len()))
    }

    async fn discover_models(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<ProviderModelInfo>, ProviderConnError> {
        let url = self.auth_url(base_url, api_key, "v1/models");
        let resp = self.client.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderConnError::Timeout(format!("discovery timed out to {base_url}"))
            } else if e.is_connect() {
                ProviderConnError::Network(format!("cannot reach {base_url}: {e}"))
            } else {
                ProviderConnError::Transport(e.to_string())
            }
        })?;
        let http_status = resp.status();
        if !http_status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let err_class = Self::classify_error(http_status, &body);
            return Err(match err_class {
                ProviderErrorClass::Auth => ProviderConnError::InvalidCredentials(body),
                other => ProviderConnError::HttpError {
                    status: http_status.as_u16(),
                    body,
                    error_class: other,
                },
            });
        }
        let models_json = resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ProviderConnError::Protocol(format!("failed to parse /v1/models: {e}")))?;
        let items = models_json
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let mut models = Vec::new();
        for item in items {
            let id = item["id"].as_str().unwrap_or_default().to_string();
            let name = item["name"].as_str().map(String::from);
            let created_by = item
                .get("created_by")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    let owner = item.get("owner");
                    owner.and_then(|v| v.as_str())
                })
                .map(String::from);
            let ctx = item["context_length"]
                .as_u64()
                .or_else(|| item["context_window"].as_u64())
                .unwrap_or(0) as u32;
            models.push(ProviderModelInfo {
                id,
                name,
                created_by,
                context_window: ctx,
                pricing: None,
                accessible: true,
            });
        }
        Ok(models)
    }
}

// ─── ModelAdapter (per-connected-model) ────────────────────────────────

/// Wraps a single connected model's configuration and provides execute/stream.
///
/// Credentials are resolved at call time from a shared `CredentialStore`, so
/// revocation takes effect immediately without rebuilding the adapter instance.
#[derive(Clone)]
pub struct ModelAdapter {
    base_url: String,
    upstream_model: String,
    credential_store: std::sync::Arc<std::sync::Mutex<crate::CredentialStore>>,
    credential_key_id: String,
    max_output_tokens: u32,
    temperature: f32,
    top_p: f32,
}

impl ModelAdapter {
    pub fn new(
        base_url: impl Into<String>,
        upstream_model: impl Into<String>,
        credential_store: Arc<std::sync::Mutex<crate::CredentialStore>>,
        credential_key_id: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            upstream_model: upstream_model.into(),
            credential_store,
            credential_key_id: credential_key_id.into(),
            max_output_tokens: 8192,
            temperature: 0.7,
            top_p: 0.9,
        }
    }

    pub fn set_sampling(&mut self, temperature: f32, top_p: f32) {
        self.temperature = temperature;
        self.top_p = top_p;
    }

    fn make_backend(&self) -> Result<OpenAiCompatibleBackend, ProviderInferError> {
        let api_key = self
            .credential_store
            .lock()
            .map_err(|_| ProviderInferError::CredentialLock)?
            .get_secret(&self.credential_key_id)
            .ok_or_else(|| ProviderInferError::CredentialNotFound(self.credential_key_id.clone()))?
            .to_string();
        let config = BackendConfig {
            base_url: self.base_url.clone(),
            model: self.upstream_model.clone(),
            api_key: Some(api_key),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(120),
            max_prompt_bytes: 500_000,
            max_output_tokens: self.max_output_tokens,
            engine: decentraai_inference_adapter::EngineKind::RemoteOpenAI,
            backend_url_resolver: None,
        };
        OpenAiCompatibleBackend::new(config).map_err(|e| ProviderInferError::Backend(e.to_string()))
    }

    pub async fn complete(
        &self,
        request: &BackendRequest,
    ) -> Result<decentraai_inference_adapter::BackendResponse, ProviderInferError> {
        let backend = self.make_backend()?;
        backend
            .complete(request.clone())
            .await
            .map_err(map_backend_error)
    }

    pub async fn stream(
        &self,
        request: &BackendRequest,
    ) -> Result<TokenStream, ProviderInferError> {
        let backend = self.make_backend()?;
        backend
            .stream(request.clone())
            .await
            .map_err(map_backend_error)
    }

    pub async fn health(&self) -> Result<(), ProviderInferError> {
        let backend = self.make_backend()?;
        backend.health().await.map_err(map_health_error)
    }
}

fn map_backend_error(e: decentraai_inference_adapter::BackendError) -> ProviderInferError {
    match e {
        decentraai_inference_adapter::BackendError::Timeout => ProviderInferError::Timeout,
        decentraai_inference_adapter::BackendError::PromptTooLarge => {
            ProviderInferError::PromptTooLarge
        }
        decentraai_inference_adapter::BackendError::OutputLimitExceeded => {
            ProviderInferError::OutputLimitExceeded
        }
        decentraai_inference_adapter::BackendError::Transport(msg) => {
            ProviderInferError::Transport(msg)
        }
        decentraai_inference_adapter::BackendError::Protocol(msg) => {
            ProviderInferError::Protocol(msg)
        }
        decentraai_inference_adapter::BackendError::Http { status, body } => {
            let err_class = {
                let _st = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                if status == 401 || status == 403 {
                    ProviderErrorClass::Auth
                } else {
                    ProviderErrorClass::Unknown
                }
            };
            ProviderInferError::ProviderError(err_class, body)
        }
    }
}

fn map_health_error(e: decentraai_inference_adapter::BackendError) -> ProviderInferError {
    match e {
        decentraai_inference_adapter::BackendError::Timeout => ProviderInferError::Timeout,
        decentraai_inference_adapter::BackendError::Transport(msg) => {
            ProviderInferError::Transport(msg)
        }
        decentraai_inference_adapter::BackendError::Http { status, body } => {
            let err_class = if status == 401 || status == 403 {
                ProviderErrorClass::Auth
            } else {
                ProviderErrorClass::Unknown
            };
            ProviderInferError::ProviderError(err_class, body)
        }
        other => ProviderInferError::Backend(other.to_string()),
    }
}

// ─── Error types ───────────────────────────────────────────────────────

/// Errors specific to adapter connection tests.
#[derive(Debug, Error)]
pub enum ProviderConnError {
    #[error("credentials invalid")]
    InvalidCredentials(String),
    #[error("network unreachable: {0}")]
    Network(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("request timed out")]
    Timeout(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("unexpected HTTP {status}: {body}")]
    HttpError {
        status: u16,
        body: String,
        error_class: ProviderErrorClass,
    },
}

/// Inference execution errors (never contain secrets).
#[derive(Debug, Error)]
pub enum ProviderInferError {
    #[error("credential not found: {0}")]
    CredentialNotFound(String),
    #[error("credential lock held by another thread")]
    CredentialLock,
    #[error("provider returned error: {1}")]
    ProviderError(ProviderErrorClass, String),
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("request timed out")]
    Timeout,
    #[error("prompt too large")]
    PromptTooLarge,
    #[error("output limit exceeded")]
    OutputLimitExceeded,
    #[error("backend error: {0}")]
    Backend(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Convert a `ProviderInferError` to a human-safe diagnostic string.
pub fn infer_diagnostic(err: &ProviderInferError) -> (&'static str, String) {
    match err {
        ProviderInferError::CredentialNotFound(_) => (
            "AUTHENTICATION_FAILED",
            "Provider credential rejected".into(),
        ),
        ProviderInferError::CredentialLock => (
            "INTERNAL_ERROR",
            "Credential store temporarily unavailable".into(),
        ),
        ProviderInferError::ProviderError(class, _) => match class {
            ProviderErrorClass::Auth => (
                "AUTHENTICATION_FAILED",
                "Provider authentication failed".into(),
            ),
            ProviderErrorClass::RateLimited => ("RATE_LIMITED", "Provider rate-limited".into()),
            ProviderErrorClass::QuotaExhausted => {
                ("QUOTA_EXHAUSTED", "Provider budget exhausted".into())
            }
            ProviderErrorClass::ModelUnavailable => (
                "MODEL_UNAVAILABLE",
                "The requested model is not available".into(),
            ),
            ProviderErrorClass::Timeout => (
                "UPSTREAM_TIMEOUT",
                "Provider did not respond in time".into(),
            ),
            ProviderErrorClass::Upstream => {
                ("UPSTREAM_ERROR", "Provider server error (5xx)".into())
            }
            ProviderErrorClass::Protocol => {
                ("PROTOCOL_ERROR", "Malformed response from provider".into())
            }
            ProviderErrorClass::Policy => {
                ("POLICY_DENIED", "Provider policy denied the request".into())
            }
            ProviderErrorClass::Unknown => ("UNKNOWN_ERROR", "Unexpected provider error".into()),
        },
        ProviderInferError::Transport(msg) => ("TRANSPORT_ERROR", msg.clone()),
        ProviderInferError::Timeout => ("TIMEOUT", "Request timed out".into()),
        ProviderInferError::PromptTooLarge => {
            ("PROMPT_TOO_LARGE", "Input exceeds token limit".into())
        }
        ProviderInferError::OutputLimitExceeded => {
            ("OUTPUT_LIMIT_EXCEEDED", "Output limit too small".into())
        }
        ProviderInferError::Backend(msg) => ("BACKEND_ERROR", msg.clone()),
        ProviderInferError::Protocol(msg) => ("PROTOCOL_ERROR", msg.clone()),
    }
}
