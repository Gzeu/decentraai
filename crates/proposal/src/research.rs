//! Primordial Mind v0.4 — Experiment CONSTRUCTION (not just selection).
//!
//! The agent no longer picks among three fixed rules. It CONSTRUCTS new
//! candidate experiments from observed signals, their deltas against the
//! persisted snapshot, and the family-level learning accumulated in
//! [`crate::curiosity::CuriosityState`].
//!
//! Pipeline (all pure, deterministic, bounded):
//!
//! ```text
//! observation text ──extract_signals──▶ Vec<Signal>
//!     ──delta vs persisted snapshot──▶ Vec<SignalDelta> (sorted)
//!     ──parameter_space──▶ bounded grid (amounts × action kinds)
//!     ──skip exact duplicate signatures──▶ Vec<CandidateExperiment>
//! ```
//!
//! Hypothesis families: every constructed hypothesis id is prefixed
//! `fam:<family>:<hash>` so learning in one family (Supported) suppresses
//! useless repetition across ALL members; inconclusive families accumulate
//! uncertainty and attract the next cycle's budget.
//!
//! Safety: construction never escapes the substrate — every candidate is
//! still scored/validated by `selection.rs`, authorized by the testnet
//! lane, bounded by cycle budget and minimal-viable budgets, and the
//! kill switch remains the operator's. Same anti-loop as v0.3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::action::ProposedAction;
use crate::budget::TestnetAsset;
use crate::curiosity::CuriosityState;
use crate::risk::{ExperimentRiskClass, ResourceCommitment};
use crate::selection::{CandidateExperiment, SuccessCriterion};
use crate::store::ExperimentStore;

/// One extracted numeric signal from an observation text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// Signal key (lowercased word before the number).
    pub key: String,
    /// Signal value.
    pub value: i64,
}

/// Delta of one signal against the persisted previous snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDelta {
    /// Signal key.
    pub key: String,
    /// Previous value (0 when the signal is new).
    pub prev: i64,
    /// Current value.
    pub curr: i64,
    /// `curr − prev` (signed).
    pub delta: i64,
}

/// Persisted observation snapshot: the LAST seen value per signal key.
/// Lives in a JSON file chosen by the operator; the agent reads/writes
/// through it so memory survives restarts (v0.4's longitudinal spine).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationSnapshot {
    /// key → last value.
    pub values: BTreeMap<String, i64>,
}

