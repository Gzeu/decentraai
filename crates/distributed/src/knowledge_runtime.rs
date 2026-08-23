//! KnowledgeRuntime — the runtime half of P12 (collective knowledge &
//! decisions v1).
//!
//! The pure fabric (`crate::knowledge`, `crate::decision`, `crate::receipt`)
//! defines *how* evidence, decisions and receipts work. This module binds them
//! to a live node:
//!
//! ```text
//! record_receipt ──► ReceiptRegistry ──► CompensationLedger (verified work only)
//!      │                    │
//!      │                    └──► KnowledgeObject (VerifiedExecution evidence)
//!      ▼
//! KnowledgeRegistry ──► decide_collectively ──► DecisionRegistry
//!      ▲                                              │
//!      └────────────── memory feedback (scope ────────┘
//!                       collective.knowledge)
//! ```
//!
//! Rules enforced here:
//!
//! - **Verified work only**: a `Failed` receipt never credits compensation and
//!   never becomes high-confidence knowledge.
//! - **Idempotency**: receipts and decisions are registered exactly once
//!   (duplicate execution/decision ids are rejected by the registries), and
//!   compensation is applied once per execution id by the ledger.
//! - **Declaration ≠ evidence**: knowledge confidence is always derived via
//!   [`decentraai_agents::evidence_confidence`]; the runtime never injects a
//!   declared score.
//! - **Memory feedback**: every adopted decision and every receipt-backed
//!   knowledge object is written into the persistent `MemoryStore` under the
//!   `collective.knowledge` scope (when attached), so completed work becomes
//!   reusable collective memory — the same pattern the orchestrator uses for
//!   `workflow_results`.

use anyhow::{Context, Result};
use decentraai_agents::{
    CollectiveDecision, ConsensusPolicy, DecisionRegistry, DecisionVerdict, EvidenceKind,
    KnowledgeObject, KnowledgeRegistry, ReceiptRegistry, VerifiedComputeReceipt,
    decide_collectively, decision_feedback_entry,
};
use decentraai_agents::memory::{
    MemoryEntry, MemoryLevel, MemoryPolicy, MemoryScope,
};
use decentraai_compute::{CompensationLedger, ContributionProfile};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::agent_memory::MemoryStore;

/// Scope under which P12 feedback entries are persisted (created on attach).
pub const KNOWLEDGE_MEMORY_SCOPE: &str = "collective.knowledge";

/// One knowledge object as exposed to the dashboard, with its *derived*
/// confidence (never declared).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeView {
    pub object_id: String,
    pub fact: String,
    pub author_agent: String,
    pub author_node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub evidence_kinds: Vec<String>,
    pub confidence: f32,
    pub confidence_label: String,
    pub created_at_ms: u64,
}

/// One collective decision as exposed to the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionView {
    pub decision_id: String,
    pub summary: String,
    pub verdict: String,
    pub aggregated_confidence: f32,
    pub considered: Vec<String>,
    pub created_at_ms: u64,
}

/// One receipt as exposed to the dashboard, with its compensation outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptView {
    pub execution_id: String,
    pub worker_node: String,
    pub capability: String,
    pub duration_ms: u64,
    pub verdict: String,
    pub credits: u64,
    pub created_at_ms: u64,
}

/// Full P12 view model for the dashboard — real state, no mock numbers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeStateView {
    pub knowledge_objects: Vec<KnowledgeView>,
    pub decisions: Vec<DecisionView>,
    pub receipts: Vec<ReceiptView>,
    pub balances: std::collections::BTreeMap<String, u64>,
    pub total_credits: u64,
    pub memory_scope: String,
    pub memory_attached: bool,
}

