//! RAG retrieval foundation (pure) — the index that a real embedding model
//! (e.g. `nomic-embed-text-v1.5`) will populate at runtime.
//!
//! The architecture chain `Dataset → indexed knowledge → embeddings →
//! retrieval capability → agent capability → execution` needs a concrete,
//! deterministic retrieval index. This module is that index, kept pure (no
//! I/O, no async) so it is unit-testable and trivially serializable — the same
//! pattern as the rest of `crates/agents`. The embedding vectors come from an
//! external model at runtime; here they are just `Vec<f32>`.
//!
//! Honesty: similarity here is plain cosine over whatever vectors the runtime
//! provides. The index does not claim to understand semantics — it ranks by
//! cosine distance. A document is only retrieved by its embedding; garbage
//! embeddings in, garbage ranks out.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A document indexed for retrieval, carrying its embedding vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedDocument {
    /// Stable document id.
    pub id: String,
    /// The text this document carries (for display / passing to an agent).
    pub text: String,
    /// Capability the document supports (e.g. Retrieval, Coding, Knowledge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// The embedding vector (from an external model at runtime).
    pub embedding: Vec<f32>,
    /// Tags for coarse filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// A ranked retrieval hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalResult {
    /// Document id.
    pub doc_id: String,
    /// Cosine similarity in 0..=1 (higher = more similar).
    pub score: f32,
    /// The document's text.
    pub text: String,
    /// The document's capability, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}

/// Deterministic in-memory retrieval index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetrievalIndex {
    docs: BTreeMap<String, IndexedDocument>,
}

impl RetrievalIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Indexes a document; replaces any document with the same id.
    pub fn add(&mut self, doc: IndexedDocument) {
        self.docs.insert(doc.id.clone(), doc);
    }

    /// Removes a document by id; returns whether it existed.
    pub fn remove(&mut self, doc_id: &str) -> bool {
        self.docs.remove(doc_id).is_some()
    }

    /// Looks up a document by id.
    pub fn get(&self, doc_id: &str) -> Option<&IndexedDocument> {
        self.docs.get(doc_id)
    }

    /// Number of indexed documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// All document ids, sorted (deterministic).
    pub fn ids(&self) -> Vec<String> {
        self.docs.keys().cloned().collect()
    }

    /// Searches the index by a query embedding and returns the top `k`
    /// documents by cosine similarity, highest first, ties by doc id asc.
    /// Empty-vector queries match nothing.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<RetrievalResult> {
        if query.is_empty() || self.docs.is_empty() {
            return Vec::new();
        }
        let qn = norm(query);
        if qn <= f32::EPSILON {
            return Vec::new();
        }
        let mut scored: Vec<(&IndexedDocument, f32)> = self
            .docs
            .values()
            .filter_map(|d| {
                let s = cosine_similarity(query, &d.embedding);
                (s > f32::EPSILON).then_some((d, s))
            })
            .collect();
        scored.sort_by(|a, b| {
            // score desc, then doc id asc (deterministic).
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        scored
            .into_iter()
            .take(k)
            .map(|(d, s)| RetrievalResult {
                doc_id: d.id.clone(),
                score: s,
                text: d.text.clone(),
                capability: d.capability.clone(),
            })
            .collect()
    }

    /// Searches by a text whose embedding is provided by the caller (the
    /// runtime computes it with the embedding model).
    pub fn search_embedding(&self, query_embedding: &[f32], k: usize) -> Vec<RetrievalResult> {
        self.search(query_embedding, k)
    }
}

/// Cosine similarity between two non-empty vectors, in 0..=1 (returns 0 for
/// orthogonal/zero vectors). Pure and deterministic.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(0.0, 1.0)
    }
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, text: &str, embedding: Vec<f32>) -> IndexedDocument {
        IndexedDocument {
            id: id.to_string(),
            text: text.to_string(),
            capability: None,
            embedding,
            tags: vec![],
        }
    }

    #[test]
    fn cosine_similarity_identical_is_one() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
        assert!((cosine_similarity(&[1.0, 1.0], &[1.0, 1.0]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    }

    #[test]
    fn cosine_similarity_length_mismatch_is_zero() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn index_add_remove_get_len() {
        let mut idx = RetrievalIndex::new();
        assert!(idx.is_empty());
        idx.add(doc("a", "text a", vec![1.0, 0.0]));
        idx.add(doc("b", "text b", vec![0.0, 1.0]));
        assert_eq!(idx.len(), 2);
        assert!(idx.get("a").is_some());
        assert!(idx.remove("a"));
        assert!(!idx.remove("a"));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.ids(), vec!["b"]);
    }

    #[test]
    fn search_returns_most_similar_first() {
        let mut idx = RetrievalIndex::new();
        idx.add(doc("b", "less similar", vec![0.1, 0.9]));
        idx.add(doc("a", "similar", vec![0.9, 0.1]));
        let results = idx.search(&[1.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        // 'a' is most similar to [1,0]; then 'b'.
        assert_eq!(results[0].doc_id, "a");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn search_top_k_and_empty_query() {
        let mut idx = RetrievalIndex::new();
        idx.add(doc("a", "a", vec![1.0, 0.0]));
        idx.add(doc("b", "b", vec![0.9, 0.1]));
        let top1 = idx.search(&[1.0, 0.0], 1);
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].doc_id, "a");
        // Empty query matches nothing.
        assert!(idx.search(&[], 5).is_empty());
    }

    #[test]
    fn tie_break_by_doc_id_asc() {
        let mut idx = RetrievalIndex::new();
        idx.add(doc("b", "b", vec![1.0, 0.0]));
        idx.add(doc("a", "a", vec![1.0, 0.0]));
        let results = idx.search(&[1.0, 0.0], 2);
        assert_eq!(results[0].doc_id, "a");
        assert_eq!(results[1].doc_id, "b");
    }

    #[test]
    fn index_and_results_round_trip_over_json() {
        let mut idx = RetrievalIndex::new();
        idx.add(doc("a", "text a", vec![1.0, 0.0]));
        let json = serde_json::to_string(&idx).unwrap();
        let back: RetrievalIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.get("a").unwrap().text, "text a");
    }
}
