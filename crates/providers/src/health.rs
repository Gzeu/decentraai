//! Per-provider and per-model health tracking with circuit-breaker state machine.
//!
//! The circuit breaker prevents retry storms when a provider is degraded or down.
//! It applies at both the provider level (affects all models from that provider)
//! and the model level (fine-grained).
//!
//! State machine: HEALTHY --> DEGRADED (failures >= degraded_threshold) -->
//! OPEN (failures >= open_threshold) --> (after cooldown) HALF_OPEN -->
//! (on success) HEALTHY; on failure from HALF_OPEN, reopens to OPEN.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::{CircuitState, ModelHealth, ProviderHealth};

/// Configuration for the health + circuit-breaker subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HealthConfig {
    /// How many consecutive failures trigger DEGRADED state.
    pub degraded_threshold: u32,
    /// How many consecutive failures trigger OPEN (breaker).
    pub open_threshold: u32,
    /// Cooldown in seconds before moving from OPEN → HALF_OPEN.
    pub open_cooldown_secs: u64,
    /// Max concurrent probes allowed during HALF_OPEN.
    pub half_open_probe_limit: u32,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            degraded_threshold: 3,
            open_threshold: 5,
            open_cooldown_secs: 30,
            half_open_probe_limit: 2,
        }
    }
}

/// Runtime health state for one provider (aggregate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderHealthState {
    pub health: ProviderHealth,
    pub circuit: CircuitState,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    /// Milliseconds of the last health check (provider-level probe).
    pub last_check_at_ms: Option<u64>,
    /// Unix ms at which an OPEN breaker transitions to HALF_OPEN.
    pub half_open_at_ms: Option<u64>,
    /// Number of probes already sent during HALF_OPEN.
    pub half_open_probes_sent: u32,
}

impl ProviderHealthState {
    pub fn new(_config: &HealthConfig) -> Self {
        Self {
            health: ProviderHealth::Unknown,
            circuit: CircuitState::Healthy,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_check_at_ms: None,
            half_open_at_ms: None,
            half_open_probes_sent: 0,
        }
    }

    /// Record a successful operation. Transitions:
    /// - CLOSED → HEALTHY
    /// - HALF_OPEN → HEALTHY (and resets the circuit)
    /// - OPEN → still OPEN (probes must succeed first)
    pub fn record_success(&mut self, now_ms: u64, _config: &HealthConfig) {
        let now = SystemTime::now();
        let _ = now; // used for time comparisons below

        match self.circuit {
            CircuitState::HalfOpen => {
                self.consecutive_successes += 1;
                if self.consecutive_successes >= 1 {
                    // First success in HALF_OPEN → close fully.
                    self.circuit = CircuitState::Healthy;
                    self.health = ProviderHealth::Healthy;
                    self.consecutive_failures = 0;
                    self.half_open_at_ms = None;
                    self.half_open_probes_sent = 0;
                }
            }
            CircuitState::Healthy | CircuitState::Degraded => {
                self.consecutive_successes += 1;
                self.health = ProviderHealth::Healthy;
                self.consecutive_failures = 0;
            }
            CircuitState::Open => {
                // Already OPEN — don't auto-recover, need explicit probe.
                // But we reset counters so next transition starts fresh.
                self.consecutive_successes += 1;
                self.consecutive_failures = 0;
            }
        }
        self.last_check_at_ms = Some(now_ms);
    }

    /// Record a failed operation. Applies the state machine transitions.
    pub fn record_failure(&mut self, now_ms: u64, config: &HealthConfig) {
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;
        self.last_check_at_ms = Some(now_ms);

        match self.circuit {
            CircuitState::Healthy => {
                if self.consecutive_failures >= config.open_threshold {
                    self.circuit = CircuitState::Open;
                    self.health = ProviderHealth::Offline;
                    self.half_open_at_ms = Some(now_ms + config.open_cooldown_secs * 1000);
                    self.half_open_probes_sent = 0;
                } else if self.consecutive_failures >= config.degraded_threshold {
                    self.circuit = CircuitState::Degraded;
                    self.health = ProviderHealth::Degraded;
                }
            }
            CircuitState::Degraded => {
                if self.consecutive_failures >= config.open_threshold {
                    self.circuit = CircuitState::Open;
                    self.health = ProviderHealth::Offline;
                    self.half_open_at_ms = Some(now_ms + config.open_cooldown_secs * 1000);
                    self.half_open_probes_sent = 0;
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in HALF_OPEN → re-open.
                self.circuit = CircuitState::Open;
                self.health = ProviderHealth::Offline;
                self.half_open_at_ms = Some(now_ms + config.open_cooldown_secs * 1000);
                self.half_open_probes_sent = 0;
            }
            CircuitState::Open => {
                // Already open — no state change.
            }
        }
    }

    /// Check if an OPEN breaker should transition to HALF_OPEN.
    /// Returns true if the state has moved to HALF_OPEN.
    pub fn check_open_to_half_open(&mut self, now_ms: u64) -> bool {
        if self.circuit != CircuitState::Open {
            return false;
        }
        if let Some(hoat) = self.half_open_at_ms {
            if now_ms >= hoat {
                self.circuit = CircuitState::HalfOpen;
                self.consecutive_successes = 0;
                self.consecutive_failures = 0;
                self.half_open_probes_sent = 0;
                return true;
            }
        }
        false
    }

    /// Increment the half-open probe counter. Returns an error if the limit
    /// would be exceeded.
    pub fn can_accept_probe(&self, config: &HealthConfig) -> bool {
        if self.circuit != CircuitState::HalfOpen {
            return true; // not half-open, probing handled elsewhere
        }
        self.half_open_probes_sent < config.half_open_probe_limit
    }

    pub fn increment_half_open_probe(&mut self) {
        if self.circuit == CircuitState::HalfOpen {
            self.half_open_probes_sent += 1;
        }
    }

    /// Whether operations may go through at all.
    pub fn allows_requests(&self) -> bool {
        match self.circuit {
            CircuitState::Healthy | CircuitState::Degraded => true,
            CircuitState::HalfOpen => self.can_accept_probe(&HealthConfig::default()),
            CircuitState::Open => false,
        }
    }
}

/// Runtime health state for one connected model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelHealthState {
    pub health: ModelHealth,
    pub circuit: CircuitState,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub last_latency_ms: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_failure_at_ms: Option<u64>,
    pub half_open_at_ms: Option<u64>,
    pub half_open_probes_sent: u32,
}

