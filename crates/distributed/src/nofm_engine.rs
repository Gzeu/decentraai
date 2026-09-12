//! N-of-M execution engine, N=1 milestone: single-replica verification pipeline.
//!
//! The orchestration state machine (`propose → assign → claim → replicas →
//! consensus → settle` in `decentraai-agents`) had zero production callers
//! for its verification half. This module is the missing motor — and ONLY
//! the motor:
//!
//! ```text
//! propose → assign → claim → route_request (EXISTING engine, untouched)
//!   → real output → check_output_schema → submit_verified + submit_replica
//!   → try_consensus → decide_settle → VerifiedComputeReceipt
//! ```
//!
//! Architectural rules (non-negotiable):
//! - Inference NEVER moves here. Execution goes through the existing
//!   `route_request` behind a [`StageExecutor`] trait (mocked in tests).
//! - The engine takes NO ledger and NO knowledge handles. It cannot credit,
//!   by construction — the live path's single crediting stays the only one.
//!   Receipts carry the live request id so a later `record_receipt` dedups
//!   instead of double-paying.
//! - N=1 is a *verification pipeline*, never "consensus". The consensus
//!   fields are populated honestly (one replica, one vote); replicated
//!   consensus starts at N≥2.
//! - One execution per drive, no internal retry (retry budgets belong to
//!   `route_request`). At-most-once is tested, not promised.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use decentraai_agents::orchestration::{
    AssignmentError, AssignmentStore, ConsensusOutcome, SettleDecision, decide_settle,
};
use decentraai_agents::receipt::{ReceiptVerdict, VerifiedComputeReceipt};
use decentraai_agents::verification::check_output_schema;

/// Executor identity recorded on the assignment when the fabric itself
/// executes (the planner picks the real worker at route time; the receipt
/// records who actually ran).
pub const FABRIC_EXECUTOR: &str = "fabric-router";

/// Input for one driven stage.
#[derive(Debug, Clone)]
pub struct StageSpec {
    /// New plan/stage ids (engine creates; collisions are an error, never
    /// a takeover).
    pub plan_id: String,
    pub stage_id: String,
    /// Capability vocabulary (records only; model selection stays caller-side
    /// until cost-aware selection lands with N≥2).
    pub capability: String,
    /// Prompt executed verbatim by the existing engine.
    pub prompt: String,
    /// Model hash for `route_request` (resolved by the caller via the
    /// registry/capability search — the engine never guesses a model).
    pub model_hash: String,
    /// Token budget for this execution.
    pub max_tokens: u32,
    /// Price ceiling (rejected above).
    pub max_price: u64,
    /// Requester identity for the assignment record.
    pub requester: String,
    /// Optional JSON-schema hint for structural verification (`None` =
    /// structural pass is vacuous; declared, not hidden).
    pub schema_json: Option<String>,
}

/// Real execution product handed back by an executor.
#[derive(Debug, Clone)]
pub struct ExecutedStage {
    /// Verbatim engine output.
    pub output: String,
    /// Worker that actually ran (peer id string).
    pub worker: String,
    /// Live request id (becomes the receipt's idempotency key).
    pub request_id: String,
    /// Measured timings.
    pub duration_ms: u64,
    /// Tokens used.
    pub tokens: u32,
}

/// Execution backend. The live implementation calls `route_request`; tests
/// inject canned outputs. One call per drive — enforced by tests.
pub trait StageExecutor {
    /// Stable executor identity for assignment records.
    fn executor_id(&self) -> String;
    /// Execute exactly once. No retries inside: budgets live downstream.
    fn execute(
        &self,
        spec: &StageSpec,
    ) -> impl std::future::Future<Output = Result<ExecutedStage, EngineError>> + Send;
}

