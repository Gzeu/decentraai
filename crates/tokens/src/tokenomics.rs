//! Tokenomics and rewards distribution

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Duration};
use std::collections::HashMap;

use crate::{ContributionRecord, TrustRecord};

/// Total token supply and distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokenomics {
    pub total_supply: u64,              // Total tokens (e.g., 1_000_000_000)
    pub circulating_supply: u64,         // Currently in circulation
    pub distribution: Distribution,
    pub emission: EmissionSchedule,
}

/// Token distribution breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    pub community_rewards: u64,   // 50% - Worker rewards
    pub treasury: u64,            // 20% - Protocol treasury
    pub team: u64,                // 15% - Team (vested)
    pub investors: u64,           // 10% - Investors (vested)
    pub staking_rewards: u64,     // 5% - Staking APY
}

impl Distribution {
    pub fn from_total(total_supply: u64) -> Self {
        Self {
            community_rewards: total_supply * 50 / 100,
            treasury: total_supply * 20 / 100,
            team: total_supply * 15 / 100,
            investors: total_supply * 10 / 100,
            staking_rewards: total_supply * 5 / 100,
        }
    }
}

/// Emission schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub initial_supply: u64,
    pub annual_inflation_rate: f32,  // e.g., 0.05 = 5%
    pub max_supply: u64,
    pub halving_interval_blocks: u64, // Blocks per halving
    pub current_block: u64,
}

impl EmissionSchedule {
    pub fn new(max_supply: u64, inflation_rate: f32) -> Self {
        Self {
            initial_supply: max_supply * 10 / 100, // 10% at launch
            annual_inflation_rate: inflation_rate,
            max_supply,
            halving_interval_blocks: 2_102_400, // ~4 years at 1 block/15s
            current_block: 0,
        }
    }

    pub fn current_emission(&self) -> u64 {
        let halvings = self.current_block / self.halving_interval_blocks;
        let base_emission = self.initial_supply / (2u64.pow(halvings as u32));
        base_emission.min(self.max_supply - self.current_supply())
    }

    pub fn current_supply(&self) -> u64 {
        self.initial_supply + (self.current_emission() * self.current_block)
    }

    pub fn increment_block(&mut self) {
        self.current_block += 1;
    }
}

/// Epoch-based reward distribution
#[derive(Debug, Clone)]
pub struct EpochRewards {
    pub epoch_number: u64,
    pub epoch_start: DateTime<Utc>,
    pub epoch_end: DateTime<Utc>,
    pub total_rewards: u64,
    pub total_contributions: u64,  // Total work done in epoch
    pub contributions: HashMap<String, ContributionRecord>,
    pub distribution_complete: bool,
}

impl EpochRewards {
    pub fn new(epoch_number: u64, duration_hours: u64) -> Self {
        let now = Utc::now();
        Self {
            epoch_number,
            epoch_start: now,
            epoch_end: now + Duration::hours(duration_hours as i64),
            total_rewards: 0,
            total_contributions: 0,
            contributions: HashMap::new(),
            distribution_complete: false,
        }
    }

    pub fn add_contribution(&mut self, worker_id: String, record: ContributionRecord) {
        self.total_contributions += record.tokens_generated as u64;
        self.contributions.insert(worker_id, record);
    }

    /// Calculate rewards per worker based on contribution share
    pub fn calculate_rewards(&self) -> HashMap<String, u64> {
        let mut rewards = HashMap::new();

        if self.total_contributions == 0 {
            return rewards;
        }

        for (worker_id, contribution) in &self.contributions {
            let share = contribution.tokens_generated as f64 / self.total_contributions as f64;
            let reward = (self.total_rewards as f64 * share) as u64;
            rewards.insert(worker_id.clone(), reward);
        }

        rewards
    }

    /// Distribute rewards to workers
    pub fn distribute(&mut self) -> HashMap<String, u64> {
        let rewards = self.calculate_rewards();
        self.distribution_complete = true;
        rewards
    }
}

/// Vesting schedule for team/investors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VestingSchedule {
    pub total_amount: u64,
    pub cliff_months: u64,         // Lock period (e.g., 12 months)
    pub vesting_months: u64,       // Total vesting (e.g., 48 months)
    pub start_date: DateTime<Utc>,
    pub released: u64,
}

