//! Lightweight per-agent decision policy — defined here in
//! `agent-runtime` to avoid a hard dependency on the `policy-engine`
//! crate, which currently has compile errors from a parallel work
//! stream. The shape mirrors what `policy-engine` will eventually
//! provide (a CEL-backed spec loader), but the live implementation
//! is intentionally minimal so the runtime is shippable today.
//!
//! When `policy-engine` becomes compilable, this module is replaced
//! by a re-export. The trait surface (name + decide) is what the
//! runtime depends on; everything else is internal.

use crate::{AgentDecision, AgentObservation, DecisionType};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A per-agent decision policy. The runtime calls `decide()` to
/// produce a `Decision` from an `Observation`.
pub trait DecisionPolicy: Send + Sync {
    fn name(&self) -> &str;
    fn decide(&self, obs: &AgentObservation) -> AgentDecision;
}

/// v1-equivalent: bid on the first open task whose
/// `required_capability` is in the agent's declared capabilities
/// and whose reward is > 0. Otherwise wait.
pub struct DefaultBidPolicy;

impl DecisionPolicy for DefaultBidPolicy {
    fn name(&self) -> &str {
        "default_v1"
    }

    fn decide(&self, obs: &AgentObservation) -> AgentDecision {
        let capabilities: HashSet<String> = obs.available_capabilities.iter().cloned().collect();

        let task = pick_open_task(&obs.hub_state_summary, &capabilities);

        let (decision_type, reasoning, context) = match task {
            Some(t) => {
                let tid = t
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let reward = t.get("reward").and_then(|v| v.as_u64()).unwrap_or(0);
                let cap = t
                    .get("required_capability")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (
                    DecisionType::Bid,
                    format!(
                        "default: open task {} matches capability '{}' with reward {}",
                        tid, cap, reward
                    ),
                    serde_json::json!({
                        "policy": "default_v1",
                        "task": t,
                    }),
                )
            }
            None => (
                DecisionType::Wait,
                "default: no open task matches a declared capability".to_string(),
                serde_json::json!({"policy": "default_v1"}),
            ),
        };

        AgentDecision {
            agent_id: obs.agent_id.clone(),
            timestamp: now_ms(),
            decision_type,
            reasoning,
            confidence: 0.5,
            expected_outcome: serde_json::Value::Null,
            context,
        }
    }
}

