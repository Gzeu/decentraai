//! M15 — Pressure Trigger: the World gains initiative.
//!
//! The autonomous loop (v0.7) proved the Mind can research the World when
//! ASKED (`autonomous-cycle --world-url`). This module answers the next
//! question: WHEN should it ask on its own? The answer is a PURE,
//! deterministic pressure detector — AI proposes nothing here, Rust decides:
//!
//! ```text
//! World tick → pressure signals → deterministic score → Fire | Skip
//!   → existing autonomous loop (child process, read-only/bounded)
//!   → mission → evidence → learning → new World event → next tick
//! ```
//!
//! Rules enforced by construction:
//! - FIRST evaluation only establishes the baseline (observe, never fire).
//! - The SAME tick evaluated twice is a Skip (duplicate-tick idempotency).
//! - A mission still open from OUR last trigger is a Skip (no duplicates;
//!   the World's 409-keep is the second net, never the first).
//! - Cooldown in ticks separates two triggers (no overlap, no busy loop).
//! - Pressure accumulates AGAINST THE LAST TRIGGER, not against the last
//!   look: small changes that keep coming eventually cross the threshold,
//!   while a single small change never fires alone.
//! - Refutation bursts count since the last trigger (repeated failure is
//!   pressure); the burst counter resets ONLY on Fire.
//! - The detector NEVER authorizes testnet: it returns Fire/Skip, and the
//!   trigger path spawns the loop WITHOUT `--enable-live-testnet`, always.

use serde::{Deserialize, Serialize};

use crate::journal::ResearchJournal;
use crate::world_loop::WorldView;

/// Pressure thresholds + scoring. Every field bounded and validated at the
/// config edge; defaults fire rarely (combined pressure, never a blip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PressureThresholds {
    /// Ticks of world motion since the last trigger that count as drift.
    pub min_tick_drift: u64,
    /// New events (bounded-log length delta) since the last trigger.
    pub min_new_events: i64,
    /// |entity count delta| since the last trigger.
    pub min_entity_delta: i64,
    /// |minted + burned flow| since the last trigger (world credits).
    pub min_treasury_delta: i64,
    /// New refuted journal entries since the last trigger.
    pub refuted_burst: u32,
    /// Open research lines revisited only after this many ticks past trigger.
    pub revisit_ticks: u64,
    /// Ticks that must pass after a trigger before the next may fire.
    pub cooldown_ticks: u64,
    /// Points needed to fire (each signal below is worth exactly 1).
    pub fire_threshold: u32,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            min_tick_drift: 5,
            min_new_events: 1,
            min_entity_delta: 2,
            min_treasury_delta: 1,
            refuted_burst: 2,
            revisit_ticks: 50,
            cooldown_ticks: 20,
            fire_threshold: 2,
        }
    }
}

/// Durable trigger bookkeeping. Owned by the node (atomic JSON file);
/// reload-first on restart, so recovery decides identically.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PressureState {
    /// Tick of the last evaluation (0 = never — baseline next).
    #[serde(default)]
    pub last_evaluated_tick: u64,
    /// World position AT THE LAST TRIGGER (the accumulation base).
    #[serde(default)]
    pub base_tick: u64,
    #[serde(default)]
    pub base_events: usize,
    #[serde(default)]
    pub base_entities: usize,
    #[serde(default)]
    pub base_minted: u64,
    #[serde(default)]
    pub base_burned: u64,
    /// Mission pointer at the last trigger (duplicate-mission guard).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_mission: Option<String>,
    /// Refuted journal total at the last trigger (burst base).
    #[serde(default)]
    pub refuted_seen: u32,
    /// Lifetime trigger count.
    #[serde(default)]
    pub total_triggers: u64,
}

impl PressureState {
    /// Nothing evaluated yet — the next evaluation only sets the baseline.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.last_evaluated_tick == 0 && self.total_triggers == 0
    }

    /// Serialize for the node's atomic file write.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload (fail closed on garbage — recovery never invents pressure).
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

/// One pressure signal that fired (1 point each, listed for the audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureSignal {
    /// World moved enough ticks since the last trigger.
    TickDrift,
    /// New events accumulated since the last trigger.
    NewEvents,
    /// Mission pointer changed since the last trigger.
    MissionChanged,
    /// Entity population shifted since the last trigger.
    EntityShift,
    /// Treasury flow (minted + burned) since the last trigger.
    TreasuryFlow,
    /// Refuted burst since the last trigger (repeated failure).
    RefutedBurst,
    /// A location holds more entities than its capacity.
    Crowded,
    /// Open research lines gone stale while the world moved on.
    StaleOpen,
}

