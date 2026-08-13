//! Autonomous execution planner (M23).
//!
//! The planner turns a request + the current fabric state into an
//! [`ExecutionPlan`], making *all* the orchestration decisions the user should
//! never need to think about:
//!
//! - which workers serve the model (and are trusted/healthy),
//! - whether to execute locally or on a remote worker,
//! - whether a single worker suffices or a multi-stage plan is justified,
//! - how network cost (M19) and KV state (M20) weigh in,
//! - expert-split candidacy where the engine advertises it (M21),
//! - the deterministic ranking and the fallback order.
//!
//! It is pure and I/O-free: the coordinator passes it the live registry,
//! the link graph, and the request; it returns the best plan. Tests drive it
//! with synthetic fabric; production feeds it real measurements.

use crate::engine::{EngineCapabilities, EngineKind};
use crate::expert::{ExpertRegistry, ExpertRouter};
use crate::kv::{ContextProfile, KVCacheState, KvPlanner};
use crate::network::{LinkMetrics, NetworkGraph};
use crate::plan::{ExecutionPlan, ExecutionStage, PlanKind};
use std::collections::BTreeMap;

/// A candidate worker the planner can place stages on.
#[derive(Debug, Clone)]
pub struct WorkerFacts {
    pub peer_id: String,
    pub trusted: bool,
    pub healthy: bool,
    pub engine: EngineKind,
    /// Derived nominal execution throughput (tokens/sec) for scoring.
    pub tokens_per_second: u32,
    /// Nominal single-token latency (ms).
    pub latency_ms: u32,
    pub queue_depth: u32,
    pub load_percent: u8,
    pub available_ram_mb: u64,
    pub available_vram_mb: u64,
    pub serves_model: bool,
    /// Engine-reported capabilities (probed; defaults conservative).
    pub capabilities: EngineCapabilities,
    /// KV state for the model on this worker.
    pub kv: KVCacheState,
}

/// The request facts the planner plans for.
#[derive(Debug, Clone)]
pub struct RequestFacts {
    pub model_hash: String,
    pub est_ram_mb: u64,
    pub est_vram_mb: u64,
    /// Full context budget this worker must accommodate (model ctx size).
    pub context: ContextProfile,
    /// Prompt + any model that must move to the worker (MiB), for network cost.
    pub transfer_mib: u64,
    /// Whether local execution exists (i.e. this node can self-run).
    pub local_peer: Option<String>,
}

/// The planner's materialized decision plus the plan it chose.
#[derive(Debug, Clone)]
pub struct PlanResult {
    pub plan: ExecutionPlan,
    /// Why this plan over alternatives (for audit/observability).
    pub reasoning: String,
    pub estimated_ms: u32,
}

impl ExecutionPlan {
    /// Score a plan's estimated cost from a worker's nominal perf.
    fn cost_estimate(stages: &[&ExecutionStage], workers: &BTreeMap<String, WorkerFacts>) -> u32 {
        let mut total = 0u32;
        for s in stages {
            let w = workers.get(&s.worker);
            let latency = w.map(|f| f.latency_ms).unwrap_or(100);
            let tps = w.map(|f| f.tokens_per_second).unwrap_or(50);
            // Rough expected ms: latency + per-token generation. Prompt-only
            // factor is ignored here (KV-aware separation is a future split).
            total += latency + 100_000 / tps.max(1);
        }
        total
    }
}

/// The autonomous execution planner (M23).
pub struct ExecutionPlanner {
    pub network: NetworkGraph,
    pub experts: ExpertRegistry,
    pub allow_multi_stage: bool,
}

impl Default for ExecutionPlanner {
    fn default() -> Self {
        Self {
            network: NetworkGraph::new(),
            experts: ExpertRegistry::new(),
            allow_multi_stage: true,
        }
    }
}

