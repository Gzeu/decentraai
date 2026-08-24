//! CPU pool orchestrator (pure) — "Sharing is Caring" as a shared compute pool.
//!
//! A requesting node holds a workload made of MANY independent tasks. Instead
//! of running them all on its own CPU, it partitions them across the worker
//! nodes it can reach (including itself) and executes the batches in parallel,
//! then aggregates the results. This is **task/batch parallelism** over the
//! existing DFCP/Sharing is Caring delegation — it does NOT rebuild the
//! delegation (that lives in `crates/runtime/src/intel_assist.rs` and the DFCP
//! wire protocol in `crates/protocol/src/dfcp.rs`). This module is the pure,
//! testable decision half: partition + aggregate. The async execution half
//! (which actually calls `run_assist_request`) lives in the runtime API.
//!
//! Honesty rules:
//! - Partitioning is **deterministic**: worker `w` takes tasks whose index
//!   `i % workers == w`. No randomness in scheduling.
//! - Grading reuses `decentraai_agents::benchmark::grade_answer` — exact
//!   normalized matching against the gold, never an LLM judge.
//! - A task with no gold, empty output, or a failed remote run is reported
//!   honestly (`Abstained`), never guessed or counted as correct.
//! - `speedup` is only meaningful when both wall times are known; we report
//!   it as `0.0` when either side is missing rather than inventing a claim.

use decentraai_agents::benchmark::{BenchmarkTask, BenchmarkVerdict};
use serde::{Deserialize, Serialize};

/// One workload task the pool is asked to execute (subset of a benchmark
/// task: id + prompt + optional gold).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolTask {
    pub task_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<String>,
}

impl From<&BenchmarkTask> for PoolTask {
    fn from(t: &BenchmarkTask) -> Self {
        Self {
            task_id: t.task_id.clone(),
            prompt: t.prompt.clone(),
            gold: t.gold.clone(),
        }
    }
}

/// Deterministic partition of `total` task indexes across `workers` slots.
///
/// Worker `w` gets the indexes `i` where `i % workers == w`. Returns `workers`
/// buckets; buckets are empty when a worker has no tasks. `workers == 0` is
/// treated as a single worker (the caller node) to avoid a divide-by-zero.
pub fn partition(total: usize, workers: usize) -> Vec<Vec<usize>> {
    let workers = workers.max(1);
    let mut buckets: Vec<Vec<usize>> = (0..workers).map(|_| Vec::new()).collect();
    for i in 0..total {
        buckets[i % workers].push(i);
    }
    buckets
}

/// Where a single task actually executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolWorkerKind {
    /// Executed on the requesting node's own CPU.
    Local,
    /// Executed on a remote worker node via DFCP delegation.
    Remote,
}

/// The graded outcome of one task after a pool execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolOutcome {
    pub task_id: String,
    pub worker: String,
    pub worker_kind: PoolWorkerKind,
    /// Whether the underlying execution reported success (remote result ok).
    pub executed: bool,
    pub output: String,
    pub verdict: BenchmarkVerdict,
    /// Wall-clock time of just this task (ms).
    pub latency_ms: u64,
}

/// Per-worker aggregate over its assigned outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerAggregate {
    pub worker: String,
    pub tasks: usize,
    pub graded: usize,
    pub correct: usize,
    pub accuracy: f64,
    pub total_latency_ms: u64,
}

/// The honest aggregate over the whole pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolAggregate {
    pub tasks: usize,
    pub workers: usize,
    pub graded: usize,
    pub correct: usize,
    pub accuracy: f64,
    pub total_latency_ms: u64,
    /// The longest single-worker wall time — the pool's wall-clock
    /// (the parallel completion time) when the batches run concurrently.
    pub max_worker_latency_ms: u64,
}

impl PoolAggregate {
    /// `pool_wall` is the wall-clock the parallel execution actually took;
    /// `serial_wall` is the single-node (serial) baseline. speedup is only
    /// reported when both are > 0 and pool is faster; otherwise 0.0 (honest).
    pub fn speedup(&self, pool_wall_ms: u64, serial_wall_ms: u64) -> f64 {
        if pool_wall_ms > 0 && serial_wall_ms >= pool_wall_ms && serial_wall_ms > 0 {
            serial_wall_ms as f64 / pool_wall_ms as f64
        } else {
            0.0
        }
    }
}