impl Default for ModelHealthState {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelHealthState {
    pub fn new() -> Self {
        Self {
            health: ModelHealth::Unknown,
            circuit: CircuitState::Healthy,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_latency_ms: None,
            last_success_at_ms: None,
            last_failure_at_ms: None,
            half_open_at_ms: None,
            half_open_probes_sent: 0,
        }
    }

    pub fn record_success(&mut self, now_ms: u64, latency_ms: Option<u64>) {
        self.consecutive_successes += 1;
        self.consecutive_failures = 0;
        self.health = ModelHealth::Healthy;
        if let Some(lat) = latency_ms {
            self.last_latency_ms = Some(lat);
        }
        self.last_success_at_ms = Some(now_ms);
    }

    pub fn record_failure(&mut self, now_ms: u64) {
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;
        self.last_failure_at_ms = Some(now_ms);
        self.health = ModelHealth::Degraded;
        // If too many failures, mark offline.
        if self.consecutive_failures >= 5 {
            self.health = ModelHealth::Offline;
        }
    }

    /// Whether this model's circuit allows requests.
    pub fn allows_requests(&self) -> bool {
        !matches!(self.circuit, CircuitState::Open)
            && !matches!(self.health, ModelHealth::Disabled | ModelHealth::Offline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HealthConfig {
        HealthConfig {
            degraded_threshold: 2,
            open_threshold: 3,
            open_cooldown_secs: 1,
            half_open_probe_limit: 2,
        }
    }

    #[test]
    fn healthy_becomes_degraded_after_threshold() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        assert_eq!(state.circuit, CircuitState::Healthy);
        assert_eq!(state.health, ProviderHealth::Unknown);

        // One failure — still healthy.
        state.record_failure(1000, &config);
        assert_eq!(state.circuit, CircuitState::Healthy);
        assert_eq!(state.consecutive_failures, 1);

        // Two failures — degraded.
        state.record_failure(2000, &config);
        assert_eq!(state.circuit, CircuitState::Degraded);
        assert_eq!(state.health, ProviderHealth::Degraded);
    }

    #[test]
    fn degraded_becomes_open_after_threshold() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        // Fail twice → degraded.
        state.record_failure(1000, &config);
        state.record_failure(2000, &config);
        assert_eq!(state.circuit, CircuitState::Degraded);

        // Third failure → open.
        state.record_failure(3000, &config);
        assert_eq!(state.circuit, CircuitState::Open);
        assert_eq!(state.health, ProviderHealth::Offline);
        assert!(state.half_open_at_ms.is_some());
    }

    #[test]
    fn open_transitions_to_half_open_after_cooldown() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        // Get to open state.
        state.record_failure(1000, &config);
        state.record_failure(2000, &config);
        state.record_failure(3000, &config);
        assert_eq!(state.circuit, CircuitState::Open);
        let open_at = state.half_open_at_ms.unwrap();

        // Before cooldown — still open.
        assert!(!state.check_open_to_half_open(open_at - 500));
        assert_eq!(state.circuit, CircuitState::Open);

        // After cooldown — half_open.
        assert!(state.check_open_to_half_open(open_at + 1500));
        assert_eq!(state.circuit, CircuitState::HalfOpen);
    }

    #[test]
    fn half_open_success_closes_circuit() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        // Get to open, then advance to half_open.
        state.record_failure(1000, &config);
        state.record_failure(2000, &config);
        state.record_failure(3000, &config);
        let open_at = state.half_open_at_ms.unwrap();
        state.check_open_to_half_open(open_at + 1500);
        assert_eq!(state.circuit, CircuitState::HalfOpen);

        // Success → healthy.
        state.record_success(5000, &config);
        assert_eq!(state.circuit, CircuitState::Healthy);
        assert_eq!(state.health, ProviderHealth::Healthy);
    }

    #[test]
    fn half_open_failure_reopens_circuit() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        state.record_failure(1000, &config);
        state.record_failure(2000, &config);
        state.record_failure(3000, &config);
        let open_at = state.half_open_at_ms.unwrap();
        state.check_open_to_half_open(open_at + 1500);
        assert_eq!(state.circuit, CircuitState::HalfOpen);

        // Failure → back to open.
        state.record_failure(5000, &config);
        assert_eq!(state.circuit, CircuitState::Open);
        assert!(state.half_open_at_ms.unwrap() > 5000);
    }

    #[test]
    fn allows_requests_respects_breaker_state() {
        let config = test_config();
        let mut state = ProviderHealthState::new(&config);
        assert!(state.allows_requests());

        // Degraded — still allows.
        state.record_failure(1000, &config);
        state.record_failure(2000, &config);
        assert!(state.allows_requests());

        // Open — blocks.
        state.record_failure(3000, &config);
        assert!(!state.allows_requests());
    }
}
