//! Capability proof — end-to-end demonstration of the "unanticipated
//! agent" pattern, isolated from any specific registry crate.
//!
//! This module does NOT depend on `decentraai-capability-registry`,
//! `decentraai-policy-engine`, or `introspection-api`. It only uses
//! types from `agent-runtime` and `event-bus`. The integration test
//! in `INTEGRATION.md` Step 6 wires these together through the
//! production crates; this module is the *local* proof that the
//! foundation is generic.
//!
//! The pattern:
//! 1. An agent declares a brand-new capability (`quantum_simulation_v0`)
//!    that is not in any v1 enumeration.
//! 2. The capability is validated only as a non-empty string
//!    (the runtime does not check the registry).
//! 3. The agent's policy decides to act on a task with that
//!    capability — without any v1 enum variant knowing about it.
//! 4. The action is recorded and emitted on the bus.

use crate::policy::JsonSpecPolicyLite;
use crate::{AgentConfig, AgentObservation, AgentRuntime, AgentStatus};
use async_trait::async_trait;
use dashmap::DashMap;
use decentraai_event_bus::EventBus;
use decentraai_protocol::AgentId as ProtocolAgentId;
use serde_json::Value;
use std::sync::Arc;

/// A minimal in-test registry. Production code uses
/// `decentraai-capability-registry`; this stub is here only to
/// prove the foundation's genericity, not to be the production
/// registry.
pub struct TinyRegistry {
    pub ids: DashMap<String, ()>,
}

impl TinyRegistry {
    pub fn new() -> Self {
        Self {
            ids: DashMap::new(),
        }
    }
    pub fn register(&self, id: &str) {
        self.ids.insert(id.to_string(), ());
    }
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }
}

/// A minimal in-test observation builder. Production code wires the
/// v1 fabric surfaces (hub_state, society_state, arena_state).
pub struct TinyObserver {
    pub hub_state: Value,
}

#[async_trait]
impl crate::local::ObservationBuilder for TinyObserver {
    async fn build(
        &self,
        _agent_id: &ProtocolAgentId,
        capabilities: &[String],
    ) -> AgentObservation {
        AgentObservation {
            agent_id: _agent_id.clone(),
            timestamp: 0,
            hub_state_summary: self.hub_state.clone(),
            society_state_summary: Value::Null,
            personal_memory_summary: Value::Null,
            arena_state_summary: None,
            queue_depth: 0,
            available_capabilities: capabilities.to_vec(),
        }
    }
}

/// End-to-end smoke test that the foundation is generic over the
/// capability vocabulary. Does NOT depend on the policy-engine or
/// capability-registry crates.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalAgentRuntime;

    #[tokio::test]
    async fn end_to_end_unanticipated_capability_with_isolated_registry() {
        // 1. Build the foundation: a bus and a runtime with a custom
        //    observer. None of the v1 fabric crates are imported.
        let bus = Arc::new(EventBus::new(Arc::new(
            decentraai_event_bus::InMemoryEventStore::new(1024),
        )));
        let mut rx = bus.subscribe_broadcast();
        let observer = Arc::new(TinyObserver {
            hub_state: serde_json::json!({
                "tasks": [
                    {
                        "id": "q1",
                        "status": "open",
                        "required_capability": "quantum_simulation_v0",
                        "reward": 100
                    }
                ]
            }),
        });
        let runtime: Arc<dyn AgentRuntime> =
            Arc::new(LocalAgentRuntime::new(bus.clone(), observer));

        // 2. A tiny in-test registry declares the new capability.
        //    This is the call-site validation pattern: the runtime
        //    does not check the registry; the caller does.
        let registry = TinyRegistry::new();
        registry.register("quantum_simulation_v0");
        assert!(registry.contains("quantum_simulation_v0"));

        // 3. Spawn an agent that declares the unanticipated capability.
        //    The runtime accepts the capability purely on shape
        //    (non-empty string). The v1 enum `ActionType` /
        //    `DecisionType` are not extended.
        let cfg = AgentConfig {
            agent_id: ProtocolAgentId::from("agent-quantum-001"),
            name: "quantum-bot".to_string(),
            capabilities: vec!["quantum_simulation_v0".to_string()],
            initial_goals: vec![],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: crate::ResourceLimits::default(),
        };
        let h = runtime.spawn(cfg.clone()).await.unwrap();
        assert_eq!(h.status, AgentStatus::Ready);
        let state = runtime.get_state(&cfg.agent_id).await.unwrap();
        assert_eq!(
            state.config.capabilities,
            vec!["quantum_simulation_v0".to_string()]
        );

        // 4. Install a custom policy that branches on the new
        //    capability. The runtime does not interpret the
        //    condition; the policy does.
        let policy = JsonSpecPolicyLite {
            name: "quantum_bid".to_string(),
            rules: vec![crate::policy::JsonSpecRuleLite {
                name: "if_quantum".to_string(),
                condition_contains: "quantum_simulation_v0".to_string(),
                action: "bid".to_string(),
                rationale: "quantum work is interesting".to_string(),
            }],
        };
        // We need a concrete LocalAgentRuntime to install the
        // per-agent policy (the trait doesn't expose that method,
        // because policy is an implementation detail of the
        // concrete runtime, not a trait surface).
        let concrete = Arc::new(LocalAgentRuntime::new(
            bus.clone(),
            Arc::new(TinyObserver {
                hub_state: serde_json::json!({
                    "tasks": [
                        {
                            "id": "q1",
                            "status": "open",
                            "required_capability": "quantum_simulation_v0",
                            "reward": 100
                        }
                    ]
                }),
            }),
        ));
        concrete.install_json_spec_for(&cfg.agent_id, policy);
        let h2 = concrete.spawn(cfg.clone()).await.unwrap();
        let observation = concrete.observe(&h2.agent_id).await.unwrap();
        let decision = concrete.decide(&h2.agent_id, &observation).await.unwrap();
        assert!(matches!(decision.decision_type, crate::DecisionType::Bid));
        assert!(decision.reasoning.contains("quantum work"));

        // 5. The default policy (no spec) also bids because the task
        //    matches the declared capability. This shows the runtime
        //    is generic over the capability *name* — it only checks
        //    membership, not the v1 taxonomy.
        let observation = runtime.observe(&h.agent_id).await.unwrap();
        // The default policy is what the runtime uses when no
        // per-agent policy is installed. Since we did not install
        // one for `h`, the default runs.
        let decision = runtime.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(decision.decision_type, crate::DecisionType::Bid));

        // 6. The act() emits an event on the bus. A subscriber can
        //    observe it. The capability name is in the action's
        //    parameters (transitively, via the task).
        let _a = runtime.act(&h.agent_id, &decision).await.unwrap();
        let mut found = None;
        for _ in 0..20 {
            match rx.try_recv() {
                Ok(ev) => {
                    if ev.event_type == "agent.action" && ev.source == h.agent_id {
                        found = Some(ev);
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let ev = found.expect("expected an agent.action event on the bus");
        let params = ev.payload.get("parameters").expect("parameters");
        assert_eq!(
            params.get("decision_type").and_then(|v| v.as_str()),
            Some("Bid")
        );

        // 7. Cleanup is straightforward: stop the agent.
        runtime.stop(&h.agent_id).await.unwrap();
        let state = runtime.get_state(&h.agent_id).await.unwrap();
        assert_eq!(state.status, AgentStatus::Stopped);
    }
}
