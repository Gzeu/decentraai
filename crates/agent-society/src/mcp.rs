//! MCP integration for Agent Society Rules
//!
//! Exposes social state, reputation, and decision hints via MCP tools.

use crate::{
    AgentId, DecisionContext, SocietyRules, SocietyState, TaskId, reputation::ReputationStore,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// MCP tool definitions for society
pub fn society_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "society_state".to_string(),
            description: "Agent Society live state: relationships, trust scores, reputation, contributions, outcomes. Read-only projection.".to_string(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "society_trust".to_string(),
            description: "Get trust score between two agents (observer -> subject). Returns -1.0 to 1.0.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "observer": { "type": "string", "description": "Observing agent" },
                    "subject": { "type": "string", "description": "Subject agent" }
                },
                "required": ["observer", "subject"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_reputation".to_string(),
            description: "Get reputation for an agent (optionally scoped to capability).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "capability": { "type": "string", "description": "Optional capability scope" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_relationships".to_string(),
            description: "Get social relationships for an agent (as observer or subject).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "as_observer": { "type": "boolean", "description": "If true, relationships where agent is observer; if false, where agent is subject" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_contributions".to_string(),
            description: "Get contribution records for a task.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task to query" }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_outcomes".to_string(),
            description: "Get task outcomes for an agent.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "limit": { "type": "integer", "description": "Max outcomes to return" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_decision_hints".to_string(),
            description: "Get decision hints for the current agent based on society rules. Requires agent context.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent requesting hints" },
                    "hub_state": { "type": "string", "description": "JSON snapshot of hub state" },
                    "resources": { "type": "string", "description": "JSON snapshot of resource state" }
                },
                "required": ["agent_id", "hub_state", "resources"],
                "additionalProperties": false
            }),
        },
    ]
}

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Extract society_state request
pub fn society_state_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return false;
    }
    msg.get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some("society_state")
}

/// Extract society_trust request
pub fn society_trust_request(raw: &str) -> Option<(String, String)> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_trust"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let observer = args.get("observer").and_then(|v| v.as_str())?.to_string();
    let subject = args.get("subject").and_then(|v| v.as_str())?.to_string();
    Some((observer, subject))
}

/// Extract society_reputation request
pub fn society_reputation_request(raw: &str) -> Option<(String, Option<String>)> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_reputation"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let capability = args
        .get("capability")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((agent_id, capability))
}

/// Extract society_relationships request
pub fn society_relationships_request(raw: &str) -> Option<(String, bool)> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_relationships"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let as_observer = args
        .get("as_observer")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    Some((agent_id, as_observer))
}

/// Extract society_contributions request
pub fn society_contributions_request(raw: &str) -> Option<String> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_contributions"
    {
        return None;
    }
    msg.get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("task_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract society_outcomes request
pub fn society_outcomes_request(raw: &str) -> Option<(String, usize)> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_outcomes"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    Some((agent_id, limit))
}

