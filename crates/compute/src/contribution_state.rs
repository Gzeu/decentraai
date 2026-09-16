//! Node-local contribution state (P14 Phase E).
//!
//! Tracks verified executions, resource contribution, credits, and projections
//! by time range, model, worker, and execution strategy. All state is derived
//! from real execution evidence; nothing is invented.
//!
//! The state is intentionally node-local: there is no centralized economy. A
//! node knows its own contribution and the receipts it has verified; P2P
//! advertisements carry the same primitives already used by the scheduler.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::resource_contribution::ResourceContribution;

/// Aggregate contribution state for one node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeContributionState {
    pub verified_executions: u64,
    pub failed_executions: u64,
    pub total_credits_earned: u64,
    pub total_credits_consumed: u64,
    pub balance: u64,
    #[serde(default)]
    pub ledger_version: u64,
    pub by_resource: BTreeMap<String, f64>,
    pub by_model: BTreeMap<String, ModelContribution>,
    pub by_worker: BTreeMap<String, WorkerContribution>,
    pub by_time_range: BTreeMap<String, TimeRangeContribution>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelContribution {
    pub model: String,
    pub executions: u64,
    pub tokens: u64,
    pub credits: u64,
    /// Requested-model ids (verbatim) attributed to this served model.
    /// Bounded: at most 8 named ids, overflow folds into "other", so the
    /// map can never grow without bound. The honest asked==got entry is
    /// included, so Σ requested == executions for rows born after this
    /// tracking landed (legacy rows predate it — see `record_execution`).
    #[serde(default)]
    pub requested: BTreeMap<String, u64>,
    /// Executions whose requested id differs from the served model.
    /// Always derivable as Σ requested − requested[served]; published
    /// explicitly so readers never have to derive it.
    #[serde(default)]
    pub routed: u64,
}

