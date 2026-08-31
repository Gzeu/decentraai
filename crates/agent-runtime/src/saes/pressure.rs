//! SAES 0.5 Phase 1 — Pressure Trigger.
//!
//! Turns the "I cannot continue alone" situation into an explicit,
//! deterministic collaboration signal that Phase 2 (Placement Fairness) and
//! Phase 3 (Agent Gateway / BYOA) can consume without redesign.
//!
//! # Relationship to the existing M15 engine
//!
//! `decentraai_compute::pressure` implements the *runtime* autonomous-assist
//! engine that the node daemon drives (`node-cli`, M15). This module is the
//! *SAES decision layer* view of the same concept: a pure, dependency-light
//! state machine that an agent reasons over inside the
//! `observe → decide → act → learn` cycle. It deliberately lives in
//! `agent-runtime` (which cannot depend on `decentraai-compute` without
//! pulling libp2p into a pure-decision crate) and adds what M15 does not own:
//!
//! 1. an EventBus event (`agent.pressure.fired` / `agent.pressure.released`)
//!    carrying a `correlation_id` that links the whole episode end-to-end;
//! 2. a typed [`CollaborationSignal`] — the *contract* handed to Phase 2.
//!
//! The scoring weights and hysteresis bands mirror the M15 engine so the two
//! layers agree on "under pressure" without drifting. There is no second
//! *runtime* path: only this decision + an event + a signal.
//!
//! # Design principles
//!
//! - **Generic**: signals and thresholds are plain data; no hardcoding for
//!   any specific agent (Pylon, OpenClaw, Cline, …).
//! - **Open vocabulary**: the capability a node requests help for is a
//!   free-form `String` (mirrors the hub taxonomy snake_case name); no closed
//!   enums.
//! - **Deterministic**: canonical ordering, no randomness, hysteresis is a
//!   two-threshold state machine so a noisy signal cannot flap the fabric.
//! - **Pure**: [`evaluate_pressure`] performs no I/O and returns a decision;
//!   the runtime applies it and emits the event.

use serde::{Deserialize, Serialize};

/// Raw local signals feeding the trigger. All optional-in-semantics: callers
/// feed only what they can measure honestly; missing signals contribute no
/// score. Kept as plain fields for extensibility (new signals are additive).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PressureSignals {
    /// Backend-local queue depth (requests waiting for this agent).
    pub queue_depth: u32,
    /// Current measured response latency in milliseconds (EWMA).
    pub latency_ms: u64,
    /// CPU load percent of this node (0–100).
    pub cpu_percent: f32,
    /// RAM usage percent of this node (0–100).
    pub ram_percent: f32,
    /// Whether a recent task needed a capability this agent lacks locally —
    /// the strongest signal: local cannot serve it at all.
    pub missing_local_capability: bool,
}

/// Per-signal thresholds. Defaults encode "assist only under REAL sustained
/// pressure, never on noise" (same posture as the M15 engine).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PressureThresholds {
    pub queue_depth_high: u32,
    pub latency_ms_high: u64,
    pub cpu_percent_high: f32,
    pub ram_percent_high: f32,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            queue_depth_high: 2,
            latency_ms_high: 5_000,
            cpu_percent_high: 90.0,
            ram_percent_high: 85.0,
        }
    }
}

impl PressureThresholds {
    /// Boot-time sanity: zero/negative high-thresholds would fire constantly.
    /// Fails closed so a misconfiguration never floods the fabric.
    pub fn validate(&self) -> Result<(), String> {
        if self.queue_depth_high == 0 {
            return Err("queue_depth_high must be > 0".into());
        }
        if self.latency_ms_high == 0 {
            return Err("latency_ms_high must be > 0".into());
        }
        if !(1.0..=100.0).contains(&self.cpu_percent_high) {
            return Err("cpu_percent_high must be within [1,100]".into());
        }
        if !(1.0..=100.0).contains(&self.ram_percent_high) {
            return Err("ram_percent_high must be within [1,100]".into());
        }
        Ok(())
    }
}

/// Urgency of an assist decision, in evaluated order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    Elevated,
    High,
}

