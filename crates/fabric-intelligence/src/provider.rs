//! Intelligence providers: the interchangeable sources of planning.
//!
//! Contract: a provider receives a [`TaskBrief`] and returns the model's RAW
//! text answer. It NEVER parses that answer into a plan — parsing/validation
//! lives in [`crate::plan`] so every provider's output goes through the same
//! untrusted-input gate. A provider also never stores credentials: the
//! external implementation reads its API key from the environment at CALL
//! time, so nothing secret persists in config dumps or memory snapshots.
//!
//! Dispatch is an enum over the two concrete providers rather than `dyn`:
//! there are exactly two today (local llama.cpp backend, OpenAI-compatible
//! external endpoint), static dispatch keeps the async plumbing box-free,
//! and adding a future provider (Groq, vLLM peer…) means one new variant —
//! the deterministic fabric planner stays untouched either way.

use serde_json::json;

use crate::limits::ArtifactLimit;
use crate::redact::redact_secrets;
pub use crate::telemetry::ProviderKind;
use crate::{SYSTEM_PROMPT, TaskBrief};

/// Why a provider call failed. Every variant is redacted on construction:
/// upstream error bodies sometimes echo Authorization headers or URLs with
/// embedded tokens, and none of that may reach logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The configured environment variable for the external API key is unset.
    AuthMissing(String),
    /// Transport-level failure (connection refused, timeout, non-2xx).
    Http(String),
    /// 2xx but the body has no usable assistant message.
    BadResponse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthMissing(var) => write!(
                f,
                "external API key environment variable `{var}` is not set; \
                 set it in the service environment to use external intelligence"
            ),
            Self::Http(s) => write!(f, "provider transport failure: {s}"),
            Self::BadResponse(s) => write!(f, "provider returned unusable response: {s}"),
        }
    }
}

impl std::error::Error for ProviderError {}

fn err_http(raw: String) -> ProviderError {
    ProviderError::Http(redact_secrets(&raw))
}
fn err_bad(raw: String) -> ProviderError {
    ProviderError::BadResponse(redact_secrets(&raw))
}

/// Shared OpenAI-compatible chat-completions request builder. Both providers
/// speak this shape; only base URL and authentication differ.
fn build_chat_body(model: Option<&str>, brief: &TaskBrief<'_>) -> serde_json::Value {
    let mut body = json!({
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": brief.user_message()},
        ],
        // A plan is a few hundred tokens of JSON at most; bounding it keeps a
        // runaway generation from burning minutes on CPU nodes.
        "max_tokens": 1024,
        // Deterministic-ish classification output; creativity is the enemy
        // of a strict JSON contract.
        "temperature": 0.1,
        "stream": false,
    });
    if let Some(m) = model {
        body["model"] = json!(m);
    }
    body
}

/// Qwen3-family models (the default local intelligence) emit a
/// `<think>…</think>` reasoning block BEFORE the visible answer unless
/// thinking is disabled. llama.cpp accepts this as a chat-template override;
/// without it the plan parser would see an empty content field (observed
/// live during the V1 nucleus rollout).
const THINKING_OFF_KWARGS: &str = "{\"enable_thinking\":false}";

/// Strips ONE leading `<think>…</think>` block from raw model output.
///
/// Defensive layering: local requests disable thinking via template kwargs,
/// but EXTERNAL models may think regardless of what we ask — and a plan
/// hidden behind a thinking block must still parse instead of failing with
/// NotJson. Anything after the block is passed through untouched; the strict
/// plan parser remains the final authority.
pub fn strip_think_block(raw: &str) -> &str {
    let trimmed = raw.trim_start();
    let Some(rest) = trimmed.strip_prefix("<think>") else {
        return trimmed;
    };
    match rest.find("</think>") {
        Some(end) => rest[end + "</think>".len()..].trim_start(),
        None => trimmed, // unterminated think block: leave as-is, parser rejects
    }
}

/// The intelligence source abstraction. One method, one contract: turn a
/// task brief into RAW model output that [`crate::TaskPlan::parse`] can then
/// judge. Providers never see fabric internals and never store secrets.
pub trait IntelligenceProvider {
    /// Telemetry bucket for this provider.
    fn kind(&self) -> ProviderKind;

    /// Human-readable identity for status output (no credentials).
    fn name(&self) -> String;