/// Max distinct requested ids kept per served model; overflow folds into
/// the "other" bucket (counts preserved, keys bounded).
pub const MAX_REQUESTED_IDS: usize = 8;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerContribution {
    pub worker: String,
    pub executions: u64,
    pub tokens: u64,
    pub credits: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeRangeContribution {
    pub range: String,
    pub executions: u64,
    pub credits: u64,
}

impl NodeContributionState {
    pub fn record_execution(&mut self, rc: &ResourceContribution, credits: u64) {
        self.ledger_version = self.ledger_version.saturating_add(1);
        if rc.success {
            self.verified_executions += 1;
        } else {
            self.failed_executions += 1;
        }
        self.total_credits_earned = self.total_credits_earned.saturating_add(credits);
        self.balance = self.balance.saturating_add(credits);

        // by_resource
        if let Some(d) = &rc.tokens_processed {
            *self.by_resource.entry("tokens".to_string()).or_default() += d.value;
        }
        if let Some(d) = &rc.execution_duration_ms {
            *self
                .by_resource
                .entry("duration_ms".to_string())
                .or_default() += d.value;
        }
        if let Some(d) = &rc.cpu_time_seconds {
            *self.by_resource.entry("cpu_time".to_string()).or_default() += d.value;
        }
        if let Some(d) = &rc.gpu_time_seconds {
            *self.by_resource.entry("gpu_time".to_string()).or_default() += d.value;
        }

        // by_model (+ reqserv requested/routed, nested in the row).
        if let Some(model) = &rc.model {
            let mc = self
                .by_model
                .entry(model.clone())
                .or_insert_with(|| ModelContribution {
                    model: model.clone(),
                    ..Default::default()
                });
            mc.executions += 1;
            if let Some(d) = &rc.tokens_processed {
                mc.tokens = mc.tokens.saturating_add(d.value as u64);
            }
            mc.credits = mc.credits.saturating_add(credits);
            // reqserv: attribute the requested id verbatim. Unknown
            // requested falls back to asked==got (the only honest default
            // when the caller never saw a request id — e.g. legacy paths).
            // Every execution adds exactly one, so Σ requested == executions
            // for rows born after this tracking landed.
            let asked = rc
                .requested_model
                .clone()
                .unwrap_or_else(|| model.clone());
            if asked != *model {
                mc.routed += 1;
            }
            if mc.requested.contains_key(&asked) || mc.requested.len() < MAX_REQUESTED_IDS {
                *mc.requested.entry(asked).or_default() += 1;
            } else {
                *mc.requested.entry("other".to_string()).or_default() += 1;
            }
        }

        // by_worker
        let wc = self
            .by_worker
            .entry(rc.worker_node.clone())
            .or_insert_with(|| WorkerContribution {
                worker: rc.worker_node.clone(),
                ..Default::default()
            });
        wc.executions += 1;
        if let Some(d) = &rc.tokens_processed {
            wc.tokens = wc.tokens.saturating_add(d.value as u64);
        }
        wc.credits = wc.credits.saturating_add(credits);

        // by_time_range: bucket executions per UTC calendar day so the
        // dimension carries real data instead of staying present-but-empty
        // on the wire. Day granularity keeps cardinality bounded (one
        // entry per day) while giving the console a durable daily series.
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let trc = self
            .by_time_range
            .entry(day.clone())
            .or_insert_with(|| TimeRangeContribution {
                range: day,
                ..Default::default()
            });
        trc.executions += 1;
        trc.credits = trc.credits.saturating_add(credits);
    }

    pub fn consume(&mut self, amount: u64) -> bool {
        if self.balance < amount {
            return false;
        }
        self.balance -= amount;
        self.total_credits_consumed = self.total_credits_consumed.saturating_add(amount);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_contribution::{ResourceContributionBuilder, ResourceDimension};

    #[test]
    fn state_tracks_executions_and_credits() {
        let mut state = NodeContributionState::default();
        let rc = ResourceContributionBuilder::new("exec-1", "peer-a")
            .capability("inference")
            .model("llama.gguf")
            .success(true)
            .dimension(ResourceDimension::new("tokens_processed", 50.0, "tokens"))
            .build();
        state.record_execution(&rc, 100);
        assert_eq!(state.verified_executions, 1);
        assert_eq!(state.balance, 100);
        assert!(state.by_model.contains_key("llama.gguf"));
    }

    #[test]
    fn time_range_buckets_per_utc_day() {
        let mut state = NodeContributionState::default();
        let rc = ResourceContributionBuilder::new("exec-3", "peer-a")
            .capability("inference")
            .success(true)
            .dimension(ResourceDimension::new("tokens_processed", 10.0, "tokens"))
            .build();
        state.record_execution(&rc, 50);
        state.record_execution(&rc, 25);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let trc = state.by_time_range.get(&today).expect("today bucket exists");
        assert_eq!(trc.executions, 2);
        assert_eq!(trc.credits, 75);
        assert_eq!(trc.range, today);
    }

    #[test]
    fn requested_routed_tracks_verbatim_and_stays_bounded() {
        let mut state = NodeContributionState::default();
        let req = |asked: &str| {
            ResourceContributionBuilder::new("e", "peer-a")
                .capability("inference")
                .model("served.gguf")
                .requested_model(asked)
                .success(true)
                .dimension(ResourceDimension::new("tokens_processed", 1.0, "tokens"))
                .build()
        };
        // asked == got: no routing.
        state.record_execution(&req("served.gguf"), 10);
        // asked != got: routed.
        state.record_execution(&req("other.gguf"), 10);
        // Overflow: 10 distinct ids against a cap of 8 named.
        for i in 0..10 {
            state.record_execution(&req(&format!("m-{i}")), 1);
        }
        let mc = state.by_model.get("served.gguf").unwrap();
        assert_eq!(mc.executions, 12);
        let sum: u64 = mc.requested.values().sum();
        assert_eq!(sum, 12, "every execution lands exactly once in requested");
        assert_eq!(mc.requested.get("served.gguf"), Some(&1));
        assert_eq!(mc.requested.get("other.gguf"), Some(&1));
        assert!(mc.requested.len() <= super::MAX_REQUESTED_IDS + 1);
        assert_eq!(mc.routed, 11);
    }

    #[test]
    fn failed_execution_tracks_separately() {
        let mut state = NodeContributionState::default();
        let mut rc = ResourceContributionBuilder::new("exec-2", "peer-a")
            .capability("inference")
            .success(false)
            .build();
        // builder always sets success, so override manually
        rc.success = false;
        state.record_execution(&rc, 0);
        assert_eq!(state.failed_executions, 1);
        assert_eq!(state.verified_executions, 0);
    }
}
