//! Agent Arena — persistent deterministic world for autonomous agents.
//! Design: reuse Governor/Evidence/Quota/Reputation, no duplicate scheduler.
//! V1 vertical slice: 3 agents in shared grid world, OBSERVE/MOVE/REQUEST_COMPUTE.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Stable action kinds — closed schema, validated server-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Observe,
    Move,
    Scout,
    Negotiate,
    RequestCompute,
    Build,
    Trade,
    Cooperate,
    Compete,
    Defend,
    Rest,
}

impl ActionKind {
    pub fn all() -> &'static [ActionKind] {
        &[
            ActionKind::Observe,
            ActionKind::Move,
            ActionKind::Scout,
            ActionKind::Negotiate,
            ActionKind::RequestCompute,
            ActionKind::Build,
            ActionKind::Trade,
            ActionKind::Cooperate,
            ActionKind::Compete,
            ActionKind::Defend,
            ActionKind::Rest,
        ]
    }
    pub fn cost_quota(&self) -> u64 {
        match self {
            ActionKind::RequestCompute => 5,
            ActionKind::Build => 3,
            ActionKind::Trade => 1,
            ActionKind::Move => 1,
            _ => 0,
        }
    }
    pub fn cooldown_ticks(&self) -> u64 {
        match self {
            ActionKind::RequestCompute => 2,
            ActionKind::Build => 3,
            ActionKind::Move => 1,
            _ => 0,
        }
    }
}

/// One agent inside the arena — wraps external dca_ identity + deterministic state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaAgent {
    pub agent_id: String,
    pub account_id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub resources: u64,
    pub reputation: i32,
    pub last_action_tick: u64,
    pub goal: String,
}

impl ArenaAgent {
    pub fn new(agent_id: String, account_id: String, name: String, x: i32, y: i32) -> Self {
        Self {
            agent_id,
            account_id,
            name,
            x,
            y,
            resources: 10,
            reputation: 0,
            last_action_tick: u64::MAX,
            goal: "explore".to_string(),
        }
    }
    pub fn has_acted(&self) -> bool {
        self.last_action_tick != u64::MAX
    }
}

/// Append-only event — every meaningful action, traceable to evidence when compute involved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaEvent {
    pub tick: u64,
    pub agent_id: String,
    pub action: ActionKind,
    pub from: (i32, i32),
    pub to: Option<(i32, i32)>,
    pub rationale: String,
    pub evidence_id: Option<String>,
    pub success: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Building {
    pub owner: String,
    pub built_tick: u64,
    pub kind: String,
}

/// Deterministic world — single season/match, grid, tick counter, agent map, event log, shared structures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaWorld {
    pub tick: u64,
    pub width: i32,
    pub height: i32,
    pub agents: BTreeMap<String, ArenaAgent>,
    pub events: VecDeque<ArenaEvent>,
    pub max_events: usize,
    #[serde(default)]
    pub buildings: BTreeMap<String, Building>,
    #[serde(default)]
    pub alliances: std::collections::BTreeSet<(String, String)>,
    #[serde(default)]
    pub trades: Vec<String>,
}

