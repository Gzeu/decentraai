//! Smart Sharing Strategies for the DecentraAI Inference Credit Economy.
//!
//! Handles all real-world provider subscription and quota scenarios:
//!
//! 1. **`DrainUntilRenewal` (End-of-Cycle Burst)**:
//!    Connect your API key to drain dying/expiring tokens before the monthly/daily
//!    subscription renewal wipes them out, converting them into permanent DecentraAI CU.
//! 2. **`BalancedDrip` (Sustainable Sharing)**:
//!    Shares a controlled percentage of the quota steadily throughout the cycle
//!    without exhausting tokens needed for personal work.
//! 3. **`SelectiveTierJuggling` (Free-Tier Only / Multi-Model Cascade)**:
//!    Selectively shares only zero-cost/free models from a subscription (e.g. OpenRouter free
//!    tiers) while keeping paid flagship models private, auto-swapping models if one is throttled.
//! 4. **`LocalGpuMonetize` (Zero API Cost)**:
//!    Monetizes idle local GPU compute (Ollama / vLLM) during downtime/night hours into CU.
//! 5. **`SafetyBuffer`**:
//!    Preserves a dedicated personal reserve (e.g. 20,000 tokens) and auto-pauses
//!    sharing if the remaining quota dips below the threshold.

use serde::{Deserialize, Serialize};

/// Mode of sharing applied to a connected provider model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SmartSharingStrategy {
    /// Maximize concurrency and consumption to convert dying tokens before subscription reset.
    DrainUntilRenewal {
        renewal_timestamp_ms: u64,
        max_concurrency_boost: u32,
        minimum_safety_reserve_tokens: u64,
    },
    /// Controlled steady sharing with a percentage cap.
    BalancedDrip {
        share_percentage: u8, // 1..=100
        rate_limit_rpm: u32,
        personal_reserve_tokens: u64,
    },
    /// Only share free/unmetered models from subscription; paid models stay private.
    SelectiveFreeTierOnly {
        allowed_model_patterns: Vec<String>,
        auto_cascade_on_rate_limit: bool,
    },
    /// Share local GPU when system is idle.
    LocalGpuIdleMonetize {
        active_hours_start_utc: u8,
        active_hours_end_utc: u8,
        max_gpu_temp_celsius: u8,
    },
}

impl Default for SmartSharingStrategy {
    fn default() -> Self {
        Self::BalancedDrip {
            share_percentage: 50,
            rate_limit_rpm: 60,
            personal_reserve_tokens: 10_000,
        }
    }
}

impl SmartSharingStrategy {
    /// Evaluates whether sharing should be active given current time and remaining tokens.
    pub fn should_accept_work(
        &self,
        current_time_ms: u64,
        remaining_quota_tokens: u64,
        is_free_tier: bool,
    ) -> (bool, &'static str) {
        match self {
            Self::DrainUntilRenewal {
                renewal_timestamp_ms,
                minimum_safety_reserve_tokens,
                ..
            } => {
                if current_time_ms >= *renewal_timestamp_ms {
                    return (false, "renewal window passed, waiting for quota reset");
                }
                if remaining_quota_tokens <= *minimum_safety_reserve_tokens {
                    return (false, "safety reserve threshold reached");
                }
                (true, "drain mode active: maximum speed")
            }
            Self::BalancedDrip {
                personal_reserve_tokens,
                ..
            } => {
                if remaining_quota_tokens <= *personal_reserve_tokens {
                    return (false, "personal token reserve reached");
                }
                (true, "balanced drip active")
            }
            Self::SelectiveFreeTierOnly { .. } => {
                if !is_free_tier {
                    return (false, "paid model locked in free-tier-only strategy");
                }
                if remaining_quota_tokens == 0 {
                    return (false, "free tier rate-limited");
                }
                (true, "free tier sharing active")
            }
            Self::LocalGpuIdleMonetize { .. } => {
                (true, "local GPU compute ready")
            }
        }
    }

    /// Label for UI badge.
    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::DrainUntilRenewal { .. } => "⚡ DRAIN BURST",
            Self::BalancedDrip { .. } => "⚖ BALANCED DRIP",
            Self::SelectiveFreeTierOnly { .. } => "🆓 FREE-TIER ONLY",
            Self::LocalGpuIdleMonetize { .. } => "🎮 IDLE GPU",
        }
    }
}