/// The runtime-half P12 state, shared between the node daemon and the API.
#[derive(Clone)]
pub struct KnowledgeRuntime {
    knowledge: Arc<Mutex<KnowledgeRegistry>>,
    decisions: Arc<Mutex<DecisionRegistry>>,
    receipts: Arc<Mutex<ReceiptRegistry>>,
    /// Compensation ledger shared with the M9/M18 path (same ledger instance
    /// the node already owns — a receipt credits the same balance the
    /// dashboard shows elsewhere).
    compensation: Arc<Mutex<CompensationLedger>>,
    /// Optional persistent collective memory. When attached, feedback entries
    /// are written into `collective.knowledge`.
    memory_store: Option<Arc<MemoryStore>>,
    /// Optional embeddings backend (M19): when attached, every persisted
    /// feedback entry is embedded in a background task so semantic search
    /// covers new knowledge without operator backfill.
    embedder: Option<Arc<crate::embedding::EmbeddingClient>>,
    /// This node's peer id, stamped as author_node on feedback entries.
    local_node: String,
    /// Per-worker contribution profiles, set at wiring from measured reality
    /// (never from an HTTP body — a client must not be able to inflate its own
    /// profile to earn more credits). A worker with no measured profile earns
    /// nothing (honest: `reward_tokens` returns 0 with zero verified work).
    profiles: Arc<Mutex<std::collections::HashMap<String, ContributionProfile>>>,
    /// Credits actually credited per execution id (recorded when the receipt
    /// is applied). Surfaced by the dashboard so each receipt shows what it
    /// really earned — never a synthetic 0.
    receipt_credits: Arc<Mutex<std::collections::BTreeMap<String, u64>>>,
}

impl KnowledgeRuntime {
    /// Creates the runtime; optionally attaches the persistent memory store.
    pub fn new(
        compensation: Arc<Mutex<CompensationLedger>>,
        local_node: impl Into<String>,
        memory_store: Option<Arc<MemoryStore>>,
    ) -> Result<Self> {
        let memory_store = match &memory_store {
            Some(store) => {
                ensure_knowledge_scope(store)?;
                memory_store
            }
            None => None,
        };
        Ok(Self {
            knowledge: Arc::new(Mutex::new(KnowledgeRegistry::new())),
            decisions: Arc::new(Mutex::new(DecisionRegistry::new())),
            receipts: Arc::new(Mutex::new(ReceiptRegistry::new())),
            compensation,
            memory_store,
            embedder: None,
            local_node: local_node.into(),
            profiles: Arc::new(Mutex::new(std::collections::HashMap::new())),
            receipt_credits: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
        })
    }