impl PressureSignal {
    /// Stable slug for reasons and logs.
    #[must_use]
    pub fn slug(&self) -> &'static str {
        match self {
            PressureSignal::TickDrift => "tick-drift",
            PressureSignal::NewEvents => "new-events",
            PressureSignal::MissionChanged => "mission-changed",
            PressureSignal::EntityShift => "entity-shift",
            PressureSignal::TreasuryFlow => "treasury-flow",
            PressureSignal::RefutedBurst => "refuted-burst",
            PressureSignal::Crowded => "crowded",
            PressureSignal::StaleOpen => "stale-open",
        }
    }
}

/// The detector's verdict. Fire carries the contributing signals; Skip
/// carries the single honest reason. Both advance `last_evaluated_tick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PressureDecision {
    /// Start the existing autonomous loop (read-only/bounded, never live).
    Fire {
        /// Points that crossed the threshold.
        points: u32,
        /// Contributing signals, deterministic order.
        signals: Vec<PressureSignal>,
    },
    /// Do nothing this tick.
    Skip {
        /// The one reason (baseline, duplicate, cooldown, open-mission,
        /// or below-threshold with the live score).
        reason: String,
    },
}

/// Refuted journal entries, total (burst base).
#[must_use]
pub fn refuted_total(journal: &ResearchJournal) -> u32 {
    journal
        .family_tallies()
        .values()
        .map(|(_, refuted, _)| *refuted)
        .sum()
}

/// Open research lines: tried, unsupported, not yet dead (refuted < 2).
#[must_use]
pub fn open_families(journal: &ResearchJournal) -> usize {
    journal
        .family_tallies()
        .values()
        .filter(|(supported, refuted, inconclusive)| {
            *supported == 0 && *refuted < 2 && (*inconclusive > 0 || *refuted > 0)
        })
        .count()
}

