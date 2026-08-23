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

    /// Build the URL for a provider API path. The base URL may already carry
    /// a `/v1` suffix (defaults like `https://api.openai.com/v1` do), while
    /// the callers pass API-relative paths like `v1/models` — joining them
    /// naively would produce `/v1/v1/models`. Deduplicate a doubled `/v1`.
    fn auth_url(&self, base_url: &str, _api_key: &str, path: &str) -> String {
        Self::auth_url_impl(base_url, path)
    }

    fn auth_url_impl(base_url: &str, path: &str) -> String {
        let base = base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        let combined = format!("{base}/{path}");
        if path.starts_with("v1/") && base.ends_with("/v1") && !combined.contains("//v1/") {
            // base ends with /v1 and path starts with v1/ → strip one.
            combined.replacen("/v1/v1/", "/v1/", 1)
        } else {
            combined
        }
    }

    fn auth(request: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
        if api_key.is_empty() {
            request
        } else {
            request.bearer_auth(api_key)
        }
    }

    #[cfg(test)]
    fn auth_url_for_test(base_url: &str, path: &str) -> String {
        Self::auth_url_impl(base_url, path)
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
        let resp = Self::auth(self.client.get(&url), api_key)
            .send()
            .await
            .map_err(|e| {
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
        let resp = Self::auth(self.client.get(&url), api_key)
            .send()
            .await
            .map_err(|e| {
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
                match status {
                    // 401 Unauthorized / 403 Forbidden — bad or missing key.
                    401 | 403 => ProviderErrorClass::Auth,
                    // 402 Payment Required — account has no funds (the real
                    // DeepSeek "Insufficient Balance" case).
                    402 => ProviderErrorClass::QuotaExhausted,
                    // 429 Too Many Requests — provider rate limits.
                    429 => ProviderErrorClass::RateLimited,
                    // 404 — the requested upstream model does not exist.
                    404 => ProviderErrorClass::ModelUnavailable,
                    // 408 Request Timeout / 504 Gateway Timeout.
                    408 | 504 => ProviderErrorClass::Timeout,
                    // 5xx — the provider itself is failing.
                    500..=599 => ProviderErrorClass::Upstream,
                    // Everything else (4xx) — we cannot classify it.
                    _ => ProviderErrorClass::Unknown,
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
        ProviderInferError::ProviderError(class, body) => match class {
            ProviderErrorClass::Auth => (
                "AUTHENTICATION_FAILED",
                format!("Provider authentication failed: {body}"),
            ),
            ProviderErrorClass::RateLimited => {
                ("RATE_LIMITED", format!("Provider rate-limited: {body}"))
            }
            ProviderErrorClass::QuotaExhausted => (
                "QUOTA_EXHAUSTED",
                format!("Provider budget exhausted: {body}"),
            ),
            ProviderErrorClass::ModelUnavailable => (
                "MODEL_UNAVAILABLE",
                format!("The requested model is not available: {body}"),
            ),
            ProviderErrorClass::Timeout => (
                "UPSTREAM_TIMEOUT",
                format!("Provider did not respond in time: {body}"),
            ),
            ProviderErrorClass::Upstream => (
                "UPSTREAM_ERROR",
                format!("Provider server error (5xx): {body}"),
            ),
            ProviderErrorClass::Protocol => (
                "PROTOCOL_ERROR",
                format!("Malformed response from provider: {body}"),
            ),
            ProviderErrorClass::Policy => (
                "POLICY_DENIED",
                format!("Provider policy denied the request: {body}"),
            ),
            ProviderErrorClass::Unknown => (
                "UNKNOWN_ERROR",
                format!("Unexpected provider error: {body}"),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_url_never_doubles_v1() {
        // Base URLs that already carry /v1 (like the OpenAI default) must not
        // produce /v1/v1 when the caller passes an API-relative path.
        assert_eq!(
            OpenAICompatibleProvider::auth_url_for_test("https://api.deepseek.com/v1", "v1/models"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            OpenAICompatibleProvider::auth_url_for_test("https://api.openai.com/v1", "v1/models"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            OpenAICompatibleProvider::auth_url_for_test("https://api.openai.com/v1/", "v1/models"),
            "https://api.openai.com/v1/models"
        );
        // A base without /v1 stays unchanged (bare host + path).
        assert_eq!(
            OpenAICompatibleProvider::auth_url_for_test("https://api.deepseek.com", "v1/models"),
            "https://api.deepseek.com/v1/models"
        );
        // A non-v1 path is joined normally.
        assert_eq!(
            OpenAICompatibleProvider::auth_url_for_test("https://api.deepseek.com/v1", "health"),
            "https://api.deepseek.com/v1/health"
        );
    }

    #[test]
    fn map_backend_error_classifies_http_status() {
        use decentraai_inference_adapter::BackendError;
        // 402 Payment Required → quota (DeepSeek "Insufficient Balance").
        match map_backend_error(BackendError::Http {
            status: 402,
            body: "Insufficient Balance".into(),
        }) {
            ProviderInferError::ProviderError(ProviderErrorClass::QuotaExhausted, body) => {
                assert_eq!(body, "Insufficient Balance")
            }
            other => panic!("expected quota, got {other:?}"),
        }
        // 401 → auth, 429 → rate limited, 404 → model unavailable,
        // 500 → upstream, 418 → unknown.
        assert!(matches!(
            map_backend_error(BackendError::Http {
                status: 401,
                body: "x".into()
            }),
            ProviderInferError::ProviderError(ProviderErrorClass::Auth, _)
        ));
        assert!(matches!(
            map_backend_error(BackendError::Http {
                status: 429,
                body: "x".into()
            }),
            ProviderInferError::ProviderError(ProviderErrorClass::RateLimited, _)
        ));
        assert!(matches!(
            map_backend_error(BackendError::Http {
                status: 404,
                body: "x".into()
            }),
            ProviderInferError::ProviderError(ProviderErrorClass::ModelUnavailable, _)
        ));
        assert!(matches!(
            map_backend_error(BackendError::Http {
                status: 500,
                body: "x".into()
            }),
            ProviderInferError::ProviderError(ProviderErrorClass::Upstream, _)
        ));
        assert!(matches!(
            map_backend_error(BackendError::Http {
                status: 418,
                body: "x".into()
            }),
            ProviderInferError::ProviderError(ProviderErrorClass::Unknown, _)
        ));
    }
}
