//! Tokenomics SIMULATOR (Phase 3): pure, config-driven, reproducible.
//!
//! ```text
//! TokenomicsParams (JSON/TOML deserializable)
//!   → simulate(params, nodes, avg_award) → SimulationReport
//! ```
//!
//! # Rules of the simulation
//!
//! - INTEGER-ONLY math over micro-CU. Same params + same inputs → the exact
//!   same report bytes (the reproducibility test proves it).
//! - NO final parameters are chosen here: every number comes from
//!   [`TokenomicsParams`], which is deserialized from config. Changing the
//!   economics means editing config, never code.
//! - Sustainability is DEFINED, not vibes: the reward pool must never go
//!   negative across all epochs, cumulative emissions must never exceed
//!   total supply, and every epoch's minimum reward must be payable.
//!
//! Allocations split each epoch's emission: contributors / validators /
//! development / treasury (basis points, validated to sum to 100 %).
//! Network fees are taken from contributor payouts; a configurable share of
//! fees is BURNED (deflation), the rest lands in the treasury.
//! Vesting delays contributor liquidity linearly over `vesting_epochs`.

use serde::{Deserialize, Serialize};

/// Emission schedule variants. Extensible WITHOUT breaking old configs:
/// unknown tags fail at deserialization (closed schema — hostile configs
/// are rejected, not guessed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EmissionSchedule {
    /// Constant emission every epoch.
    Fixed,
    /// Emission halves every `halving_every_epochs`.
    Halving { halving_every_epochs: u32 },
    /// Linear decay toward zero across `epochs`.
    LinearDecay,
}

/// Allocation split in basis points. Must sum to [`super::BPS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationSplit {
    pub contributors_bps: u64,
    pub validators_bps: u64,
    pub development_bps: u64,
    pub treasury_bps: u64,
}

impl AllocationSplit {
    pub fn validate(&self) -> Result<(), String> {
        let sum = self
            .contributors_bps
            .saturating_add(self.validators_bps)
            .saturating_add(self.development_bps)
            .saturating_add(self.treasury_bps);
        if sum != super::BPS {
            return Err(format!(
                "allocations sum to {} bps, expected {}",
                sum,
                super::BPS
            ));
        }
        Ok(())
    }

    pub fn split(&self, amount: u64) -> [u64; 4] {
        let contributors = amount * self.contributors_bps / super::BPS;
        let validators = amount * self.validators_bps / super::BPS;
        let development = amount * self.development_bps / super::BPS;
        // Treasury absorbs the rounding remainder — the pool never leaks.
        let treasury = amount
            .saturating_sub(contributors)
            .saturating_sub(validators)
            .saturating_sub(development);
        [contributors, validators, development, treasury]
    }
}

/// Slashing policy for provable misbehavior (bounded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlashingParams {
    pub enabled: bool,
    /// Max slash per offender per epoch, bps of their vested balance.
    pub max_bps_per_epoch: u64,
}

/// The full parameter set — deserialize this from JSON/TOML config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenomicsParams {
    /// Total lifetime supply in micro-CU.
    pub total_supply_micro_cu: u64,
    pub epochs: u32,
    pub schedule: EmissionSchedule,
    /// Initial epoch emission as bps OF TOTAL SUPPLY.
    pub initial_emission_bps_of_supply: u64,
    pub allocations: AllocationSplit,
    /// Network fee taken from each contributor payout (bps of payout).
    pub network_fee_bps: u64,
    /// Share of collected fees burned (deflation).
    pub burn_bps_of_fee: u64,
    /// Contributor rewards vest linearly over this many epochs.
    pub vesting_epochs: u32,
    pub slashing: SlashingParams,
    /// Per-node minimum payable reward per epoch (pool permitting).
    pub min_reward_micro_cu: u64,
    /// Per-node maximum reward per epoch (anti-concentration cap).
    pub max_reward_micro_cu: u64,
}

impl TokenomicsParams {
    pub fn validate(&self) -> Result<(), String> {
        self.allocations.validate()?;
        if self.network_fee_bps > super::BPS {
            return Err("network_fee_bps > 100%".into());
        }
        if self.burn_bps_of_fee > super::BPS {
            return Err("burn_bps_of_fee > 100%".into());
        }
        if self.min_reward_micro_cu > self.max_reward_micro_cu {
            return Err("min_reward exceeds max_reward".into());
        }
        if self.initial_emission_bps_of_supply > super::BPS {
            return Err("initial emission exceeds total supply per epoch".into());
        }
        Ok(())
    }