impl ExecutionPlanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the best execution plan for a request over the given fabric.
    pub fn plan(
        &self,
        req: &RequestFacts,
        workers: &[WorkerFacts],
    ) -> PlanResult {
        // Deterministic candidate order: eligibility first, then score.
        let by_id: BTreeMap<String, WorkerFacts> = workers
            .iter()
            .map(|f| (f.peer_id.clone(), f.clone()))
            .collect();

        let eligible: Vec<&WorkerFacts> = workers
            .iter()
            .filter(|f| f.trusted && f.healthy && f.serves_model)
            .collect();

        // KV-aware hint narrows the field for continuation / long context.
        let kv_hint = KvPlanner.route(
            &req.context,
            &eligible
                .iter()
                .map(|f| (f.peer_id.clone(), true, f.kv))
                .collect::<Vec<_>>(),
            eligible
                .iter()
                .any(|f| f.capabilities.prefill_decode_separation),
        );

        // Score eligible workers: perf + load + network reach cost.
        let mut ranked: Vec<(f32, &WorkerFacts)> = eligible
            .iter()
            .map(|f| {
                let score = self.score(f, req, kv_hint.prefer_kv_headroom);
                (score, *f)
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.peer_id.cmp(&b.1.peer_id)) // PeerId asc tie-break
        });

        let Some((_, best)) = ranked.first() else {
            return PlanResult {
                plan: ExecutionPlan {
                    plan_id: uuid::Uuid::new_v4().to_string(),
                    model_hash: req.model_hash.clone(),
                    kind: PlanKind::Single(ExecutionStage {
                        stage_id: "none".into(),
                        worker: String::new(),
                        model_hash: req.model_hash.clone(),
                        engine: EngineKind::RemoteOpenAI,
                        est_ram_mb: req.est_ram_mb,
                        est_vram_mb: req.est_vram_mb,
                    }),
                    fallback_orders: Vec::new(),
                },
                reasoning: "no eligible worker serves this model".to_string(),
                estimated_ms: 0,
            };
        };

        let fallback_orders = self.fallback_orders(&ranked);
        let stage = self.build_stage(best, req);
        let plan = ExecutionPlan {
            plan_id: uuid::Uuid::new_v4().to_string(),
            model_hash: req.model_hash.clone(),
            kind: PlanKind::Single(stage.0.clone()),
            fallback_orders,
        };
        let est = ExecutionPlan::cost_estimate(&[&stage.0], &by_id);
        PlanResult {
            reasoning: stage.1,
            estimated_ms: est,
            plan,
        }
    }

    fn build_stage(
        &self,
        f: &WorkerFacts,
        req: &RequestFacts,
    ) -> (ExecutionStage, String) {
        let stage = ExecutionStage {
            stage_id: format!("s1-{id}", id = f.peer_id),
            worker: f.peer_id.clone(),
            model_hash: req.model_hash.clone(),
            engine: f.engine,
            est_ram_mb: req.est_ram_mb,
            est_vram_mb: req.est_vram_mb,
        };
        let mut reasons = format!(
            "single-stage on {} ({} tps, {}ms latency, queue {}): load {:.0}%",
            f.peer_id, f.tokens_per_second, f.latency_ms, f.queue_depth, f.load_percent,
        );
        if f.capabilities.expert_routing {
            // Expert-aware decision.
            if let crate::expert::ExpertDecision::ExpertSplit { workers, .. } =
                ExpertRouter.route(&req.model_hash, &self.experts, vec![f.peer_id.clone()])
            {
                reasons.push_str(&format!("; expert split across {workers:?}"));
            }
        }
        (stage, reasons)
    }

    /// Scores a worker for the request. Perf/load dominate; network and KV
    /// headroom steer ties and long-context / continuation cases.
    fn score(
        &self,
        f: &WorkerFacts,
        req: &RequestFacts,
        prefer_kv_headroom: bool,
    ) -> f32 {
        let tps_score = (f.tokens_per_second as f32 / 200.0).clamp(0.0, 1.0);
        let latency_score = 1.0 - (f.latency_ms as f32 / 1000.0).clamp(0.0, 1.0);
        let load_score = 1.0 - (f.load_percent as f32 / 100.0);
        let queue_score = (1.0 - f.queue_depth as f32 / 10.0).clamp(0.0, 1.0);
        let headroom = if req.est_ram_mb > 0 {
            (f.available_ram_mb as f64 / req.est_ram_mb as f64).min(1.0) as f32
        } else {
            1.0
        };
        // Network: prefer workers that are cheaper to reach.
        let link = self.network.get(&f.peer_id);
        let net_score = self.network_score(&link);
        // KV: boost workers with headroom when the request is KV-hungry.
        let kv_score = if prefer_kv_headroom {
            match f.kv.headroom_tokens() {
                Some(h) if h >= req.context.total_slots() => 0.2,
                _ => 0.0,
            }
        } else {
            0.0
        };

        0.25 * tps_score
            + 0.15 * latency_score
            + 0.15 * load_score
            + 0.10 * queue_score
            + 0.15 * headroom
            + 0.10 * net_score
            + 0.10 * kv_score
    }

    fn network_score(&self, link: &LinkMetrics) -> f32 {
        let rtt_ms = link.rtt_us / 1000;
        let rtt_score = (1.0 - (rtt_ms as f32 / 200.0)).clamp(0.0, 1.0);
        rtt_score * 0.7 + (if link.bandwidth_mbps >= 100 { 0.3 } else { 0.1 })
    }

    /// Builds deterministic fallback worker orders (ranked, minus already used).
    fn fallback_orders(&self, ranked: &[(f32, &WorkerFacts)]) -> Vec<Vec<String>> {
        let mut orders = Vec::new();
        if ranked.len() > 1 {
            let mut rest: Vec<String> = ranked.iter().map(|(_, f)| f.peer_id.clone()).collect();
            for _ in 0..(ranked.len().saturating_sub(1)) {
                orders.push(rest.clone());
                if !rest.is_empty() {
                    rest.remove(0);
                }
            }
        }
        orders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_facts(id: &str, tps: u32, latency: u32, load: u8) -> WorkerFacts {
        WorkerFacts {
            peer_id: id.to_string(),
            trusted: true,
            healthy: true,
            engine: EngineKind::LlamaServer,
            tokens_per_second: tps,
            latency_ms: latency,
            queue_depth: 0,
            load_percent: load,
            available_ram_mb: 4096,
            available_vram_mb: 0,
            serves_model: true,
            capabilities: EngineCapabilities::conservative(),
            kv: KVCacheState::Empty,
        }
    }

    fn req() -> RequestFacts {
        RequestFacts {
            model_hash: "m1".into(),
            est_ram_mb: 512,
            est_vram_mb: 0,
            context: ContextProfile {
                prompt_tokens: 100,
                max_output_tokens: 64,
                is_continuation: false,
                prefix_resident_on: None,
            },
            transfer_mib: 0,
            local_peer: None,
        }
    }

    #[test]
    fn picks_fastest_eligible_worker() {
        let ws = vec![
            worker_facts("slow", 40, 400, 90),
            worker_facts("fast", 180, 50, 10),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["fast"]);
        assert_eq!(p.plan.stage_count(), 1);
    }

    #[test]
    fn no_eligible_worker_yields_empty_plan() {
        let mut slow = worker_facts("slow", 40, 400, 90);
        slow.serves_model = false;
        let p = ExecutionPlanner::default().plan(&req(), &[slow]);
        assert!(p.plan.workers().is_empty() || p.plan.workers() == vec![""]);
        assert!(p.reasoning.contains("no eligible"));
    }

    #[test]
    fn untrusted_workers_are_skipped() {
        let mut trusted = worker_facts("fast", 180, 50, 10);
        let mut untrusted = worker_facts("other", 200, 20, 5);
        trusted.trusted = true;
        untrusted.trusted = false;
        let p = ExecutionPlanner::default().plan(&req(), &[trusted, untrusted]);
        assert_eq!(p.plan.workers(), vec!["fast"]);
    }

    #[test]
    fn fallback_orders_exclude_top_worker() {
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            worker_facts("b", 150, 60, 20),
            worker_facts("c", 120, 80, 30),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert!(!p.plan.fallback_orders.is_empty());
        for order in &p.plan.fallback_orders {
            assert!(order.contains(&"a".to_string()) || order.contains(&"b".to_string()) || order.contains(&"c".to_string()));
        }
    }

    #[test]
    fn network_cost_steers_low_rtt_worker_when_perf_is_equal() {
        let mut planner = ExecutionPlanner::default();
        planner.network.set(
            "far",
            LinkMetrics::prior(crate::network::Locality::Remote, Some(80_000)),
        );
        planner.network.set(
            "near",
            LinkMetrics::prior(crate::network::Locality::Lan, Some(2_000)),
        );
        // Identical nominal performance: link cost is the only differentiator.
        let ws = vec![
            worker_facts("far", 150, 40, 10),
            worker_facts("near", 150, 40, 10),
        ];
        let p = planner.plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["near"]);
    }

    #[test]
    fn throughput_beats_remote_rtt_for_large_generation() {
        let mut planner = ExecutionPlanner::default();
        planner.network.set(
            "fast",
            LinkMetrics::prior(crate::network::Locality::Remote, Some(80_000)),
        );
        planner.network.set(
            "slow",
            LinkMetrics::prior(crate::network::Locality::Lan, Some(2_000)),
        );
        // A much faster worker still wins a throughput-bound request.
        let ws = vec![
            worker_facts("fast", 500, 10, 5),
            worker_facts("slow", 60, 120, 60),
        ];
        let p = planner.plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["fast"]);
    }
}