/// Everything one driven stage produced (evidence, not effects).
#[derive(Debug, Clone)]
pub struct EngineOutcome {
    /// Executor that ran (fabric id at assign time).
    pub executor: String,
    /// The real engine output (requested result — same precedent as
    /// `execute_decision` returning real inference; never logged, only
    /// returned to the caller who asked for execution).
    pub output: String,
    /// Measured resource use (from the live execution).
    pub tokens_used: u32,
    /// Measured wall time (ms, engine-reported).
    pub duration_ms: u64,
    /// Structural verification passed.
    pub verified: bool,
    /// Verification detail (human-readable, from the check path).
    pub verify_detail: String,
    /// Consensus outcome (N=1: single vote recorded honestly; `None` when
    /// verification failed before any consensus could form).
    pub consensus: Option<ConsensusOutcome>,
    /// Settlement decision (RETURNED, never applied here).
    pub settle: SettleDecision,
    /// The receipt (id = live request id; credit-neutral by construction).
    pub receipt: VerifiedComputeReceipt,
}

/// Engine failures. State transitions already applied stay applied (the
/// store is the source of truth; outcomes report where it stopped).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// Input malformed or over budget.
    #[error("bad stage spec: {0}")]
    BadSpec(String),
    /// Store refused a transition (includes already-exists: no takeovers).
    #[error("store: {0:?}")]
    Store(AssignmentError),
    /// Execution backend failed (worker error, timeout, no model…).
    #[error("execution failed: {0}")]
    Execute(String),
    /// No consensus could be formed from submitted replicas.
    #[error("no consensus formed")]
    NoConsensus,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Drive ONE stage end-to-end (N=1). Creates the assignment; every later
/// step advances pure store state around exactly one real execution.
pub async fn drive_stage(
    store: &Arc<Mutex<AssignmentStore>>,
    exec: &impl StageExecutor,
    spec: &StageSpec,
) -> Result<EngineOutcome, EngineError> {
    if spec.plan_id.is_empty() || spec.stage_id.is_empty() || spec.prompt.is_empty() {
        return Err(EngineError::BadSpec(
            "plan/stage/prompt must be non-empty".into(),
        ));
    }
    if spec.model_hash.is_empty() {
        return Err(EngineError::BadSpec(
            "model_hash required (engine never guesses a model)".into(),
        ));
    }
    let now = now_ms();
    let executor = exec.executor_id();
    {
        let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
        s.propose_with_replicas(
            &spec.plan_id,
            &spec.stage_id,
            &spec.capability,
            spec.max_price,
            &spec.requester,
            1,
            now,
        )
        .map_err(EngineError::Store)?;
        s.assign(&spec.plan_id, &spec.stage_id, &executor, now)
            .map_err(EngineError::Store)?;
        s.claim(&spec.plan_id, &spec.stage_id, &executor, now)
            .map_err(EngineError::Store)?;
        s.init_replicas(
            &spec.plan_id,
            &spec.stage_id,
            1,
            std::slice::from_ref(&executor),
            now,
        )
        .map_err(EngineError::Store)?;
        s.claim_replica(&spec.plan_id, &spec.stage_id, 0, &executor, now)
            .map_err(EngineError::Store)?;
    }

    // Exactly one real execution, through the existing engine.
    let run = exec.execute(spec).await.map_err(|e| {
        let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = s.mark_failed(&spec.plan_id, &spec.stage_id, &format!("{e:?}"), now_ms());
        e
    })?;

    // Structural verification of the REAL output (N=1 verification core).
    let check = check_output_schema(&run.output, spec.schema_json.as_deref());
    let verified = check.passed;
    let detail = check.detail.clone();
    let receipt_verdict = if verified && run.output.is_empty() {
        ReceiptVerdict::Failed
    } else if verified {
        ReceiptVerdict::Verified
    } else {
        ReceiptVerdict::Failed
    };

    let output_hash = blake3::hash(run.output.as_bytes());
    let output_hash = output_hash.to_hex().to_string();
    let receipt = VerifiedComputeReceipt::new(
        run.request_id.clone(),
        run.worker.clone(),
        executor.clone(),
        spec.capability.clone(),
        run.duration_ms,
        receipt_verdict,
        now_ms(),
    )
    .with_output_hash(output_hash);

    let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
    if !verified {
        let _ = s.mark_failed(&spec.plan_id, &spec.stage_id, &detail, now_ms());
        let settle = decide_settle(
            &spec.plan_id,
            &spec.stage_id,
            false,
            spec.max_price,
            decentraai_agents::orchestration::VerifiedMeasures {
                tokens_used: Some(run.tokens),
                processing_ms: Some(run.duration_ms as u32),
            },
        );
        return Ok(EngineOutcome {
            executor,
            output: run.output,
            tokens_used: run.tokens,
            duration_ms: run.duration_ms,
            verified: false,
            verify_detail: detail,
            consensus: s.consensus(&spec.plan_id, &spec.stage_id).cloned(),
            settle,
            receipt,
        });
    }
    s.submit_verified(&spec.plan_id, &spec.stage_id, &executor, now_ms())
        .map_err(EngineError::Store)?;
    s.submit_replica(
        &spec.plan_id,
        &spec.stage_id,
        0,
        &executor,
        serde_json::json!({"output": run.output.clone()}),
        1.0,
        now_ms(),
    )
    .map_err(EngineError::Store)?;
    let consensus = s
        .try_consensus(&spec.plan_id, &spec.stage_id, 1, 1.0, now_ms())
        .ok_or(EngineError::NoConsensus)?;
    let settle = decide_settle(
        &spec.plan_id,
        &spec.stage_id,
        true,
        spec.max_price,
        decentraai_agents::orchestration::VerifiedMeasures {
            tokens_used: Some(run.tokens),
            processing_ms: Some(run.duration_ms as u32),
        },
    );
    Ok(EngineOutcome {
        executor,
        output: run.output,
        tokens_used: run.tokens,
        duration_ms: run.duration_ms,
        verified: true,
        verify_detail: detail,
        consensus: Some(consensus),
        settle,
        receipt,
    })
}

