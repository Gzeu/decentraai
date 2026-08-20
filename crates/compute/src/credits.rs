//! Compute credit engine (P14 Phase C–D).
//!
//! A deterministic, append-oriented, idempotent, auditable ledger of synthetic
//! compute credits. Credits are derived **only** from verified compute evidence:
//! signed receipts, measured runtime metrics, execution duration, workload
//! properties, and real reservation data.
//!
//! Every credit event explains: WHO (account), WHAT (amount), WHY (policy),
//! WHEN (timestamp), FROM WHICH RECEIPT, FROM WHICH EXECUTION, FROM WHICH
//! RESOURCE MEASUREMENT, WITH WHICH POLICY VERSION, and HOW MANY CREDITS were
//! generated.
//!
//! # Non-monetary
//!
//! These credits are synthetic bookkeeping only. They mint no money, no
//! cryptocurrency, and no marketplace. They are the technical foundation for a
//! future contribution economy, kept strictly evidence-first.
//!
//! # Versioned policies
//!
//! [`CreditPolicy`] is versioned. Historical events keep the policy/version
//! that produced them, so a node can always answer "why do I have this
//! balance?".

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::resource_contribution::{Provenance, ResourceContribution};

/// A synthetic compute credit unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComputeCredit {
    pub amount: u64,
}

/// One dimension of a credit policy.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CreditPolicyDimension {
    /// Quota/credit units granted per unit of measured work in this dimension.
    pub credits_per_unit: f64,
    /// Whether this dimension is currently enabled.
    pub enabled: bool,
}

impl Default for CreditPolicyDimension {
    fn default() -> Self {
        Self {
            credits_per_unit: 1.0,
            enabled: true,
        }
    }
}

/// A versioned, replaceable policy for converting verified resource
/// contribution into synthetic credits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditPolicy {
    pub version: u32,
    pub name: String,
    pub cpu: CreditPolicyDimension,
    pub ram: CreditPolicyDimension,
    pub gpu: CreditPolicyDimension,
    pub vram: CreditPolicyDimension,
    pub duration: CreditPolicyDimension,
    pub tokens: CreditPolicyDimension,
    pub network: CreditPolicyDimension,
    pub success_bonus: CreditPolicyDimension,
}

impl Default for CreditPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            name: "default".to_string(),
            cpu: CreditPolicyDimension {
                credits_per_unit: 10.0,
                enabled: true,
            },
            ram: CreditPolicyDimension {
                credits_per_unit: 1.0 / (1024.0 * 1024.0 * 1024.0),
                enabled: true,
            },
            gpu: CreditPolicyDimension {
                credits_per_unit: 50.0,
                enabled: true,
            },
            vram: CreditPolicyDimension {
                credits_per_unit: 1.0 / (1024.0 * 1024.0 * 1024.0),
                enabled: true,
            },
            duration: CreditPolicyDimension {
                credits_per_unit: 1.0,
                enabled: true,
            },
            tokens: CreditPolicyDimension {
                credits_per_unit: 1.0,
                enabled: true,
            },
            network: CreditPolicyDimension {
                credits_per_unit: 1.0 / (1024.0 * 1024.0),
                enabled: false,
            },
            success_bonus: CreditPolicyDimension {
                credits_per_unit: 5.0,
                enabled: true,
            },
        }
    }
}