impl VestingSchedule {
    pub fn new(
        total_amount: u64,
        cliff_months: u64,
        vesting_months: u64,
    ) -> Self {
        Self {
            total_amount,
            cliff_months,
            vesting_months,
            start_date: Utc::now(),
            released: 0,
        }
    }

    pub fn vested_amount(&self) -> u64 {
        let now = Utc::now();
        let elapsed_months = ((now - self.start_date).num_days() / 30) as u64;

        if elapsed_months < self.cliff_months {
            return 0; // Cliff period
        }

        if elapsed_months >= self.vesting_months {
            return self.total_amount; // Fully vested
        }

        // Linear vesting after cliff
        (self.total_amount * elapsed_months / self.vesting_months) - self.released
    }

    pub fn claim(&mut self) -> u64 {
        let vested = self.vested_amount();
        self.released += vested;
        vested
    }

    pub fn remaining(&self) -> u64 {
        self.total_amount - self.released
    }
}

/// Rewards calculator with quality multipliers
pub struct RewardsCalculator {
    base_reward_per_token: u64,  // Base reward per token generated
    quality_multiplier: f32,      // Multiplier for high-quality work
    trust_multiplier: f32,        // Multiplier for trusted workers
    reliability_multiplier: f32,  // Multiplier for uptime
}

impl RewardsCalculator {
    pub fn new(base_reward_per_token: u64) -> Self {
        Self {
            base_reward_per_token,
            quality_multiplier: 1.0,
            trust_multiplier: 1.0,
            reliability_multiplier: 1.0,
        }
    }

    pub fn calculate_reward(
        &self,
        tokens_generated: u64,
        quality_score: f32,      // 0.0 - 1.0
        trust_score: f32,         // 0.0 - 1.0
        uptime_percent: f32,      // 0.0 - 100.0
    ) -> u64 {
        let base = tokens_generated * self.base_reward_per_token;

        // Quality multiplier (0.5x - 2.0x)
        self.quality_multiplier = 0.5 + (quality_score * 1.5);

        // Trust multiplier (0.8x - 1.5x)
        self.trust_multiplier = 0.8 + (trust_score * 0.7);

        // Reliability multiplier (0.7x - 1.3x)
        self.reliability_multiplier = 0.7 + ((uptime_percent / 100.0) * 0.6);

        let total_multiplier = self.quality_multiplier
            * self.trust_multiplier
            * self.reliability_multiplier;

        (base as f32 * total_multiplier) as u64
    }

    /// Calculate rewards with example values
    pub fn example_calculation() -> Vec<(String, u64, f32, f32, f32, u64)> {
        let calc = Self::new(10); // 10 tokens per token generated

        vec![
            ("worker-1".to_string(), 10000, 0.95, 0.98, 99.0, 0),
            ("worker-2".to_string(), 8000, 0.85, 0.90, 95.0, 0),
            ("worker-3".to_string(), 12000, 0.92, 0.95, 98.0, 0),
        ]
        .into_iter()
        .map(|(id, tokens, quality, trust, uptime, _)| {
            let reward = calc.calculate_reward(tokens, quality, trust, uptime);
            (id, tokens, quality, trust, uptime, reward)
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distribution() {
        let dist = Distribution::from_total(1_000_000_000);
        assert_eq!(dist.community_rewards, 500_000_000);
        assert_eq!(dist.treasury, 200_000_000);
        assert_eq!(dist.team, 150_000_000);
        assert_eq!(dist.investors, 100_000_000);
        assert_eq!(dist.staking_rewards, 50_000_000);
    }

    #[test]
    fn test_vesting() {
        let mut vesting = VestingSchedule::new(1_000_000, 12, 48);
        assert_eq!(vesting.vested_amount(), 0); // In cliff

        // Simulate 24 months later
        vesting.start_date = Utc::now() - Duration::days(730);
        let vested = vesting.vested_amount();
        assert!(vested > 0 && vested < 1_000_000);
    }

    #[test]
    fn test_rewards_calculation() {
        let calc = RewardsCalculator::new(10);
        let reward = calc.calculate_reward(1000, 0.9, 0.95, 98.0);
        assert!(reward > 10000); // Base is 10000, multipliers should increase it
    }
}
