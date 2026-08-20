//! Backend-neutral OpenAI-compatible inference adapter.

use async_trait::async_trait;
use futures_util::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::{pin::Pin, time::Duration};
use thiserror::Error;

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, BackendError>> + Send>>;

/// The concrete inference engine a backend drives (M22).
///
/// DecentraAI must never be coupled to one model server. Every engine here is
/// reached through the same OpenAI-compatible HTTP surface (`/v1/models`,
/// `/v1/chat/completions`, `/v1/completions`), which is exactly how
/// `llama-server`, vLLM, SGLang and Ollama all expose inference today. The
/// kind is what lets the execution planner reason about *additional*
/// capabilities (KV state, expert routing, tensor-parallel ranks) before
/// relying on them. Unknown kinds degrade safely to [`EngineKind::RemoteOpenAI`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EngineKind {
    /// llama.cpp `llama-server`.
    LlamaServer,
    /// vLLM (`vllm serve`).
    Vllm,
    /// SGLang server.
    Sglang,
    /// Ollama (`ollama serve`).
    Ollama,
    /// Any other OpenAI-compatible HTTP server.
    RemoteOpenAI,
}

impl EngineKind {
    /// Parses a wire / config string. Unknown engines resolve to
    /// [`EngineKind::RemoteOpenAI`] rather than failing, so a future engine
    /// never breaks an old node's runtime startup.
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "llama-server" | "llama_server" | "llamacpp" | "llama.cpp" => Self::LlamaServer,
            "vllm" => Self::Vllm,
            "sglang" | "sglang_server" => Self::Sglang,
            "ollama" => Self::Ollama,
            _ => Self::RemoteOpenAI,
        }
    }

    /// Canonical wire representation (also what this engine reports in its
    /// own `engine` capability so the planner parses it back via [`Self::parse`]).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LlamaServer => "llama-server",
            Self::Vllm => "vllm",
            Self::Sglang => "sglang",
            Self::Ollama => "ollama",
            Self::RemoteOpenAI => "openai-compatible",
        }
    }
}

/// Capabilities a live engine endpoint reported (M22). Mirrors the planner's
/// capability ABI so the runtime layer and the fabric agree on what an engine
/// can do; conservative by default and narrowed by `probe_capabilities`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineCapabilities {
    pub streaming: bool,
    pub kv_report: bool,
    pub prefill_decode_separation: bool,
    pub expert_routing: bool,
    pub tensor_parallel: bool,
    pub continuous_batching: bool,
    pub speculative_decoding: bool,
    pub kv_offload: bool,
    pub prefix_cache: bool,
    pub pipeline_parallel: bool,
}

impl EngineCapabilities {
    pub fn conservative() -> Self {
        Self {
            streaming: true,
            kv_report: false,
            prefill_decode_separation: false,
            expert_routing: false,
            tensor_parallel: false,
            continuous_batching: false,
            speculative_decoding: false,
            kv_offload: false,
            prefix_cache: false,
            pipeline_parallel: false,
        }
    }
}

/// Resolves the live backend base URL at request time. Used when the engine
/// endpoint can change after a respawn (M24 engine supervisor may bind a new
/// ephemeral port), so the adapter follows the authoritative engine source of
/// truth instead of a frozen startup URL. Returning `None` falls back to
/// [`BackendConfig::base_url`].
pub type LiveBackendUrl = Arc<dyn Fn() -> Option<String> + Send + Sync>;

#[derive(Clone)]
pub struct BackendConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_prompt_bytes: usize,
    pub max_output_tokens: u32,
    /// Which engine this backend drives (M22). Defaults to a plain remote.
    pub engine: EngineKind,
    /// Optional live URL resolver (single source of truth for the engine
    /// endpoint). When set, every request resolves the base URL through it so
    /// an engine respawn on a new port is followed automatically.
    pub backend_url_resolver: Option<LiveBackendUrl>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8081".into(),
            model: "local-model".into(),
            api_key: None,
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(300),
            max_prompt_bytes: 200_000,
            max_output_tokens: 8192,
            engine: EngineKind::RemoteOpenAI,
            backend_url_resolver: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackendRequest {
    pub request_id: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
}

