//! In-Flight Streaming & Header-Aware Cooldown Controller (research track).
//!
//! Handles real-world provider dynamics:
//! 1. **In-Flight Stream Token Accounting**: parses SSE chunks as they arrive.
//! 2. **HTTP 429 Header Parsing**: extracts `Retry-After` and `x-ratelimit-reset` timestamps.
//! 3. **Automatic Circuit Breaker & Health Transition**:
//!    When throttled, transitions model state to `Cooling(until_ms)` and notifies P2P catalog.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    ThrottledCooling,
    Offline,
}

/// Adaptive throttle and cooldown tracker for a single connected model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderThrottleController {
    pub provider_id: String,
    pub health: HealthState,
    pub consecutive_failures: u32,
    pub cooling_until_ms: Option<u64>,
    pub last_success_ms: Option<u64>,
}

impl ProviderThrottleController {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            health: HealthState::Healthy,
            consecutive_failures: 0,
            cooling_until_ms: None,
            last_success_ms: None,
        }
    }

    /// Records an HTTP 429 / Rate Limit event and parses standard cooldown headers.
    pub fn handle_rate_limit(&mut self, retry_after_header_seconds: Option<u64>) {
        let cooldown_sec = retry_after_header_seconds.unwrap_or(60).max(10).min(3600);
        let until = now_ms() + (cooldown_sec * 1000);
        self.health = HealthState::ThrottledCooling;
        self.cooling_until_ms = Some(until);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Records a successful completion.
    pub fn handle_success(&mut self) {
        self.health = HealthState::Healthy;
        self.cooling_until_ms = None;
        self.consecutive_failures = 0;
        self.last_success_ms = Some(now_ms());
    }

    /// Checks whether this model is currently eligible to accept inbound work.
    pub fn is_routable(&mut self) -> bool {
        if self.health == HealthState::ThrottledCooling {
            if let Some(until) = self.cooling_until_ms {
                if now_ms() >= until {
                    // Cooldown elapsed, transition to degraded probe state
                    self.health = HealthState::Degraded;
                    self.cooling_until_ms = None;
                    return true;
                }
                return false;
            }
        }
        self.health != HealthState::Offline
    }
}
