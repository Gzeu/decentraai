//! Primordial World autonomous loop (v0.7) — the World as a continuously
//! evolving research environment.
//!
//! The vertical slice (PR #89) proved ONE cycle: World snapshot → question
//! → multi-lens consensus → World mission → bounded execution → evidence.
//! This module turns that single cycle into a PERSISTENT LOOP without
//! creating parallel systems:
//!
//! ```text
//! WorldEvent → Observation → Question → Hypothesis → Experiment → Mission
//!   → Execution → Evidence → Verdict → Learning → Next Question
//!   → (new World event via mission record) → next tick
//! ```
//!
//! All types here are PURE (no I/O, deterministic, bounded). The operator
//! (`node-cli`) owns the files and the HTTP calls; this crate owns the
//! decisions. Crash recovery falls out of that split: every boundary state
//! (cursor, graph, activity ledger) serializes to JSON the operator writes
//! atomically (tmp + rename) and reloads first on restart.
//!
//! Rules enforced by construction:
//! - Incremental consumption: [`WorldCursor`] tracks the last processed
//!   tick/event/mission/treasury; [`diff_world`] reports ONLY what changed.
//! - Information-gain gate: a new question is generated ONLY when
//!   [`WorldDelta::should_research`] is true (new events, mission change,
//!   treasury/entity movement, or tick drift ≥ [`TICK_DRIFT_GATE`]).
//! - No fake agents: [`assign_lenses`] maps the REAL entity ids from the
//!   snapshot onto lens slugs deterministically (sorted, round-robin).
//!   Empty World → empty map (lenses run unattributed, never invented).
//! - Refuted ≠ failure: [`economy_note`] + curiosity semantics — refutation
//!   is information gain, penalizes nothing, closes a line only after ≥2
//!   refutations (journal rule, reused, not redefined here).
//! - Cheapest valid execution: [`WorldView::cheapest_service`] exposes the
//!   cheapest World service; selection already breaks ties cheapest-first.
//!   The trace records provider/cost/capability/result for audit.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::learning::HypothesisVerdict;
use crate::lenses::Lens;

/// Tick drift that alone justifies a fresh look (no other signal).
/// Bounded polling without busy-looping: the World ticks on its own
/// schedule; the Mind re-asks at most every N ticks of silence.
pub const TICK_DRIFT_GATE: u64 = 10;

/// Last processed World position. Persisted by the operator (JSON file).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldCursor {
    /// Last consumed `tick` (0 = never — fresh cursor).
    #[serde(default)]
    pub last_tick: u64,
    /// Last consumed `events.len()`.
    #[serde(default)]
    pub last_event_count: usize,
    /// Last consumed `entities.len()`.
    #[serde(default)]
    pub last_entity_count: usize,
    /// Last consumed mission (`mission_task_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_mission: Option<String>,
    /// Last consumed treasury minted.
    #[serde(default)]
    pub last_minted: u64,
    /// Last consumed treasury burned.
    #[serde(default)]
    pub last_burned: u64,
}

impl WorldCursor {
    /// Fresh cursor — nothing consumed yet (first observation always runs).
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.last_tick == 0
            && self.last_event_count == 0
            && self.last_entity_count == 0
            && self.last_mission.is_none()
    }

    /// Advance the cursor to a consumed view (call AFTER the cycle seals
    /// its evidence — or after an information-gain SKIP, so the skip is
    /// itself durable and restart-safe).
    #[must_use]
    pub fn advanced(&self, view: &WorldView) -> Self {
        Self {
            last_tick: view.tick,
            last_event_count: view.event_count,
            last_entity_count: view.entity_count,
            last_mission: view.mission_task_id.clone(),
            last_minted: view.minted,
            last_burned: view.burned,
        }
    }

    /// Serialize for the operator's atomic file write.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload (fail closed on garbage — crash recovery never invents).
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