/// Hysteresis state machine, persisted across ticks by the runtime so a noisy
/// signal hovers without flapping between "help me" / "stop".
///
/// ```text
/// NORMAL ──score ≥ 0.35──▶ ASSIST_REQUESTED ──score ≤ 0.20──▶ NORMAL
///            ▲                       │ score stays > 0.20
///            └─────── (stays) ◀──────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistState {
    #[default]
    Normal,
    AssistRequested,
}

/// The trigger's verdict for one evaluation tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PressureDecision {
    /// Whether the agent should request external collaboration now.
    pub should_assist: bool,
    /// The new hysteresis state (caller persists it for the next tick).
    pub new_state: AssistState,
    /// Machine-readable factors that fired, in evaluated order — every
    /// decision must be explainable from recorded facts.
    pub reasons: Vec<String>,
    /// Normalized 0..1 composite score (pre-hysteresis).
    pub score: f32,
    pub urgency: Urgency,
    /// Correlation id for the pressure episode (stable while under pressure;
    /// a new one is minted each time the agent enters ASSIST_REQUESTED).
    pub correlation_id: String,
}

/// The typed contract handed to Phase 2 (Placement Fairness). When the
/// decision fires, the runtime turns it into one of these so Placement can
/// route it without re-deriving the "why".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollaborationSignal {
    /// Agent requesting help (free-form id).
    pub agent_id: String,
    /// Capability the agent needs help with (hub taxonomy snake_case name;
    /// free-form, not an enum — future capabilities are additive).
    pub capability: String,
    /// Why collaboration was requested, in evaluated order.
    pub reasons: Vec<String>,
    pub urgency: Urgency,
    /// Correlation id linking pressure → placement → gateway.
    pub correlation_id: String,
    /// Desired CPU cores for the assist workload (0 = no specific request).
    pub cpu_cores: u16,
    /// Desired RAM headroom in MiB (0 = no specific request).
    pub ram_mb: u64,
    /// Maximum lease the requester is willing to work under (seconds).
    pub max_lease_seconds: u64,
}

/// Pure, deterministic evaluation: raw signals + thresholds + previous state
/// → decision. No I/O, no randomness; same inputs always yield the same
/// output.
pub fn evaluate_pressure(
    signals: &PressureSignals,
    thresholds: &PressureThresholds,
    previous_state: AssistState,
    previous_correlation_id: Option<&str>,
) -> PressureDecision {
    let mut reasons: Vec<String> = Vec::new();
    let mut score = 0.0f32;

    if signals.missing_local_capability {
        reasons.push("missing_local_capability".to_string());
        score += 0.35;
    }
    if signals.queue_depth >= thresholds.queue_depth_high {
        reasons.push("queue_depth".to_string());
        score += 0.20;
    }
    if signals.latency_ms >= thresholds.latency_ms_high {
        reasons.push("latency".to_string());
        score += 0.20;
    }
    if signals.cpu_percent >= thresholds.cpu_percent_high {
        reasons.push("cpu".to_string());
        score += 0.15;
    }
    if signals.ram_percent >= thresholds.ram_percent_high {
        reasons.push("memory".to_string());
        score += 0.10;
    }

    let score = score.min(1.0);

    // Hysteresis: entering ASSIST needs ≥0.35; leaving needs ≤0.20. Between
    // the two the previous state holds. Mirrors the M15 engine exactly.
    let (should_assist, new_state) = match previous_state {
        AssistState::Normal => {
            let fire = score >= 0.35;
            (
                fire,
                if fire {
                    AssistState::AssistRequested
                } else {
                    AssistState::Normal
                },
            )
        }
        AssistState::AssistRequested => {
            let still = score > 0.20;
            (
                still,
                if still {
                    AssistState::AssistRequested
                } else {
                    AssistState::Normal
                },
            )
        }
    };

    let urgency = if score >= 0.75 {
        Urgency::High
    } else if score >= 0.5 {
        Urgency::Elevated
    } else {
        Urgency::Low
    };

    let correlation_id = if should_assist {
        // Keep the episode id stable while under pressure; mint a fresh one
        // only when (re-)entering ASSIST_REQUESTED from a released episode.
        match previous_state {
            AssistState::Normal => format!("pressure-{}", uuid_simple()),
            AssistState::AssistRequested => previous_correlation_id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("pressure-{}", uuid_simple())),
        }
    } else {
        previous_correlation_id
            .map(str::to_owned)
            .unwrap_or_default()
    };

    PressureDecision {
        should_assist,
        new_state,
        reasons,
        score,
        urgency,
        correlation_id,
    }
}