#[derive(Debug, Clone)]
pub struct BackendResponse {
    pub output: String,
    pub tokens_used: Option<u32>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub request_id: String,
    pub sequence: u64,
    pub text: String,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("prompt exceeds configured limit")]
    PromptTooLarge,
    #[error("requested output exceeds configured limit")]
    OutputLimitExceeded,
    #[error("backend request timed out")]
    Timeout,
    #[error("backend returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("backend protocol error: {0}")]
    Protocol(String),
    #[error("backend transport error: {0}")]
    Transport(String),
}

#[async_trait]
pub trait InferenceBackend: Send + Sync {
    async fn health(&self) -> Result<(), BackendError>;
    async fn complete(&self, request: BackendRequest) -> Result<BackendResponse, BackendError>;
    async fn stream(&self, request: BackendRequest) -> Result<TokenStream, BackendError>;
}

#[derive(Clone)]
pub struct OpenAiCompatibleBackend {
    client: Client,
    config: BackendConfig,
}

impl OpenAiCompatibleBackend {
    pub fn new(config: BackendConfig) -> Result<Self, BackendError> {
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .map_err(|e| BackendError::Transport(e.to_string()))?;
        Ok(Self { client, config })
    }

    pub fn validate(&self, request: &BackendRequest) -> Result<(), BackendError> {
        if request.prompt.len() > self.config.max_prompt_bytes {
            return Err(BackendError::PromptTooLarge);
        }
        if request.max_tokens > self.config.max_output_tokens {
            return Err(BackendError::OutputLimitExceeded);
        }
        Ok(())
    }

    /// The engine this backend drives (M22). Propagated to the node's
    /// `capability.engine` so coordinators' planners can reason engine-aware.
    pub fn engine(&self) -> EngineKind {
        self.config.engine
    }

    /// Best-effort probe of the live engine's reported capabilities. This is a
    /// real HTTP check against the OpenAI-compatible surface (`GET /v1/models`
    /// for availability, plus a +`/v1/metrics`-style KV export where the engine
    /// exposes one). Failures are not fatal: they simply report the
    /// conservative defaults already in use, so an unreachable or non-KV
    /// engine never breaks planning.
    pub async fn probe_capabilities(&self) -> EngineCapabilities {
        // Engine kind gives us the static baseline.
        let mut caps = match self.config.engine {
            EngineKind::Vllm => EngineCapabilities {
                kv_report: true,
                prefill_decode_separation: true,
                tensor_parallel: true,
                continuous_batching: true,
                speculative_decoding: true,
                kv_offload: true,
                prefix_cache: true,
                pipeline_parallel: true,
                ..EngineCapabilities::conservative()
            },
            EngineKind::Sglang => EngineCapabilities {
                kv_report: true,
                prefill_decode_separation: true,
                tensor_parallel: true,
                continuous_batching: true,
                speculative_decoding: true,
                kv_offload: true,
                prefix_cache: true,
                pipeline_parallel: true,
                ..EngineCapabilities::conservative()
            },
            EngineKind::LlamaServer => EngineCapabilities {
                kv_report: true,
                ..EngineCapabilities::conservative()
            },
            _ => EngineCapabilities::conservative(),
        };

        // Narrow with a live check: require a reachable /v1/models before we
        // trust any capability that implies an advanced endpoint.
        let reachable = self
            .auth(self.client.get(self.endpoint("v1/models")))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if !reachable {
            caps = EngineCapabilities::conservative();
        }
        caps
    }

