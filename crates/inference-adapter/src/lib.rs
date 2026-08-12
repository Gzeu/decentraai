//! Backend-neutral OpenAI-compatible inference adapter.

use async_trait::async_trait;
use futures_util::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::{pin::Pin, time::Duration};
use thiserror::Error;

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, BackendError>> + Send>>;

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_prompt_bytes: usize,
    pub max_output_tokens: u32,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self { base_url: "http://127.0.0.1:8081".into(), model: "local-model".into(), api_key: None, connect_timeout: Duration::from_secs(3), request_timeout: Duration::from_secs(300), max_prompt_bytes: 200_000, max_output_tokens: 8192 }
    }
}

#[derive(Debug, Clone)]
pub struct BackendRequest { pub request_id: String, pub prompt: String, pub max_tokens: u32, pub temperature: f32, pub top_p: f32 }

#[derive(Debug, Clone)]
pub struct BackendResponse { pub output: String, pub tokens_used: Option<u32>, pub finish_reason: Option<String> }

#[derive(Debug, Clone)]
pub struct StreamChunk { pub request_id: String, pub sequence: u64, pub text: String, pub finish_reason: Option<String> }

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
pub struct OpenAiCompatibleBackend { client: Client, config: BackendConfig }

impl OpenAiCompatibleBackend {
    pub fn new(config: BackendConfig) -> Result<Self, BackendError> {
        let client = Client::builder().connect_timeout(config.connect_timeout).timeout(config.request_timeout).build().map_err(|e| BackendError::Transport(e.to_string()))?;
        Ok(Self { client, config })
    }

    fn validate(&self, request: &BackendRequest) -> Result<(), BackendError> {
        if request.prompt.len() > self.config.max_prompt_bytes { return Err(BackendError::PromptTooLarge); }
        if request.max_tokens > self.config.max_output_tokens { return Err(BackendError::OutputLimitExceeded); }
        Ok(())
    }

    fn endpoint(&self, path: &str) -> String { format!("{}/{}", self.config.base_url.trim_end_matches('/'), path) }
    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder { self.config.api_key.as_ref().map_or(request, |key| request.bearer_auth(key)) }

    async fn http_error(response: reqwest::Response) -> BackendError {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_else(|_| "unreadable body".into());
        BackendError::Http { status, body }
    }
}

#[async_trait]
impl InferenceBackend for OpenAiCompatibleBackend {
    async fn health(&self) -> Result<(), BackendError> {
        let response = self.auth(self.client.get(self.endpoint("health"))).send().await.map_err(|e| if e.is_timeout() { BackendError::Timeout } else { BackendError::Transport(e.to_string()) })?;
        if response.status().is_success() { Ok(()) } else { Err(Self::http_error(response).await) }
    }

    async fn complete(&self, request: BackendRequest) -> Result<BackendResponse, BackendError> {
        self.validate(&request)?;
        let body = serde_json::json!({"model": self.config.model, "messages": [{"role": "user", "content": request.prompt}], "max_tokens": request.max_tokens, "temperature": request.temperature, "top_p": request.top_p, "stream": false});
        let response = self.auth(self.client.post(self.endpoint("v1/chat/completions"))).json(&body).send().await.map_err(|e| if e.is_timeout() { BackendError::Timeout } else { BackendError::Transport(e.to_string()) })?;
        if !response.status().is_success() { return Err(Self::http_error(response).await); }
        let raw: OpenAiResponse = response.json().await.map_err(|e| BackendError::Protocol(e.to_string()))?;
        let choice = raw.choices.into_iter().next().ok_or_else(|| BackendError::Protocol("missing choice".into()))?;
        Ok(BackendResponse { output: choice.message.content, tokens_used: raw.usage.and_then(|u| u.completion_tokens), finish_reason: choice.finish_reason })
    }

    async fn stream(&self, request: BackendRequest) -> Result<TokenStream, BackendError> {
        self.validate(&request)?;
        let body = serde_json::json!({"model": self.config.model, "messages": [{"role": "user", "content": request.prompt}], "max_tokens": request.max_tokens, "temperature": request.temperature, "top_p": request.top_p, "stream": true});
        let response = self.auth(self.client.post(self.endpoint("v1/chat/completions"))).json(&body).send().await.map_err(|e| if e.is_timeout() { BackendError::Timeout } else { BackendError::Transport(e.to_string()) })?;
        if !response.status().is_success() { return Err(Self::http_error(response).await); }
        Ok(Box::pin(parse_sse(response.bytes_stream(), request.request_id)))
    }
}

#[derive(Debug, Deserialize)] struct OpenAiResponse { choices: Vec<OpenAiChoice>, usage: Option<OpenAiUsage> }
#[derive(Debug, Deserialize)] struct OpenAiChoice { message: OpenAiMessage, finish_reason: Option<String> }
#[derive(Debug, Deserialize)] struct OpenAiMessage { content: String }
#[derive(Debug, Deserialize)] struct OpenAiUsage { completion_tokens: Option<u32> }
#[derive(Debug, Deserialize)] struct OpenAiStreamResponse { choices: Vec<OpenAiStreamChoice> }
#[derive(Debug, Deserialize)] struct OpenAiStreamChoice { delta: OpenAiDelta, finish_reason: Option<String> }
#[derive(Debug, Deserialize)] struct OpenAiDelta { content: Option<String> }

async fn parse_sse<S, E>(mut input: S, request_id: String) -> impl Stream<Item = Result<StreamChunk, BackendError>>
where S: Stream<Item = Result<bytes::Bytes, E>> + Unpin + Send + 'static, E: std::fmt::Display {
    use futures_util::StreamExt;
    async_stream::stream! {
        let mut buffer = String::new(); let mut sequence = 0;
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
    #[test] fn rejects_large_prompt() { let b = OpenAiCompatibleBackend::new(BackendConfig { max_prompt_bytes: 2, ..Default::default() }).unwrap(); let r = BackendRequest { request_id: "r".into(), prompt: "abc".into(), max_tokens: 1, temperature: 0.7, top_p: 0.9 }; assert!(matches!(b.validate(&r), Err(BackendError::PromptTooLarge))); }
    #[test] fn rejects_large_output() { let b = OpenAiCompatibleBackend::new(BackendConfig { max_output_tokens: 2, ..Default::default() }).unwrap(); let r = BackendRequest { request_id: "r".into(), prompt: "ok".into(), max_tokens: 3, temperature: 0.7, top_p: 0.9 }; assert!(matches!(b.validate(&r), Err(BackendError::OutputLimitExceeded))); }
}
