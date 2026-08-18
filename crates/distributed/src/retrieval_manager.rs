//! RetrievalManager — the runtime RAG half: a `RetrievalIndex` fed by the
//! embeddings backend (nomic-embed via `EmbeddingClient`), with index/query
//! operations exposed through the node API.
//!
//! The pure index (`decentraai_agents::RetrievalIndex`) stays in `crates/agents`;
//! this manager is the thin, stateful bridge that calls the embeddings backend
//! and holds the live index (Mutex-guarded, no async under the lock).

use anyhow::Result;
use decentraai_agents::{IndexedDocument, RetrievalIndex, RetrievalResult};
use std::sync::{Arc, Mutex};

use crate::embedding::EmbeddingClient;

/// Runtime RAG retrieval: index documents and query them by semantic
/// similarity using a live embeddings backend.
#[derive(Clone)]
pub struct RetrievalManager {
    index: Arc<Mutex<RetrievalIndex>>,
    embedding: Arc<EmbeddingClient>,
}

impl RetrievalManager {
    /// A manager that embeds via `embedding` and holds an empty index.
    pub fn new(embedding: Arc<EmbeddingClient>) -> Self {
        Self {
            index: Arc::new(Mutex::new(RetrievalIndex::new())),
            embedding,
        }
    }

    /// Embeds `text` and indexes it under `doc_id` (replaces on duplicate id).
    /// Returns the new document count.
    pub async fn index(
        &self,
        doc_id: &str,
        text: &str,
        capability: Option<String>,
    ) -> Result<usize> {
        if text.trim().is_empty() {
            anyhow::bail!("document text must not be empty");
        }
        let embedding = self.embedding.embed(text).await?;
        let doc = IndexedDocument {
            id: doc_id.to_string(),
            text: text.to_string(),
            capability,
            embedding,
            tags: vec![],
        };
        self.index.lock().unwrap().add(doc);
        Ok(self.index.lock().unwrap().len())
    }

    /// Removes a document by id; returns whether it existed.
    pub fn remove(&self, doc_id: &str) -> bool {
        self.index.lock().unwrap().remove(doc_id)
    }

    /// Embeds `text` and returns the top `k` most similar documents.
    pub async fn query(&self, text: &str, k: usize) -> Result<Vec<RetrievalResult>> {
        if text.trim().is_empty() {
            anyhow::bail!("query text must not be empty");
        }
        let k = k.clamp(1, 50);
        let embedding = self.embedding.embed(text).await?;
        Ok(self.index.lock().unwrap().search(&embedding, k))
    }

    /// Number of indexed documents.
    pub fn doc_count(&self) -> usize {
        self.index.lock().unwrap().len()
    }

    /// Whether the index holds no documents.
    pub fn is_empty(&self) -> bool {
        self.index.lock().unwrap().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_starts_empty() {
        let m = RetrievalManager::new(Arc::new(EmbeddingClient::new("http://127.0.0.1:1".into())));
        assert!(m.is_empty());
        assert_eq!(m.doc_count(), 0);
    }
}
