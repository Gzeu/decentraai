//! Subscription Capacity Models & Output-Driven Settlement (research track).
//!
//! Real-world subscription types:
//! 1. **`RollingWindow` (Hourly / 5-hour limit)**: Claude Pro/Team, ChatGPT Plus (e.g. 45 messages / 5 hours).
//! 2. **`DailyCreditReset` (Free / Tier 1 daily reset)**: Groq, DeepSeek free/tier 1, Google AI Studio daily limits.
//! 3. **`MonthlyAllotment` (Prepaid / monthly quota)**: OpenRouter balance, GitHub Copilot / Cursor monthly requests.
//! 4. **`AutoThrottledIdleShare` (Zero-config "Share today")**:
//!    The user doesn't know their token count. They simply toggle "I'm not working today, share my capacity".
//!    The node accepts work, generates output, measures exact tokens from provider responses,
//!    and awards CU proportional to the real output. If a 429 / hourly throttle is hit, it auto-cooldowns.

use serde::{Deserialize, Serialize};

/// Real-world provider subscription type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionType {
    /// Rolling time limit (e.g. Claude 45 msgs / 5 hours, ChatGPT 80 msgs / 3 hours).
    RollingHourlyLimit {
        max_requests_per_window: u32,
        window_duration_minutes: u32,
    },
    /// Daily allowance that resets at midnight (e.g. Groq free tier, DeepSeek tier 1 daily).
    DailyReset {
        reset_time_utc: String, // e.g. "00:00"
    },
    /// Fixed monthly / prepaid credit pool.
    MonthlyPrepaid {
        renewal_day_of_month: u8,
    },
    /// Unlimited / Pay-as-you-go or unknown capacity: auto-throttle on provider 429.
    DynamicAutoThrottleOn429,
}

/// Sharing configuration for a model based on user intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum UserSharingIntent {
    /// "I'm on a day off / not coding today. Donate/share my active model.
    /// Whatever output tokens my model generates for others, I receive equivalent durable CU."
    ShareDayOffIdle {
        auto_pause_on_429: bool,
        cooldown_minutes_on_throttle: u32,
    },
    /// Controlled sharing while working: share only surplus capacity.
    ShareSurplusOnly {
        reserve_requests_per_hour: u32,
    },
    /// Free tier monetization: share only 100% free models to earn CU for personal paid models later.
    FreeModelsOnly,
}

impl Default for UserSharingIntent {
    fn default() -> Self {
        Self::ShareDayOffIdle {
            auto_pause_on_429: true,
            cooldown_minutes_on_throttle: 60,
        }
    }
}

/// Dynamic tracker for provider rate limits and output generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelLiveCapacityTracker {
    pub total_prompt_tokens_served: u64,
    pub total_completion_tokens_generated: u64,
    pub total_cu_earned: u64,
    pub total_requests_served: u64,
    pub is_currently_throttled: bool,
    pub throttled_until_ms: Option<u64>,
    pub consecutive_429_count: u32,
}

impl ModelLiveCapacityTracker {
    pub fn record_successful_generation(
        &mut self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cu_earned: u64,
    ) {
        self.total_prompt_tokens_served = self.total_prompt_tokens_served.saturating_add(prompt_tokens);
        self.total_completion_tokens_generated = self.total_completion_tokens_generated.saturating_add(completion_tokens);
        self.total_cu_earned = self.total_cu_earned.saturating_add(cu_earned);
        self.total_requests_served = self.total_requests_served.saturating_add(1);
        self.consecutive_429_count = 0;
        self.is_currently_throttled = false;
        self.throttled_until_ms = None;
    }

    pub fn record_throttle_429(&mut self, current_time_ms: u64, cooldown_minutes: u32) {
        self.consecutive_429_count = self.consecutive_429_count.saturating_add(1);
        self.is_currently_throttled = true;
        self.throttled_until_ms = Some(current_time_ms + (u64::from(cooldown_minutes) * 60_000));
    }

    pub fn is_available_at(&self, current_time_ms: u64) -> bool {
        if !self.is_currently_throttled {
            return true;
        }
        if let Some(until) = self.throttled_until_ms {
            current_time_ms >= until
        } else {
            true
        }
    }
}