    /// Runs one analysis. Returns the model's raw text — parsing is NOT the
    /// provider's job.
    fn analyze(
        &self,
        brief: &TaskBrief<'_>,
    ) -> impl std::future::Future<Output = Result<String, ProviderError>> + Send;
}

/// The local provider: the node's OWN managed llama-server backend
/// (loopback). This keeps task content on-node by default — the privacy
/// posture behind `local_first` being the default policy.
#[derive(Debug, Clone)]
pub struct LocalLlamaProvider {
    /// Node backend root, e.g. `http://127.0.0.1:8080`. The `/v1/chat/completions`
    /// suffix is appended here.
    pub base_url: String,
    /// Preferred intelligence model name (advisory): if the node currently
    /// serves something else, the served model answers anyway — the node
    /// owns its engine, the provider does not swap it. `None` lets the
    /// backend decide.
    pub model: Option<String>,
    pub client: reqwest::Client,
}

impl LocalLlamaProvider {
    pub fn new(base_url: impl Into<String>, model: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model,
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                // Same shared inference budget as every other layer: slow CPU
                // prefill of the planning prompt is legitimate work.
                .timeout(decentraai_config::backend_request_timeout())
                .build()
                .unwrap_or_default(),
        }
    }
}

impl IntelligenceProvider for LocalLlamaProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn name(&self) -> String {
        format!("local llama.cpp ({})", self.base_url)
    }

    async fn analyze(&self, brief: &TaskBrief<'_>) -> Result<String, ProviderError> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let mut body = build_chat_body(self.model.as_deref(), brief);
        // Disable Qwen3-style thinking blocks server-side (see
        // [`strip_think_block`] for why this matters).
        body["chat_template_kwargs"] =
            serde_json::from_str::<serde_json::Value>(THINKING_OFF_KWARGS)
                .expect("static JSON literal");

        let res = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| err_http(format!("{url}: {e}")))?;
        if !res.status().is_success() {
            return Err(err_http(format!("{} → HTTP {}", url, res.status())));
        }
        let payload: serde_json::Value = res
            .json()
            .await
            .map_err(|e| err_bad(format!("{url}: {e}")))?;
        extract_content(&payload)
    }
}

/// An external OpenAI-compatible provider (OpenAI, Groq, OpenRouter, any
/// `/v1/chat/completions` endpoint). Credentials come from the environment
/// named by `api_key_env`, read AT CALL TIME and never stored.
#[derive(Debug, Clone)]
pub struct OpenAiCompatProvider {
    /// Root INCLUDING version path, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    /// Environment variable holding the API key (never the key itself).
    pub api_key_env: String,
    /// Model identifier at the external provider.
    pub model: String,
    pub client: reqwest::Client,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key_env: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key_env: api_key_env.into(),
            model: model.into(),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(decentraai_config::backend_request_timeout())
                .build()
                .unwrap_or_default(),
        }
    }

    /// The error to surface when the key is missing: names ONLY the
    /// environment variable, never any value.
    pub fn missing_key_error(&self) -> ProviderError {
        ProviderError::AuthMissing(self.api_key_env.clone())
    }

    /// Whether the configured key variable resolves right now. Used by the
    /// policy layer to avoid selecting a provider that cannot authenticate.
    pub fn key_available(&self) -> bool {
        std::env::var(&self.api_key_env)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    }
}

impl IntelligenceProvider for OpenAiCompatProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::External
    }

    fn name(&self) -> String {
        format!("external {} ({})", self.model, self.base_url)
    }

    async fn analyze(&self, brief: &TaskBrief<'_>) -> Result<String, ProviderError> {
        let Some(key) = std::env::var(&self.api_key_env)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            // Only the VARIABLE NAME is reported — never any value.
            return Err(ProviderError::AuthMissing(self.api_key_env.clone()));
        };
        let url = format!(
            "{}/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let body = build_chat_body(Some(&self.model), brief);
        let res = self
            .client
            .post(&url)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| err_http(format!("{url}: {e}")))?;
        if !res.status().is_success() {
            return Err(err_http(format!("{} → HTTP {}", url, res.status())));
        }
        let payload: serde_json::Value = res
            .json()
            .await
            .map_err(|e| err_bad(format!("{url}: {e}")))?;
        extract_content(&payload)
    }
}

/// Pulls the assistant message out of an OpenAI-shaped completion response.
fn extract_content(payload: &serde_json::Value) -> Result<String, ProviderError> {
    let content = payload
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            err_bad(format!(
                "no choices[0].message.content in {}-byte response",
                payload.to_string().len()
            ))
        })?;
    Ok(strip_think_block(content).to_string())
}