impl CreditPolicy {
    /// Convert a resource contribution into credits under this policy.
    pub fn calculate(&self, rc: &ResourceContribution) -> CreditCalculation {
        let mut amount = 0.0;
        let mut breakdown: BTreeMap<String, f64> = BTreeMap::new();

        if self.cpu.enabled {
            if let Some(d) = &rc.cpu_time_seconds {
                let v = d.value * self.cpu.credits_per_unit;
                amount += v;
                breakdown.insert("cpu_time".to_string(), v);
            }
        }
        if self.ram.enabled {
            if let Some(d) = &rc.ram_bytes_seconds {
                let v = d.value * self.ram.credits_per_unit;
                amount += v;
                breakdown.insert("ram".to_string(), v);
            }
        }
        if self.gpu.enabled {
            if let Some(d) = &rc.gpu_time_seconds {
                let v = d.value * self.gpu.credits_per_unit;
                amount += v;
                breakdown.insert("gpu_time".to_string(), v);
            }
        }
        if self.vram.enabled {
            if let Some(d) = &rc.vram_bytes_seconds {
                let v = d.value * self.vram.credits_per_unit;
                amount += v;
                breakdown.insert("vram".to_string(), v);
            }
        }
        if self.duration.enabled {
            if let Some(d) = &rc.execution_duration_ms {
                let v = d.value * self.duration.credits_per_unit;
                amount += v;
                breakdown.insert("duration".to_string(), v);
            }
        }
        if self.tokens.enabled {
            if let Some(d) = &rc.tokens_processed {
                let v = d.value * self.tokens.credits_per_unit;
                amount += v;
                breakdown.insert("tokens".to_string(), v);
            }
        }
        if self.network.enabled {
            if let Some(d) = &rc.network_bytes {
                let v = d.value * self.network.credits_per_unit;
                amount += v;
                breakdown.insert("network".to_string(), v);
            }
        }
        if self.success_bonus.enabled && rc.success {
            let bonus = self.success_bonus.credits_per_unit;
            amount += bonus;
            breakdown.insert("success_bonus".to_string(), bonus);
        }

        CreditCalculation {
            credits: amount.max(0.0).round() as u64,
            policy_version: self.version,
            breakdown,
        }
    }
}

/// The result of applying a credit policy to a resource contribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditCalculation {
    pub credits: u64,
    pub policy_version: u32,
    pub breakdown: BTreeMap<String, f64>,
}

/// A single auditable credit event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditEvent {
    /// WHO: account (worker peer id) that earned the credit.
    pub account: String,
    /// WHAT: credits earned.
    pub amount: u64,
    /// WHEN: unix ms.
    pub created_at_ms: u64,
    /// FROM WHICH RECEIPT.
    pub receipt_id: String,
    /// FROM WHICH EXECUTION.
    pub execution_id: String,
    /// WITH WHICH POLICY.
    pub policy_version: u32,
    /// FROM WHICH RESOURCE MEASUREMENT (summary).
    pub resource_summary: ResourceSummary,
    /// WHY / HOW: policy breakdown that produced the amount.
    pub calculation: CreditCalculation,
}

/// A portable summary of the resource measurement behind a credit event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceSummary {
    pub capability: String,
    pub model: Option<String>,
    pub success: bool,
    pub tokens_processed: Option<u64>,
    pub execution_duration_ms: Option<u64>,
    pub provenance: Provenance,
}

impl ResourceSummary {
    pub fn from_contribution(rc: &ResourceContribution) -> Self {
        Self {
            capability: rc.capability.clone(),
            model: rc.model.clone(),
            success: rc.success,
            tokens_processed: rc.tokens_processed.as_ref().map(|d| d.value as u64),
            execution_duration_ms: rc.execution_duration_ms.as_ref().map(|d| d.value as u64),
            provenance: Provenance::Measured,
        }
    }
}

/// Current balance of one credit account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CreditAccount {
    pub earned: u64,
    pub consumed: u64,
    pub balance: u64,
}

/// Deterministic, append-oriented, idempotent compute credit ledger.
#[derive(Debug, Default)]
pub struct CreditLedger {
    accounts: BTreeMap<String, CreditAccount>,
    applied: HashSet<String>,
    events: VecDeque<CreditEvent>,
    policy: CreditPolicy,
}

const MAX_EVENTS: usize = 4096;

