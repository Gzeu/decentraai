//! Model performance observations (Model Colony) — verified executions
//! become Collective Memory facts that the deterministic router consumes.
//!
//! # Flow
//!
//! ```text
//! model execution (benchmark or shadow task)
//!   → graded verdict + evidence reference        (Training Lab)
//!   → record_observation()                       (THIS module)
//!   → Collective Memory entry, kind = model_evaluation, status = VERIFIED
//!   → aggregate_model()                          (router's input)
//! ```
//!
//! # Honesty rules
//!
//! - An observation is written ONLY with an evidence reference — an
//!   unverified claim about a model's performance is exactly the kind of
//!   noise the colony must not learn from.
//! - Entries are `verified` on arrival BECAUSE their evidence was checked
//!   upstream; they are still ordinary memory: subject-grouped, auditable,
//!   lifecycle-managed.
//! - NOTHING here trains a model. Training candidates come only through the
//!   explicit export path.
//! - Aggregation is integer math over verified entries only — same state,
//!   same numbers, always.

use crate::agent_memory::MemoryStore;
use decentraai_agents::memory::{
    KnowledgeKind, MemoryEntry, MemoryProvenance, MemoryStatus,
};
use decentraai_hub::capability::Provenance;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The scope every colony observation lives in. Team-level by policy:
/// shared across the node's agents, never public without an operator's
/// explicit widening.
pub const MODEL_INTEL_SCOPE: &str = "model.intel";

/// One verified execution observation for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionObservation {
    /// Model id (registry key).
    pub model_id: String,
    /// Task id within the benchmark corpus (e.g. `"mi_dfcp_order"`).
    pub task_id: String,
    /// Whether the graded verdict was Correct.
    pub success: bool,
    /// Measured end-to-end latency in milliseconds.
    pub latency_ms: u64,
    /// Evidence reference (benchmark run id / execution receipt). REQUIRED.
    pub evidence_ref: String,
}

