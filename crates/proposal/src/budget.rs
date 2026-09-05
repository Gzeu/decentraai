//! Experiment budgets — every live testnet effect needs explicit limits.
//!
//! A budget is the complete blast-radius declaration: HOW MUCH
//! (`max_amount_wei`), HOW EXPENSIVE (`max_gas`), HOW MANY steps
//! (`max_actions`), HOW MANY tries (`max_retries`), UNTIL WHEN
//! (`expiry_unix`), WHAT (`allowed_assets`) and WHERE
//! (`allowed_destinations`). No field is optional and no value is
//! unbounded: [`ExperimentBudget::validate`] rejects anything that is not
//! a finite, conservative, fully-specified cap.
//!
//! Deny-by-default: [`ExperimentBudget::conservative_default`] ships with an
//! EMPTY asset/destination allow-list, so nothing is permitted until the
//! operator names it explicitly.

use serde::{Deserialize, Serialize};

use crate::error::ProposalError;

/// The real DCAI token on MultiversX testnet (issued, zero initial supply).
/// A configurable asset identifier — NOT tokenomics, NOT emission logic,
/// NOT a mint instruction. DCAI can only appear in an experiment that names
/// it explicitly AND proves balance AND passes authorization.
pub const DCAI_TESTNET_ID: &str = "DCAI-51cb9b";

/// MultiversX testnet chain id. The ONLY chain the v0.2 lane knows.
/// There is no mainnet value anywhere in this crate.
pub const TESTNET_CHAIN_ID: &str = "T";

/// Conservative caps: plain value transfers only.
pub const MAX_BUDGET_WEI: u64 = 10_000_000_000_000; // 1e13 wei = 0.00001 EGLD
/// Max gas for one budgeted action (plain transfer costs 50_000).
pub const MAX_GAS_LIMIT: u64 = 100_000;
/// Max actions inside one budgeted experiment.
pub const MAX_BUDGET_ACTIONS: u32 = 8;
/// Max submit retries inside one budgeted experiment.
pub const MAX_BUDGET_RETRIES: u32 = 3;
/// Max destination label length.
pub const MAX_DESTINATION_LEN: usize = 128;
/// Max allow-list sizes (bounded everywhere).
pub const MAX_ALLOW_LIST: usize = 8;

/// Assets a testnet experiment may touch. Closed set; anything else
/// (including any mainnet asset) fails parsing, not just policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TestnetAsset {
    /// Testnet xEGLD (gas + minimal value).
    Xegld,
    /// The real DCAI testnet token. Spendable only with proven balance
    /// and explicit authorization — never minted by experiments.
    Dcai,
}

impl TestnetAsset {
    /// Machine-readable name (stable strings for evidence/denials).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Xegld => "xegld",
            Self::Dcai => "dcai",
        }
    }
}

/// The complete, finite blast-radius declaration for one experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentBudget {
    /// Budget id, referenced by the experiment record.
    pub id: String,
    /// Max total value across ALL actions (wei). Must be 1..=MAX_BUDGET_WEI.
    pub max_amount_wei: u64,
    /// Max gas per action. Must be 1..=MAX_GAS_LIMIT.
    pub max_gas: u64,
    /// Max actions. Must be 1..=MAX_BUDGET_ACTIONS.
    pub max_actions: u32,
    /// Max submit retries. Must be 0..=MAX_BUDGET_RETRIES.
    pub max_retries: u32,
    /// Expiry (unix seconds). Must be in the future at validation time —
    /// checked against `now_unix` by the caller (pure: clock is a parameter).
    pub expiry_unix: u64,
    /// Explicitly allowed assets. Empty = nothing allowed.
    pub allowed_assets: Vec<TestnetAsset>,
    /// Explicitly allowed destinations (bech32 addresses / scopes).
    /// Empty = nowhere allowed.
    pub allowed_destinations: Vec<String>,
}

impl ExperimentBudget {
    /// Deny-by-default budget: finite micro-caps, EMPTY allow-lists.
    /// The operator must name assets and destinations explicitly.
    #[must_use]
    pub fn conservative_default(id: &str, expiry_unix: u64) -> Self {
        Self {
            id: id.to_string(),
            max_amount_wei: 1_000,
            max_gas: 60_000,
            max_actions: 1,
            max_retries: 1,
            expiry_unix,
            allowed_assets: Vec::new(),
            allowed_destinations: Vec::new(),
        }
    }