/// Extract society_decision_hints request
pub fn society_decision_hints_request(
    raw: &str,
) -> Option<(String, serde_json::Value, serde_json::Value)> {
    let msg: serde_json::Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    if msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?
        != "society_decision_hints"
    {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let hub_state = args.get("hub_state").cloned().unwrap_or(json!({}));
    let resources = args.get("resources").cloned().unwrap_or(json!({}));
    Some((agent_id, hub_state, resources))
}

/// Build society state response for MCP
pub fn build_society_state_response(state: &SocietyState, agent_id: &AgentId) -> serde_json::Value {
    // Trust scores for this agent toward others
    let mut trust_scores = std::collections::BTreeMap::new();
    if let Some(subjects) = state.relationships.get(agent_id) {
        for subject in subjects.keys() {
            let score = state.trust_score(agent_id, subject);
            trust_scores.insert(subject.clone(), score);
        }
    }

    // Also get trust from others toward this agent
    for (observer, subjects) in &state.relationships {
        if subjects.contains_key(agent_id) {
            let score = state.trust_score(observer, agent_id);
            trust_scores.insert(format!("{}_toward_me", observer), score);
        }
    }

    json!({
        "tick": state.tick,
        "trust_scores": trust_scores,
        "my_relationships": state.get_all_for_agent(agent_id).len(),
        "relationships_about_me": state.get_about_agent(agent_id).len(),
        "my_contributions": state.contributions.values().flatten().filter(|c| c.agent_id == *agent_id).count(),
        "my_outcomes": state.outcomes.values().filter(|o| o.team_members.contains(agent_id) || o.issuer == *agent_id).count(),
        "reputation_events": state.reputation.get(agent_id).map(|v| v.len()).unwrap_or(0),
    })
}

/// Build trust score response
pub fn build_trust_response(
    state: &SocietyState,
    observer: &AgentId,
    subject: &AgentId,
) -> serde_json::Value {
    let score = state.trust_score(observer, subject);
    let rels = state.get_relationships(observer, subject);
    json!({
        "observer": observer,
        "subject": subject,
        "trust_score": score,
        "relationship_count": rels.len(),
        "relationships": rels.iter().map(|r| json!({
            "kind": r.kind,
            "tick": r.tick,
            "task_id": r.task_id,
            "detail": r.detail,
            "strength": r.strength,
        })).collect::<Vec<_>>()
    })
}

/// Build reputation response
pub fn build_reputation_response(
    store: &ReputationStore,
    agent_id: &AgentId,
    capability: Option<&str>,
) -> serde_json::Value {
    if let Some(rep) = store.get(agent_id, capability) {
        json!({
            "agent_id": agent_id,
            "capability": capability,
            "overall": rep.overall,
            "signals": rep.signals.iter().map(|(s, score)| json!({
                "signal": s,
                "value": score.value,
                "samples": score.samples,
                "meaningful": score.is_meaningful(),
            })).collect::<Vec<_>>(),
            "sample_count": rep.sample_count,
            "updated_at": rep.updated_at,
        })
    } else {
        json!({ "agent_id": agent_id, "capability": capability, "overall": 0.0, "signals": [], "sample_count": 0 })
    }
}

/// Build relationships response
pub fn build_relationships_response(
    state: &SocietyState,
    agent_id: &AgentId,
    as_observer: bool,
) -> serde_json::Value {
    let rels = if as_observer {
        state.get_all_for_agent(agent_id)
    } else {
        state.get_about_agent(agent_id)
    };

    json!({
        "agent_id": agent_id,
        "as_observer": as_observer,
        "relationships": rels.iter().map(|r| json!({
            "observer": r.observer,
            "subject": r.subject,
            "kind": r.kind,
            "tick": r.tick,
            "task_id": r.task_id,
            "detail": r.detail,
            "strength": r.strength,
        })).collect::<Vec<_>>()
    })
}

/// Build contributions response
pub fn build_contributions_response(state: &SocietyState, task_id: &TaskId) -> serde_json::Value {
    let contribs = state
        .contributions
        .get(task_id)
        .cloned()
        .unwrap_or_default();
    json!({
        "task_id": task_id,
        "contributions": contribs.iter().map(|c| json!({
            "agent_id": c.agent_id,
            "planned_share": c.planned_share,
            "verified_contribution": c.verified_contribution,
            "evidence_id": c.evidence_id,
            "quality": c.quality,
            "met_sla": c.met_sla,
            "recorded_tick": c.recorded_tick,
            "verified_tick": c.verified_tick,
            "effective_share": c.effective_share(),
        })).collect::<Vec<_>>()
    })
}

/// Build outcomes response
pub fn build_outcomes_response(
    state: &SocietyState,
    agent_id: &AgentId,
    limit: usize,
) -> serde_json::Value {
    let outcomes = state.recent_outcomes(agent_id, limit);
    json!({
        "agent_id": agent_id,
        "outcomes": outcomes.iter().map(|o| json!({
            "task_id": o.task_id,
            "issuer": o.issuer,
            "team_members": o.team_members,
            "status": o.status,
            "evidence_id": o.evidence_id,
            "settled_tick": o.settled_tick,
            "total_reward": o.total_reward,
            "distributions": o.distributions,
        })).collect::<Vec<_>>()
    })
}

/// Build decision hints response
pub fn build_decision_hints_response(
    rules: &SocietyRules,
    ctx: &DecisionContext,
) -> serde_json::Value {
    let hints = rules.evaluate(ctx);
    json!({
        "agent_id": ctx.agent_id,
        "tick": ctx.tick,
        "hints": hints.iter().map(|h| json!({
            "action": h.action,
            "rationale": h.rationale,
            "confidence": h.confidence,
            "alternatives": h.alternatives.iter().map(|a| json!({
                "action": a.action,
                "rationale": a.rationale,
                "confidence": a.confidence,
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}
