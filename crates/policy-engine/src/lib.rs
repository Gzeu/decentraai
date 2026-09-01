//! Policy engine — thin shim for Sprint 0.1.
//!
//! `policy-engine` was a v0.1 skeleton with multiple syntax errors
//! (unclosed delimiters, duplicate `Clone` derive, missing `Default`,
//! mismatched `cel::Env` API). Since no other crate in the workspace
//! imports it, this module is a small, compilable replacement that
//! preserves the original type names and the original `evaluate`
//! shape, but is implemented in terms of simple Rust (no CEL — that
//! dependency was unused and itself a separate integration point).
//!
//! The local agent-runtime has its own policy layer
//! (`agent_runtime::policy::DecisionPolicy`) which is the runtime's
//! actual policy surface. This crate is a placeholder; when SAES 0.2
//! is in scope, it can be re-implemented properly (CEL-backed, with
//! signals from the agent-runtime pipeline).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicyEngineError {
    #[error("policy not found: {0}")]
    NotFound(String),
    #[error("policy evaluation failed: {0}")]
    EvaluationError(String),
    #[error("policy compilation failed: {0}")]
    CompilationError(String),
    #[error("policy already exists: {0}")]
    AlreadyExists(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct PolicyId(pub String);

impl PolicyId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum PolicyType {
    #[default]
    AccessControl,
    ResourceLimit,
    RateLimit,
    Trust,
    Reputation,
    Economic,
    Governance,
    Safety,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum PolicyEffect {
    #[default]
    Allow,
    Deny,
    RequireApproval,
    RequireQuota,
    RequireReputation, // simplified: no f32 (avoids Eq/Hash bound)
    RequireTrust,      // simplified: no f32
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyContext {
    pub agent_id: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub resource_type: Option<String>,
    pub amount: Option<u64>,
    pub capability: Option<String>,
    pub trust_score: Option<f32>,
    pub reputation_score: Option<f32>,
    pub current_quota: Option<u64>,
    pub quota_ceiling: Option<u64>,
    pub current_load: Option<f32>,
    pub timestamp: u64,
    pub custom: HashMap<String, serde_json::Value>,
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self {
            agent_id: None,
            action: None,
            resource: None,
            resource_type: None,
            amount: None,
            capability: None,
            trust_score: None,
            reputation_score: None,
            current_quota: None,
            quota_ceiling: None,
            current_load: None,
            timestamp: current_timestamp(),
            custom: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimeWindow {
    pub start_hour: u8,
    pub end_hour: u8,
    pub days_of_week: Vec<u8>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyRule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub effect: PolicyEffect,
    pub condition: String,
    pub priority: i32,
    pub enabled: bool,
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyScope {
    pub agent_ids: Option<Vec<String>>,
    pub agent_roles: Option<Vec<String>>,
    pub capabilities: Option<Vec<String>>,
    pub resources: Option<Vec<String>>,
    pub actions: Option<Vec<String>>,
    pub time_windows: Option<Vec<TimeWindow>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequiredAction {
    pub action_type: String,
    pub parameters: serde_json::Value,
    pub deadline: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub effect: PolicyEffect,
    pub reason: String,
    pub matched_rules: Vec<String>,
    pub required_actions: Vec<RequiredAction>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Policy {
    pub id: PolicyId,
    pub name: String,
    pub description: String,
    pub policy_type: PolicyType,
    pub version: u32,
    pub rules: Vec<PolicyRule>,
    pub default_effect: PolicyEffect,
    pub enabled: bool,
    pub scope: PolicyScope,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Minimal in-memory policy engine. `register_policy` and
/// `evaluate` preserve the original signature. `evaluate` uses
/// substring matching (not CEL — that dependency was unused).
pub struct PolicyEngine {
    policies: HashMap<PolicyId, Policy>,
    #[allow(dead_code)]
    default_policies: Vec<PolicyId>,
}

impl PolicyEngine {
    pub fn new() -> Result<Self, PolicyEngineError> {
        Ok(Self {
            policies: HashMap::new(),
            default_policies: Vec::new(),
        })
    }

    pub async fn register_policy(&self, _policy: Policy) -> Result<(), PolicyEngineError> {
        // The surface is async for source-compat. The in-memory
        // store does not persist in this Sprint 0.1 shim; SAES 0.2
        // is in scope for proper storage.
        Ok(())
    }

    pub async fn evaluate(
        &self,
        policy_id: &PolicyId,
        context: &PolicyContext,
    ) -> Result<PolicyDecision, PolicyEngineError> {
        let policy = match self.policies.get(policy_id) {
            Some(p) => p,
            None => {
                return Ok(PolicyDecision {
                    allowed: true,
                    effect: PolicyEffect::Allow,
                    reason: format!("policy {} not found; defaulting to Allow", policy_id.0),
                    ..Default::default()
                });
            }
        };
        let ctx_str = serde_json::to_string(context).unwrap_or_default();
        for rule in policy.rules.iter().filter(|r| r.enabled) {
            if ctx_str.contains(&rule.condition) {
                return Ok(PolicyDecision {
                    allowed: matches!(rule.effect, PolicyEffect::Allow),
                    effect: rule.effect.clone(),
                    reason: format!("matched rule {}", rule.id),
                    matched_rules: vec![rule.id.clone()],
                    ..Default::default()
                });
            }
        }
        Ok(PolicyDecision {
            allowed: matches!(policy.default_effect, PolicyEffect::Allow),
            effect: policy.default_effect.clone(),
            reason: "no rule matched".to_string(),
            ..Default::default()
        })
    }

    pub async fn get_policy(&self, _id: &PolicyId) -> Option<Policy> {
        None
    }

    pub async fn list_policies(&self) -> Vec<PolicyId> {
        self.policies.keys().cloned().collect()
    }

    pub async fn update_policy(&self, policy: Policy) -> Result<(), PolicyEngineError> {
        self.register_policy(policy).await
    }

    pub async fn disable_policy(&self, _id: &PolicyId) -> Result<(), PolicyEngineError> {
        Ok(())
    }

    pub async fn enable_policy(&self, _id: &PolicyId) -> Result<(), PolicyEngineError> {
        Ok(())
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evaluate_returns_allow_when_policy_not_found() {
        let engine = PolicyEngine::new().unwrap();
        let decision = engine
            .evaluate(&PolicyId::new("nonexistent"), &PolicyContext::default())
            .await
            .unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.effect, PolicyEffect::Allow);
    }

    #[tokio::test]
    async fn register_then_evaluate_compiles() {
        let engine = PolicyEngine::new().unwrap();
        let policy = Policy {
            id: PolicyId::new("p1"),
            name: "p1".to_string(),
            default_effect: PolicyEffect::Deny,
            rules: vec![PolicyRule {
                id: "r1".to_string(),
                enabled: true,
                effect: PolicyEffect::Allow,
                condition: "analysis".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        engine.register_policy(policy).await.unwrap();
        // The shim does not actually store policies, but the call
        // must succeed.
    }

    #[test]
    fn context_default_is_sensible() {
        let ctx = PolicyContext::default();
        assert!(ctx.agent_id.is_none());
        assert!(ctx.capability.is_none());
        assert!(ctx.custom.is_empty());
        assert!(ctx.timestamp > 0);
    }

    #[test]
    fn policy_id_round_trip() {
        let id = PolicyId::new("test-policy-1");
        assert_eq!(id.0, "test-policy-1");
    }

    #[test]
    fn policy_effect_default_is_allow() {
        assert_eq!(PolicyEffect::default(), PolicyEffect::Allow);
    }
}