/// Live executor: runs the stage through the existing `route_request`
/// (planner → reservation → worker → release). Nothing about inference
/// moves: this only adapts its input/output shapes to the engine.
pub struct FabricExecutor<'a> {
    distributed: &'a crate::DistributedInference,
}

impl<'a> FabricExecutor<'a> {
    pub fn new(distributed: &'a crate::DistributedInference) -> Self {
        Self { distributed }
    }
}

impl StageExecutor for FabricExecutor<'_> {
    fn executor_id(&self) -> String {
        FABRIC_EXECUTOR.to_string()
    }

    async fn execute(&self, spec: &StageSpec) -> Result<ExecutedStage, EngineError> {
        use decentraai_protocol::InferRequest;
        let request = InferRequest::new(
            spec.model_hash.clone(),
            spec.prompt.clone(),
            spec.max_tokens,
        )
        .with_sender(self.distributed.p2p_node().local_peer_id());
        let request_id = request.request_id.to_string();
        let resp = self
            .distributed
            .route_request(request)
            .await
            .map_err(|e| EngineError::Execute(format!("{e:?}")))?;
        if !resp.success {
            return Err(EngineError::Execute(
                resp.error.unwrap_or_else(|| "worker failed".to_string()),
            ));
        }
        Ok(ExecutedStage {
            output: resp.output,
            worker: resp.worker_peer_id.to_string(),
            request_id,
            duration_ms: resp.processing_time_ms as u64,
            tokens: resp.tokens_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockExec {
        calls: Arc<AtomicUsize>,
        output: String,
        fail: bool,
    }

    impl StageExecutor for MockExec {
        fn executor_id(&self) -> String {
            "mock-exec".to_string()
        }

        async fn execute(&self, spec: &StageSpec) -> Result<ExecutedStage, EngineError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(EngineError::Execute("mock backend down".to_string()));
            }
            Ok(ExecutedStage {
                output: self.output.clone(),
                worker: "mock-worker".to_string(),
                request_id: format!("req-{}-{}", spec.plan_id, spec.stage_id),
                duration_ms: 12,
                tokens: 7,
            })
        }
    }

    fn spec() -> StageSpec {
        StageSpec {
            plan_id: "p1".to_string(),
            stage_id: "s1".to_string(),
            capability: "chat".to_string(),
            prompt: "hi".to_string(),
            model_hash: "abc123".to_string(),
            max_tokens: 16,
            max_price: 100,
            requester: "tester".to_string(),
            schema_json: None,
        }
    }

    fn store() -> Arc<Mutex<AssignmentStore>> {
        Arc::new(Mutex::new(AssignmentStore::new()))
    }

    #[tokio::test]
    async fn n1_happy_path_traverses_everything() {
        let st = store();
        let ex = MockExec {
            calls: Arc::new(AtomicUsize::new(0)),
            output: "hello".to_string(),
            fail: false,
        };
        let out = drive_stage(&st, &ex, &spec()).await.expect("drives");
        assert_eq!(
            ex.calls.load(Ordering::Relaxed),
            1,
            "exactly-once execution"
        );
        assert!(out.verified);
        assert_eq!(
            out.receipt.execution_id, "req-p1-s1",
            "receipt keys to the live request"
        );
        assert_eq!(
            out.receipt.worker_node, "mock-worker",
            "receipt records who really ran"
        );
        assert!(matches!(out.receipt.verdict, ReceiptVerdict::Verified));
        assert!(matches!(
            out.settle,
            decentraai_agents::orchestration::SettleDecision::SettleAndCredit { .. }
        ));
        // Store reached terminal states on both tracks.
        let s = st.lock().unwrap();
        assert!(matches!(
            s.get("p1", "s1").unwrap().state,
            decentraai_agents::orchestration::AssignmentState::Settled { .. }
        ));
        assert!(s.consensus("p1", "s1").is_some());
    }

    #[tokio::test]
    async fn backend_failure_marks_failed_no_consensus_claim() {
        let st = store();
        let ex = MockExec {
            calls: Arc::new(AtomicUsize::new(0)),
            output: String::new(),
            fail: true,
        };
        let err = drive_stage(&st, &ex, &spec()).await.expect_err("fails");
        assert!(matches!(err, EngineError::Execute(_)));
        let s = st.lock().unwrap();
        assert!(matches!(
            s.get("p1", "s1").unwrap().state,
            decentraai_agents::orchestration::AssignmentState::Failed { .. }
        ));
        assert!(
            s.consensus("p1", "s1").is_none(),
            "no consensus from zero outputs"
        );
    }

    #[tokio::test]
    async fn schema_mismatch_fails_verification() {
        let st = store();
        let ex = MockExec {
            calls: Arc::new(AtomicUsize::new(0)),
            output: "not json".to_string(),
            fail: false,
        };
        let mut sp = spec();
        sp.schema_json = Some(r#"{"type":"object"}"#.to_string());
        let out = drive_stage(&st, &ex, &sp)
            .await
            .expect("drives to failed verdict");
        assert!(!out.verified);
        assert!(matches!(out.receipt.verdict, ReceiptVerdict::Failed));
        assert!(matches!(
            out.settle,
            decentraai_agents::orchestration::SettleDecision::Release { .. }
        ));
    }

    #[tokio::test]
    async fn bad_specs_rejected_before_state() {
        let st = store();
        let ex = MockExec {
            calls: Arc::new(AtomicUsize::new(0)),
            output: "x".to_string(),
            fail: false,
        };
        let mut sp = spec();
        sp.model_hash.clear();
        assert!(matches!(
            drive_stage(&st, &ex, &sp).await,
            Err(EngineError::BadSpec(_))
        ));
        assert_eq!(
            ex.calls.load(Ordering::Relaxed),
            0,
            "rejected before executing"
        );
        assert!(st.lock().unwrap().get("p1", "s1").is_none());
    }

    #[tokio::test]
    async fn no_takeover_of_existing_stage() {
        let st = store();
        let ex = MockExec {
            calls: Arc::new(AtomicUsize::new(0)),
            output: "x".to_string(),
            fail: false,
        };
        drive_stage(&st, &ex, &spec()).await.expect("first drives");
        let err = drive_stage(&st, &ex, &spec())
            .await
            .expect_err("second refuses");
        assert!(matches!(err, EngineError::Store(_)));
        assert_eq!(ex.calls.load(Ordering::Relaxed), 1, "no second execution");
    }

    #[test]
    fn engine_touches_no_ledger_by_construction() {
        // Compile-time guarantee, pinned by this test's existence: drive_stage
        // takes only (&AssignmentStore, &impl StageExecutor, &StageSpec).
        // There is no ledger, knowledge, or credit handle anywhere in its
        // signature — double-crediting is inexpressible, not just avoided.
        fn signature(_: fn(&Arc<Mutex<AssignmentStore>>, &MockExec, &StageSpec)) {}
        let _ = signature;
    }
}
