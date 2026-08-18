//! EmbeddingClient — a thin client to a local OpenAI-compatible embeddings
//! backend (a llama-server launched with `--embedding`, e.g. on
//! `nomic-embed-text-v1.5`). Feeds the RAG retrieval index
//! (`decentraai_agents::RetrievalIndex`).
//!
//! This is a pure HTTP client over the standard `/v1/embeddings` shape; it
//! does not manage the backend process (the operator launches it) and never
//! inspects prompts/outputs for logging.

use anyhow::{Context, Result};
use std::sync::Arc;

/// Client for an OpenAI-compatible embeddings backend.
#[derive(Clone)]
pub struct EmbeddingClient {
    client: Arc<reqwest::Client>,
    backend_url: String,
}

impl EmbeddingClient {
    /// A client pointed at `backend_url` (must expose `/v1/embeddings`).
    pub fn new(backend_url: String) -> Self {
        Self {
            client: Arc::new(reqwest::Client::new()),
            backend_url,
        }
    }

    /// Embeds a single text and returns the embedding vector.
    pub async fn embed(&self, input: &str) -> Result<Vec<f32>> {
        let body = serde_json::json!({ "input": input });
        let endpoint = format!("{}/v1/embeddings", self.backend_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .context("calling embeddings backend")?;
        let status = resp.status();
        let payload: serde_json::Value =
            resp.json().await.context("parsing embeddings response")?;
        if !status.is_success() {
            anyhow::bail!(
                "embeddings backend returned {status}: {}",
                payload
                    .get("error")
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            );
        }
        payload
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|d| d.first())
            .and_then(|d| d.get("embedding"))
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_f64().map(|x| x as f32))
                    .collect()
            })
            .ok_or_else(|| anyhow::anyhow!("embeddings response had no embedding vector"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_has_backend_url() {
        let c = EmbeddingClient::new("http://127.0.0.1:9999".to_string());
        assert_eq!(c.backend_url, "http://127.0.0.1:9999");
    }
}
