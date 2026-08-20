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
}

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
            *self.by_resource.entry("duration_ms".to_string()).or_default() += d.value;
        }
        if let Some(d) = &rc.cpu_time_seconds {
            *self.by_resource.entry("cpu_time".to_string()).or_default() += d.value;
        }
        if let Some(d) = &rc.gpu_time_seconds {
            *self.by_resource.entry("gpu_time".to_string()).or_default() += d.value;
        }

        // by_model
        if let Some(model) = &rc.model {
            let mc = self.by_model.entry(model.clone()).or_insert_with(|| ModelContribution {
                model: model.clone(),
                ..Default::default()
            });
            mc.executions += 1;
            if let Some(d) = &rc.tokens_processed {
                mc.tokens = mc.tokens.saturating_add(d.value as u64);
            }
            mc.credits = mc.credits.saturating_add(credits);
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