/// Cheapest World service offer (marketplace/compute discovery projection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldService {
    /// Location offering it (`research-lab-main`, …).
    pub location_id: String,
    /// Capability (`research`, `coding`, `inference`, …).
    pub capability: String,
    /// Price in world credits.
    pub price: u64,
}

/// Typed view over a raw `GET /v1/world` snapshot. No invented facts:
/// missing arrays read as empty, missing treasury as 0, missing mission
/// as None. `tick` is REQUIRED (fail closed without it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldView {
    /// World tick.
    pub tick: u64,
    /// `entities.len()` (fallback: `agents.len()`).
    pub entity_count: usize,
    /// `events.len()`.
    pub event_count: usize,
    /// `mission_task_id` (or nested `mission.task_id`).
    pub mission_task_id: Option<String>,
    /// Treasury minted (flat or `economy.treasury.minted`).
    pub minted: u64,
    /// Treasury burned (flat or `economy.treasury.burned`).
    pub burned: u64,
    /// Sorted real entity ids (empty when the World is empty — never faked).
    pub entity_ids: Vec<String>,
    /// Cheapest service in `locations[].services` (None when absent).
    pub cheapest_service: Option<WorldService>,
    /// Locations holding more entities than their capacity (crowding
    /// pressure for M15; 0 when locations/entities are absent).
    #[serde(default)]
    pub crowded_locations: usize,
}

