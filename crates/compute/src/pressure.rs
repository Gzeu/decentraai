//! Autonomous Compute Pressure engine (M15).
//!
//! Pure and deterministic: turns raw local signals into an assist decision
//! WITH hysteresis, so the fabric never flaps between "help me"/"stop" when
//! a metric hovers around one threshold.
//!
//! Hysteresis model (two-threshold state machine):
//! ```text
//! NORMAL ──score ≥ HIGH──▶ ASSIST_REQUESTED ──score ≤ LOW──▶ NORMAL
//!            ▲                        │ score stays > LOW
//!            └─────── (stays) ◀───────┘
//! ```
//! The agent may OBSERVE pressure, but it cannot bypass the planner: the
//! decision here only PROPOSES an assist; routing remains deterministic.

use serde::{Deserialize, Serialize};

/// Raw local signals feeding the engine. All optional so callers can feed
/// only what they can measure honestly; missing signals contribute nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PressureSignals {
    /// Backend-local inference queue depth (requests waiting).
    pub queue_depth: u32,
    /// Current measured response latency in milliseconds (EWMA).
    pub latency_ms: u64,
    /// CPU load percent of THIS node (0–100).
    pub cpu_percent: f32,
    /// RAM usage percent of THIS node (0–100).
    pub ram_percent: f32,
    /// Whether a recent task needed a capability this node lacks locally
    /// (capability_pressure — the strongest signal: local cannot serve it).
    pub missing_local_capability: bool,
}

/// Per-signal thresholds. Defaults encode the agreed posture: assist only
/// under REAL sustained pressure, never on noise.
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
    /// Boot-time sanity: zero/negative high-thresholds would make the engine
    /// fire constantly (or divide by them). Fail closed at validation.
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

/// The engine's verdict for one evaluation tick.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PressureDecision {
    pub should_assist: bool,
    /// Machine-readable factors that fired, in evaluated order — every
    /// decision must be explainable from recorded facts.
    pub reasons: Vec<&'static str>,
    /// Normalized 0..1 composite score (pre-hysteresis).
    pub score: f32,
    pub urgency: Urgency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    Elevated,
    High,
}

/// Hysteresis state machine. Persisted across ticks by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistState {
    Normal,
    AssistRequested,
}

/// One evaluation step. `state` is the previous state; the returned tuple is
/// `(new_state, decision)`.
pub fn evaluate(
    signals: &PressureSignals,
    t: &PressureThresholds,
    state: AssistState,
) -> (AssistState, PressureDecision) {
    let mut reasons: Vec<&'static str> = Vec::new();
    let mut score = 0.0f32;

    if signals.missing_local_capability {
        reasons.push("missing_local_capability");
        score += 0.35;
    }
    if signals.queue_depth >= t.queue_depth_high {
        reasons.push("queue_depth");
        score += 0.20;
    }
    if signals.latency_ms >= t.latency_ms_high {
        reasons.push("latency");
        score += 0.20;
    }
    if signals.cpu_percent >= t.cpu_percent_high {
        reasons.push("cpu");
        score += 0.15;
    }
    if signals.ram_percent >= t.ram_percent_high {
        reasons.push("memory");
        score += 0.10;
    }

    let score = score.min(1.0);
    // Hysteresis: entering ASSIST needs ≥0.5; leaving needs ≤0.25. Between
    // the two the PREVIOUS state holds — no flapping on a noisy signal.
    // Entry 0.35 = queue+cpu or a capability gap alone; exit 0.20 = genuinely
    // quiet. Calibrated against REAL signals on a CPU node (queue depth +
    // CPU load = 0.35 under sustained load, no latency signal yet).
    let (should_assist, new_state) = match state {
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

    (
        new_state,
        PressureDecision {
            should_assist,
            reasons,
            score,
            urgency,
        },
    )
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
    fn quiet_node_stays_normal() {
        let (_, d) = evaluate(
            &signals(10.0, 0, 50),
            &PressureThresholds::default(),
            AssistState::Normal,
        );
        assert!(!d.should_assist);
        assert!(d.reasons.is_empty(), "no fabricated reasons");
    }

    #[test]
    fn sustained_pressure_fires_with_reasons() {
        let (_, d) = evaluate(
            &signals(95.0, 5, 9_000),
            &PressureThresholds::default(),
            AssistState::Normal,
        );
        assert!(d.should_assist);
        assert_eq!(d.reasons, vec!["queue_depth", "latency", "cpu"]);
        // 0.55 composite: real pressure but not catastrophic.
        assert_eq!(d.urgency, Urgency::Elevated);
        assert!((d.score - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn hysteresis_no_flapping_in_the_dead_zone() {
        let t = PressureThresholds::default();
        // Enter assist.
        let (s1, d1) = evaluate(&signals(95.0, 5, 9_000), &t, AssistState::Normal);
        assert!(d1.should_assist && s1 == AssistState::AssistRequested);
        // Score drops into the dead band (0.25 < score < entry): the only
        // remaining signal is the capability gap worth 0.35 — STAYS.
        let mut mid = PressureSignals {
            cpu_percent: 60.0,
            queue_depth: 1,
            latency_ms: 100,
            ..Default::default()
        };
        mid.missing_local_capability = true;
        let (s2, d2) = evaluate(&mid, &t, s1);
        assert!(
            d2.should_assist && s2 == AssistState::AssistRequested,
            "mid-zone must hold the assist state (no flapping)"
        );
        // Fully quiet now: releases.
        let (_, d3) = evaluate(&signals(10.0, 0, 10), &t, s2);
        assert!(!d3.should_assist);
    }

    #[test]
    fn missing_capability_is_the_strongest_single_signal() {
        let mut sig = signals(10.0, 0, 10);
        sig.missing_local_capability = true;
        let (_, d) = evaluate(&sig, &t_default(), AssistState::Normal);
        // 0.35 reaches the entry threshold alone: capability gap is actionable.
        assert!(
            d.should_assist,
            "capability gap 0.35 fires at the calibrated entry"
        );
        assert!((d.score - 0.35).abs() < f32::EPSILON);
    }

    fn t_default() -> PressureThresholds {
        PressureThresholds::default()
    }
}