/// Small, collision-resistant, dependency-free id (same helper as the other
/// SAES modules, kept std-only so this crate stays dependency-light).
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}-{:04x}", now, now & 0xffff)
}

/// Per-agent pressure episode state persisted across ticks by the runtime.
/// Tracks hysteresis state, the last `should_assist` transition timestamp
/// (for cooldown/debounce), and the current episode correlation id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PressureEpisode {
    pub state: AssistState,
    /// Wall-clock (epoch ms) when the trigger last fired an assist request —
    /// used to enforce a cooldown so we never emit RESOURCE_REQUEST every tick.
    pub last_fired_at_ms: u64,
    /// Correlation id of the current (or most recent) pressure episode.
    pub correlation_id: Option<String>,
}

impl PressureEpisode {
    /// Whether the cooldown has elapsed since the last fire (or never fired).
    /// Pure and deterministic: no wall-clock sources hidden in here.
    pub fn cooldown_elapsed(&self, now_ms: u64, cooldown_ms: u64) -> bool {
        if self.last_fired_at_ms == 0 {
            return true;
        }
        now_ms.saturating_sub(self.last_fired_at_ms) >= cooldown_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(cpu: f32, queue: u32, latency: u64) -> PressureSignals {
        PressureSignals {
            cpu_percent: cpu,
            queue_depth: queue,
            latency_ms: latency,
            ..Default::default()
        }
    }

    #[test]
    fn quiet_node_stays_normal_without_firing() {
        let d = evaluate_pressure(
            &signals(10.0, 0, 50),
            &PressureThresholds::default(),
            AssistState::Normal,
            None,
        );
        assert!(!d.should_assist);
        assert!(d.reasons.is_empty(), "no fabricated reasons");
        assert_eq!(d.new_state, AssistState::Normal);
    }

    #[test]
    fn sustained_pressure_fires_with_reasons_and_correlation_id() {
        let d = evaluate_pressure(
            &signals(95.0, 5, 9_000),
            &PressureThresholds::default(),
            AssistState::Normal,
            None,
        );
        assert!(d.should_assist);
        assert_eq!(
            d.reasons,
            vec![
                "queue_depth".to_string(),
                "latency".to_string(),
                "cpu".to_string()
            ]
        );
        assert_eq!(d.urgency, Urgency::Elevated);
        assert!((d.score - 0.55).abs() < f32::EPSILON);
        assert!(d.correlation_id.starts_with("pressure-"));
    }

    #[test]
    fn missing_capability_is_strongest_single_signal() {
        let mut sig = signals(10.0, 0, 10);
        sig.missing_local_capability = true;
        let d = evaluate_pressure(
            &sig,
            &PressureThresholds::default(),
            AssistState::Normal,
            None,
        );
        assert!(d.should_assist, "capability gap 0.35 fires at entry");
        assert_eq!(d.reasons, vec!["missing_local_capability".to_string()]);
    }

    #[test]
    fn hysteresis_holds_through_dead_zone_and_carries_correlation() {
        let t = PressureThresholds::default();
        let (d1, _) = {
            let d = evaluate_pressure(&signals(95.0, 5, 9_000), &t, AssistState::Normal, None);
            (d.clone(), d)
        };
        // Mid-band: only the capability gap remains (0.35); it STAYS assisting
        // and keeps the SAME correlation id.
        let mid = PressureSignals {
            cpu_percent: 60.0,
            queue_depth: 1,
            latency_ms: 100,
            missing_local_capability: true,
            ..Default::default()
        };
        let d2 = evaluate_pressure(&mid, &t, d1.new_state, Some(&d1.correlation_id));
        assert!(d2.should_assist);
        assert_eq!(d2.new_state, AssistState::AssistRequested);
        assert_eq!(d2.correlation_id, d1.correlation_id, "episode id stable");
        // Quiet: releases.
        let d3 = evaluate_pressure(
            &signals(10.0, 0, 10),
            &t,
            d2.new_state,
            Some(&d2.correlation_id),
        );
        assert!(!d3.should_assist);
        assert_eq!(d3.new_state, AssistState::Normal);
    }

    #[test]
    fn thresholds_validate_rejects_zero() {
        let t = PressureThresholds {
            queue_depth_high: 0,
            ..Default::default()
        };
        assert!(t.validate().is_err());
        let t2 = PressureThresholds {
            cpu_percent_high: 0.0,
            ..Default::default()
        };
        assert!(t2.validate().is_err());
        assert!(PressureThresholds::default().validate().is_ok());
    }

    /// E2E: pressure → collaboration signal → EventBus event → correlation_id.
    ///
    /// Exercises the SAES 0.5 Phase 1 integration end-to-end against a real
    /// `LocalAgentRuntime` + `EventBus`, proving the trigger turns sustained
    /// local pressure into an explicit, correlated collaboration signal.
    #[tokio::test]
    async fn pressure_fires_collaboration_signal_and_event() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use decentraai_event_bus::{EventBus, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus.clone(), obs);

        let agent_id = "agent-pressure-e2e".to_string();
        let thresholds = PressureThresholds::default();
        let signals = PressureSignals {
            cpu_percent: 95.0,
            queue_depth: 5,
            latency_ms: 9_000,
            ..Default::default()
        };

        // First tick: fires, emits event, returns a signal with a correlation id.
        let sig = runtime
            .evaluate_pressure(&agent_id, &signals, &thresholds, 0, "embeddings")
            .await
            .unwrap();
        assert!(sig.is_some(), "sustained pressure must produce a signal");
        let sig = sig.unwrap();
        assert_eq!(sig.capability, "embeddings");
        assert_eq!(sig.agent_id, agent_id);
        assert!(sig.correlation_id.starts_with("pressure-"));
        assert!(!sig.reasons.is_empty());

        // The EventBus must carry a correlated event.
        let events = bus
            .get_events(decentraai_event_bus::EventFilter::default(), 50)
            .await
            .unwrap();
        let fired: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "agent.pressure.fired")
            .collect();
        assert_eq!(fired.len(), 1, "exactly one fired event");
        assert_eq!(
            fired[0].metadata.correlation_id.as_deref(),
            Some(sig.correlation_id.as_str()),
            "event and signal share the same correlation id"
        );

