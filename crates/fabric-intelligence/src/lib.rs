//! Fabric Intelligence — the reasoning layer BETWEEN an incoming task and
//! the deterministic DecentraAI fabric planner.
//!
//! Invariant (must never be inverted):
//!
//! ```text
//!   FABRIC INTELLIGENCE  proposes a structured plan
//!          │
//!          ▼
//!   DETERMINISTIC FABRIC validates it against trust/identity/resources
//!          │
//!          ▼
//!   BEST WORKER executes
//! ```
//!
//! Everything in this crate is a PROPOSAL. Nothing here can select a peer,
//! mutate trust, bypass artifact verification or touch configuration. The
//! core (`plan`, `validation`, `policy`, `limits`, `redact`, `telemetry`) is
//! pure and I/O-free so every decision is unit-testable; the provider trait
//! plus its two implementations (local llama.cpp backend, OpenAI-compatible
//! external endpoint) are the only network-touching parts, and they only
//! ever produce raw text that the strict plan parser then treats as
//! UNTRUSTED input.

pub mod limits;
pub mod plan;
pub mod policy;
pub mod provider;
pub mod redact;
pub mod runtime;
pub mod telemetry;
pub mod validation;

pub use limits::{MAX_ARTIFACT_BYTES, RECOMMENDED_ARTIFACT_BYTES};
pub use plan::{PlanCapability, PlanError, TaskPlan};
pub use policy::{ProviderChoice, SelectionPolicy};
pub use provider::{
    IntelligenceProvider, LocalLlamaProvider, OpenAiCompatProvider, ProviderError,
};
pub use runtime::{FabricIntelligence, PlanOutcome};
pub use telemetry::{IntelTelemetry, ProviderKind};
pub use validation::PlanValidation;

use decentraai_hub::capability::CapabilityKind;

/// The intelligence prompt's role: a task classifier / capability router /
/// lightweight planner. It receives the user task plus the taxonomy hint and
/// MUST answer with one JSON object matching [`TaskPlan`]. Anything else is
/// rejected by the parser.
pub const SYSTEM_PROMPT: &str = "You are the Fabric Intelligence of DecentraAI, \
a distributed AI execution fabric. Analyze the user's TASK and answer with \
exactly ONE JSON object and nothing else:\n\
{\"intent\":\"<short_snake_case_intent>\",\"capabilities\":[{\"name\":\"<capability>\",\
\"required\":true}],\"workflow\":[\"<capability>\",...],\"confidence\":0.0}\n\
Rules: capability names MUST come from the ALLOWED list given after the task. \
workflow lists the required capabilities in preferred execution order. \
confidence is your certainty between 0 and 1. No prose, no markdown fences.";

/// One analysis run handed to a provider.
///
/// Deliberately minimal (privacy §15): the provider sees the user task and a
/// static taxonomy hint — never peer identities, model hashes, credentials,
/// audit data or fabric topology.
#[derive(Debug, Clone)]
pub struct TaskBrief<'a> {
    pub task: &'a str,
}

impl TaskBrief<'_> {
    /// Builds the full user message for the provider: system role carries the
    /// contract ([`SYSTEM_PROMPT`]); this message carries the task + the
    /// allowed capability vocabulary.
    pub fn user_message(&self) -> String {
        let names: Vec<&'static str> = CapabilityKind::ALL_NAMES.to_vec();
        format!(
            "TASK: {}\n\nALLOWED capabilities: {}",
            self.task.trim(),
            names.join(", ")
        )
    }
}