/// Parse a raw World snapshot into a typed view (pure, deterministic).
pub fn parse_world_view(
    world: &serde_json::Value,
) -> Result<WorldView, crate::error::ProposalError> {
    let err = |m: &str| crate::error::ProposalError::Parse(m.to_string());
    let tick = world
        .get("tick")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| err("world snapshot lacks tick"))?;
    // Entities: prefer `entities[].id`, fall back to legacy `agents[].agent_id`.
    let mut entity_ids: Vec<String> = world
        .get("entities")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if entity_ids.is_empty() {
        entity_ids = world
            .get("agents")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        e.get("agent_id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    entity_ids.sort();
    entity_ids.dedup();
    let entity_count = entity_ids.len();
    let event_count = world
        .get("events")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let mission_task_id = world
        .get("mission_task_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            world
                .get("mission")
                .and_then(|m| m.get("task_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    let treasury = world.get("economy").and_then(|e| e.get("treasury"));
    let minted = world
        .get("treasury_minted")
        .or_else(|| treasury.and_then(|t| t.get("minted")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let burned = world
        .get("treasury_burned")
        .or_else(|| treasury.and_then(|t| t.get("burned")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    // Cheapest service across locations (deterministic: price, capability, location).
    let mut best: Option<WorldService> = None;
    if let Some(locs) = world.get("locations").and_then(|v| v.as_array()) {
        for loc in locs {
            let lid = loc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if lid.is_empty() {
                continue;
            }
            if let Some(svcs) = loc.get("services").and_then(|v| v.as_object()) {
                for (cap, price_v) in svcs {
                    let price = price_v.as_u64().unwrap_or(u64::MAX);
                    let cand = WorldService {
                        location_id: lid.clone(),
                        capability: cap.clone(),
                        price,
                    };
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            (
                                cand.price,
                                cand.capability.clone(),
                                cand.location_id.clone(),
                            ) < (b.price, b.capability.clone(), b.location_id.clone())
                        }
                    };
                    if better {
                        best = Some(cand);
                    }
                }
            }
        }
    }
    // Crowding: locations holding more entities than their capacity.
    // Counts use the raw `entities[].location_id` values (unknown or
    // missing locations are ignored, never guessed); capacity defaults
    // to 50 exactly like the World's own default.
    let mut per_location: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    if let Some(ents) = world.get("entities").and_then(|v| v.as_array()) {
        for e in ents {
            if let Some(lid) = e.get("location_id").and_then(|v| v.as_str()) {
                *per_location.entry(lid.to_string()).or_default() += 1;
            }
        }
    }
    let mut crowded_locations = 0usize;
    if let Some(locs) = world.get("locations").and_then(|v| v.as_array()) {
        for loc in locs {
            let lid = loc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let cap = loc
                .get("capacity")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(50) as usize;
            if !lid.is_empty() && per_location.get(lid).copied().unwrap_or(0) > cap {
                crowded_locations += 1;
            }
        }
    }
    Ok(WorldView {
        tick,
        entity_count,
        event_count,
        mission_task_id,
        minted,
        burned,
        entity_ids,
        cheapest_service: best,
        crowded_locations,
    })
}

/// Incremental delta between cursor and view + the information-gain gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldDelta {
    /// `view.tick − cursor.last_tick` (saturating).
    pub tick_advance: u64,
    /// `view.events − cursor.events` (signed: World event log is bounded).
    pub new_events: i64,
    /// `view.entities − cursor.entities` (signed).
    pub entity_delta: i64,
    /// Mission pointer changed.
    pub mission_changed: bool,
    /// Treasury minted delta (signed).
    pub minted_delta: i64,
    /// Treasury burned delta (signed).
    pub burned_delta: i64,
    /// Gate: generate a new question ONLY when true.
    pub should_research: bool,
    /// Human-readable reason (deterministic, for logs/evidence).
    pub reason: String,
}

/// Diff a cursor against a fresh view (pure, deterministic).
#[must_use]
pub fn diff_world(cursor: &WorldCursor, view: &WorldView) -> WorldDelta {
    let tick_advance = view.tick.saturating_sub(cursor.last_tick);
    let new_events = view.event_count as i64 - cursor.last_event_count as i64;
    let entity_delta = view.entity_count as i64 - cursor.last_entity_count as i64;
    let mission_changed = view.mission_task_id != cursor.last_mission;
    let minted_delta = view.minted as i64 - cursor.last_minted as i64;
    let burned_delta = view.burned as i64 - cursor.last_burned as i64;
    let mut triggers: Vec<String> = Vec::new();
    if cursor.is_fresh() {
        triggers.push("first-observation".to_string());
    }
    if new_events > 0 {
        triggers.push(format!("new-events:{new_events}"));
    }
    if mission_changed {
        triggers.push("mission-changed".to_string());
    }
    if minted_delta != 0 {
        triggers.push(format!("minted:{minted_delta:+}"));
    }
    if burned_delta != 0 {
        triggers.push(format!("burned:{burned_delta:+}"));
    }
    if entity_delta != 0 {
        triggers.push(format!("entities:{entity_delta:+}"));
    }
    if tick_advance >= TICK_DRIFT_GATE {
        triggers.push(format!("tick-drift:+{tick_advance}"));
    }
    let should_research = !triggers.is_empty();
    let reason = if triggers.is_empty() {
        format!("no-gain: tick={} events steady", view.tick)
    } else {
        format!("gain: {}", triggers.join(","))
    };
    WorldDelta {
        tick_advance,
        new_events,
        entity_delta,
        mission_changed,
        minted_delta,
        burned_delta,
        should_research,
        reason,
    }
}

/// Persistent research lifecycle of a World agent.
/// Survives restart via [`ActivityLedger`] (operator file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResearchActivity {
    /// Asking / scouting the World.
    #[default]
    Exploring,
    /// An experiment/mission is executing.
    Working,
    /// Supported work settled into the economy (trust/reputation up).
    Trading,
    /// Refuted or idle — resting, NOT punished.
    Resting,
}

impl ResearchActivity {
    /// Stable slug for logs and traces.
    #[must_use]
    pub fn slug(&self) -> &'static str {
        match self {
            ResearchActivity::Exploring => "exploring",
            ResearchActivity::Working => "working",
            ResearchActivity::Trading => "trading",
            ResearchActivity::Resting => "resting",
        }
    }

    /// Lifecycle transition on a sealed verdict. Refuted maps to Resting
    /// (information gain, no penalty); Supported maps to Trading (economy
    /// feedback); Inconclusive returns to Exploring (try different).
    #[must_use]
    pub fn on_verdict(&self, verdict: HypothesisVerdict) -> Self {
        match verdict {
            HypothesisVerdict::Supported => ResearchActivity::Trading,
            HypothesisVerdict::Refuted => ResearchActivity::Resting,
            HypothesisVerdict::Inconclusive => ResearchActivity::Exploring,
        }
    }

    /// Next-tick rotation: terminal states return to Exploring so the
    /// research lifecycle reflects actual progress across ticks.
    #[must_use]
    pub fn next_tick(&self) -> Self {
        match self {
            ResearchActivity::Trading | ResearchActivity::Resting => ResearchActivity::Exploring,
            ResearchActivity::Exploring => ResearchActivity::Working,
            ResearchActivity::Working => ResearchActivity::Working,
        }
    }
}

/// One agent's persisted activity record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRecord {
    /// World entity id (REAL — never invented).
    pub entity_id: String,
    /// Lifecycle state.
    pub state: ResearchActivity,
    /// Free-text status (bounded by the operator, ≤128 chars).
    pub activity: String,
    /// World tick when last updated.
    pub updated_tick: u64,
}

/// Persistent activity ledger: entity_id → record. Deterministic order.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityLedger {
    /// Records keyed by entity id.
    pub records: BTreeMap<String, ActivityRecord>,
}

impl ActivityLedger {
    /// Empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert one record (activity text truncated to 128 chars).
    pub fn set(&mut self, entity_id: &str, state: ResearchActivity, activity: &str, tick: u64) {
        let text: String = activity.chars().take(128).collect();
        self.records.insert(
            entity_id.to_string(),
            ActivityRecord {
                entity_id: entity_id.to_string(),
                state,
                activity: text,
                updated_tick: tick,
            },
        );
    }

    /// Read one record.
    #[must_use]
    pub fn get(&self, entity_id: &str) -> Option<&ActivityRecord> {
        self.records.get(entity_id)
    }

    /// Serialize for the operator's atomic file write.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload (fail closed on garbage).
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

/// Assign REAL World entity ids to lens slugs. Deterministic: ids are
/// sorted, lenses iterate in [`Lens::ALL`] order round-robin. Fewer
/// entities than lenses → early lenses win, the rest run unattributed.
/// Empty World → empty map (callers must NOT invent ids).
#[must_use]
pub fn assign_lenses(entity_ids: &[String]) -> BTreeMap<String, String> {
    let mut sorted: Vec<String> = entity_ids.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut out = BTreeMap::new();
    if sorted.is_empty() {
        return out;
    }
    for (i, lens) in Lens::ALL.iter().enumerate() {
        if let Some(eid) = sorted.get(i % sorted.len()) {
            out.insert(lens.slug().to_string(), eid.clone());
        }
    }
    out
}

/// Economy feedback note for a sealed verdict (canonical Hub/Economy
/// semantics in words the operator logs and the trace persists):
/// success settles trust upward; refutation is information gain, never
/// an automatic failure; inconclusive settles nothing and keeps curiosity.
#[must_use]
pub fn economy_note(verdict: HypothesisVerdict) -> &'static str {
    match verdict {
        HypothesisVerdict::Supported => {
            "trust+: useful research settles upward (evidence quality, replication, efficiency)"
        }
        HypothesisVerdict::Refuted => {
            "not-failure: refutation is information gain; no penalty, line closes only after x2"
        }
        HypothesisVerdict::Inconclusive => {
            "no-settlement: curiosity stays high; next cycle tries different"
        }
    }
}

/// One sealed node of the research graph:
/// WorldEvent → Observation → Question → Hypothesis → Experiment → Mission
/// → Execution → Evidence → Verdict → Learning → Next Question.
/// Stable ids throughout (`trace_id` derives from tick + observation id;
/// every other id is the canonical id of its own subsystem).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchTrace {
    /// Stable id: `trace:world:{tick}:{observation_id}`.
    pub trace_id: String,
    /// World tick observed.
    pub world_tick: u64,
    /// Observation id (`obs:world:{tick}`).
    pub observation_id: String,
    /// Agent-generated question.
    pub question: String,
    /// Hypothesis under test (`fam:<lens>:…`).
    pub hypothesis_id: String,
    /// Winning proposal id.
    pub proposal_id: String,
    /// World mission task (None when 409-kept or post failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_task_id: Option<String>,
    /// Sealed evidence id.
    pub evidence_id: String,
    /// Inferred verdict.
    pub verdict: HypothesisVerdict,
    /// Learning summary (includes [`economy_note`]).
    pub learning_summary: String,
    /// Next autonomous action (preview string).
    pub next_question: String,
    /// Lens slug → REAL entity id (empty when World had no entities).
    #[serde(default)]
    pub lens_assignments: BTreeMap<String, String>,
    /// Lifecycle state at seal time (`exploring|working|trading|resting`).
    pub activity: String,
    /// Cheapest World provider used/recorded (`location/capability`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Recorded cost (world credits), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<u64>,
}