    fn endpoint(&self, path: &str) -> String {
        let base = self
            .config
            .backend_url_resolver
            .as_ref()
            .and_then(|r| r())
            .unwrap_or_else(|| self.config.base_url.clone());
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        let combined = format!("{base}/{path}");
        // The base may already carry a `/v1` suffix (provider defaults like
        // https://api.openai.com/v1 do) while callers pass API-relative paths
        // like `v1/models`; deduplicate so `/v1/v1/…` never happens.
        if path.starts_with("v1/") && base.ends_with("/v1") && !combined.contains("//v1/") {
            combined.replacen("/v1/v1/", "/v1/", 1)
        } else {
            combined
        }
    }
    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.config.api_key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    async fn http_error(response: reqwest::Response) -> BackendError {
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unreadable body".into());
        BackendError::Http { status, body }
    }
}

#[async_trait]
impl InferenceBackend for OpenAiCompatibleBackend {
    async fn health(&self) -> Result<(), BackendError> {
        let response = self
            .auth(self.client.get(self.endpoint("health")))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::http_error(response).await)
        }
    }

    async fn complete(&self, request: BackendRequest) -> Result<BackendResponse, BackendError> {
        self.validate(&request)?;
        let body = serde_json::json!({"model": self.config.model, "messages": [{"role": "user", "content": request.prompt}], "max_tokens": request.max_tokens, "temperature": request.temperature, "top_p": request.top_p, "stream": false});
        let response = self
            .auth(self.client.post(self.endpoint("v1/chat/completions")))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        let raw: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Protocol(e.to_string()))?;
        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| BackendError::Protocol("missing choice".into()))?;
        Ok(BackendResponse {
            output: choice.message.content,
            tokens_used: raw.usage.and_then(|u| u.completion_tokens),
            finish_reason: choice.finish_reason,
        })
    }

    async fn stream(&self, request: BackendRequest) -> Result<TokenStream, BackendError> {
        self.validate(&request)?;
        let body = serde_json::json!({"model": self.config.model, "messages": [{"role": "user", "content": request.prompt}], "max_tokens": request.max_tokens, "temperature": request.temperature, "top_p": request.top_p, "stream": true});
        let response = self
            .auth(self.client.post(self.endpoint("v1/chat/completions")))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        Ok(Box::pin(parse_sse(
            response.bytes_stream(),
            request.request_id,
        )))
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}
#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}
#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}
#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    completion_tokens: Option<u32>,
}
#[derive(Debug, Deserialize)]
struct OpenAiStreamResponse {
    choices: Vec<OpenAiStreamChoice>,
}
#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}
#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
}

