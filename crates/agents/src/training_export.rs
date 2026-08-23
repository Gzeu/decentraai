//! Training Lab export path (M18): verified, evidence-backed collective
//! knowledge → candidate dataset records.
//!
//! # The learning loop
//!
//! ```text
//! execution → result → verification → memory → learning candidate
//!     → (explicit operator action) → Training Lab dataset builder
//! ```
//!
//! This module is the last-but-one arrow: it filters the collective memory
//! down to entries that earned the right to become training data and renders
//! them as bounded JSONL records. It does NOT train anything, does NOT touch
//! [`crate::dataset`] state, and never runs automatically: an operator (or a
//! future, explicitly gated workflow) calls `to_jsonl` and feeds the output
//! to the dataset builder by hand.
//!
//! Honesty rules:
//! - Only entries whose lifecycle status is `verified`/`trusted` qualify.
//! - Only evidence-backed entries qualify (an explicit evidence reference —
//!   a verified execution/audit record — must exist).
//! - Only knowledge kinds that generalize (`learning`, `solution`,
//!   `model_evaluation`) are exported; raw observations and failures stay in
//!   memory as context, not as curriculum.
//!
//! Pure (no I/O) — same pattern as the rest of `crates/agents`.

use crate::memory::{KnowledgeKind, MemoryEntry, MemoryStatus};
use serde::{Deserialize, Serialize};

/// One candidate record for the Training Lab dataset builder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingCandidate {
    /// Source memory entry id (traceability back into the collective).
    pub entry_id: String,
    /// Source scope name.
    pub scope: String,
    /// Knowledge kind (`learning` | `solution` | `model_evaluation`).
    pub kind: KnowledgeKind,
    /// The generalized knowledge itself.
    pub content: String,
    /// Evidence reference that backs this claim (audit/execution record).
    pub evidence_ref: String,
    /// Claimed confidence percent 0..=100.
    pub confidence: u8,
    /// Authoring agent (collective contribution attribution).
    pub author_agent: String,
    /// Node the author ran on.
    pub author_node: String,
    /// When the source entry was created (unix ms).
    pub created_at_ms: u64,
    /// Source entry version at export time.
    pub version: u32,
}

impl TrainingCandidate {
    /// Serializes candidates as newline-delimited JSON, in input order
    /// (deterministic; the caller decides ordering via its input slice).
    pub fn to_jsonl(candidates: &[TrainingCandidate]) -> String {
        let mut out = String::new();
        for c in candidates {
            if let Ok(line) = serde_json::to_string(c) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }
}

/// Whether this knowledge kind may become training data.
///
/// Raw observations/executions/failures are *context* (they live in memory),
/// not curriculum; only generalizations that survived verification do.
fn exportable_kind(kind: KnowledgeKind) -> bool {
    matches!(
        kind,
        KnowledgeKind::Learning | KnowledgeKind::Solution | KnowledgeKind::ModelEvaluation
    )
}

/// Filters memory entries down to Training Lab candidates.
///
/// A candidate must be ALL of: lifecycle-verified (`verified`/`trusted`),
/// evidence-backed (explicit evidence reference present), an exportable
/// knowledge kind, and non-empty content. Input order is preserved so the
/// export is reproducible from the same memory state.
pub fn training_candidates(entries: &[MemoryEntry]) -> Vec<TrainingCandidate> {
    entries
        .iter()
        .filter(|e| {
            e.meta.status != MemoryStatus::Candidate
                && e.meta.status != MemoryStatus::Obsolete
                && e.meta.is_evidence_backed()
                && exportable_kind(e.meta.kind)
                && !e.content.trim().is_empty()
        })
        .map(|e| TrainingCandidate {
            entry_id: e.entry_id.clone(),
            scope: e.scope.clone(),
            kind: e.meta.kind,
            content: e.content.clone(),
            evidence_ref: e
                .meta
                .detail
                .as_ref()
                .and_then(|d| d.evidence_ref.clone())
                .unwrap_or_default(),
            confidence: e
                .meta
                .detail
                .as_ref()
                .map(|d| d.confidence)
                .unwrap_or(0),
            author_agent: e.author_agent.clone(),
            author_node: e.author_node.clone(),
            created_at_ms: e.created_at_ms,
            version: e.meta.version,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryMeta, MemoryProvenance};

    fn evidenced_entry(
        id: &str,
        kind: KnowledgeKind,
        status: MemoryStatus,
        evidence: Option<&str>,
        content: &str,
    ) -> MemoryEntry {
        let mut e = MemoryEntry::new(id, "learnings", "researcher", "node-1", content)
            .with_kind(kind);
        e.meta.status = status;
        e.meta.detail = Some(
            MemoryProvenance::new("execution", "researcher", "node-1", 42, 90)
                .with_evidence(evidence.unwrap_or("")),
        );
        if evidence.is_none() {
            e.meta.detail.as_mut().unwrap().evidence_ref = None;
        }
        e
    }

    #[test]
    fn only_verified_and_evidenced_generalizations_export() {
        let entries = vec![
            evidenced_entry("ok", KnowledgeKind::Learning, MemoryStatus::Verified, Some("aud-1"), "retry with backoff"),
            // Unverified candidate: excluded even with evidence ref.
            evidenced_entry("cand", KnowledgeKind::Learning, MemoryStatus::Candidate, Some("aud-2"), "guess"),
            // Verified but no evidence ref: unverified assertion, excluded.
            evidenced_entry("noev", KnowledgeKind::Solution, MemoryStatus::Verified, None, "assertion only"),
            // Obsolete: excluded even though it was trusted once.
            evidenced_entry("old", KnowledgeKind::ModelEvaluation, MemoryStatus::Obsolete, Some("aud-3"), "stale"),
            // Raw observation: context, not curriculum.
            evidenced_entry("obs", KnowledgeKind::Observation, MemoryStatus::Verified, Some("aud-4"), "raw fact"),
            evidenced_entry("sol", KnowledgeKind::Solution, MemoryStatus::Trusted, Some("aud-5"), "restart llama-server on OOM"),
        ];
        let got = training_candidates(&entries);
        let ids: Vec<&str> = got.iter().map(|c| c.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["ok", "sol"], "only verified+evidenced generalizations");
        assert_eq!(got[0].evidence_ref, "aud-1");
        assert_eq!(got[0].confidence, 90);
        assert_eq!(got[0].kind, KnowledgeKind::Learning);
    }

    #[test]
    fn jsonl_is_deterministic_and_round_trips() {
        let entries = vec![
            evidenced_entry("a", KnowledgeKind::Learning, MemoryStatus::Verified, Some("e1"), "one"),
            evidenced_entry("b", KnowledgeKind::Solution, MemoryStatus::Trusted, Some("e2"), "two"),
        ];
        let jsonl = TrainingCandidate::to_jsonl(&training_candidates(&entries));
        let again = TrainingCandidate::to_jsonl(&training_candidates(&entries));
        assert_eq!(jsonl, again, "same input → same bytes");
        let lines: Vec<TrainingCandidate> = jsonl
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].entry_id, "a");
        assert_eq!(lines[1].content, "two");
    }

    #[test]
    fn default_meta_never_exports() {
        // A bare legacy entry (no meta) is candidate/observation/no-evidence:
        // exactly what must NOT leak into training data automatically.
        let bare = vec![MemoryEntry::new("x", "s", "a", "n", "legacy")];
        assert!(bare[0].meta == MemoryMeta::default());
        assert!(training_candidates(&bare).is_empty());
    }
}