/// Errors from the observation path.
#[derive(Debug, Error)]
pub enum ObservationError {
    #[error(transparent)]
    Store(#[from] crate::agent_memory::MemoryStoreError),
    #[error("observation without evidence_ref is rejected")]
    MissingEvidence,
}

use thiserror::Error;

/// Registers the model-intel scope if absent (idempotent).
pub fn ensure_scope(store: &MemoryStore) -> Result<(), ObservationError> {
    use decentraai_agents::memory::{MemoryAccess, MemoryLevel, MemoryPolicy, MemoryScope};
    if store.get_scope(MODEL_INTEL_SCOPE)?.is_none() {
        let policy = MemoryPolicy {
            level: MemoryLevel::Team,
            access: MemoryAccess::TeamOnly,
            ..MemoryPolicy::default()
        };
        store.register_scope(&MemoryScope::new(
            MODEL_INTEL_SCOPE,
            "governor",
            MemoryLevel::Team,
        )
        .with_policy(policy))?;
    }
    Ok(())
}

/// Persists one observation as a VERIFIED `model_evaluation` memory entry.
///
/// Deterministic entry ids (`mi:<model>:<task>:<evidence>`) make re-runs of
/// the same evidence exact-duplicates at the store level — re-importing a
/// benchmark batch can never double-count.
pub fn record_observation(
    store: &MemoryStore,
    obs: &ExecutionObservation,
) -> Result<MemoryStatus, ObservationError> {
    if obs.evidence_ref.trim().is_empty() {
        return Err(ObservationError::MissingEvidence);
    }
    ensure_scope(store)?;
    let entry_id = format!("mi:{}:{}:{}", obs.model_id, obs.task_id, obs.evidence_ref);
    let content = format!(
        "model={} task={} success={} latency_ms={}",
        obs.model_id, obs.task_id, obs.success, obs.latency_ms
    );
    let mut entry = MemoryEntry::new(&entry_id, MODEL_INTEL_SCOPE, "model-intel", "local", &content)
        .with_kind(KnowledgeKind::ModelEvaluation)
        .with_subject(format!("model:{}:{}", obs.model_id, obs.task_id));
    entry.created_at_ms = now_ms();
    // Verified ON ARRIVAL: the caller had to produce an evidence reference;
    // the store still enforces its own gates and dedup.
    entry.meta.status = MemoryStatus::Verified;
    entry.meta.detail = Some(
        MemoryProvenance::new(
            "execution",
            "model-intel",
            "local",
            entry.created_at_ms,
            100,
        )
        .with_evidence(obs.evidence_ref.as_str()),
    );
    let _ = Provenance::Verified; // claim strength stays expressed via meta.status
    store.write_checked(MODEL_INTEL_SCOPE, &entry, "governor", true, false, false)?;
    Ok(entry.meta.status)
}

/// Aggregated verified performance for one model, per task and overall.
/// Integer-only math; BTreeMap ordering keeps output deterministic.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub model_id: String,
    /// Verified observations counted.
    pub samples: u32,
    /// Overall success percent (0 when no samples — honest zero).
    pub success_percent: u32,
    /// Mean latency over all samples (0 when none), rounded down.
    pub mean_latency_ms: u64,
    /// Per-task breakdown, task_id ascending.
    pub per_task: BTreeMap<String, TaskPerformance>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPerformance {
    pub attempts: u32,
    pub successes: u32,
    pub success_percent: u32,
    pub mean_latency_ms: u64,
}

/// Aggregates ALL verified observations for one model from the intel scope.
pub fn aggregate_model(store: &MemoryStore, model_id: &str) -> Result<PerformanceSummary, ObservationError> {
    let entries = store.read(MODEL_INTEL_SCOPE, "governor", true)?;
    let prefix = format!("model={model_id} ");
    let mut summary = PerformanceSummary {
        model_id: model_id.to_string(),
        ..Default::default()
    };

    #[derive(Default)]
    struct Acc {
        attempts: u32,
        successes: u32,
        latency_sum: u64,
    }
    let mut tasks: BTreeMap<String, Acc> = BTreeMap::new();
    let mut total_latency: u64 = 0;

    for e in &entries {
        if e.meta.kind != KnowledgeKind::ModelEvaluation || e.meta.status != MemoryStatus::Verified
        {
            continue;
        }
        if !e.content.starts_with(&prefix) {
            continue;
        }
        // Parse the fixed-format content line (written by record_observation).
        let mut success = false;
        let mut latency: u64 = 0;
        for field in e.content.split_whitespace() {
            if let Some(v) = field.strip_prefix("success=") {
                success = v == "true";
            } else if let Some(v) = field.strip_prefix("latency_ms=") {
                latency = v.parse().unwrap_or(0);
            }
        }
        let task = e
            .meta
            .subject_key
            .strip_prefix(&format!("model:{model_id}:"))
            .unwrap_or_default()
            .to_string();
        let acc = tasks.entry(task).or_default();
        acc.attempts += 1;
        acc.latency_sum += latency;
        if success {
            acc.successes += 1;
        }
        total_latency += latency;
        summary.samples += 1;
    }

    if summary.samples > 0 {
        summary.mean_latency_ms = total_latency / u64::from(summary.samples);
    }
    for (task, acc) in &tasks {
        let perf = TaskPerformance {
            attempts: acc.attempts,
            successes: acc.successes,
            success_percent: percent(acc.successes, acc.attempts),
            mean_latency_ms: acc.latency_sum / u64::from(acc.attempts.max(1)),
        };
        summary.per_task.insert(task.clone(), perf);
    }
    let total_successes: u32 = tasks.values().map(|a| a.successes).sum();
    summary.success_percent = percent(total_successes, summary.samples);
    Ok(summary)
}

fn percent(part: u32, total: u32) -> u32 {
    if total == 0 { 0 } else { part * 100 / total }
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
    use std::path::Path;

    fn store() -> MemoryStore {
        let s = MemoryStore::open(Path::new(":memory:")).unwrap();
        ensure_scope(&s).unwrap();
        s
    }

    fn obs(task: &str, ok: bool, ms: u64) -> ExecutionObservation {
        ExecutionObservation {
            model_id: "qwen3-1.7b-q4".into(),
            task_id: task.into(),
            success: ok,
            latency_ms: ms,
            evidence_ref: format!("run-{task}-{ms}-{ok}"),
        }
    }

    #[test]
    fn observations_persist_verified_and_aggregate_deterministically() {
        let s = store();
        for o in [
            obs("mi_dfcp_order", true, 800),
            obs("mi_dfcp_order", true, 1200),
            obs("mi_dfcp_order", false, 900),
            obs("mi_romanian", true, 400),
        ] {
            record_observation(&s, &o).unwrap();
        }
        let sum = aggregate_model(&s, "qwen3-1.7b-q4").unwrap();
        assert_eq!(sum.samples, 4);
        assert_eq!(sum.success_percent, 75);
        assert_eq!(sum.mean_latency_ms, 825);
        let dfcp = sum.per_task.get("mi_dfcp_order").unwrap();
        assert_eq!((dfcp.attempts, dfcp.successes, dfcp.success_percent), (3, 2, 66));
        assert_eq!(dfcp.mean_latency_ms, 966);
        // Another model has nothing — honest zeros, not fabricated data.
        let empty = aggregate_model(&s, "gemma-3-1b-q4").unwrap();
        assert_eq!((empty.samples, empty.success_percent, empty.mean_latency_ms), (0, 0, 0));
        // Same state → identical aggregation bytes.
        let again = aggregate_model(&s, "qwen3-1.7b-q4").unwrap();
        assert_eq!(sum, again);
    }

    #[test]
    fn missing_evidence_is_rejected_and_reruns_are_exact_duplicates() {
        let s = store();
        let no_evidence = ExecutionObservation {
            model_id: "m".into(),
            task_id: "t".into(),
            success: true,
            latency_ms: 1,
            evidence_ref: "  ".into(),
        };
        assert!(matches!(
            record_observation(&s, &no_evidence),
            Err(ObservationError::MissingEvidence)
        ));
        let good = obs("t", true, 100);
        record_observation(&s, &good).unwrap();
        // Re-running the SAME evidence lands as an exact duplicate (same
        // deterministic entry id): counts stay honest.
        record_observation(&s, &good).unwrap();
        let sum = aggregate_model(&s, "qwen3-1.7b-q4").unwrap();
        assert_eq!(sum.samples, 1);
    }
}