fn pick_open_task(
    hub_summary: &serde_json::Value,
    capabilities: &HashSet<String>,
) -> Option<serde_json::Value> {
    let tasks = hub_summary.get("tasks")?.as_array()?;
    for t in tasks {
        let status = t.get("status")?.as_str()?;
        if status != "open" && status != "bidding" {
            continue;
        }
        let cap = t.get("required_capability")?.as_str()?;
        if !capabilities.contains(cap) {
            continue;
        }
        let reward = t.get("reward")?.as_u64()?;
        if reward == 0 {
            continue;
        }
        return Some(t.clone());
    }
    None
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A policy that always waits. Useful for tests and as a "do
/// nothing" baseline.
pub struct AlwaysWaitPolicy;

impl DecisionPolicy for AlwaysWaitPolicy {
    fn name(&self) -> &str {
        "always_wait"
    }

    fn decide(&self, obs: &AgentObservation) -> AgentDecision {
        AgentDecision {
            agent_id: obs.agent_id.clone(),
            timestamp: now_ms(),
            decision_type: DecisionType::Wait,
            reasoning: "always_wait: explicitly waiting".to_string(),
            confidence: 1.0,
            expected_outcome: serde_json::Value::Null,
            context: serde_json::json!({"policy": "always_wait"}),
        }
    }
}

/// A policy loaded from a JSON spec. The spec is a list of
/// `{name, condition_contains, action}` rules. The `condition_contains`
/// is a *substring match* against the JSON-serialised observation
/// (not CEL — CEL requires the `cel` crate, which we do not pull in
/// here to keep `agent-runtime` standalone).
///
/// The first rule whose `condition_contains` appears in the
/// serialised observation wins. If no rule matches, the policy
/// falls back to `Wait`.
///
/// This is intentionally a degraded version of what `policy-engine`
/// will provide. It exists so that the foundation is testable
/// without depending on a non-compiling crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonSpecPolicyLite {
    pub name: String,
    pub rules: Vec<JsonSpecRuleLite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonSpecRuleLite {
    pub name: String,
    pub condition_contains: String,
    /// snake_case `DecisionType` variant. Anything not in the
    /// enum maps to `Wait`.
    pub action: String,
    #[serde(default)]
    pub rationale: String,
}

impl JsonSpecPolicyLite {
    pub fn from_spec(spec: JsonSpecPolicyLite) -> Self {
        spec
    }
}

impl DecisionPolicy for JsonSpecPolicyLite {
    fn name(&self) -> &str {
        "json_spec_lite"
    }

    fn decide(&self, obs: &AgentObservation) -> AgentDecision {
        let observation_str = serde_json::to_string(obs).unwrap_or_default();
        for rule in &self.rules {
            if observation_str.contains(&rule.condition_contains) {
                let dt = parse_decision_type(&rule.action);
                return AgentDecision {
                    agent_id: obs.agent_id.clone(),
                    timestamp: now_ms(),
                    decision_type: dt,
                    reasoning: if rule.rationale.is_empty() {
                        format!(
                            "spec rule '{}' matched (substring '{}')",
                            rule.name, rule.condition_contains
                        )
                    } else {
                        rule.rationale.clone()
                    },
                    confidence: 0.7,
                    expected_outcome: serde_json::Value::Null,
                    context: serde_json::json!({
                        "policy": "json_spec_lite",
                        "rule": rule.name,
                    }),
                };
            }
        }
        AgentDecision {
            agent_id: obs.agent_id.clone(),
            timestamp: now_ms(),
            decision_type: DecisionType::Wait,
            reasoning: "json_spec_lite: no rule matched".to_string(),
            confidence: 0.3,
            expected_outcome: serde_json::Value::Null,
            context: serde_json::json!({"policy": "json_spec_lite"}),
        }
    }
}

fn parse_decision_type(s: &str) -> DecisionType {
    match s {
        "bid" => DecisionType::Bid,
        "propose" => DecisionType::Propose,
        "form_team" => DecisionType::FormTeam,
        "execute" => DecisionType::Execute,
        "publish_task" => DecisionType::PublishTask,
        "wait" => DecisionType::Wait,
        "publish" => DecisionType::Publish,
        "search" => DecisionType::Search,
        "learn" => DecisionType::Learn,
        "reflect" => DecisionType::Reflect,
        "dream" => DecisionType::Dream,
        "rest" => DecisionType::Rest,
        _ => DecisionType::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentObservation;
    use decentraai_protocol::AgentId;

    fn obs(caps: Vec<&str>, hub: serde_json::Value) -> AgentObservation {
        AgentObservation {
            agent_id: AgentId::from("test"),
            timestamp: 0,
            hub_state_summary: hub,
            society_state_summary: serde_json::json!({}),
            personal_memory_summary: serde_json::json!({}),
            arena_state_summary: None,
            queue_depth: 0,
            available_capabilities: caps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn default_bids_on_match() {
        let o = obs(
            vec!["analysis"],
            serde_json::json!({"tasks": [
                {"id": "t1", "status": "open", "required_capability": "analysis", "reward": 30}
            ]}),
        );
        let d = DefaultBidPolicy.decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Bid));
    }

    #[test]
    fn default_waits_when_no_match() {
        let o = obs(
            vec!["coding"],
            serde_json::json!({"tasks": [
                {"id": "t1", "status": "open", "required_capability": "analysis", "reward": 30}
            ]}),
        );
        let d = DefaultBidPolicy.decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Wait));
    }

    #[test]
    fn always_wait_always_waits() {
        let o = obs(
            vec!["analysis"],
            serde_json::json!({"tasks": [
                {"id": "t1", "status": "open", "required_capability": "analysis", "reward": 1000}
            ]}),
        );
        let d = AlwaysWaitPolicy.decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Wait));
    }

    #[test]
    fn json_spec_substring_match() {
        let o = obs(
            vec!["quantum_simulation_v0"],
            serde_json::json!({"tasks": [
                {"id": "q1", "status": "open", "required_capability": "quantum_simulation_v0", "reward": 100}
            ]}),
        );
        let spec = JsonSpecPolicyLite {
            name: "bid_quantum".to_string(),
            rules: vec![JsonSpecRuleLite {
                name: "r1".to_string(),
                condition_contains: "quantum_simulation_v0".to_string(),
                action: "bid".to_string(),
                rationale: "quantum task".to_string(),
            }],
        };
        let d = JsonSpecPolicyLite::from_spec(spec).decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Bid));
        assert!(d.reasoning.contains("quantum task"));
    }

    #[test]
    fn json_spec_no_match_waits() {
        let o = obs(vec!["a"], serde_json::json!({}));
        let spec = JsonSpecPolicyLite {
            name: "never".to_string(),
            rules: vec![JsonSpecRuleLite {
                name: "r1".to_string(),
                condition_contains: "this_string_does_not_appear".to_string(),
                action: "bid".to_string(),
                rationale: "".to_string(),
            }],
        };
        let d = JsonSpecPolicyLite::from_spec(spec).decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Wait));
    }

    #[test]
    fn first_matching_rule_wins() {
        let o = obs(vec!["x"], serde_json::json!({"tasks": []}));
        let spec = JsonSpecPolicyLite {
            name: "first_wins".to_string(),
            rules: vec![
                JsonSpecRuleLite {
                    name: "r1".to_string(),
                    condition_contains: "tasks".to_string(), // matches: serialised has "tasks"
                    action: "bid".to_string(),
                    rationale: "first".to_string(),
                },
                JsonSpecRuleLite {
                    name: "r2".to_string(),
                    condition_contains: "tasks".to_string(), // also matches, but ignored
                    action: "wait".to_string(),
                    rationale: "second".to_string(),
                },
            ],
        };
        let d = JsonSpecPolicyLite::from_spec(spec).decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Bid));
        assert!(d.reasoning.contains("first"));
    }

    #[test]
    fn unanticipated_capability_via_spec() {
        // A spec that branches on a brand-new capability, with no
        // mention of any v1 capability kind. This is the test that
        // proves the fallback policy mechanism is generic over
        // capability names, not just v1's 26 builtin kinds.
        let o = obs(
            vec!["quantum_simulation_v0"],
            serde_json::json!({
                "tasks": [
                    {"id": "q1", "status": "open", "required_capability": "quantum_simulation_v0", "reward": 100}
                ]
            }),
        );
        let spec = JsonSpecPolicyLite {
            name: "quantum_agent".to_string(),
            rules: vec![JsonSpecRuleLite {
                name: "if_quantum".to_string(),
                condition_contains: "quantum_simulation_v0".to_string(),
                action: "bid".to_string(),
                rationale: "quantum found".to_string(),
            }],
        };
        let d = JsonSpecPolicyLite::from_spec(spec).decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Bid));
        assert!(d.reasoning.contains("quantum found"));
    }

    #[test]
    fn empty_spec_falls_back_to_wait() {
        // A spec with zero rules must not panic; it must return Wait.
        let o = obs(vec!["x"], serde_json::json!({}));
        let spec = JsonSpecPolicyLite {
            name: "empty".to_string(),
            rules: vec![],
        };
        let d = JsonSpecPolicyLite::from_spec(spec).decide(&o);
        assert!(matches!(d.decision_type, DecisionType::Wait));
        assert!(d.reasoning.contains("no rule matched"));
    }

    #[test]
    fn parse_decision_type_handles_all_known_variants() {
        // Regression: every DecisionType variant must be parseable.
        let known = [
            ("bid", DecisionType::Bid),
            ("propose", DecisionType::Propose),
            ("form_team", DecisionType::FormTeam),
            ("execute", DecisionType::Execute),
            ("publish_task", DecisionType::PublishTask),
            ("wait", DecisionType::Wait),
            ("publish", DecisionType::Publish),
            ("search", DecisionType::Search),
            ("learn", DecisionType::Learn),
            ("reflect", DecisionType::Reflect),
            ("dream", DecisionType::Dream),
            ("rest", DecisionType::Rest),
        ];
        for (s, expected) in known {
            assert_eq!(parse_decision_type(s), expected, "mismatch for {s}");
        }
        // Unknown -> Wait, never panic.
        assert!(matches!(
            parse_decision_type("totally_new_action"),
            DecisionType::Wait
        ));
    }
}
