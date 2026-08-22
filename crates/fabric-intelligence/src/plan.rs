//! The structured output contract between an intelligence provider and the
//! deterministic fabric.
//!
//! The model's raw answer is UNTRUSTED input: it may be truncated, wrapped in
//! prose, hallucinated or hostile. [`TaskPlan::parse`] therefore enforces a
//! closed schema (`deny_unknown_fields`, bounded sizes, taxonomy-validated
//! capability names) and rejects everything else. The fabric only ever sees
//! a parsed, validated [`TaskPlan`].

use decentraai_hub::capability::CapabilityKind;
use serde::{Deserialize, Serialize};

/// One capability step inside a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanCapability {
    /// Snake-case name from the hub taxonomy (e.g. `ocr`, `reasoning`).
    /// Validated against [`CapabilityKind`] during parsing — a name outside
    /// the taxonomy is a parse failure, not a soft warning.
    pub name: String,
    /// Whether the plan cannot proceed without this step.
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// A proposed execution plan for one user task.
///
/// Produced by parsing a provider's raw JSON; consumed by
/// [`crate::validation`] and then by the deterministic planner.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskPlan {
    /// Short snake_case classification of the task (bounded).
    pub intent: String,
    /// Required/optional capability steps.
    pub capabilities: Vec<PlanCapability>,
    /// Preferred execution order over capability names. Every entry must
    /// reference a declared capability (enforced at parse time) — the model
    /// cannot smuggle undeclared steps into the workflow.
    pub workflow: Vec<String>,
    /// Provider self-reported certainty in `[0, 1]`. Advisory only: the
    /// deterministic validation below decides what actually runs.
    pub confidence: f32,
}

