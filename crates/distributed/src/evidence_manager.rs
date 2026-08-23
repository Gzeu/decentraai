//! Evidence RAG runtime — the fabric's experimental memory, wired to live
//! sources.
//!
//! `EvidenceManager` owns the pure `EvidenceIndex` and feeds it from the real
//! runtime sources, idempotently (each source record maps to a stable id, so
//! syncing twice never duplicates):
//! - `ComputeManager::executions()` → `exec:<request_id>` (M18/M20/M23 plans);
//! - `KnowledgeRuntime::view()` receipts/decisions → `receipt:<exec_id>`,
//!   `decision:<id>` (P12 verified work + collective decisions);
//! - `MemoryStore` collective scopes → `memory:<entry_id>` (P5).
//!
//! The source of truth stays the sources: the index is a derived, bounded view
//! and is rebuilt from them, so it is never persisted on its own.
//!
//! Query is honest about what it used: with an embedding backend configured it
//! ranks semantically (`mode: "semantic"`); without one it matches
//! structurally (`mode: "structural"`) — the caller can see which. Lessons are
//! derived deterministically from whatever evidence exists; zero evidence in,
//! zero lessons out.

use std::sync::{Arc, Mutex};

use decentraai_agents::evidence::{
    EvidenceEntry, EvidenceFamily, EvidenceHit, EvidenceIndex, EvidenceSummary, lessons,
};

use crate::agent_memory::MemoryStore;
use crate::compute::ComputeManager;
use crate::embedding::EmbeddingClient;
use crate::knowledge_runtime::{KNOWLEDGE_MEMORY_SCOPE, KnowledgeRuntime};

/// Maximum number of memory entries indexed per scope on one sync (bounded
/// index; the store keeps the full history).
const MAX_MEMORY_ENTRIES_PER_SCOPE: usize = 200;
/// Maximum number of executions indexed on one sync (the compute manager
/// keeps its own bounded ring).
const MAX_EXECUTIONS_PER_SYNC: usize = 100;

/// Runtime wrapper over the pure evidence index.
pub struct EvidenceManager {
    index: Arc<Mutex<EvidenceIndex>>,
    /// Optional real embedding backend. `None` → structural-only querying
    /// (honest `mode: "structural"`).
    embedding: Option<Arc<EmbeddingClient>>,
}

impl EvidenceManager {
    /// Creates an empty evidence manager.
    pub fn new(embedding: Option<Arc<EmbeddingClient>>) -> Self {
        Self {
            index: Arc::new(Mutex::new(EvidenceIndex::new())),
            embedding,
        }
    }

    /// The live index handle (for tests and control-plane reads).
    pub fn index(&self) -> &Arc<Mutex<EvidenceIndex>> {
        &self.index
    }

    /// Records one executed plan from the compute manager's real history.
    fn record_execution(&self, plan: &crate::compute::ExecutedPlan) {
        let text = format!(
            "execution {} → worker {} (outcome {}){}",
            plan.request_id,
            plan.selected_worker,
            plan.outcome,
            if plan.is_continuation {
                " [KV continuation]"
            } else {
                ""
            }
        );
        let mut entry = EvidenceEntry::new(
            format!("exec:{}", plan.request_id),
            EvidenceFamily::Execution,
            text,
            plan.ts * 1000,
        )
        .tagged(format!("outcome:{}", plan.outcome))
        .tagged(format!("worker:{}", plan.selected_worker))
        .tagged(format!("model:{}", plan.model_hash));
        if let Some(dur) = plan.processing_time_ms {
            entry = entry.tagged(format!("duration_ms:{dur}"));
        }
        entry = entry.tagged(format!("rtt_ms:{}", plan.network_rtt_ms));
        if let Some(tokens) = plan.tokens_used {
            entry = entry.tagged(format!("tokens:{tokens}"));
        }
        if let Ok(mut ix) = self.index.lock() {
            ix.add(entry);
        }
    }

    /// Records one memory entry from a collective scope (P5).
    fn record_memory(&self, entry: &decentraai_agents::memory::MemoryEntry) {
        let e = EvidenceEntry::new(
            format!("memory:{}", entry.entry_id),
            EvidenceFamily::Memory,
            entry.content.clone(),
            entry.created_at_ms,
        )
        .tagged(format!("scope:{}", entry.scope))
        .tagged(format!("author:{}", entry.author_agent));
        if let Ok(mut ix) = self.index.lock() {
            ix.add(e);
        }
    }