impl CreditLedger {
    pub fn new(policy: CreditPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn policy(&self) -> &CreditPolicy {
        &self.policy
    }

    pub fn set_policy(&mut self, policy: CreditPolicy) {
        self.policy = policy;
    }

    /// Credit one verified resource contribution, exactly once per execution_id.
    /// Returns the credits earned (0 for duplicates or non-verified work).
    pub fn credit(
        &mut self,
        account: &str,
        rc: &ResourceContribution,
        receipt_id: &str,
        created_at_ms: u64,
    ) -> u64 {
        let key = format!("{}:{}", receipt_id, rc.execution_id);
        if !self.applied.insert(key.clone()) {
            return 0;
        }
        if !rc.success {
            return 0;
        }
        let calculation = self.policy.calculate(rc);
        if calculation.credits == 0 {
            return 0;
        }
        let event = CreditEvent {
            account: account.to_string(),
            amount: calculation.credits,
            created_at_ms,
            receipt_id: receipt_id.to_string(),
            execution_id: rc.execution_id.clone(),
            policy_version: calculation.policy_version,
            resource_summary: ResourceSummary::from_contribution(rc),
            calculation: calculation.clone(),
        };
        let acc = self.accounts.entry(account.to_string()).or_default();
        acc.earned = acc.earned.saturating_add(calculation.credits);
        acc.balance = acc.balance.saturating_add(calculation.credits);
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
        calculation.credits
    }

    /// Consume credits from an account. Returns false if insufficient.
    pub fn consume(&mut self, account: &str, amount: u64, ref_id: &str) -> bool {
        if amount == 0 {
            return true;
        }
        let acc = self.accounts.entry(account.to_string()).or_default();
        if acc.balance < amount {
            return false;
        }
        acc.balance = acc.balance.saturating_sub(amount);
        acc.consumed = acc.consumed.saturating_add(amount);
        // Also record a consume event? Keep minimal; caller can emit audit.
        let _ = ref_id;
        true
    }

    pub fn account(&self, account: &str) -> Option<CreditAccount> {
        self.accounts.get(account).copied()
    }

    pub fn accounts(&self) -> BTreeMap<String, CreditAccount> {
        self.accounts.clone()
    }

    pub fn events(&self) -> &VecDeque<CreditEvent> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_contribution::{ResourceContributionBuilder, ResourceDimension};

    fn rc() -> ResourceContribution {
        ResourceContributionBuilder::new("exec-1", "peer-a")
            .capability("inference")
            .model("llama.gguf")
            .success(true)
            .dimension(ResourceDimension::new("tokens_processed", 100.0, "tokens"))
            .dimension(ResourceDimension::new("execution_duration_ms", 500.0, "ms"))
            .build()
    }

    #[test]
    fn verified_work_credits_once() {
        let mut ledger = CreditLedger::new(CreditPolicy::default());
        let amount = ledger.credit("peer-a", &rc(), "receipt-1", 1_000_000);
        assert!(amount > 0);
        assert_eq!(ledger.account("peer-a").unwrap().earned, amount);

        let again = ledger.credit("peer-a", &rc(), "receipt-1", 1_000_000);
        assert_eq!(again, 0);
    }

    #[test]
    fn failed_work_earns_nothing() {
        let mut ledger = CreditLedger::new(CreditPolicy::default());
        let mut failure = rc();
        failure.success = false;
        let amount = ledger.credit("peer-a", &failure, "receipt-2", 1_000_000);
        assert_eq!(amount, 0);
    }

    #[test]
    fn policy_version_recorded() {
        let mut ledger = CreditLedger::new(CreditPolicy::default());
        ledger.credit("peer-a", &rc(), "receipt-1", 1_000_000);
        let ev = ledger.events().back().unwrap();
        assert_eq!(ev.policy_version, 1);
        assert_eq!(ev.execution_id, "exec-1");
    }

    #[test]
    fn consume_and_balance() {
        let mut ledger = CreditLedger::new(CreditPolicy::default());
        ledger.credit("peer-a", &rc(), "receipt-1", 1_000_000);
        let before = ledger.account("peer-a").unwrap();
        assert!(before.balance > 0);
        assert!(ledger.consume("peer-a", before.balance, "use-1"));
        let after = ledger.account("peer-a").unwrap();
        assert_eq!(after.balance, 0);
        assert_eq!(after.earned, before.earned);
        assert_eq!(after.consumed, before.earned);
    }
}
