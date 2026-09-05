//! v0.6 — Multi-Agent Research: the same problem, seen by several lenses.
//!
//! No agent researches alone any more. Bounded LENSES act as distinct
//! researcher personas inside the deterministic substrate; each looks at
//! the same observation + journal + curiosity and CONSTRUCTS its own
//! candidates with its own hypothesis family and epistemic stance:
//!
//! - `Generative`   — pushes the parameter frontier (novel amounts first);
//! - `Conservative` — prefers read-only evidence (delta probes);
//! - `Skeptic`      — demands replication (re-test the largest confirmed).
//!
//! They disagree by construction (distinct hypothesis families), they can
//! CONVERGE: when two lenses emit the same action signature, both get a
//! consensus gain uplift — evidence of independent agreement. Selection,
//! policy, authorization, budgets and anti-loop are unchanged and shared:
//! lenses compete INSIDE the deterministic arena, never around it.

use crate::research::{ConstructInput, construct_candidates};
use crate::risk::ExperimentRiskClass;
use crate::selection::{CandidateExperiment, action_signature};

/// The researcher lenses. Closed set; new lenses land behind tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    /// Pushes the parameter frontier.
    Generative,
    /// Prefers read-only, observation-bound evidence.
    Conservative,
    /// Demands replication of existing confirmations.
    Skeptic,
}

impl Lens {
    /// All built-in lenses, deterministic order.
    pub const ALL: &'static [Lens] = &[Lens::Generative, Lens::Conservative, Lens::Skeptic];

    /// Stable slug (used in hypothesis families and candidate ids).
    #[must_use]
    pub fn slug(&self) -> &'static str {
        match self {
            Lens::Generative => "generative",
            Lens::Conservative => "conservative",
            Lens::Skeptic => "skeptic",
        }
    }
}

/// Consensus: identical action signatures produced by ≥2 lenses get a
/// shared gain uplift (independent agreement is evidence, not noise).
pub const CONSENSUS_GAIN_BONUS_BP: u32 = 1_000;

/// Bounded multi-agent construction: run the base constructor, then stamp
/// each candidate with its lens-authored family/id prefixes, and apply
/// consensus uplift where lenses agree on an action signature.
pub fn construct_multi_lens(input: &ConstructInput<'_>) -> Vec<CandidateExperiment> {
    let base = construct_candidates(input);
    let mut out: Vec<CandidateExperiment> = Vec::new();

    for lens in Lens::ALL {
        for base_cand in &base {
            // Lens stance filter (deterministic):
            // - Generative keeps economic probes (the frontier);
            // - Conservative keeps read-only probes (certainty first);
            // - Skeptic keeps everything but marks replication families.
            let keep = match lens {
                Lens::Generative => base_cand.risk == ExperimentRiskClass::TestnetEconomic,
                Lens::Conservative => base_cand.risk == ExperimentRiskClass::ReadOnly,
                Lens::Skeptic => true,
            };
            if !keep {
                continue;
            }
            let mut c = base_cand.clone();
            // Lens-authored identity: the hypothesis family becomes
            //   fam:<lens>:<original-family> — agents can disagree and
            //   their learning stays separable in curiosity + journal.
            let orig_family = crate::journal::family_of(&c.hypothesis_id);
            c.hypothesis_id = match orig_family.is_empty() {
                true => format!("fam:{}:{}", lens.slug(), c.hypothesis_id),
                false => format!(
                    "fam:{}:{}",
                    lens.slug(),
                    c.hypothesis_id.trim_start_matches("fam:")
                ),
            };
            c.id = format!("{}:{}", c.id, lens.slug());
            c.reason = format!("[lens={}] {}", lens.slug(), c.reason);
            out.push(c);
        }
    }

    // Consensus uplift: signatures appearing under ≥2 lenses gain.
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    for c in &out {
        let sig = action_signature(&c.action);
        *counts.entry(sig).or_default() += 1;
    }
    for c in &mut out {
        let sig = action_signature(&c.action);
        if counts.get(&sig).is_some_and(|&n| n >= 2) {
            c.expected_gain_bp = c.expected_gain_bp.saturating_add(CONSENSUS_GAIN_BONUS_BP);
            c.reason = format!("{} (+consensus)", c.reason);
        }
    }

    out.truncate(crate::research::MAX_CONSTRUCTED * Lens::ALL.len());
    out
}

/// Deterministic replicated-observation helper exposed for Skeptic
/// lenses in tests: identical candidates under two lenses must hash
/// identically via [`action_signature`].
#[must_use]
pub fn lenses_agree(a: &CandidateExperiment, b: &CandidateExperiment) -> bool {
    action_signature(&a.action) == action_signature(&b.action)
}