    /// Syncs the compute manager's real execution history (idempotent on
    /// request id). Best-effort: never fails the caller.
    pub fn sync_from_compute(&self, compute: &ComputeManager) {
        for plan in compute
            .executions()
            .into_iter()
            .take(MAX_EXECUTIONS_PER_SYNC)
        {
            self.record_execution(&plan);
        }
    }

    /// Syncs P12 receipts + decisions from the knowledge runtime (idempotent).
    pub fn sync_from_knowledge(&self, knowledge: &KnowledgeRuntime) {
        let view = knowledge.view();
        for receipt in &view.receipts {
            // ReceiptView has no created_at_ms of its own; build the entry
            // from the view's fact fields only — credits are not evidence.
            let entry = EvidenceEntry::new(
                format!("receipt:{}", receipt.execution_id),
                EvidenceFamily::Receipt,
                format!(
                    "execution {} on {} ({}) {}",
                    receipt.execution_id, receipt.worker_node, receipt.capability, receipt.verdict
                ),
                receipt.created_at_ms,
            )
            .tagged(format!("verdict:{}", receipt.verdict))
            .tagged(format!("worker:{}", receipt.worker_node))
            .tagged(format!("capability:{}", receipt.capability));
            if let Ok(mut ix) = self.index.lock() {
                ix.add(entry);
            }
        }
        for decision in &view.decisions {
            let entry = EvidenceEntry::new(
                format!("decision:{}", decision.decision_id),
                EvidenceFamily::Consensus,
                format!(
                    "decision {}: {} ({})",
                    decision.decision_id, decision.summary, decision.verdict
                ),
                decision.created_at_ms,
            )
            .tagged(format!("verdict:{}", decision.verdict));
            if let Ok(mut ix) = self.index.lock() {
                ix.add(entry);
            }
        }
    }

    /// Syncs collective memory scopes from the persistent store (bounded,
    /// idempotent on entry id).
    pub fn sync_from_memory(&self, store: &MemoryStore) {
        let Ok(scopes) = store.list_scopes() else {
            return;
        };
        for scope in scopes {
            if !scope.name.starts_with("collective.") {
                continue;
            }
            let Ok(entries) = store.read(&scope.name, "evidence", true) else {
                continue;
            };
            for entry in entries.into_iter().take(MAX_MEMORY_ENTRIES_PER_SCOPE) {
                self.record_memory(&entry);
            }
        }
    }

    /// Best-effort sync from every attached live source.
    pub fn sync_all(
        &self,
        compute: Option<&ComputeManager>,
        knowledge: Option<&KnowledgeRuntime>,
        memory: Option<&MemoryStore>,
    ) {
        if let Some(c) = compute {
            self.sync_from_compute(c);
        }
        if let Some(k) = knowledge {
            self.sync_from_knowledge(k);
        }
        if let Some(m) = memory {
            self.sync_from_memory(m);
        }
    }

    /// Queries the evidence index. Semantic when a real embedding backend is
    /// configured (best-effort — a backend failure falls back to structural
    /// terms, and the hit mode says which path produced each hit); structural
    /// otherwise. `k` bounds semantic results (structural returns all matches).
    pub async fn query(&self, text: &str, k: usize) -> Vec<EvidenceHit> {
        if let Some(emb) = &self.embedding {
            if let Ok(vec) = emb.embed(text).await {
                if let Ok(ix) = self.index.lock() {
                    let hits = ix.semantic(&vec, k);
                    if !hits.is_empty() {
                        return hits;
                    }
                    // No semantic matches (or no vectors indexed) — the honest
                    // fallback is structural matching, clearly labeled.
                }
            }
        }
        let terms: Vec<String> = text
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        self.index
            .lock()
            .map(|ix| ix.query(&terms))
            .unwrap_or_default()
    }

    /// Control-plane snapshot with derived lessons.
    pub fn summary(&self, limit: usize) -> EvidenceSummary {
        self.index
            .lock()
            .map(|ix| ix.summary(limit))
            .unwrap_or_else(|_| EvidenceSummary {
                total: 0,
                counts: Default::default(),
                recent: Vec::new(),
                lessons: lessons(&[]),
            })
    }
}