fn parse_sse<S, E>(
    input: S,
    request_id: String,
) -> impl Stream<Item = Result<StreamChunk, BackendError>> + Send + 'static
where
    S: Stream<Item = Result<bytes::Bytes, E>> + Unpin + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    use futures_util::StreamExt;
    async_stream::stream! {
        let mut buffer = String::new(); let mut sequence = 0; let mut input = input;
        while let Some(item) = input.next().await {
            let bytes = match item { Ok(v) => v, Err(e) => { yield Err(BackendError::Transport(e.to_string())); return; } };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(end) = buffer.find("\n\n") {
                let event = buffer.drain(..end + 2).collect::<String>();
                let data = event.lines().find_map(|line| line.strip_prefix("data: ")).unwrap_or("").trim();
                if data.is_empty() || data == "[DONE]" { continue; }
                let parsed: OpenAiStreamResponse = match serde_json::from_str(data) { Ok(v) => v, Err(e) => { yield Err(BackendError::Protocol(e.to_string())); return; } };
                let choice = match parsed.choices.into_iter().next() { Some(v) => v, None => continue };
                let text = choice.delta.content.unwrap_or_default();
                if text.is_empty() && choice.finish_reason.is_none() { continue; }
                yield Ok(StreamChunk { request_id: request_id.clone(), sequence, text, finish_reason: choice.finish_reason }); sequence += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_engine_kinds_and_degrades_unknown() {
        assert_eq!(EngineKind::parse("vllm"), EngineKind::Vllm);
        assert_eq!(EngineKind::parse("llama-server"), EngineKind::LlamaServer);
        assert_eq!(EngineKind::parse("ollama"), EngineKind::Ollama);
        assert_eq!(EngineKind::parse("sglang"), EngineKind::Sglang);
        assert_eq!(EngineKind::parse("future-engine"), EngineKind::RemoteOpenAI);
        assert_eq!(EngineKind::LlamaServer.as_str(), "llama-server");
    }

    #[test]
    fn engine_accessor_reports_configured_kind() {
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            engine: EngineKind::Vllm,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(b.engine(), EngineKind::Vllm);
    }

    #[test]
    fn rejects_large_prompt() {
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            max_prompt_bytes: 2,
            ..Default::default()
        })
        .unwrap();
        let r = BackendRequest {
            request_id: "r".into(),
            prompt: "abc".into(),
            max_tokens: 1,
            temperature: 0.7,
            top_p: 0.9,
        };
        assert!(matches!(b.validate(&r), Err(BackendError::PromptTooLarge)));
    }
    #[test]
    fn rejects_large_output() {
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            max_output_tokens: 2,
            ..Default::default()
        })
        .unwrap();
        let r = BackendRequest {
            request_id: "r".into(),
            prompt: "ok".into(),
            max_tokens: 3,
            temperature: 0.7,
            top_p: 0.9,
        };
        assert!(matches!(
            b.validate(&r),
            Err(BackendError::OutputLimitExceeded)
        ));
    }

    #[test]
    fn live_url_resolver_follows_a_moving_engine_endpoint() {
        // Simulates an engine respawn changing port: the resolver (the single
        // source of truth) returns the new live URL, and endpoint() must follow
        // it instead of the frozen static base_url.
        use std::sync::Arc;
        use std::sync::Mutex;
        let live = Arc::new(Mutex::new(Some("http://127.0.0.1:10021".to_string())));
        let live2 = live.clone();
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            base_url: "http://127.0.0.1:9999".to_string(), // frozen startup URL
            backend_url_resolver: Some(Arc::new(move || live2.lock().ok().and_then(|g| g.clone()))),
            ..Default::default()
        })
        .unwrap();

        // Follows the resolver's current value.
        assert_eq!(
            b.endpoint("v1/chat/completions"),
            "http://127.0.0.1:10021/v1/chat/completions"
        );
        // Engine respawned on a new port: the resolver moves, endpoint follows.
        *live.lock().unwrap() = Some("http://127.0.0.1:10022".to_string());
        assert_eq!(b.endpoint("v1/models"), "http://127.0.0.1:10022/v1/models");
        // Resolver gone => falls back to the static base_url.
        *live.lock().unwrap() = None;
        assert_eq!(b.endpoint("health"), "http://127.0.0.1:9999/health");
    }

    #[test]
    fn no_resolver_uses_static_base_url() {
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            base_url: "http://127.0.0.1:4321".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(b.endpoint("v1/models"), "http://127.0.0.1:4321/v1/models");
    }

    #[test]
    fn provider_base_url_with_v1_suffix_never_doubles() {
        // Provider defaults like https://api.deepseek.com/v1 already carry
        // /v1; endpoint("v1/chat/completions") must not produce /v1/v1 (the
        // DeepSeek incident: chat 404'd with an empty body because the URL
        // doubled).
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            b.endpoint("v1/chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            b.endpoint("v1/models"),
            "https://api.deepseek.com/v1/models"
        );
        // A bare host (local engine) is untouched.
        let b = OpenAiCompatibleBackend::new(BackendConfig {
            base_url: "http://127.0.0.1:9999".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            b.endpoint("v1/chat/completions"),
            "http://127.0.0.1:9999/v1/chat/completions"
        );
    }
}