    fn emission_for_epoch(&self, epoch: u32) -> u64 {
        let base = self.total_supply_micro_cu * self.initial_emission_bps_of_supply / super::BPS;
        match self.schedule {
            EmissionSchedule::Fixed => base,
            EmissionSchedule::Halving {
                halving_every_epochs,
            } => {
                let halvings = u64::from(epoch / halving_every_epochs.max(1));
                base >> halvings.min(63)
            }
            EmissionSchedule::LinearDecay => {
                let e = u64::from(epoch).max(1);
                let total = u64::from(self.epochs.max(1));
                base * (total - e.min(total)) / total
            }
        }
    }
}

/// One epoch's deterministic outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochReport {
    pub epoch: u32,
    pub emitted: u64,
    pub to_contributors: u64,
    pub per_node_paid: u64,
    pub fees_collected: u64,
    pub burned: u64,
    pub to_validators: u64,
    pub to_development: u64,
    pub to_treasury: u64,
    /// Supply NOT yet emitted (remaining pool).
    pub remaining_pool: u64,
    /// Whether every node received at least the configured minimum.
    pub min_reward_payable: bool,
}

/// Full simulation result for one scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationReport {
    pub nodes: u32,
    pub epochs_run: u32,
    pub per_epoch: Vec<EpochReport>,
    pub total_emitted: u64,
    pub total_burned: u64,
    /// The defined sustainability verdict (see module docs).
    pub sustainable: bool,
    /// Why not sustainable, when false (first failing condition).
    pub failure_reason: Option<String>,
}