impl ObservationSnapshot {
    /// Serialize.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }
    /// Reload (fail closed on garbage).
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

/// v0.4 SignalExtractor: pulls `(word, integer)` pairs out of raw text.
/// Deterministic and bounded (≤64 pairs). No NLP, no randomness — the
/// same bytes always yield the same signals, in left-to-right order.
#[must_use]
pub fn extract_signals(text: &str) -> Vec<Signal> {
    let toks: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .filter(|t| !t.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < toks.len() && out.len() < 64 {
        let (k, v) = (toks[i], toks[i + 1]);
        if k.chars().any(|c| c.is_ascii_alphabetic())
            && !k.chars().all(|c| c.is_ascii_digit())
            && v.chars().all(|c| c.is_ascii_digit() || c == '-')
            && let Ok(value) = v.parse::<i64>()
        {
            out.push(Signal {
                key: k.to_ascii_lowercase(),
                value,
            });
            i += 1; // consume the number
        }
        i += 1;
    }
    out
}

/// Compute deltas between the persisted snapshot and fresh signals,
/// then UPDATE the snapshot in place. Deterministic ordering:
/// |delta| desc, then key asc. New keys report `prev = 0`.
#[must_use]
pub fn compute_deltas(snapshot: &mut ObservationSnapshot, signals: &[Signal]) -> Vec<SignalDelta> {
    let mut out = Vec::new();
    for s in signals {
        let prev = snapshot.values.get(&s.key).copied().unwrap_or(0);
        out.push(SignalDelta {
            key: s.key.clone(),
            prev,
            curr: s.value,
            delta: s.value - prev,
        });
        snapshot.values.insert(s.key.clone(), s.value);
    }
    out.sort_by(|a, b| {
        b.delta
            .unsigned_abs()
            .cmp(&a.delta.unsigned_abs())
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// v0.4 hypothesis family: beliefs in `CuriosityState` whose ids share
/// the `fam:<family>:` prefix. Family-level truth for construction:
/// a family with any Supported member is CLOSED (no re-tests);
/// families where everything is inconclusive stay HIGH uncertainty
/// and attract the next cycle.
#[must_use]
pub fn family_closed(curiosity_json: &str, family: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(curiosity_json)
        .ok()
        .and_then(|v| v.get("beliefs").cloned())
        .and_then(|b| serde_json::from_value::<BTreeMap<String, serde_json::Value>>(b).ok())
        .map(|beliefs| {
            beliefs.iter().any(|(k, v)| {
                k.starts_with(&format!("fam:{family}:"))
                    && v.get("last_outcome").is_some_and(|o| o == "success")
            })
        })
        .unwrap_or(false)
}

/// Family uncertainty (max members' uncertainty; 10_000 when unknown).
#[must_use]
pub fn family_uncertainty(curiosity: &CuriosityState, family: &str) -> u32 {
    // Probe a canonical member id; unknown members default to 10_000.
    // Any closed family reports 0 — the matter is settled.
    let probe = format!("fam:{family}:0");
    curiosity.uncertainty_bp(&probe)
}

/// Bounded parameter grid for economic probes (wei). Small enough to
/// stay inside any sane testnet cycle budget; large enough that deltas
/// remain measurable. Never includes amounts above `cycle_max_wei/2`.
const PARAM_AMOUNT_GRID: &[u64] = &[100, 250, 500, 1_000, 2_000];
/// Maximum candidates a cycle may construct (bounded construction).
pub const MAX_CONSTRUCTED: usize = 8;

/// Construct the candidate set for this cycle from deltas + learning.
/// Rules (all deterministic):
///   1. Economic probe: smallest grid amount whose signature is NOT in
///      the store (skip duplicates) and fits the cycle budget. Its
///      hypothesis family is `transfer-health`, keyed by the question
///      hash — a NEW experiment, built from data, not a canned rule.
///   2. Scaling probe: 2× the largest CONFIRMED amount seen in the
///      store (bounded by budget) — learning-driven construction: the
///      last success seeds the next parameter step.
///   3. Delta probe (read-only): if the top delta is non-zero, an
///      Observe candidate whose criterion requires the changed key to
///      appear in the observed text — hypothesis family `signal-delta`.
///
/// Candidates from closed families are skipped entirely (learning has
/// already answered the question).
/// Inputs for one construction pass — bundled to keep the signature
/// honest (7+ free args hide coupling; a struct makes it explicit).
pub struct ConstructInput<'a> {
    /// Cycle id (naming + post-mortem).
    pub cycle_id: &'a str,
    /// Observation id (reason strings).
    pub observation_id: &'a str,
    /// Agent-generated question (families keyed off its hash).
    pub question: &'a str,
    /// Signal deltas vs the persisted snapshot.
    pub deltas: &'a [SignalDelta],
    /// Experiment store (duplicate / learning-aware skipping).
    pub store: &'a ExperimentStore,
    /// Curiosity state (family closure probes).
    pub curiosity: &'a CuriosityState,
    /// v0.5: research journal — dead families (refuted ≥2) are skipped.
    pub journal: Option<&'a crate::journal::ResearchJournal>,
    /// Cycle budget ceiling (economic candidates ⊆ budget/2 each).
    pub cycle_max_wei: u64,
    /// Allow-listed operator destination.
    pub operator_destination: &'a str,
    /// Now (seconds) — expiry timestamps.
    pub now_unix: u64,
}

pub fn construct_candidates(input: &ConstructInput<'_>) -> Vec<CandidateExperiment> {
    let ConstructInput {
        cycle_id,
        observation_id,
        question: _question,
        deltas,
        store,
        curiosity,
        journal,
        cycle_max_wei,
        operator_destination,
        now_unix,
    } = *input;
    let curiosity_json = curiosity.to_json().unwrap_or_default();
    let mut out: Vec<CandidateExperiment> = Vec::new();
    // v0.5: families are STABLE across cycles (they name the research
    // domain, not the question instance) — this is what makes learning
    // longitudinal instead of per-cycle.
    let health_family = "transfer-health".to_string();
    let health_closed = family_closed(&curiosity_json, &health_family)
        || journal.is_some_and(|j| j.family_is_dead(&health_family));

    // (1) Economic probe on the smallest unseen grid amount.
    if !health_closed {
        let mut chosen: Option<u64> = None;
        for &amt in PARAM_AMOUNT_GRID {
            if amt == 0 || amt > cycle_max_wei / 2 {
                continue;
            }
            let sig = format!("testnet_transfer:xegld:{amt}:{operator_destination}");
            let dup = store.records().any(|r| {
                format!(
                    "testnet_transfer:{}:{}:{}",
                    r.asset.name(),
                    r.amount_wei,
                    r.destination
                ) == sig
            });
            if !dup {
                chosen = Some(amt);
                break;
            }
        }
        if let Some(amt) = chosen {
            let hid = format!("fam:{health_family}:probe-{amt}");
            out.push(CandidateExperiment {
                id: format!("{cycle_id}:probe-{amt}"),
                hypothesis_id: hid.clone(),
                hypothesis_text: format!(
                    "A {amt}-wei testnet transfer confirms; transfer health holds"
                ),
                action: ProposedAction::TestnetTransfer {
                    asset: TestnetAsset::Xegld,
                    destination: operator_destination.to_string(),
                    amount_wei: amt,
                },
                criterion: SuccessCriterion::TxConfirmation,
                amount_wei: amt,
                risk: ExperimentRiskClass::TestnetEconomic,
                commitment: ResourceCommitment::Cr,
                budget: crate::selection::minimal_budget(
                    &format!("budget:{cycle_id}:probe-{amt}"),
                    amt,
                    operator_destination,
                    TestnetAsset::Xegld,
                    now_unix,
                ),
                expected_gain_bp: 7_000,
                reason: format!(
                    "constructed: smallest untested transfer amount ({amt} wei), family {health_family}"
                ),
            });
        }
    }

    // (2) Learning-driven scaling probe: 2× largest confirmed amount.
    let max_confirmed: u64 = store
        .records()
        .filter(|r| matches!(r.status, crate::store::ExperimentStatus::Confirmed { .. }))
        .map(|r| r.amount_wei)
        .max()
        .unwrap_or(0);
    if !health_closed && max_confirmed > 0 {
        let amt = max_confirmed.saturating_mul(2);
        if amt <= cycle_max_wei / 2 {
            let sig = format!("testnet_transfer:xegld:{amt}:{operator_destination}");
            let dup = store.records().any(|r| {
                format!(
                    "testnet_transfer:{}:{}:{}",
                    r.asset.name(),
                    r.amount_wei,
                    r.destination
                ) == sig
            });
            if !dup {
                let hid = format!("fam:{health_family}:scale-{amt}");
                out.push(CandidateExperiment {
                    id: format!("{cycle_id}:scale-{amt}"),
                    hypothesis_id: hid,
                    hypothesis_text: format!(
                        "Transfers scale: {amt} wei (2× largest confirmed {max_confirmed}) confirms too"
                    ),
                    action: ProposedAction::TestnetTransfer {
                        asset: TestnetAsset::Xegld,
                        destination: operator_destination.to_string(),
                        amount_wei: amt,
                    },
                    criterion: SuccessCriterion::TxConfirmation,
                    amount_wei: amt,
                    risk: ExperimentRiskClass::TestnetEconomic,
                    commitment: ResourceCommitment::Cr,
                    budget: crate::selection::minimal_budget(
                        &format!("budget:{cycle_id}:scale-{amt}"),
                        amt,
                        operator_destination,
                        TestnetAsset::Xegld,
                        now_unix,
                    ),
                    expected_gain_bp: 6_000,
                    reason: format!(
                        "constructed from learning: last confirmed {max_confirmed} wei → probe 2× scale"
                    ),
                });
            }
        }
    }

    // (3) Delta probe (read-only): the top changed signal must appear
    // in the observation — hypothesis family `signal-delta`.
    let delta_family = "signal-delta".to_string();
    let delta_closed = family_closed(&curiosity_json, &delta_family)
        || journal.is_some_and(|j| j.family_is_dead(&delta_family));
    if delta_closed {
        out.truncate(MAX_CONSTRUCTED);
        return out;
    }
    if let Some(top) = deltas.iter().find(|d| d.delta != 0) {
        out.push(CandidateExperiment {
            id: format!("{cycle_id}:observe-{}", top.key),
            hypothesis_id: format!("fam:{delta_family}:{}", top.key),
            hypothesis_text: format!(
                "Signal '{}' changed {}→{}; the change is re-observable in the text",
                top.key, top.prev, top.curr
            ),
            action: ProposedAction::Observe {
                source: "world".to_string(),
                query: format!("signal {}", top.key),
            },
            criterion: SuccessCriterion::ObservationContains {
                needle: top.key.clone(),
            },
            amount_wei: 0,
            risk: ExperimentRiskClass::ReadOnly,
            commitment: ResourceCommitment::None,
            budget: crate::selection::minimal_readonly_budget(now_unix),
            expected_gain_bp: 4_000,
            reason: format!(
                "constructed from delta: {observation_id} key '{}' moved {}→{}",
                top.key, top.prev, top.curr
            ),
        });
    }

    out.truncate(MAX_CONSTRUCTED);
    out
}