impl ResearchTrace {
    /// Stable trace id derivation (no randomness, no wall-clock).
    #[must_use]
    pub fn make_trace_id(world_tick: u64, observation_id: &str) -> String {
        format!("trace:world:{world_tick}:{observation_id}")
    }
}

/// Append-only research graph (operator JSON file, insertion order).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchGraph {
    /// Sealed traces, oldest first.
    pub traces: Vec<ResearchTrace>,
}

impl ResearchGraph {
    /// Empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one sealed trace.
    pub fn append(&mut self, trace: ResearchTrace) {
        self.traces.push(trace);
    }

    /// Latest sealed trace, if any (drives the next question preview).
    #[must_use]
    pub fn last(&self) -> Option<&ResearchTrace> {
        self.traces.last()
    }

    /// Serialize for the operator's atomic file write.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload (fail closed on garbage).
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn world_a() -> serde_json::Value {
        json!({
            "tick": 100,
            "entities": [{"id": "agent-b"}, {"id": "agent-a"}],
            "events": [{"kind": "x", "tick": 100}],
            "mission_task_id": null,
            "treasury_minted": 10,
            "treasury_burned": 1,
            "locations": [
                {"id": "research-lab-main", "services": {"research": 20, "inference": 5}},
                {"id": "forge-workshop", "services": {"coding": 25}}
            ]
        })
    }

