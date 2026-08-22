//! The runtime facade over providers + policy + telemetry.
//!
//! [`FabricIntelligence::plan`] executes the configured selection policy:
//! it may try the local backend and/or the external endpoint, parses every
//! raw answer through the strict plan gate, and returns an honest outcome.
//! It performs NO fabric actions — the caller validates and routes.

use decentraai_config::{FabricIntelligencePolicy, FabricIntelligenceSection};

use crate::limits::ArtifactLimit;
use crate::policy::{select_provider, ProviderChoice};
use crate::provider::{
    ConfiguredProvider, LocalLlamaProvider, OpenAiCompatProvider, ProviderKind,
};
use crate::telemetry::IntelTelemetry;
use crate::TaskPlan;

/// Everything the intelligence layer needs at runtime, built once from
/// config in the node daemon. Cheap to share (`Arc<FabricIntelligence>`).
#[derive(Clone)]
pub struct FabricIntelligence {
    policy: FabricIntelligencePolicy,
    min_confidence: f32,
    local: Option<LocalLlamaProvider>,
    external: Option<OpenAiCompatProvider>,
    artifact_limit: ArtifactLimit,
    telemetry: std::sync::Arc<IntelTelemetry>,
}

/// What one planning run produced. Honest even in failure: `attempts` keeps
/// the per-provider outcome so telemetry and status stay informative.
#[derive(Debug)]
pub struct PlanOutcome {
    pub plan: Option<TaskPlan>,
    /// Per-attempt facts: (provider kind, raw answer parsed OK?, latency ms).
    pub attempts: Vec<(ProviderKind, bool, u64)>,
    pub error: Option<String>,
}

impl PlanOutcome {
    /// A usable plan came out of some provider.
    pub fn succeeded(&self) -> bool {
        self.plan.is_some()
    }
}

impl std::fmt::Debug for FabricIntelligence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual Debug: telemetry holds atomics/mutexes and MUST never print
        // anything beyond counters anyway.
        f.debug_struct("FabricIntelligence")
            .field("policy", &self.policy)
            .field("min_confidence", &self.min_confidence)
            .field("local", &self.local.as_ref().map(|l| l.base_url.clone()))
            .field(
                "external",
                &self
                    .external
                    .as_ref()
                    .map(|e| (e.base_url.clone(), e.model.clone())),
            )
            .finish()
    }
}