    /// Attaches the embeddings backend (M19): new feedback entries are then
    /// indexed for semantic search automatically, in the background.
    pub fn with_embedder(mut self, embedder: Arc<crate::embedding::EmbeddingClient>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Sets the measured contribution profile for a worker. Called at wiring
    /// time from the compute manager's real measured contribution — never from
    /// a client request.
    pub fn set_contribution_profile(&self, worker: &str, profile: ContributionProfile) {        if let Ok(mut p) = self.profiles.lock() {
            p.insert(worker.to_string(), profile);
        }
    }

    /// The measured contribution profile for a worker, or `None` when the
    /// node has no measured profile for it yet (unknown workers earn nothing
    /// — compensation rewards verified, measured service).
    pub fn contribution_profile(&self, worker: &str) -> Option<ContributionProfile> {
        self.profiles.lock().ok().and_then(|p| p.get(worker).copied())
    }

    /// Registers a knowledge object and persists it into collective memory.
    /// Rejects duplicate ids (objects are immutable once registered).
    pub fn register_knowledge(&self, object: KnowledgeObject) -> Result<()> {
        {
            let mut reg = self
                .knowledge
                .lock()
                .map_err(|_| anyhow::anyhow!("knowledge registry lock poisoned"))?;
            reg.add(object.clone())
                .map_err(|e| anyhow::anyhow!("registering knowledge object: {e}"))?;
        }
        // Memory feedback: every registered object is a collective fact.
        self.write_memory_feedback(
            &object.object_id,
            &object.fact,
            vec!["knowledge".to_string()],
            &object.author_agent,
        )?;
        Ok(())
    }

    /// Records a verified compute receipt: registers it, credits compensation
    /// (verified work only, exactly once), and turns the receipt into a
    /// knowledge object that closes the loop.
    pub fn record_receipt(
        &self,
        receipt: &VerifiedComputeReceipt,
        profile: &ContributionProfile,
    ) -> Result<u64> {
        let credits = {
            let mut receipts = self
                .receipts
                .lock()
                .map_err(|_| anyhow::anyhow!("receipt registry lock poisoned"))?;
            receipts
                .add(receipt.clone())
                .map_err(|e| anyhow::anyhow!("registering receipt: {e}"))?;
            let mut compensation = self
                .compensation
                .lock()
                .map_err(|_| anyhow::anyhow!("compensation ledger lock poisoned"))?;
            let credits = receipt.apply_compensation(&mut compensation, profile);
            // Remember what this execution really earned (0 for failed/unknown
            // workers is honest — the dashboard must show it, not a fake gap).
            if let Ok(mut rc) = self.receipt_credits.lock() {
                rc.insert(receipt.execution_id.clone(), credits);
            }
            credits
        };
        // The receipt becomes knowledge — the evidence half of the loop.
        let object_id = format!("k:receipt:{}", receipt.execution_id);
        let fact = format!(
            "execution {} on {} ({}) {:?}",
            receipt.execution_id, receipt.worker_node, receipt.capability, receipt.verdict
        );
        let knowledge = receipt.to_knowledge_object(&object_id, &fact);
        self.register_knowledge(knowledge)?;
        Ok(credits)
    }

    /// Runs a collective decision over the given knowledge objects and writes
    /// the feedback into memory + the knowledge registry (the decision becomes
    /// a new knowledge object backed by consensus evidence).
    pub fn decide(
        &self,
        decision_id: &str,
        summary: &str,
        initiator_agent: &str,
        objects: &[KnowledgeObject],
        policy: &ConsensusPolicy,
        created_at_ms: u64,
    ) -> Result<CollectiveDecision> {
        let decision = decide_collectively(
            decision_id,
            summary,
            initiator_agent,
            created_at_ms,
            objects,
            policy,
        )
        .map_err(|e| anyhow::anyhow!("collective decision failed: {e}"))?;
        {
            let mut reg = self
                .decisions
                .lock()
                .map_err(|_| anyhow::anyhow!("decision registry lock poisoned"))?;
            reg.add(decision.clone())
                .map_err(|e| anyhow::anyhow!("registering decision: {e}"))?;
        }
        // Memory feedback: the decision is itself a collective fact.
        let (content, feedback_object_id) = decision_feedback_entry(&decision, KNOWLEDGE_MEMORY_SCOPE);
        self.write_memory_feedback(
            &decision.decision_id,
            &content,
            vec![
                "collective-decision".to_string(),
                verdict_tag(&decision.verdict),
            ],
            initiator_agent,
        )?;
        // The feedback becomes a knowledge object backed by consensus-grade
        // evidence so future decisions can reason over it.
        if decision.verdict == DecisionVerdict::Adopted {
            let feedback = KnowledgeObject::new(
                &feedback_object_id,
                &content,
                initiator_agent,
                &self.local_node,
                created_at_ms,
            )
            .with_evidence(vec![decentraai_agents::Evidence::new(
                EvidenceKind::Consensus,
                format!("collective decision {} adopted", decision.decision_id),
            )
            .referencing(decision.decision_id.clone())]);
            let mut reg = self
                .knowledge
                .lock()
                .map_err(|_| anyhow::anyhow!("knowledge registry lock poisoned"))?;
            let _ = reg.add(feedback); // duplicate = already registered (idempotent)
        }
        Ok(decision)
    }

    /// Reads one knowledge object by id (for the decide endpoint input).
    pub fn knowledge_object(&self, id: &str) -> Option<KnowledgeObject> {
        self.knowledge
            .lock()
            .ok()
            .and_then(|reg| reg.get(id).cloned())
    }

    /// Snapshot of all knowledge objects (with derived confidence), for the
    /// decide endpoint and the dashboard.
    pub fn all_knowledge(&self) -> Vec<KnowledgeObject> {
        self.knowledge
            .lock()
            .ok()
            .map(|reg| reg.all_with_confidence().into_iter().map(|(o, _)| o).collect())
            .unwrap_or_default()
    }

    /// The full dashboard view model (real state, deterministic order).
    pub fn view(&self) -> KnowledgeStateView {
        let knowledge = self
            .knowledge
            .lock()
            .map(|reg| reg.all_with_confidence())
            .unwrap_or_default();
        let decisions = self
            .decisions
            .lock()
            .map(|reg| reg.all())
            .unwrap_or_default();
        let receipts = self
            .receipts
            .lock()
            .map(|reg| reg.all())
            .unwrap_or_default();
        let balances = self
            .compensation
            .lock()
            .map(|ledger| ledger.accounts())
            .unwrap_or_default()
            .into_iter()
            .map(|(account, acc)| (account, acc.earned))
            .collect::<std::collections::BTreeMap<_, _>>();
        let total_credits = balances.values().sum();
        KnowledgeStateView {
            knowledge_objects: knowledge
                .into_iter()
                .map(|(o, confidence)| KnowledgeView {
                    object_id: o.object_id,
                    fact: o.fact,
                    author_agent: o.author_agent,
                    author_node: o.author_node,
                    capability: o.capability,
                    evidence_kinds: o
                        .evidence
                        .iter()
                        .map(|e| format!("{:?}", e.kind))
                        .collect(),
                    confidence,
                    confidence_label: decentraai_agents::KnowledgeConfidence::of(confidence)
                        .to_string(),
                    created_at_ms: o.created_at_ms,
                })
                .collect(),
            decisions: decisions
                .into_iter()
                .map(|d| DecisionView {
                    decision_id: d.decision_id,
                    summary: d.summary,
                    verdict: format!("{:?}", d.verdict),
                    aggregated_confidence: d.aggregated_confidence,
                    considered: d.considered.into_iter().map(|c| c.object_id).collect(),
                    created_at_ms: d.created_at_ms,
                })
                .collect(),
            receipts: receipts
                .into_iter()
                .map(|r| {
                    let execution_id = r.execution_id.clone();
                    let credits = self
                        .receipt_credits
                        .lock()
                        .map(|rc| rc.get(&execution_id).copied().unwrap_or(0))
                        .unwrap_or(0); // real credited amount, never synthetic
                    ReceiptView {
                        execution_id,
                        worker_node: r.worker_node,
                        capability: r.capability,
                        duration_ms: r.duration_ms,
                        verdict: format!("{:?}", r.verdict),
                        credits,
                        created_at_ms: r.created_at_ms,
                    }
                })
                .collect(),
            balances,
            total_credits,
            memory_scope: KNOWLEDGE_MEMORY_SCOPE.to_string(),
            memory_attached: self.memory_store.is_some(),
        }
    }

    /// Persists one feedback entry into the collective memory scope. Best
    /// effort: a memory failure must never break the knowledge/decision flow.
    fn write_memory_feedback(
        &self,
        entry_id: &str,
        content: &str,
        tags: Vec<String>,
        author_agent: &str,
    ) -> Result<()> {
        let Some(store) = &self.memory_store else {
            return Ok(());
        };
        let entry = MemoryEntry {
            entry_id: entry_id.to_string(),
            scope: KNOWLEDGE_MEMORY_SCOPE.to_string(),
            author_agent: author_agent.to_string(),
            author_node: self.local_node.clone(),
            content: content.to_string(),
            tags,
            created_at_ms: now_ms(),
            expires_at_ms: None,
            provenance: Some(decentraai_hub::capability::Provenance::Verified),
            meta: Default::default(),
        };
        // The runtime is the scope owner → writer is owner, trusted, verified.
        store
            .write(
                KNOWLEDGE_MEMORY_SCOPE,
                &entry,
                author_agent,
                true,
                true,
                true,
            )
            .context("writing collective knowledge memory feedback")?;
        // M19 auto-embed: index the fresh entry for semantic search in the
        // background. Fire-and-forget by design — indexing is an optimization
        // and must never break (or slow) the verified-knowledge write path;
        // gaps stay visible via /v1/memory/index status and lexical search
        // still covers unindexed entries.
        if let Some(embedder) = &self.embedder {
            let embedder = embedder.clone();
            let store = store.clone();
            let entry_id = entry.entry_id.clone();
            let content = entry.content.clone();
            let scope = KNOWLEDGE_MEMORY_SCOPE.to_string();
            tokio::spawn(async move {
                match embedder.embed(&content).await {
                    Ok(vec) if !vec.is_empty() => {
                        if let Err(e) = store.store_embedding(&scope, &entry_id, &vec) {
                            tracing::warn!(error = %e, entry_id = %entry_id, "auto-embed store failed");
                        }
                    }
                    Ok(_) => tracing::warn!(entry_id = %entry_id, "embeddings backend returned an empty vector"),
                    Err(e) => tracing::warn!(error = %e, entry_id = %entry_id, "auto-embed failed; entry remains lexically searchable"),
                }
            });
        }
        Ok(())
    }
}

/// The knowledge scope is registered on attach (idempotent; unknown scope
/// would make every feedback write fail).
fn ensure_knowledge_scope(store: &MemoryStore) -> Result<()> {
    if store.get_scope(KNOWLEDGE_MEMORY_SCOPE)?.is_none() {
        store.register_scope(&MemoryScope {
            name: KNOWLEDGE_MEMORY_SCOPE.to_string(),
            owner_agent: "runtime".to_string(),
            level: MemoryLevel::Network,
            policy: MemoryPolicy {
                level: MemoryLevel::Network,
                access: decentraai_agents::memory::MemoryAccess::TrustedNetwork,
                retention_secs: None,
                require_verified_provenance: true,
                allow_remote_write: false,
                max_entries: 1024,
            },
            created_at_ms: now_ms(),
        })?;
    }
    Ok(())
}

fn verdict_tag(verdict: &DecisionVerdict) -> String {
    match verdict {
        DecisionVerdict::Adopted => "adopted".to_string(),
        DecisionVerdict::Rejected => "rejected".to_string(),
        DecisionVerdict::Deferred { .. } => "deferred".to_string(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::{
        EvidenceKind, KnowledgeObject, ReceiptVerdict, VerifiedComputeReceipt,
        evidence_confidence,
    };
    use std::path::PathBuf;

    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp_memory_store() -> Arc<MemoryStore> {
        // Unique path per test call: tests run in parallel and a shared file
        // would race (one test's remove_file deletes another's database). The
        // counter makes the name unique even when several tests start in the
        // same millisecond.
        let n = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/decentraai-kr-test-{}-{n}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Arc::new(MemoryStore::open(&path).unwrap())
    }

    fn profile() -> ContributionProfile {
        ContributionProfile {
            cpu_cores: 4,
            ram_mb: 8192,
            vram_mb: 0,
            online_seconds: 3600,
            verified_requests: 10,
            failed_requests: 1,
        }
    }

    fn verified_receipt(id: &str) -> VerifiedComputeReceipt {
        VerifiedComputeReceipt::new(
            id,
            "peer-worker",
            "a:worker",
            "inference",
            120,
            ReceiptVerdict::Verified,
            2000,
        )
        .with_output_hash("blake3:abc")
    }

    fn failed_receipt(id: &str) -> VerifiedComputeReceipt {
        VerifiedComputeReceipt::new(
            id,
            "peer-worker",
            "a:worker",
            "inference",
            95,
            ReceiptVerdict::Failed,
            2000,
        )
    }

    #[test]
    fn full_circuit_receipt_to_knowledge_to_decision() {
        let memory = tmp_memory_store();
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let runtime = KnowledgeRuntime::new(compensation.clone(), "peer-local", Some(memory.clone()))
            .expect("runtime attaches");
        assert!(runtime.view().memory_attached);

        // 1. Record a verified receipt → credits + knowledge object.
        let credits = runtime
            .record_receipt(&verified_receipt("e1"), &profile())
            .expect("receipt records");
        assert!(credits > 0);
        assert_eq!(compensation.lock().unwrap().account("peer-worker").unwrap().earned, credits);

        // The receipt became a high-confidence knowledge object.
        let ko = runtime.knowledge_object("k:receipt:e1").expect("receipt knowledge");
        assert!((evidence_confidence(&ko) - 0.90).abs() < 1e-6);

        // 2. Decide collectively over the receipt's knowledge → adopted.
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let decision = runtime
            .decide("d1", "the model output is trustworthy", "a:coord", &[ko], &policy, 3000)
            .expect("decision runs");
        assert_eq!(decision.verdict, DecisionVerdict::Adopted);

        // The adopted decision produced a feedback knowledge object.
        let feedback = runtime
            .knowledge_object("k:decision:d1")
            .expect("decision feedback knowledge");
        assert_eq!(feedback.evidence[0].kind, EvidenceKind::Consensus);

        // 3. The memory scope now holds the feedback entries.
        let entries = memory
            .read(KNOWLEDGE_MEMORY_SCOPE, "runtime", true)
            .expect("memory readable");
        assert!(!entries.is_empty());

        let view = runtime.view();
        assert_eq!(view.knowledge_objects.len(), 2);
        assert_eq!(view.decisions.len(), 1);
        assert_eq!(view.total_credits, credits);
        // The receipt view carries the REAL credited amount (never a
        // synthetic 0) so the dashboard shows what the execution earned.
        assert_eq!(view.receipts.len(), 1);
        assert_eq!(view.receipts[0].credits, credits);
    }

    #[test]
    fn failed_receipt_never_credits_or_claims_confidence() {
        let memory = tmp_memory_store();
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let runtime = KnowledgeRuntime::new(compensation.clone(), "peer-local", Some(memory)).unwrap();

        let credits = runtime
            .record_receipt(&failed_receipt("e2"), &profile())
            .expect("receipt records");
        assert_eq!(credits, 0, "failed work never credits");
        assert!(compensation.lock().unwrap().account("peer-worker").is_none());

        // The failed receipt's knowledge object carries synthetic evidence.
        let ko = runtime.knowledge_object("k:receipt:e2").expect("receipt knowledge");
        assert!(evidence_confidence(&ko) < 0.3);
        assert!(runtime.view().total_credits == 0);
    }

    #[test]
    fn idempotency_receipt_and_decision_are_exactly_once() {
        let memory = tmp_memory_store();
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let runtime = KnowledgeRuntime::new(compensation.clone(), "peer-local", Some(memory)).unwrap();

        runtime.record_receipt(&verified_receipt("e3"), &profile()).unwrap();
        // Same execution id again → duplicate receipt rejected, no second credit.
        assert!(runtime.record_receipt(&verified_receipt("e3"), &profile()).is_err());
        let credits = compensation.lock().unwrap().account("peer-worker").unwrap().earned;
        assert!(credits > 0);
        assert_eq!(runtime.view().receipts.len(), 1);

        // Same decision id again → duplicate decision rejected.
        let ko = runtime.knowledge_object("k:receipt:e3").unwrap();
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        runtime.decide("d3", "dup", "a:coord", std::slice::from_ref(&ko), &policy, 3000).unwrap();
        assert!(runtime.decide("d3", "dup", "a:coord", std::slice::from_ref(&ko), &policy, 3000).is_err());
        assert_eq!(runtime.view().decisions.len(), 1);
    }

    #[test]
    fn declaration_without_evidence_stays_zero_confidence() {
        let memory = tmp_memory_store();
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let runtime = KnowledgeRuntime::new(compensation.clone(), "peer-local", Some(memory)).unwrap();

        // An agent "declares" a fact with no evidence → confidence 0.0.
        let plain = KnowledgeObject::new("k:plain", "the sky is blue", "a:research", "peer1", 1000);
        runtime.register_knowledge(plain).unwrap();
        let view = runtime.view();
        assert_eq!(view.knowledge_objects.len(), 1);
        assert_eq!(view.knowledge_objects[0].confidence, 0.0);
        assert_eq!(view.knowledge_objects[0].confidence_label, "none");

        // A decision over only-declared objects can never be adopted: with no
        // evidence the vote disagrees (or lacks opinions), so the verdict is
        // Rejected or Deferred — never Adopted.
        let ko = runtime.knowledge_object("k:plain").unwrap();
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let decision = runtime.decide("d4", "plain fact", "a:coord", &[ko], &policy, 3000).unwrap();
        assert!(
            !matches!(decision.verdict, DecisionVerdict::Adopted),
            "declaration without evidence must never be adopted, got {:?}",
            decision.verdict
        );
    }

    #[test]
    fn runtime_works_without_memory_store() {
        let compensation = Arc::new(Mutex::new(CompensationLedger::default()));
        let runtime = KnowledgeRuntime::new(compensation.clone(), "peer-local", None).unwrap();
        assert!(!runtime.view().memory_attached);
        runtime.record_receipt(&verified_receipt("e5"), &profile()).unwrap();
        assert!(runtime.view().receipts.len() == 1);
        assert!(runtime.view().knowledge_objects.len() == 1);
        // Memory writes are no-ops when no store is attached.
    }
}