    #[test]
    fn parse_is_typed_and_sorted() {
        let v = parse_world_view(&world_a()).unwrap();
        assert_eq!(v.tick, 100);
        assert_eq!(v.entity_ids, vec!["agent-a", "agent-b"]);
        assert_eq!(v.event_count, 1);
        assert!(v.mission_task_id.is_none());
        let svc = v.cheapest_service.unwrap();
        assert_eq!(svc.capability, "inference");
        assert_eq!(svc.price, 5);
    }

    #[test]
    fn missing_tick_fails_closed() {
        assert!(parse_world_view(&json!({"entities": []})).is_err());
    }

    #[test]
    fn fresh_cursor_always_researches() {
        let c = WorldCursor::default();
        assert!(c.is_fresh());
        let v = parse_world_view(&world_a()).unwrap();
        let d = diff_world(&c, &v);
        assert!(d.should_research);
        assert!(d.reason.contains("first-observation"));
    }

    #[test]
    fn steady_state_skips() {
        let v = parse_world_view(&world_a()).unwrap();
        let c = WorldCursor::default().advanced(&v);
        let d = diff_world(&c, &v);
        assert!(!d.should_research);
        assert!(d.reason.contains("no-gain"));
    }

    #[test]
    fn new_events_or_mission_trigger() {
        let v = parse_world_view(&world_a()).unwrap();
        let mut c = WorldCursor::default().advanced(&v);
        c.last_event_count = 0;
        let d = diff_world(&c, &v);
        assert!(d.should_research);
        assert!(d.reason.contains("new-events"));
        let mut v2 = v.clone();
        v2.mission_task_id = Some("task-1".to_string());
        let d2 = diff_world(&c, &v2);
        assert!(d2.mission_changed);
        assert!(d2.should_research);
    }