impl FabricIntelligence {
    /// Builds from config. The local provider's base URL is NOT baked in:
    /// llama-server ports are EPHEMERAL and change on every engine respawn
    /// (M24 supervisor), so [`FabricIntelligence::plan`] receives the live
    /// URL from the caller at request time — the same pattern the chat
    /// proxy uses.
    pub fn from_config(section: &FabricIntelligenceSection) -> Self {
        let local = if section.enabled {
            Some(LocalLlamaProvider::new(String::new(), section.local_model.clone()))
        } else {
            None
        };
        let external = section.external.as_ref().map(|ext| {
            OpenAiCompatProvider::new(
                ext.base_url.clone(),
                ext.api_key_env.clone(),
                ext.model.clone(),
            )
        });
        Self {
            policy: section.policy,
            min_confidence: section.min_confidence,
            local,
            external,
            artifact_limit: ArtifactLimit {
                max_bytes: section.max_artifact_bytes,
            },
            telemetry: std::sync::Arc::new(IntelTelemetry::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.local.is_some() || self.external.is_some()
    }

    pub fn policy(&self) -> FabricIntelligencePolicy {
        self.policy
    }

    pub fn min_confidence(&self) -> f32 {
        self.min_confidence
    }

    pub fn artifact_limit(&self) -> ArtifactLimit {
        self.artifact_limit
    }

    pub fn telemetry(&self) -> &IntelTelemetry {
        &self.telemetry
    }

    /// Non-sensitive identity for status output.
    pub fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled(),
            "policy": format!("{:?}", self.policy).to_lowercase(),
            "min_confidence": self.min_confidence,
            "local_model": self.local.as_ref().and_then(|l| l.model.clone()),
            "external_configured": self.external.as_ref().is_some(),
            // NEVER: api keys, key env VALUES (the env NAME is safe), tasks.
            "external_api_key_env": "configured".to_string(),
            "artifact_limit_bytes": self.artifact_limit.max_bytes,
        })
    }

    fn provider_for(&self, choice: ProviderChoice) -> Option<ConfiguredProvider> {
        match choice {
            ProviderChoice::Local => self
                .local
                .as_ref()
                .map(|p| ConfiguredProvider::Local(p.clone())),
            ProviderChoice::External => self
                .external
                .as_ref()
                .map(|p| ConfiguredProvider::External(p.clone())),
            ProviderChoice::None => None,
        }
    }

    /// Whether the external provider could authenticate right now (key env
    /// resolvable). Unconfigured/unkeyed externals are not selectable under
    /// policies that would otherwise pick them.
    fn external_ready(&self) -> bool {
        self.external
            .as_ref()
            .is_some_and(|e| e.key_available())
    }

    async fn run_provider(
        &self,
        provider: &ConfiguredProvider,
        task: &str,
    ) -> (Result<TaskPlan, String>, u64) {
        let started = std::time::Instant::now();
        let brief = crate::TaskBrief { task };
        let outcome = match provider.analyze(&brief).await {
            Ok(raw) => crate::TaskPlan::parse(&raw).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        (outcome, started.elapsed().as_millis() as u64)
    }

    /// Executes the selection policy for ONE user task.
    ///
    /// Flow (local_first example): local attempt → strict parse; success with
    /// confidence ≥ threshold returns immediately; anything else falls back
    /// to the external IF the policy allows and the endpoint can auth. The
    /// returned plan is still only a PROPOSAL — mesh validation and routing
    /// happen in the deterministic layer.
    pub async fn plan(&self, task: &str, backend_base_url: &str) -> PlanOutcome {
        // Fail FAST with the real cause when the policy routes externally
        // but the key cannot resolve: a generic "no usable provider" would
        // hide the actionable fix (set the env var).
        if matches!(
            self.policy,
            FabricIntelligencePolicy::ExternalOnly | FabricIntelligencePolicy::ExternalFirst
        ) && self.external.as_ref().is_some_and(|e| !e.key_available())
        {
            return PlanOutcome {
                plan: None,
                attempts: Vec::new(),
                error: Some(
                    self.external
                        .as_ref()
                        .unwrap()
                        .missing_key_error()
                        .to_string(),
                ),
            };
        }

        let mut attempts: Vec<(ProviderKind, bool, u64)> = Vec::new();
        let mut last_error: Option<String> = None;

        let mut local_failed = false;
        loop {
            let choice = select_provider(
                self.map_policy(),
                self.external_ready(),
                local_failed,
            );
            let Some(provider) = self.provider_for(choice) else {
                last_error.get_or_insert_with(|| {
                    "no usable intelligence provider for the configured policy".to_string()
                });
                break;
            };

            // ExternalOnly must never silently degrade to local: the pure
            // selector already encodes that, but guard again at the call site.
            if self.policy == FabricIntelligencePolicy::ExternalOnly
                && provider.kind() == ProviderKind::Local
            {
                break;
            }
            if choice == ProviderChoice::Local && self.local.is_none() {
                local_failed = true;
                continue;
            }

            // Live backend URL: an engine respawn changes the ephemeral
            // port, so a URL captured at boot would point at a dead socket.
            let mut provider = provider.clone();
            if let ConfiguredProvider::Local(local) = &mut provider {
                if local.base_url.is_empty() {
                    local.base_url = backend_base_url.to_string();
                }
            }

            let kind = provider.kind();
            self.telemetry.record_plan_generated();
            let (result, latency_ms) = self.run_provider(&provider, task).await;
            match result {
                Ok(plan) => {
                    let confident = plan.confidence >= self.min_confidence;
                    attempts.push((kind, true, latency_ms));
                    self.telemetry.record_attempt(kind, true, latency_ms);
                    self.telemetry.record_plan_outcome(true);
                    if confident || !self.fallback_allowed_after_low_confidence() {
                        return PlanOutcome {
                            plan: Some(plan),
                            attempts,
                            error: None,
                        };
                    }
                    // Valid but low-confidence: treat as a soft failure and
                    // let the policy try the next source (if any).
                    local_failed = true;
                    last_error.get_or_insert_with(|| {
                        format!(
                            "plan confidence {} below threshold {}",
                            plan.confidence, self.min_confidence
                        )
                    });
                }
                Err(e) => {
                    attempts.push((kind, false, latency_ms));
                    self.telemetry.record_attempt(kind, false, latency_ms);
                    self.telemetry.record_plan_outcome(false);
                    if kind == ProviderKind::Local {
                        local_failed = true;
                    }
                    last_error = Some(e);
                }
            }
            // Guard against pathological loops: at most two sources today.
            if attempts.len() >= 2 {
                break;
            }
        }

        PlanOutcome {
            plan: None,
            attempts,
            error: last_error,
        }
    }

    /// Whether a below-threshold plan may trigger the next provider.
    /// `Fallback` and both *First policies allow it; *_ONLY never leave
    /// their lane (ExternalOnly cannot fall back to local at all).
    fn fallback_allowed_after_low_confidence(&self) -> bool {
        matches!(
            self.policy,
            FabricIntelligencePolicy::LocalFirst | FabricIntelligencePolicy::Fallback
        )
    }

    // Policy mapping helper — the config enum IS the decision enum; this
    // exists so `select_provider` stays independent of the config crate.
    fn map_policy(&self) -> crate::policy::SelectionPolicy {
        match self.policy {
            FabricIntelligencePolicy::LocalFirst => crate::policy::SelectionPolicy::LocalFirst,
            FabricIntelligencePolicy::ExternalFirst => {
                crate::policy::SelectionPolicy::ExternalFirst
            }
            FabricIntelligencePolicy::LocalOnly => crate::policy::SelectionPolicy::LocalOnly,
            FabricIntelligencePolicy::ExternalOnly => crate::policy::SelectionPolicy::ExternalOnly,
            FabricIntelligencePolicy::Fallback => crate::policy::SelectionPolicy::Fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_config::{FabricIntelExternalSection, FabricIntelligenceSection};

    fn cfg(policy: FabricIntelligencePolicy, external: Option<FabricIntelExternalSection>) -> FabricIntelligenceSection {
        FabricIntelligenceSection {
            enabled: true,
            policy,
            min_confidence: 0.5,
            local_model: None,
            external,
            max_artifact_bytes: crate::MAX_ARTIFACT_BYTES,
        }
    }

    #[tokio::test]
    async fn from_config_reports_status_without_baking_backend_url() {
        let fi = FabricIntelligence::from_config(&cfg(FabricIntelligencePolicy::LocalFirst, None));
        assert!(fi.enabled());
        let status = fi.describe();
        assert_eq!(status["enabled"], true);
        assert_eq!(status["external_configured"], false);
        assert_eq!(status["artifact_limit_bytes"], crate::MAX_ARTIFACT_BYTES);
    }

    /// ExternalOnly with an UNSET key env must fail closed: the pure
    /// selector refuses to hand out the local provider and the run ends
    /// with a clear error — never a silent local execution.
    #[tokio::test]
    async fn external_only_with_missing_key_fails_closed() {
        let ext = FabricIntelExternalSection {
            base_url: "https://api.example.com/v1".into(),
            api_key_env: "DECENTRAAI_INTEL_TEST_UNSET_KEY".into(),
            model: "test-model".into(),
        };
        let fi = FabricIntelligence::from_config(&cfg(FabricIntelligencePolicy::ExternalOnly, Some(ext)));
        // NOTE(test): remove_var is unsafe in Rust 2024 and unnecessary —
        // this env name is never set anywhere in the test suite.
        let outcome = fi.plan("classify me", "http://127.0.0.1:9").await;
        assert!(!outcome.succeeded(), "must not produce a plan");
        assert!(
            outcome.error.as_deref().is_some_and(|e| e.contains("DECENTRAAI_INTEL_TEST_UNSET_KEY")),
            "error names the missing env var: {:?}",
            outcome.error
        );
        // And NO attempt ever touched the local provider.
        assert!(outcome.attempts.iter().all(|(k, _, _)| *k != crate::ProviderKind::Local));
    }
}