impl Default for ArenaWorld {
    fn default() -> Self {
        Self {
            tick: 0,
            width: 20,
            height: 20,
            agents: BTreeMap::new(),
            events: VecDeque::new(),
            max_events: 1000,
            buildings: BTreeMap::new(),
            alliances: std::collections::BTreeSet::new(),
            trades: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArenaError {
    #[error("agent already in arena")]
    AlreadyJoined,
    #[error("arena full (max 32)")]
    Full,
    #[error("unknown agent")]
    UnknownAgent,
    #[error("action not allowed: {0}")]
    ActionNotAllowed(String),
    #[error("out of bounds")]
    OutOfBounds,
    #[error("cooldown: wait {0} ticks")]
    Cooldown(u64),
    #[error("insufficient resources: need {need} have {have}")]
    InsufficientResources { need: u64, have: u64 },
}

impl ArenaWorld {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            ..Default::default()
        }
    }

    pub fn join(&mut self, agent: ArenaAgent) -> Result<(), ArenaError> {
        if self.agents.contains_key(&agent.agent_id) {
            return Err(ArenaError::AlreadyJoined);
        }
        if self.agents.len() >= 32 {
            return Err(ArenaError::Full);
        }
        if agent.x < 0 || agent.x >= self.width || agent.y < 0 || agent.y >= self.height {
            return Err(ArenaError::OutOfBounds);
        }
        self.agents.insert(agent.agent_id.clone(), agent);
        Ok(())
    }

    pub fn leave(&mut self, agent_id: &str) -> Option<ArenaAgent> {
        self.agents.remove(agent_id)
    }

    /// Deterministic tick — validates, applies, emits event. Caller supplies evidence_id if REQUEST_COMPUTE succeeded via Governor.
    pub fn apply(
        &mut self,
        agent_id: &str,
        action: ActionKind,
        target: Option<(i32, i32)>,
        rationale: String,
        evidence_id: Option<String>,
    ) -> Result<ArenaEvent, ArenaError> {
        // Phase 1: immutable checks
        let (from_x, from_y, agent_resources, agent_last) = {
            let a = self.agents.get(agent_id).ok_or(ArenaError::UnknownAgent)?;
            (a.x, a.y, a.resources, a.last_action_tick)
        };
        let need_wait = action.cooldown_ticks();
        let has_acted = agent_last != u64::MAX;
        if need_wait > 0 && has_acted && self.tick < agent_last + need_wait {
            let wait = (agent_last + need_wait) - self.tick;
            return Err(ArenaError::Cooldown(wait));
        }
        let cost = action.cost_quota();
        if agent_resources < cost {
            return Err(ArenaError::InsufficientResources { need: cost, have: agent_resources });
        }
        let from = (from_x, from_y);
        let mut to = None;
        let mut success = true;
        let detail: String;
        // Precompute nearest for actions needing it (immutable)
        let nearest_trade: Option<String> = if matches!(action, ActionKind::Trade) {
            let mut best = 999;
            let mut nearest = None;
            for (oid, other) in self.agents.iter() {
                if oid == agent_id { continue; }
                let d = (other.x - from_x).abs() + (other.y - from_y).abs();
                if d < best && d <= 3 {
                    best = d;
                    nearest = Some(oid.clone());
                }
            }
            nearest
        } else { None };
        let nearest_ally: Option<String> = if matches!(action, ActionKind::Negotiate | ActionKind::Cooperate) {
            let mut best = 999;
            let mut nearest = None;
            for (oid, other) in self.agents.iter() {
                if oid == agent_id { continue; }
                let d = (other.x - from_x).abs() + (other.y - from_y).abs();
                if d < best && d <= 5 {
                    best = d;
                    nearest = Some(oid.clone());
                }
            }
            nearest
        } else { None };
        match action {
            ActionKind::Move => {
                let (tx, ty) = target.ok_or_else(|| ArenaError::ActionNotAllowed("move requires target".into()))?;
                if tx < 0 || tx >= self.width || ty < 0 || ty >= self.height {
                    return Err(ArenaError::OutOfBounds);
                }
                if (tx - from_x).abs() > 1 || (ty - from_y).abs() > 1 {
                    return Err(ArenaError::ActionNotAllowed("move too far (max 1)".into()));
                }
                {
                    let agent = self.agents.get_mut(agent_id).unwrap();
                    agent.x = tx;
                    agent.y = ty;
                    agent.resources = agent.resources.saturating_sub(cost);
                }
                to = Some((tx, ty));
                detail = format!("moved to {},{}", tx, ty);
            }
            ActionKind::Observe | ActionKind::Scout | ActionKind::Rest | ActionKind::Compete | ActionKind::Defend => {
                detail = format!("{:?} at {},{}", action, from_x, from_y);
            }
            ActionKind::RequestCompute => {
                if evidence_id.is_some() {
                    {
                        let agent = self.agents.get_mut(agent_id).unwrap();
                        agent.resources = agent.resources.saturating_add(5);
                        agent.reputation += 1;
                    }
                    detail = "compute verified".to_string();
                } else {
                    success = false;
                    detail = "compute failed/unverified".to_string();
                }
                {
                    let agent = self.agents.get_mut(agent_id).unwrap();
                    agent.resources = agent.resources.saturating_sub(cost);
                }
            }
            ActionKind::Build => {
                let key = format!("{},{}", from_x, from_y);
                if self.buildings.contains_key(&key) {
                    return Err(ArenaError::ActionNotAllowed("already built there".into()));
                }
                {
                    let agent = self.agents.get_mut(agent_id).unwrap();
                    agent.resources = agent.resources.saturating_sub(cost);
                    agent.reputation += 1;
                }
                self.buildings.insert(key.clone(), Building { owner: agent_id.to_string(), built_tick: self.tick, kind: "outpost".to_string() });
                detail = format!("built outpost at {} (total {})", key, self.buildings.len());
            }
            ActionKind::Trade => {
                {
                    let agent = self.agents.get_mut(agent_id).unwrap();
                    agent.resources = agent.resources.saturating_sub(cost);
                }
                if let Some(partner) = nearest_trade.clone() {
                    if let Some(p) = self.agents.get_mut(&partner) {
                        p.resources = p.resources.saturating_add(2);
                    }
                    {
                        let agent = self.agents.get_mut(agent_id).unwrap();
                        agent.resources = agent.resources.saturating_add(1);
                    }
                    self.trades.push(format!("{}->{}:1", agent_id, partner));
                    detail = format!("traded with {} ({} trades)", partner, self.trades.len());
                } else {
                    {
                        let agent = self.agents.get_mut(agent_id).unwrap();
                        agent.resources = agent.resources.saturating_add(2);
                    }
                    self.trades.push(format!("{}:solo", agent_id));
                    detail = format!("traded solo ({} trades)", self.trades.len());
                }
            }
            ActionKind::Negotiate | ActionKind::Cooperate => {
                if let Some(partner) = nearest_ally.clone() {
                    let mut pair = (agent_id.to_string(), partner.clone());
                    if pair.0 > pair.1 { pair = (pair.1, pair.0); }
                    self.alliances.insert(pair.clone());
                    detail = format!("{:?} with {} (alliances {})", action, partner, self.alliances.len());
                } else {
                    detail = format!("{:?} alone at {},{}", action, from_x, from_y);
                }
            }
        }
        {
            let agent = self.agents.get_mut(agent_id).unwrap();
            agent.last_action_tick = self.tick;
        }
        let ev = ArenaEvent {
            tick: self.tick,
            agent_id: agent_id.to_string(),
            action,
            from,
            to,
            rationale: rationale.chars().take(200).collect(),
            evidence_id,
            success,
            detail,
        };
        self.events.push_back(ev.clone());
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
        Ok(ev)
    }

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn agent(&self, id: &str) -> Option<&ArenaAgent> {
        self.agents.get(id)
    }

    pub fn events_since(&self, since_tick: u64, limit: usize) -> Vec<ArenaEvent> {
        self.events
            .iter()
            .filter(|e| e.tick >= since_tick)
            .take(limit)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_two() -> ArenaWorld {
        let mut w = ArenaWorld::new(10, 10);
        w.join(ArenaAgent::new("a1".into(), "acc1".into(), "Alpha".into(), 5, 5)).unwrap();
        w.join(ArenaAgent::new("a2".into(), "acc2".into(), "Beta".into(), 2, 2)).unwrap();
        w
    }

    #[test]
    fn join_and_move_deterministic() {
        let mut w = world_with_two();
        let ev = w.apply("a1", ActionKind::Move, Some((6, 5)), "explore east".into(), None).unwrap();
        assert_eq!(ev.from, (5, 5));
        assert_eq!(ev.to, Some((6, 5)));
        assert_eq!(w.agent("a1").unwrap().x, 6);
        w.advance_tick();
        let ev2 = w.apply("a1", ActionKind::Move, Some((6, 6)), "north".into(), None).unwrap();
        assert_eq!(ev2.tick, 1);
    }

    #[test]
    fn request_compute_rewards_on_evidence() {
        let mut w = world_with_two();
        let r_before = w.agent("a1").unwrap().resources;
        w.apply("a1", ActionKind::RequestCompute, None, "need inference".into(), Some("ev123".into())).unwrap();
        assert!(w.agent("a1").unwrap().resources > r_before.saturating_sub(5));
        assert_eq!(w.agent("a1").unwrap().reputation, 1);
    }

    #[test]
    fn cooldown_enforced() {
        let mut w = world_with_two();
        w.apply("a1", ActionKind::Move, Some((6, 5)), "m".into(), None).unwrap();
        let err = w.apply("a1", ActionKind::Move, Some((6, 6)), "m".into(), None).unwrap_err();
        assert_eq!(err, ArenaError::Cooldown(1));
        w.advance_tick();
        w.apply("a1", ActionKind::Move, Some((6, 6)), "m".into(), None).unwrap();
    }

    #[test]
    fn insufficient_resources() {
        let mut w = ArenaWorld::new(5, 5);
        w.join(ArenaAgent { resources: 0, ..ArenaAgent::new("a1".into(), "acc".into(), "A".into(), 0, 0) }).unwrap();
        let err = w.apply("a1", ActionKind::Build, None, "build".into(), None).unwrap_err();
        assert!(matches!(err, ArenaError::InsufficientResources { .. }));
    }

    #[test]
    fn out_of_bounds_rejected() {
        let mut w = world_with_two();
        let err = w.apply("a1", ActionKind::Move, Some((99, 99)), "oob".into(), None).unwrap_err();
        assert_eq!(err, ArenaError::OutOfBounds);
    }

    #[test]
    fn events_capped_and_replay() {
        let mut w = ArenaWorld::new(10, 10);
        w.max_events = 3;
        w.join(ArenaAgent::new("a1".into(), "acc".into(), "A".into(), 5, 5)).unwrap();
        for _ in 0..5 {
            w.apply("a1", ActionKind::Observe, None, "see".into(), None).unwrap();
            w.advance_tick();
        }
        assert_eq!(w.events.len(), 3);
        let since = w.events_since(3, 10);
        assert!(since.iter().all(|e| e.tick >= 3));
    }
}