/// Runtime dispatch over the configured providers. Static, cloneable, and
/// trivially extensible: a new provider kind means one more variant HERE and
/// nowhere else in the planner path.
#[derive(Debug, Clone)]
pub enum ConfiguredProvider {
    Local(LocalLlamaProvider),
    External(OpenAiCompatProvider),
}

impl ConfiguredProvider {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Local(_) => ProviderKind::Local,
            Self::External(_) => ProviderKind::External,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Local(p) => p.name(),
            Self::External(p) => p.name(),
        }
    }

    pub async fn analyze(&self, brief: &TaskBrief<'_>) -> Result<String, ProviderError> {
        match self {
            Self::Local(p) => p.analyze(brief).await,
            Self::External(p) => p.analyze(brief).await,
        }
    }
}

/// Builds the artifact-size policy hint embedded in status responses: the
/// intelligence layer may RECOMMEND models, and recommendations must respect
/// the same hard limit the provisioning pipeline enforces.
pub fn artifact_limit_hint(limit: &ArtifactLimit) -> serde_json::Value {
    json!({
        "max_artifact_bytes": limit.max_bytes,
        "recommended_artifact_bytes": crate::RECOMMENDED_ARTIFACT_BYTES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TaskPlan;

    #[test]
    fn strips_think_block_before_plan_parsing() {
        let raw = "<think>I should classify this.</think>\n{\"intent\":\"x\",\"capabilities\":[{\"name\":\"ocr\"}],\"workflow\":[\"ocr\"],\"confidence\":1}";
        let stripped = strip_think_block(raw);
        assert!(stripped.starts_with('{'), "think block removed");
        TaskPlan::parse(stripped).expect("plan after think-strip parses");
    }

    #[test]
    fn leaves_non_thinking_output_untouched() {
        assert_eq!(strip_think_block("{\"intent\":\"x\"}"), "{\"intent\":\"x\"}");
    }

    #[test]
    fn errors_never_contain_the_api_key_value() {
        // The AuthMissing variant reports only the VARIABLE NAME by design.
        let e = ProviderError::AuthMissing("MY_SECRET_ENV".to_string());
        let rendered = e.to_string();
        assert!(rendered.contains("MY_SECRET_ENV"));
        // And Http/BadResponse paths pass through redaction.
        let h = err_http("POST https://api.example.com?api_key=sk-super-secret failed".into());
        assert!(!h.to_string().contains("sk-super-secret"));
    }

    #[tokio::test]
    async fn local_provider_talks_openai_shape_to_a_mock_backend() {
        // Minimal HTTP server asserting the request contract and returning a
        // canned completion. No real engine needed for provider tests.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(req.contains("POST /v1/chat/completions"));
            assert!(req.contains("\"enable_thinking\":false"));
            assert!(!req.contains("\"tools\""), "no tool advertisement");
            let reply = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\
                {\"choices\":[{\"message\":{\"content\":\"{\\\"intent\\\":\\\"t\\\",\\\"capabilities\\\":[{\\\"name\\\":\\\"ocr\\\",\\\"required\\\":true}],\\\"workflow\\\":[\\\"ocr\\\"],\\\"confidence\\\":0.9}\"}}]}";
            sock.write_all(reply.as_bytes()).await.unwrap();
        });

        let provider = LocalLlamaProvider::new(format!("http://{addr}"), Some("Qwen3-0.6B".into()));
        let brief = TaskBrief { task: "classify me" };
        let raw = provider.analyze(&brief).await.expect("mock answered");
        let plan = crate::TaskPlan::parse(&raw).expect("raw answer parses into a plan");
        assert_eq!(plan.intent, "t");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn external_provider_requires_the_env_key_at_call_time() {
        // Construct directly with an env name we KNOW is unset in test runs.
        let var = "DECENTRAAI_INTEL_TEST_KEY_DEFINITELY_UNSET";
        // NOTE(test): not calling remove_var (unsafe in Rust 2024); the var
        // name is unique enough that no other test sets it.
        let provider =
            OpenAiCompatProvider::new("https://api.example.com/v1", var, "test-model");
        let brief = TaskBrief { task: "x" };
        match provider.analyze(&brief).await {
            Err(ProviderError::AuthMissing(name)) => assert_eq!(name, var),
            other => panic!("expected AuthMissing, got {other:?}"),
        }
    }
}
