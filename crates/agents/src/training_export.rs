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
    /// Content of the VERIFIED failure this solution resolved (same
    /// subject key), when one exists. Problem+solution pairs are a far
    /// richer training signal than solutions alone; absent = standalone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_failure: Option<String>,
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
///
/// Failure→Solution pairing (M19): a VERIFIED, evidenced `failure` entry is
/// matched to later verified `solution` entries on the same non-empty
/// subject key; the exported solution then carries the failure content in
/// `paired_failure`. A failure alone never exports — it only enriches its
/// solution. Unverified failures poison nothing: they are invisible to the
/// pairing map.
pub fn training_candidates(entries: &[MemoryEntry]) -> Vec<TrainingCandidate> {
    use std::collections::HashMap;
    // Verified failures by subject key → earliest verified failure wins
    // (deterministic: first in input order).
    let mut failures: HashMap<&str, &str> = HashMap::new();
    for e in entries {
        if e.meta.kind == KnowledgeKind::Failure
            && e.meta.status == MemoryStatus::Verified
            && e.meta.is_evidence_backed()
            && !e.meta.subject_key.is_empty()
            && !e.content.trim().is_empty()
        {
            failures.entry(e.meta.subject_key.as_str()).or_insert(&e.content);
        }
    }
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
            paired_failure: if e.meta.kind == KnowledgeKind::Solution && !e.meta.subject_key.is_empty() {
                failures.get(e.meta.subject_key.as_str()).map(|f| f.chars().take(1024).collect())
            } else {
                None
            },
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

    fn verified_with_subject(
        id: &str,
        kind: KnowledgeKind,
        status: MemoryStatus,
        evidence: Option<&str>,
        content: &str,
        subject: &str,
    ) -> MemoryEntry {
        let mut e = evidenced_entry(id, kind, status, evidence, content);
        e.meta.subject_key = subject.to_string();
        e
    }

    #[test]
    fn failure_pairs_with_its_verified_solution_only() {
        let entries = vec![
            // The verified failure on subject q:oom.
            verified_with_subject("f1", KnowledgeKind::Failure, MemoryStatus::Verified, Some("aud-f"), "llama-server OOM at 8k ctx", "q:oom"),
            // Its verified solution → must carry paired_failure.
            verified_with_subject("s1", KnowledgeKind::Solution, MemoryStatus::Verified, Some("aud-s"), "cap ctx at 4k or raise swap", "q:oom"),
            // UNVERIFIED failure on q:leak: invisible to pairing.
            verified_with_subject("f2", KnowledgeKind::Failure, MemoryStatus::Candidate, Some("aud-x"), "unconfirmed leak", "q:leak"),
            // Solution for the unverified failure → standalone, no pair.
            verified_with_subject("s2", KnowledgeKind::Solution, MemoryStatus::Trusted, Some("aud-l"), "fix leak workaround", "q:leak"),
            // Solution on a DIFFERENT failure that is obsolete → no pair.
            verified_with_subject("f3", KnowledgeKind::Failure, MemoryStatus::Obsolete, Some("aud-o"), "old stale failure", "q:stale"),
            verified_with_subject("s3", KnowledgeKind::Solution, MemoryStatus::Verified, Some("aud-t"), "standalone fix", "q:stale"),
        ];
        let got = training_candidates(&entries);
        // Failures never export alone; only solutions do.
        let ids: Vec<&str> = got.iter().map(|c| c.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "s2", "s3"]);
        let by_id = |id: &str| got.iter().find(|c| c.entry_id == id).unwrap();
        assert_eq!(by_id("s1").paired_failure.as_deref(), Some("llama-server OOM at 8k ctx"));
        assert_eq!(by_id("s2").paired_failure, None, "unverified failure poisons nothing");
        assert_eq!(by_id("s3").paired_failure, None, "obsolete failure does not pair");
    }
}