/// Evaluate pressure (pure, deterministic). The caller persists the
/// returned state atomically on EVERY path — including Skips — so a kill
/// at any boundary resumes identically.
#[must_use]
pub fn evaluate_pressure(
    view: &WorldView,
    journal: &ResearchJournal,
    state: &PressureState,
    cfg: &PressureThresholds,
) -> (PressureDecision, PressureState) {
    let mut next = state.clone();
    next.last_evaluated_tick = view.tick;

    // 1. Duplicate tick: the same world evaluated twice never refires.
    if view.tick == state.last_evaluated_tick && !state.is_fresh() {
        return (
            PressureDecision::Skip {
                reason: format!("duplicate-tick: {}", view.tick),
            },
            next,
        );
    }
    // 2. Fresh state: observe only — the baseline is the first truth.
    if state.is_fresh() {
        next.base_tick = view.tick;
        next.base_events = view.event_count;
        next.base_entities = view.entity_count;
        next.base_minted = view.minted;
        next.base_burned = view.burned;
        next.base_mission = view.mission_task_id.clone();
        next.refuted_seen = refuted_total(journal);
        return (
            PressureDecision::Skip {
                reason: format!("baseline: tick {}", view.tick),
            },
            next,
        );
    }
    // 3. Our mission is still open — or just appeared after our trigger
    // (the child posts asynchronously; the next tick sees it first).
    // Adopt it and watch: never stack a duplicate mission. A mission that
    // appears LONG after our trigger is foreign pressure and still scores.
    if state.total_triggers > 0 && view.mission_task_id.is_some() {
        let since_trigger = view.tick.saturating_sub(state.base_tick);
        if view.mission_task_id == state.base_mission {
            return (
                PressureDecision::Skip {
                    reason: format!(
                        "mission-open: {} still open from last trigger",
                        view.mission_task_id.as_deref().unwrap_or("-")
                    ),
                },
                next,
            );
        }
        if since_trigger <= cfg.cooldown_ticks.max(1) {
            next.base_mission = view.mission_task_id.clone();
            return (
                PressureDecision::Skip {
                    reason: format!(
                        "mission-adopted: {} appeared right after our trigger",
                        view.mission_task_id.as_deref().unwrap_or("-")
                    ),
                },
                next,
            );
        }
    }
    // 4. Cooldown: triggers are separated in ticks, never overlapping.
    if state.total_triggers > 0 && view.tick.saturating_sub(state.base_tick) < cfg.cooldown_ticks {
        return (
            PressureDecision::Skip {
                reason: format!(
                    "cooldown: {} < {} ticks since trigger",
                    view.tick.saturating_sub(state.base_tick),
                    cfg.cooldown_ticks
                ),
            },
            next,
        );
    }
    // 5. Score the accumulation since the last trigger (or baseline).
    let mut signals: Vec<PressureSignal> = Vec::new();
    if view.tick.saturating_sub(state.base_tick) >= cfg.min_tick_drift {
        signals.push(PressureSignal::TickDrift);
    }
    if view.event_count as i64 - state.base_events as i64 >= cfg.min_new_events {
        signals.push(PressureSignal::NewEvents);
    }
    if view.mission_task_id != state.base_mission {
        signals.push(PressureSignal::MissionChanged);
    }
    // SEC-02: unsigned_abs — abs() panics on i64::MIN (unreachable for
    // memory-bounded counts, but the idiom must be panic-free by construction).
    if (view.entity_count as i64 - state.base_entities as i64).unsigned_abs()
        >= cfg.min_entity_delta as u64
    {
        signals.push(PressureSignal::EntityShift);
    }
    let treasury_then = state.base_minted as i64 + state.base_burned as i64;
    let treasury_now = view.minted as i64 + view.burned as i64;
    if (treasury_now - treasury_then).unsigned_abs() >= cfg.min_treasury_delta as u64 {
        signals.push(PressureSignal::TreasuryFlow);
    }
    if refuted_total(journal).saturating_sub(state.refuted_seen) >= cfg.refuted_burst {
        signals.push(PressureSignal::RefutedBurst);
    }
    if view.crowded_locations > 0 {
        signals.push(PressureSignal::Crowded);
    }
    if open_families(journal) > 0 && view.tick.saturating_sub(state.base_tick) >= cfg.revisit_ticks
    {
        signals.push(PressureSignal::StaleOpen);
    }
    let points = signals.len() as u32;
    if points >= cfg.fire_threshold {
        next.base_tick = view.tick;
        next.base_events = view.event_count;
        next.base_entities = view.entity_count;
        next.base_minted = view.minted;
        next.base_burned = view.burned;
        next.base_mission = view.mission_task_id.clone();
        next.refuted_seen = refuted_total(journal);
        next.total_triggers += 1;
        (PressureDecision::Fire { points, signals }, next)
    } else {
        (
            PressureDecision::Skip {
                reason: format!("below-threshold: {points}/{} points", cfg.fire_threshold),
            },
            next,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::HypothesisVerdict;

    fn view(
        tick: u64,
        events: usize,
        entities: usize,
        mission: Option<&str>,
        minted: u64,
        burned: u64,
    ) -> WorldView {
        WorldView {
            tick,
            entity_count: entities,
            event_count: events,
            mission_task_id: mission.map(str::to_string),
            minted,
            burned,
            entity_ids: (0..entities).map(|i| format!("agent-{i}")).collect(),
            cheapest_service: None,
            crowded_locations: 0,
        }
    }

    #[test]
    fn first_evaluation_sets_baseline_without_firing() {
        let (d, next) = evaluate_pressure(
            &view(10, 50, 13, None, 100, 5),
            &ResearchJournal::new(),
            &PressureState::default(),
            &PressureThresholds::default(),
        );
        assert!(matches!(d, PressureDecision::Skip { .. }));
        assert_eq!(next.base_tick, 10);
        assert_eq!(next.total_triggers, 0);
    }

    #[test]
    fn duplicate_tick_never_refires() {
        let (_, s1) = evaluate_pressure(
            &view(10, 1, 2, None, 0, 0),
            &ResearchJournal::new(),
            &PressureState::default(),
            &PressureThresholds::default(),
        );
        let (d, _) = evaluate_pressure(
            &view(10, 99, 99, Some("task-x"), 999, 999),
            &ResearchJournal::new(),
            &s1,
            &PressureThresholds::default(),
        );
        assert!(matches!(d, PressureDecision::Skip { .. }));
        if let PressureDecision::Skip { reason } = d {
            assert!(reason.contains("duplicate-tick"));
        }
    }

    #[test]
    fn open_mission_from_last_trigger_blocks_duplicates() {
        // Baseline, then a firing world, then the same mission again.
        let cfg = PressureThresholds {
            fire_threshold: 1,
            cooldown_ticks: 0,
            ..PressureThresholds::default()
        };
        let (_, s1) = evaluate_pressure(
            &view(10, 0, 2, None, 0, 0),
            &ResearchJournal::new(),
            &PressureState::default(),
            &cfg,
        );
        // Fire by posting our mission (mission change + events).
        let (d2, s2) = evaluate_pressure(
            &view(20, 5, 2, Some("task-ours"), 0, 0),
            &ResearchJournal::new(),
            &s1,
            &cfg,
        );
        assert!(matches!(d2, PressureDecision::Fire { .. }));
        // Same mission still open next tick: skip, even with more events.
        let (d3, _) = evaluate_pressure(
            &view(21, 9, 2, Some("task-ours"), 0, 0),
            &ResearchJournal::new(),
            &s2,
            &cfg,
        );
        assert!(matches!(d3, PressureDecision::Skip { .. }));
        if let PressureDecision::Skip { reason } = d3 {
            assert!(reason.contains("mission-open"));
        }
    }

    #[test]
    fn small_change_alone_does_not_fire_but_accumulates() {
        let cfg = PressureThresholds {
            fire_threshold: 2,
            cooldown_ticks: 0,
            min_tick_drift: 100, // drift out of the picture
            ..PressureThresholds::default()
        };
        let (_, s1) = evaluate_pressure(
            &view(10, 0, 2, None, 0, 0),
            &ResearchJournal::new(),
            &PressureState::default(),
            &cfg,
        );
        // One new event = 1 point < 2: skip.
        let (d2, s2) = evaluate_pressure(
            &view(11, 1, 2, None, 0, 0),
            &ResearchJournal::new(),
            &s1,
            &cfg,
        );
        assert!(matches!(d2, PressureDecision::Skip { .. }));
        // Treasury moves too (still vs the BASELINE): 2 points → fire.
        let (d3, _) = evaluate_pressure(
            &view(12, 1, 2, None, 5, 0),
            &ResearchJournal::new(),
            &s2,
            &cfg,
        );
        assert!(matches!(d3, PressureDecision::Fire { .. }));
    }

    #[test]
    fn refuted_burst_counts_since_last_trigger() {
        let cfg = PressureThresholds {
            fire_threshold: 1,
            cooldown_ticks: 0,
            min_tick_drift: 1_000,
            min_new_events: 1_000,
            min_entity_delta: 1_000,
            min_treasury_delta: 1_000,
            ..PressureThresholds::default()
        };
        let (_, s1) = evaluate_pressure(
            &view(10, 0, 2, None, 0, 0),
            &ResearchJournal::new(),
            &PressureState::default(),
            &cfg,
        );
        let mut journal = ResearchJournal::new();
        journal.record("c1", "fam:x:a", HypothesisVerdict::Refuted, "e1", 1);
        let (d2, _) = evaluate_pressure(&view(11, 0, 2, None, 0, 0), &journal, &s1, &cfg);
        assert!(
            matches!(d2, PressureDecision::Skip { .. }),
            "one refutation is not a burst"
        );
        journal.record("c2", "fam:x:b", HypothesisVerdict::Refuted, "e2", 2);
        let (d3, s3) = evaluate_pressure(&view(12, 0, 2, None, 0, 0), &journal, &s1, &cfg);
        assert!(matches!(d3, PressureDecision::Fire { .. }));
        assert_eq!(s3.refuted_seen, 2);
    }

    #[test]
    fn state_round_trips_for_restart_recovery() {
        let (_, s) = evaluate_pressure(
            &view(10, 3, 4, Some("task-1"), 7, 1),
            &ResearchJournal::new(),
            &PressureState::default(),
            &PressureThresholds::default(),
        );
        let back = PressureState::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(s, back);
        // Reloaded state decides identically to the live one.
        let (d1, _) = evaluate_pressure(
            &view(10, 3, 4, Some("task-1"), 7, 1),
            &ResearchJournal::new(),
            &s,
            &PressureThresholds::default(),
        );
        let (d2, _) = evaluate_pressure(
            &view(10, 3, 4, Some("task-1"), 7, 1),
            &ResearchJournal::new(),
            &back,
            &PressureThresholds::default(),
        );
        assert_eq!(d1, d2);
    }

    #[test]
    fn extreme_magnitudes_never_panic_and_score() {
        // SEC-02 regression: magnitude math is panic-free by construction
        // (unsigned_abs — abs() panics on i64::MIN). The MIN input itself is
        // unreachable (usize counts), so this pins the reachable extreme:
        // near-i64::MAX populations evaluate and fire the shift signal.
        let huge = WorldView {
            tick: 100,
            entity_count: (i64::MAX / 2) as usize,
            event_count: 0,
            mission_task_id: None,
            minted: (i64::MAX / 2) as u64,
            burned: 0,
            entity_ids: vec![],
            cheapest_service: None,
            crowded_locations: 0,
        };
        let cfg = PressureThresholds {
            fire_threshold: 1,
            cooldown_ticks: 0,
            min_tick_drift: u64::MAX,
            min_new_events: i64::MAX,
            ..PressureThresholds::default()
        };
        let (_, base) = evaluate_pressure(
            &huge,
            &ResearchJournal::new(),
            &PressureState::default(),
            &PressureThresholds::default(),
        );
        // Same magnitudes, one entity fewer + treasury moved: entity-shift
        // and treasury-flow must fire without panic.
        let mut moved = huge.clone();
        moved.tick += 1;
        moved.entity_count -= 3;
        moved.minted += 5;
        let (d, _) = evaluate_pressure(&moved, &ResearchJournal::new(), &base, &cfg);
        match d {
            PressureDecision::Fire { signals, .. } => {
                assert!(signals.contains(&PressureSignal::EntityShift));
                assert!(signals.contains(&PressureSignal::TreasuryFlow));
            }
            PressureDecision::Skip { reason } => panic!("must fire at extremes: {reason}"),
        }
    }
}
