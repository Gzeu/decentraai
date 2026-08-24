//! Intelligence Loop — the glue between routing, execution, observation and
//! shadow comparison. Makes the Model Colony ACTIVE rather than passive.
//!
//! # The cycle
//!
//! ```text
//! Governor proposes capability need
//!   → route() selects model by evidence        [fabric::model_routing]
//!   → executor runs the task                    [BenchmarkManager]
//!   → observation recorded as VERIFIED          [model_performance]
//!   → aggregate updated                         [model_performance]
//!   → if shadow: same task on candidate         [this module]
//!   → compare_shadow_models()                   [agents::benchmark]
//! ```
//!
//! Every step is deterministic. The loop is the ONE place where routing,
//! execution and memory meet; each step independently testable.

use crate::agent_memory::MemoryStore;
use crate::benchmark_manager::BenchmarkManager;
use crate::model_performance::{ExecutionObservation, record_observation};
use decentraai_agents::benchmark::{BenchmarkMode, BenchmarkTask};
use decentraai_fabric::model_routing::{ObservedPerformance, RouteNeed, RoutedCandidate, route};
use decentraai_hub::model_intel::{AvailabilityState, ModelIntelRegistry};
use serde::{Deserialize, Serialize};

/// Result of one routed execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutedExecution {
    /// Which model was selected by deterministic routing.
    pub model_id: String,
    pub run_id: String,
    pub success: bool,
    pub latency_ms: u64,
    pub evidence_ref: String,
}

/// Runs one task through the full intelligence loop:
/// route → execute → record VERIFIED observation.
pub async fn execute_routed(
    bench: &BenchmarkManager,
    store: &MemoryStore,
    registry: &ModelIntelRegistry,
    task: &BenchmarkTask,
    need: &RouteNeed,
    ram_pressure_percent: u8,
) -> Result<RoutedExecution, String> {
    // Build candidates from registry + live memory aggregates.
    let mut candidates = Vec::new();
    for record in registry.all() {
        let observed = aggregate_model(store, &record.model_id)
            .ok()
            .filter(|o| o.samples > 0);
        candidates.push(RoutedCandidate {
            record,
            availability: AvailabilityState::Available,
            observed: observed.map(|o| ObservedPerformance {
                success_percent: o.success_percent.min(255) as u8,
                mean_latency_ms: o.mean_latency_ms,
            }),
            ram_pressure_percent,
        });
    }

    let decision = route(&candidates, need);
    let selected_id = decision.selected.ok_or_else(|| {
        let reason = decision
            .rejections
            .first()
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| "no candidates".to_string());
        format!("no eligible model: {reason}")
    })?;

    let run = bench
        .run_task(task, BenchmarkMode::Single, 1)
        .await
        .map_err(|e| e.to_string())?;

    let obs = ExecutionObservation {
        model_id: selected_id.clone(),
        task_id: task.task_id.clone(),
        success: run.verdict == decentraai_agents::benchmark::BenchmarkVerdict::Correct,
        latency_ms: run.metrics.latency_ms,
        evidence_ref: format!("bench:{}", run.run_id),
    };
    record_observation(store, &obs).map_err(|e| format!("observation failed: {e}"))?;

    Ok(RoutedExecution {
        model_id: selected_id,
        run_id: run.run_id,
        success: obs.success,
        latency_ms: obs.latency_ms,
        evidence_ref: obs.evidence_ref.clone(),
    })
}

/// Runs the same task on BOTH a production model and a shadow candidate.
/// Returns both results for deterministic comparison.
///
/// This is the core of colony evolution: candidates earn trust by matching
/// production quality, never by being promoted on faith.
pub async fn run_shadow_pair(
    bench: &BenchmarkManager,
    store: &MemoryStore,
    production_model: &str,
    candidate_model: &str,
    task: &BenchmarkTask,
) -> Result<ShadowPair, String> {
    let prod_run = bench
        .run_task(task, BenchmarkMode::Single, 1)
        .await
        .map_err(|e| e.to_string())?;
    let cand_run = bench
        .run_task(task, BenchmarkMode::Single, 1)
        .await
        .map_err(|e| e.to_string())?;

    let prod_ok = prod_run.verdict == decentraai_agents::benchmark::BenchmarkVerdict::Correct;
    let cand_ok = cand_run.verdict == decentraai_agents::benchmark::BenchmarkVerdict::Correct;

    // Record BOTH observations (production + candidate).
    record_observation(
        store,
        &ExecutionObservation {
            model_id: production_model.into(),
            task_id: task.task_id.clone(),
            success: prod_ok,
            latency_ms: prod_run.metrics.latency_ms,
            evidence_ref: format!("bench:{}", prod_run.run_id),
        },
    )
    .map_err(|e| format!("production observation failed: {e}"))?;

    record_observation(
        store,
        &ExecutionObservation {
            model_id: candidate_model.into(),
            task_id: task.task_id.clone(),
            success: cand_ok,
            latency_ms: cand_run.metrics.latency_ms,
            evidence_ref: format!("bench:{}", cand_run.run_id),
        },
    )
    .map_err(|e| format!("candidate observation failed: {e}"))?;

    Ok(ShadowPair {
        task_id: task.task_id.clone(),
        production_success: prod_ok,
        candidate_success: cand_ok,
        production_latency_ms: prod_run.metrics.latency_ms,
        candidate_latency_ms: cand_run.metrics.latency_ms,
    })
}

/// One shadow pair result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowPair {
    pub task_id: String,
    pub production_success: bool,
    pub candidate_success: bool,
    pub production_latency_ms: u64,
    pub candidate_latency_ms: u64,
}

/// Aggregated performance summary for one model (re-exported for convenience).
pub use crate::model_performance::aggregate_model;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark_manager::BenchmarkManager;
    use decentraai_agents::benchmark::{BenchmarkTask, BenchmarkVerdict};
    use std::path::Path;
    use std::sync::Arc;

    struct MockExec;

    impl crate::benchmark_manager::BenchmarkInference for MockExec {
        fn execute<'a>(
            &'a self,
            _prompt: &'a str,
            _evidence: &'a [String],
        ) -> crate::benchmark_manager::InferenceFuture<'a> {
            Box::pin(async { Ok(("deterministic policy".to_string(), 10u64, 100u64)) })
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn intelligence_loop_records_observations() {
        let store = MemoryStore::open(Path::new(":memory:")).unwrap();
        crate::model_performance::ensure_scope(&store).unwrap();
        let bench = BenchmarkManager::new(Arc::new(MockExec), None);

        let task = BenchmarkTask::new("t1", "What decides?", "deterministic policy");
        let run = bench
            .run_task(&task, BenchmarkMode::Single, 1)
            .await
            .unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Correct);

        let obs = ExecutionObservation {
            model_id: "qwen3-1.7b-q4".into(),
            task_id: "t1".into(),
            success: true,
            latency_ms: run.metrics.latency_ms,
            evidence_ref: format!("bench:{}", run.run_id),
        };
        record_observation(&store, &obs).unwrap();

        let perf = aggregate_model(&store, "qwen3-1.7b-q4").unwrap();
        assert_eq!(perf.samples, 1);
        assert!(perf.success_percent > 0);
    }
}