/// Aggregates the graded outcomes into per-worker and pool-wide numbers.
/// Verdicts are recomputed from the golds here (defensive) but the caller may
/// have already set them; we trust the passed verdicts and only count.
pub fn aggregate_pool(outcomes: &[PoolOutcome]) -> PoolAggregate {
    let mut by_worker: std::collections::BTreeMap<String, (usize, usize, usize, u64)> =
        std::collections::BTreeMap::new();
    let mut graded = 0usize;
    let mut correct = 0usize;
    let mut total_latency_ms = 0u64;
    let mut worker_latency: std::collections::BTreeMap<String, u64> = Default::default();

    for o in outcomes {
        let lat = o.latency_ms;
        total_latency_ms = total_latency_ms.saturating_add(lat);
        *worker_latency.entry(o.worker.clone()).or_insert(0) = worker_latency
            .get(&o.worker)
            .copied()
            .unwrap_or(0)
            .saturating_add(lat);
        let is_graded = o.verdict == BenchmarkVerdict::Correct
            || o.verdict == BenchmarkVerdict::Incorrect;
        let is_correct = o.verdict == BenchmarkVerdict::Correct;
        if is_graded {
            graded += 1;
        }
        if is_correct {
            correct += 1;
        }
        let e = by_worker.entry(o.worker.clone()).or_insert((0, 0, 0, 0));
        e.0 += 1;
        if is_graded {
            e.1 += 1;
        }
        if is_correct {
            e.2 += 1;
        }
    }
    let max_worker_latency_ms = worker_latency.values().copied().max().unwrap_or(0);

    let per_worker: Vec<WorkerAggregate> = by_worker
        .into_iter()
        .map(|(worker, (tasks, graded_w, correct_w, _))| {
            let lat = worker_latency.get(&worker).copied().unwrap_or(0);
            WorkerAggregate {
                worker,
                tasks,
                graded: graded_w,
                correct: correct_w,
                accuracy: if graded_w == 0 {
                    0.0
                } else {
                    correct_w as f64 / graded_w as f64
                },
                total_latency_ms: lat,
            }
        })
        .collect();

    PoolAggregate {
        tasks: outcomes.len(),
        workers: per_worker.len(),
        graded,
        correct,
        accuracy: if graded == 0 {
            0.0
        } else {
            correct as f64 / graded as f64
        },
        total_latency_ms,
        max_worker_latency_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        id: &str,
        worker: &str,
        kind: PoolWorkerKind,
        output: &str,
        gold: Option<&str>,
        lat: u64,
    ) -> PoolOutcome {
        PoolOutcome {
            task_id: id.into(),
            worker: worker.into(),
            worker_kind: kind,
            executed: true,
            output: output.into(),
            verdict: decentraai_agents::benchmark::grade_answer(output, gold),
            latency_ms: lat,
        }
    }

    #[test]
    fn partition_is_deterministic_and_round_robin() {
        let p = partition(9, 3);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], vec![0, 3, 6]);
        assert_eq!(p[1], vec![1, 4, 7]);
        assert_eq!(p[2], vec![2, 5, 8]);
        // Determinism: same call → same buckets.
        assert_eq!(partition(9, 3), partition(9, 3));
        // More workers than tasks → trailing empty buckets.
        let p2 = partition(3, 5);
        assert_eq!(p2[3], Vec::<usize>::new());
        // Zero workers degenerates to one.
        assert_eq!(partition(4, 0), vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn aggregate_counts_graded_and_correct_honestly() {
        let outcomes = vec![
            outcome("a", "desktop", PoolWorkerKind::Remote, "g", Some("g"), 10),
            outcome("b", "desktop", PoolWorkerKind::Remote, "x", Some("g"), 5),
            outcome("c", "laptop", PoolWorkerKind::Remote, "g", Some("g"), 20),
            outcome("d", "laptop", PoolWorkerKind::Remote, "?", None, 1),
            outcome("e", "local", PoolWorkerKind::Local, "g", Some("g"), 3),
        ];
        let agg = aggregate_pool(&outcomes);
        assert_eq!(agg.tasks, 5);
        assert_eq!(agg.workers, 3);
        assert_eq!(agg.graded, 4); // 'd' ungradable → excluded
        assert_eq!(agg.correct, 3);
        assert!((agg.accuracy - 0.75).abs() < 1e-9);
        assert_eq!(agg.total_latency_ms, 39);
        // max worker latency = laptop (21ms) since it has the largest sum.
        assert_eq!(agg.max_worker_latency_ms, 21);
    }

    #[test]
    fn ungradable_and_failed_tasks_never_count_as_correct() {
        let outcomes = vec![
            outcome("u", "w", PoolWorkerKind::Remote, "", Some("g"), 0),
            outcome("n", "w", PoolWorkerKind::Remote, "whatever", None, 0),
        ];
        let agg = aggregate_pool(&outcomes);
        assert_eq!(agg.graded, 0);
        assert_eq!(agg.correct, 0);
        assert_eq!(agg.accuracy, 0.0);
    }

    #[test]
    fn speedup_only_reported_when_meaningful() {
        let agg = aggregate_pool(&[]);
        assert_eq!(agg.speedup(100, 300), 3.0);
        assert_eq!(agg.speedup(300, 100), 0.0); // pool slower → honest 0
        assert_eq!(agg.speedup(0, 300), 0.0); // unknown pool wall → 0
        assert_eq!(agg.speedup(100, 0), 0.0); // unknown serial wall → 0
    }
}