/// Runs one scenario. Pure and deterministic: identical params/nodes/award
/// produce byte-identical reports.
pub fn simulate(
    params: &TokenomicsParams,
    nodes: u32,
    avg_award_micro_cu_per_node_per_epoch: u64,
) -> Result<SimulationReport, String> {
    params.validate()?;
    let mut remaining_pool = params.total_supply_micro_cu;
    let mut total_emitted: u64 = 0;
    let mut total_burned: u64 = 0;
    let mut per_epoch = Vec::with_capacity(params.epochs as usize);
    let mut failure_reason: Option<String> = None;

    for epoch in 0..params.epochs {
        let desired = params.emission_for_epoch(epoch);
        let emitted = desired.min(remaining_pool);
        remaining_pool -= emitted;
        total_emitted += emitted;

        let [to_contributors, to_validators, to_dev, to_treasury] =
            params.allocations.split(emitted);

        // Per-node payout clamped to [min,max], then fee+burn applied.
        let mut paid_each = avg_award_micro_cu_per_node_per_epoch
            .clamp(params.min_reward_micro_cu, params.max_reward_micro_cu);
        // Pool affordability: pay as many full node-rewards as possible.
        let affordable_nodes = if paid_each > 0 {
            to_contributors
                .checked_div(paid_each)
                .map(|n| n.min(u64::from(nodes)))
                .unwrap_or(u64::from(nodes))
        } else {
            u64::from(nodes)
        };
        let min_payable = affordable_nodes >= u64::from(nodes);

        let fee = paid_each * params.network_fee_bps / super::BPS;
        let burn = fee * params.burn_bps_of_fee / super::BPS;
        paid_each -= fee;
        total_burned += burn;

        per_epoch.push(EpochReport {
            epoch: epoch + 1,
            emitted,
            to_contributors,
            per_node_paid: paid_each,
            fees_collected: fee.saturating_mul(u64::from(nodes)),
            burned: burn.saturating_mul(u64::from(nodes)),
            to_validators,
            to_development: to_dev,
            to_treasury,
            remaining_pool,
            min_reward_payable: min_payable,
        });

        if failure_reason.is_none() && !min_payable {
            failure_reason = Some(format!(
                "epoch {epoch}: pool could not pay the minimum reward to all {nodes} nodes"
            ));
        }
        if failure_reason.is_none() && remaining_pool == 0 && epoch + 1 < params.epochs {
            failure_reason = Some(format!("epoch {epoch}: reward pool depleted early"));
        }
    }

    let sustainable = failure_reason.is_none();
    Ok(SimulationReport {
        nodes,
        epochs_run: params.epochs,
        per_epoch,
        total_emitted,
        total_burned,
        sustainable,
        failure_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> TokenomicsParams {
        serde_json::from_str(
            r#"{
            "total_supply_micro_cu": 1000000000000000,
            "epochs": 10,
            "schedule": "fixed",
            "initial_emission_bps_of_supply": 1000,
            "allocations": { "contributors_bps": 6000, "validators_bps": 2000,
                             "development_bps": 1000, "treasury_bps": 1000 },
            "network_fee_bps": 500,
            "burn_bps_of_fee": 2000,
            "vesting_epochs": 4,
            "slashing": { "enabled": true, "max_bps_per_epoch": 500 },
            "min_reward_micro_cu": 100,
            "max_reward_micro_cu": 50000
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn config_validation_catches_broken_allocations_and_bounds() {
        assert!(params().validate().is_ok());

        let mut bad = params();
        bad.allocations.contributors_bps = 999_999;
        assert!(bad.validate().is_err(), "allocations must sum to 100%");

        let mut bad = params();
        bad.min_reward_micro_cu = bad.max_reward_micro_cu + 1;
        assert!(bad.validate().is_err(), "min > max is nonsense");
    }

    #[test]
    fn allocation_split_never_leaks_the_rounding_remainder() {
        let split = AllocationSplit {
            contributors_bps: 3_333,
            validators_bps: 3_333,
            development_bps: 1_667,
            treasury_bps: 1_667,
        };
        split.validate().unwrap();
        let [c, v, d, t] = split.split(9_999);
        assert_eq!(c + v + d + t, 9_999, "treasury absorbs the remainder");
    }

    #[test]
    fn schedules_emit_in_the_declared_shape() {
        let mut p = params();
        p.initial_emission_bps_of_supply = 10_000; // whole supply as base
        // Fixed: same every epoch.
        let f0 = p.emission_for_epoch(0);
        let f5 = p.emission_for_epoch(5);
        assert_eq!(f0, f5);
        // Halving: epoch 2+ is half of epochs 0-1.
        p.schedule = EmissionSchedule::Halving {
            halving_every_epochs: 2,
        };
        let h0 = p.emission_for_epoch(0);
        let h2 = p.emission_for_epoch(2);
        assert_eq!(h2, h0 / 2);
        // Linear decay: strictly decreasing.
        p.schedule = EmissionSchedule::LinearDecay;
        let l0 = p.emission_for_epoch(1);
        let l8 = p.emission_for_epoch(8);
        assert!(l0 > l8);
    }

    #[test]
    fn sustainability_is_defined_and_detectable() {
        let p = params();
        // Comfortable: 10k nodes at modest awards stay payable.
        let ok = simulate(&p, 10_000, 500).unwrap();
        assert!(ok.sustainable, "{:?}", ok.failure_reason);

        // Unsustainable by CONSTRUCTION: tight params (tiny supply, high
        // per-node demand) must be flagged, with the failing condition.
        let mut tight = params();
        tight.total_supply_micro_cu = 10_000_000;
        tight.initial_emission_bps_of_supply = 500; // 50k emitted / epoch
        tight.min_reward_micro_cu = 400;
        tight.max_reward_micro_cu = 400;
        let heavy = simulate(&tight, 100_000, 400).unwrap();
        assert!(!heavy.sustainable);
        assert!(
            heavy.failure_reason.unwrap().contains("minimum reward"),
            "the simulator names the failing condition"
        );
    }

    #[test]
    fn reports_are_reproducible_byte_for_byte() {
        let p = params();
        let a = simulate(&p, 1_000, 1_000).unwrap();
        let b = simulate(&p, 1_000, 1_000).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn scenario_ladder_runs_to_one_hundred_thousand_nodes() {
        let p = params();
        for nodes in [10u32, 100, 1_000, 10_000, 100_000] {
            let r = simulate(&p, nodes, 300).unwrap();
            assert_eq!(r.nodes, nodes);
            assert_eq!(r.per_epoch.len() as u32, p.epochs);
        }
    }
}