    #[test]
    fn tick_drift_triggers_at_gate() {
        let v = parse_world_view(&world_a()).unwrap();
        let c = WorldCursor::default().advanced(&v);
        let mut v2 = v.clone();
        v2.tick += TICK_DRIFT_GATE;
        let d = diff_world(&c, &v2);
        assert!(d.should_research);
        assert!(d.reason.contains("tick-drift"));
        v2.tick -= 1;
        let d2 = diff_world(&c, &v2);
        assert!(!d2.should_research);
    }

    #[test]
    fn lens_assignment_reuses_real_ids() {
        let m = assign_lenses(&["z".to_string(), "a".to_string(), "m".to_string()]);
        assert_eq!(m.len(), Lens::ALL.len());
        // Sorted round-robin: generative→a, conservative→m, skeptic→z.
        assert_eq!(m["generative"], "a");
        assert_eq!(m["conservative"], "m");
        assert_eq!(m["skeptic"], "z");
        assert!(assign_lenses(&[]).is_empty());
    }

    #[test]
    fn activity_lifecycle_refuted_is_not_failure() {
        assert_eq!(
            ResearchActivity::Working.on_verdict(HypothesisVerdict::Supported),
            ResearchActivity::Trading
        );
        assert_eq!(
            ResearchActivity::Working.on_verdict(HypothesisVerdict::Refuted),
            ResearchActivity::Resting
        );
        assert_eq!(
            ResearchActivity::Working.on_verdict(HypothesisVerdict::Inconclusive),
            ResearchActivity::Exploring
        );
        assert_eq!(
            ResearchActivity::Trading.next_tick(),
            ResearchActivity::Exploring
        );
    }

    #[test]
    fn trace_ids_are_stable() {
        let a = ResearchTrace::make_trace_id(100, "obs:world:100");
        let b = ResearchTrace::make_trace_id(100, "obs:world:100");
        assert_eq!(a, b);
        assert!(a.starts_with("trace:world:100:"));
    }

    #[test]
    fn cursor_graph_ledger_round_trip() {
        let v = parse_world_view(&world_a()).unwrap();
        let c = WorldCursor::default().advanced(&v);
        let back = WorldCursor::from_json(&c.to_json().unwrap()).unwrap();
        assert_eq!(c, back);
        let mut g = ResearchGraph::new();
        assert!(g.last().is_none());
        g.append(ResearchTrace {
            trace_id: ResearchTrace::make_trace_id(100, "obs:world:100"),
            world_tick: 100,
            observation_id: "obs:world:100".to_string(),
            question: "q".to_string(),
            hypothesis_id: "h".to_string(),
            proposal_id: "p".to_string(),
            mission_task_id: None,
            evidence_id: "e".to_string(),
            verdict: HypothesisVerdict::Supported,
            learning_summary: economy_note(HypothesisVerdict::Supported).to_string(),
            next_question: "nq".to_string(),
            lens_assignments: assign_lenses(&v.entity_ids),
            activity: "trading".to_string(),
            provider: Some("research-lab-main/inference".to_string()),
            cost: Some(5),
        });
        let g2 = ResearchGraph::from_json(&g.to_json().unwrap()).unwrap();
        assert_eq!(g, g2);
        let mut l = ActivityLedger::new();
        l.set(
            "agent-a",
            ResearchActivity::Working,
            "probing transfer-health",
            100,
        );
        let l2 = ActivityLedger::from_json(&l.to_json().unwrap()).unwrap();
        assert_eq!(l, l2);
    }
}