/// Convenience: builds the memory-scope constant for the dashboard.
pub fn knowledge_memory_scope() -> &'static str {
    KNOWLEDGE_MEMORY_SCOPE
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::knowledge::{Evidence, EvidenceKind, KnowledgeObject};
    use decentraai_agents::receipt::{ReceiptVerdict, VerifiedComputeReceipt};
    use decentraai_agents::verification::ConsensusPolicy;
    use decentraai_compute::compensation::CompensationLedger;

    fn manager() -> EvidenceManager {
        EvidenceManager::new(None)
    }

    fn plan(id: &str, outcome: &str, rtt_ms: u32) -> crate::compute::ExecutedPlan {
        crate::compute::ExecutedPlan {
            request_id: id.to_string(),
            plan_id: "p".into(),
            model_hash: "m1".into(),
            selected_worker: "peer-a".into(),
            score: 0.5,
            stages: 1,
            reservation_id: "r".into(),
            is_continuation: false,
            prefix_worker: None,
            network_rtt_ms: rtt_ms,
            kv_headroom: "1/1".into(),
            outcome: outcome.to_string(),
            reasoning: "".into(),
            ts: 1000,
            tokens_used: None,
            processing_time_ms: None,
            attempt: 0,
            est_ram_mb: 100,
            est_vram_mb: 0,
        }
    }

    fn receipt(id: &str, at_ms: u64) -> VerifiedComputeReceipt {
        VerifiedComputeReceipt::new(
            id,
            "peer-worker",
            "a:worker",
            "inference",
            120,
            ReceiptVerdict::Verified,
            at_ms,
        )
        .with_output_hash("blake3:abc")
    }

    #[test]
    fn record_execution_is_idempotent_on_request_id() {
        let mgr = manager();
        mgr.record_execution(&plan("req-1", "succeeded", 15));
        mgr.record_execution(&plan("req-1", "succeeded", 15)); // duplicate id

        let ix = mgr.index().lock().unwrap();
        assert_eq!(ix.len(), 1);
        assert_eq!(ix.counts()[&EvidenceFamily::Execution], 1);
    }

    #[test]
    fn sync_from_knowledge_indexes_receipts_and_decisions() {
        let mgr = manager();
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let knowledge = KnowledgeRuntime::new(compensation, "peer-local", None).unwrap();
        knowledge
            .record_receipt(&receipt("e1", 1000), &Default::default())
            .unwrap();

        // A real decision needs a knowledge object (decide rejects empty).
        let ko = KnowledgeObject::new("k1", "the fact holds", "a:coord", "peer-local", 1000)
            .with_evidence(vec![Evidence::new(
                EvidenceKind::VerifiedExecution,
                "verified by receipt",
            )]);
        knowledge
            .decide(
                "d1",
                "the fact holds",
                "a:coord",
                &[ko],
                &ConsensusPolicy {
                    required_agents: 1,
                    agreement_threshold: 0.5,
                    require_schema: false,
                },
                2000,
            )
            .unwrap();

        mgr.sync_from_knowledge(&knowledge);

        let ix = mgr.index().lock().unwrap();
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.counts()[&EvidenceFamily::Receipt], 1);
        assert_eq!(ix.counts()[&EvidenceFamily::Consensus], 1);
    }

    #[test]
    fn query_falls_back_to_structural_without_embeddings() {
        let mgr = manager();
        mgr.record_execution(&plan("req-1", "succeeded", 15));
        mgr.record_execution(&plan("req-2", "failed", 300));

        let hits = futures::executor::block_on(mgr.query("succeeded", 10));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].mode, "structural");
        assert_eq!(hits[0].id, "exec:req-1");

        let both = futures::executor::block_on(mgr.query("peer-a", 10));
        assert_eq!(both.len(), 2); // both plans ran on peer-a
    }

    #[test]
    fn summary_derives_lessons_from_real_evidence() {
        let mgr = manager();
        mgr.record_execution(&plan("req-1", "succeeded", 15));
        mgr.record_execution(&plan("req-2", "failed", 300));

        let s = mgr.summary(10);
        assert_eq!(s.total, 2);
        let rate = s
            .lessons
            .iter()
            .find(|l| l.id == "executions/success_rate")
            .unwrap();
        assert_eq!(rate.sample, 2);
        assert_eq!(rate.value, 0.5);
        let rtt = s
            .lessons
            .iter()
            .find(|l| l.id == "network/median_rtt_ms")
            .unwrap();
        assert_eq!(rtt.value, 157.5); // median of 15,300
    }

    #[test]
    fn summary_has_no_synthetic_lessons_when_empty() {
        let s = manager().summary(10);
        assert_eq!(s.total, 0);
        assert!(s.recent.is_empty());
        assert!(s.lessons.iter().all(|l| l.sample == 0 && l.value == 0.0));
    }
}