    /// Validate every bound. No unlimited anything: zero/over-cap/empty
    /// values are all rejected with an explicit reason.
    pub fn validate(&self, now_unix: u64) -> Result<(), ProposalError> {
        if self.id.is_empty() || self.id.len() > 128 {
            return Err(ProposalError::Bound(
                "budget.id: length must be 1..=128".to_string(),
            ));
        }
        if self.max_amount_wei == 0 || self.max_amount_wei > MAX_BUDGET_WEI {
            return Err(ProposalError::Bound(format!(
                "budget.max_amount_wei: must be 1..={MAX_BUDGET_WEI}, got {}",
                self.max_amount_wei
            )));
        }
        if self.max_gas == 0 || self.max_gas > MAX_GAS_LIMIT {
            return Err(ProposalError::Bound(format!(
                "budget.max_gas: must be 1..={MAX_GAS_LIMIT}, got {}",
                self.max_gas
            )));
        }
        if self.max_actions == 0 || self.max_actions > MAX_BUDGET_ACTIONS {
            return Err(ProposalError::Bound(format!(
                "budget.max_actions: must be 1..={MAX_BUDGET_ACTIONS}, got {}",
                self.max_actions
            )));
        }
        if self.max_retries > MAX_BUDGET_RETRIES {
            return Err(ProposalError::Bound(format!(
                "budget.max_retries: must be 0..={MAX_BUDGET_RETRIES}, got {}",
                self.max_retries
            )));
        }
        if self.expiry_unix <= now_unix {
            return Err(ProposalError::Bound(format!(
                "budget.expiry_unix: must be in the future (now {now_unix})"
            )));
        }
        if self.allowed_assets.is_empty() || self.allowed_assets.len() > MAX_ALLOW_LIST {
            return Err(ProposalError::Bound(format!(
                "budget.allowed_assets: must name 1..={MAX_ALLOW_LIST} assets explicitly"
            )));
        }
        if self.allowed_destinations.is_empty() || self.allowed_destinations.len() > MAX_ALLOW_LIST
        {
            return Err(ProposalError::Bound(format!(
                "budget.allowed_destinations: must name 1..={MAX_ALLOW_LIST} destinations explicitly"
            )));
        }
        for d in &self.allowed_destinations {
            if d.is_empty() || d.len() > MAX_DESTINATION_LEN {
                return Err(ProposalError::Bound(
                    "budget.allowed_destinations: each entry must be 1..=128 chars".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// True when `now_unix` is past expiry.
    #[must_use]
    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.expiry_unix
    }
}

/// Shared valid-budget fixture for crate-wide tests.
#[cfg(test)]
pub(crate) fn valid_budget(now: u64) -> ExperimentBudget {
    ExperimentBudget {
        id: "budget:first".to_string(),
        max_amount_wei: 1_000,
        max_gas: 60_000,
        max_actions: 1,
        max_retries: 1,
        expiry_unix: now + 3_600,
        allowed_assets: vec![TestnetAsset::Xegld],
        allowed_destinations: vec!["erd1operator".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_budget_passes() {
        valid_budget(1_000_000).validate(1_000_000).unwrap();
    }

    #[test]
    fn conservative_default_denies_by_default() {
        let b = ExperimentBudget::conservative_default("b", 2_000_000);
        // Empty allow-lists: nothing permitted until named.
        assert!(b.validate(1_000_000).is_err());
    }

    #[test]
    fn no_unlimited_anything() {
        let now = 1_000_000;
        let mut b = valid_budget(now);
        b.max_amount_wei = 0;
        assert!(b.validate(now).is_err());
        b.max_amount_wei = MAX_BUDGET_WEI + 1;
        assert!(b.validate(now).is_err());
        b = valid_budget(now);
        b.max_gas = MAX_GAS_LIMIT + 1;
        assert!(b.validate(now).is_err());
        b = valid_budget(now);
        b.max_actions = MAX_BUDGET_ACTIONS + 1;
        assert!(b.validate(now).is_err());
        b = valid_budget(now);
        b.max_retries = MAX_BUDGET_RETRIES + 1;
        assert!(b.validate(now).is_err());
        b = valid_budget(now);
        b.expiry_unix = now;
        assert!(b.validate(now).is_err());
        b = valid_budget(now);
        b.allowed_destinations.clear();
        assert!(b.validate(now).is_err());
    }

    #[test]
    fn unknown_asset_fails_schema() {
        let bad = r#"{"unknown_coin": true}"#;
        assert!(serde_json::from_str::<TestnetAsset>(bad).is_err());
    }

    #[test]
    fn dcai_identifier_is_the_real_token() {
        assert_eq!(DCAI_TESTNET_ID, "DCAI-51cb9b");
        assert_eq!(TESTNET_CHAIN_ID, "T");
    }
}