        // Cooldown: with a large cooldown, a second tick must NOT re-fire.
        let sig2 = runtime
            .evaluate_pressure(&agent_id, &signals, &thresholds, 60_000, "embeddings")
            .await
            .unwrap();
        assert!(sig2.is_none(), "cooldown suppresses the second fire");
    }

    /// E2E: pressure release emits a `released` event and returns to Normal.
    #[tokio::test]
    async fn pressure_release_emits_event() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use decentraai_event_bus::{EventBus, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus.clone(), obs);

        let agent_id = "agent-release-e2e".to_string();
        let thresholds = PressureThresholds::default();

        let hot = PressureSignals {
            cpu_percent: 95.0,
            queue_depth: 5,
            latency_ms: 9_000,
            ..Default::default()
        };
        let _ = runtime
            .evaluate_pressure(&agent_id, &hot, &thresholds, 0, "embeddings")
            .await
            .unwrap();

        // Quiet now: hysteresis releases.
        let quiet = PressureSignals::default();
        let sig = runtime
            .evaluate_pressure(&agent_id, &quiet, &thresholds, 0, "embeddings")
            .await
            .unwrap();
        assert!(sig.is_none());

        let events = bus
            .get_events(decentraai_event_bus::EventFilter::default(), 50)
            .await
            .unwrap();
        let released: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "agent.pressure.released")
            .collect();
        assert_eq!(released.len(), 1, "release event emitted on quiet");
    }
}