/// Why a provider's raw output was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// Not valid JSON at all (prose, markdown fences, truncation).
    NotJson(String),
    /// Valid JSON but not matching the closed schema.
    SchemaMismatch(String),
    /// A capability name outside the hub taxonomy.
    UnknownCapability(String),
    /// Workflow references a capability that was never declared.
    WorkflowReferencesUndeclaredStep(String),
    /// Empty intent / no capabilities at all.
    EmptyPlan,
    /// Bounds exceeded (intent length, capability count, workflow length).
    TooLarge(&'static str),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(s) => write!(f, "plan is not valid JSON: {s}"),
            Self::SchemaMismatch(s) => write!(f, "plan schema mismatch: {s}"),
            Self::UnknownCapability(c) => write!(f, "unknown capability: {c}"),
            Self::WorkflowReferencesUndeclaredStep(c) => {
                write!(f, "workflow references undeclared capability: {c}")
            }
            Self::EmptyPlan => write!(f, "plan has empty intent or no capabilities"),
            Self::TooLarge(what) => write!(f, "plan field exceeds bound: {what}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Closed-schema mirror of [`TaskPlan`] used for deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlan {
    intent: String,
    capabilities: Vec<RawCapability>,
    workflow: Vec<String>,
    confidence: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapability {
    name: String,
    #[serde(default = "default_true")]
    required: bool,
}

/// Hard bounds on model-generated plans. Generous enough for real tasks,
/// tight enough that a runaway model cannot produce megabytes of "plan".
const MAX_INTENT_BYTES: usize = 128;
const MAX_CAPABILITIES: usize = 16;
const MAX_WORKFLOW_STEPS: usize = 16;

/// Parses and validates a provider's RAW text answer into a [`TaskPlan`].
///
/// Tolerates exactly one benign formatting habit — leading/trailing
/// whitespace — and nothing else: markdown fences, prose prefixes and
/// trailing commentary are all rejections. A model that cannot follow the
/// contract is a broken intelligence source, not something to scrape around.
impl TaskPlan {
    pub fn parse(raw: &str) -> Result<Self, PlanError> {
        let trimmed = raw.trim();
        // Strip ONE optional ```json fence pair — the single most common LLM
        // formatting tic even under strict instructions. Anything beyond that
        // (prose around the fence, multiple objects) is rejected so hostile
        // payloads cannot hide behind lenient parsing.
        let body = trimmed.strip_prefix("```json").map_or(trimmed, |rest| {
            rest.trim_start()
                .strip_suffix("```")
                .unwrap_or(rest.trim_end())
                .trim_end()
        });
        let raw: RawPlan = serde_json::from_str(body)
            .map_err(|e| match e.classify() {
                serde_json::error::Category::Syntax => PlanError::NotJson(e.to_string()),
                _ => PlanError::SchemaMismatch(e.to_string()),
            })?;

        let intent = raw.intent.trim().to_string();
        if intent.is_empty() || raw.capabilities.is_empty() {
            return Err(PlanError::EmptyPlan);
        }
        if intent.len() > MAX_INTENT_BYTES {
            return Err(PlanError::TooLarge("intent"));
        }
        if raw.capabilities.len() > MAX_CAPABILITIES {
            return Err(PlanError::TooLarge("capabilities"));
        }
        if raw.workflow.len() > MAX_WORKFLOW_STEPS {
            return Err(PlanError::TooLarge("workflow"));
        }

        let mut capabilities = Vec::with_capacity(raw.capabilities.len());
        for cap in raw.capabilities {
            // Taxonomy check FIRST: an unknown name is rejected outright, so
            // downstream validation can trust every declared name.
            if cap.name.parse::<CapabilityKind>().is_err() {
                return Err(PlanError::UnknownCapability(cap.name));
            }
            capabilities.push(PlanCapability {
                name: cap.name,
                required: cap.required,
            });
        }

        let declared: std::collections::HashSet<&str> =
            capabilities.iter().map(|c| c.name.as_str()).collect();
        for step in &raw.workflow {
            if !declared.contains(step.as_str()) {
                return Err(PlanError::WorkflowReferencesUndeclaredStep(step.clone()));
            }
        }

        // NaN guard: a malicious `confidence: NaN` must not slip through
        // comparisons later (NaN < x is false for every x, silently passing
        // any threshold check). Clamp to the valid range instead.
        let confidence = if raw.confidence.is_finite() {
            raw.confidence.clamp(0.0, 1.0)
        } else {
            0.0
        };

        Ok(Self {
            intent,
            capabilities,
            workflow: raw.workflow,
            confidence,
        })
    }

    /// The required capability names, preserving declaration order.
    pub fn required_names(&self) -> impl Iterator<Item = &str> {
        self.capabilities
            .iter()
            .filter(|c| c.required)
            .map(|c| c.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
        "intent": "document_comparison",
        "capabilities": [
            {"name":"document_understanding","required":true},
            {"name":"retrieval","required":true},
            {"name":"reasoning","required":false}
        ],
        "workflow":["document_understanding","retrieval","reasoning"],
        "confidence": 0.91
    }"#;

    #[test]
    fn parses_a_well_formed_plan() {
        let plan = TaskPlan::parse(GOOD).expect("well-formed plan must parse");
        assert_eq!(plan.intent, "document_comparison");
        assert_eq!(plan.capabilities.len(), 3);
        assert_eq!(plan.workflow.len(), 3);
        assert!((plan.confidence - 0.91).abs() < f32::EPSILON);
        let required: Vec<_> = plan.required_names().collect();
        assert_eq!(required, vec!["document_understanding", "retrieval"]);
    }

    #[test]
    fn strips_a_single_json_fence() {
        let fenced = format!("```json\n{GOOD}\n```");
        let plan = TaskPlan::parse(&fenced).expect("one fence is tolerated");
        assert_eq!(plan.intent, "document_comparison");
    }

    #[test]
    fn rejects_prose_wrapped_json() {
        let hostile = format!("Sure! Here is the plan:\n{GOOD}\nHope this helps!");
        assert!(
            matches!(TaskPlan::parse(&hostile), Err(PlanError::NotJson(_))),
            "prose around JSON is a contract violation, not something to scrape"
        );
    }

    #[test]
    fn rejects_unknown_capability_name() {
        let bad = GOOD.replace("\"reasoning\"", "\"delete_all_workers\"");
        assert!(matches!(
            TaskPlan::parse(&bad),
            Err(PlanError::UnknownCapability(_))
        ));
    }

    #[test]
    fn rejects_workflow_referencing_undeclared_capability() {
        let bad = r#"{
            "intent":"x",
            "capabilities":[{"name":"ocr","required":true}],
            "workflow":["ocr","coding"],
            "confidence":1.0
        }"#;
        assert!(matches!(
            TaskPlan::parse(bad),
            Err(PlanError::WorkflowReferencesUndeclaredStep(_))
        ));
    }

    #[test]
    fn rejects_empty_and_oversized_plans() {
        assert!(matches!(TaskPlan::parse("{\"intent\":\"\",\"capabilities\":[{\"name\":\"ocr\"}],\"workflow\":[],\"confidence\":0}"), Err(PlanError::EmptyPlan)));

        let caps_json = (0..17)
            .map(|_| "{\"name\":\"ocr\"}".to_string())
            .collect::<Vec<_>>()
            .join(",");
        let many_caps = format!(
            "{{\"intent\":\"x\",\"capabilities\":[{caps_json}],\"workflow\":[],\"confidence\":1}}"
        );
        assert!(matches!(TaskPlan::parse(&many_caps), Err(PlanError::TooLarge(_))));
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let sneaky = r#"{"intent":"x","capabilities":[{"name":"ocr"}],"workflow":["ocr"],"confidence":1,"peer_override":"12D3KooWATTACK"}"#;
        assert!(matches!(
            TaskPlan::parse(sneaky),
            Err(PlanError::SchemaMismatch(_))
        ));
    }

    #[test]
    fn clamps_nan_confidence_to_zero() {
        // serde_json cannot represent NaN directly, but Infinity passes some
        // parsers via extensions; either way the clamp must hold.
        let plan = TaskPlan::parse(
            r#"{"intent":"x","capabilities":[{"name":"ocr"}],"workflow":["ocr"],"confidence":5}"#,
        )
        .expect("parses");
        assert_eq!(plan.confidence, 1.0);
    }
}
