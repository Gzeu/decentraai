//! Autonomous experiment selection (v0.3) — the agent's choice, deterministically.
//!
//! The operator starts the cycle with an observation; everything after is
//! decided here, by score, not by hand:
//!
//! ```text
//! observation → candidates (generated) → scored → selected (best valid)
//!   → policy → authorization → execution → learning → curiosity update
//! ```
//!
//! [`score_candidate`] is the whole "intelligence": a pure integer function
//! of information gain, novelty, uncertainty, cost and risk. Highest valid
//! score wins; ties break toward the cheaper experiment, then by id —
//! fully reproducible, fully testable, post-mortem friendly.
//!
//! Anti-loop guards (all deterministic, all tested):
//! duplicate experiments, repetitive hypotheses, same-action replays,
//! budget exhaustion, low-information candidates, endless retry
//! (via attempt counting in the store), one-experiment-per-cycle.

use serde::{Deserialize, Serialize};

use crate::action::ProposedAction;
use crate::budget::{ExperimentBudget, TestnetAsset};
use crate::curiosity::CuriosityState;
use crate::risk::{ExperimentRiskClass, ResourceCommitment};
use crate::store::ExperimentStore;

/// Minimum score for a candidate to be executable. Below this, even the
/// best candidate is rejected: running it would teach nothing.
pub const MIN_EXECUTABLE_SCORE: i64 = 2_000;
/// Risk penalty (score points) for touching testnet. Sandbox/read-only
/// candidates pay nothing.
pub const TESTNET_RISK_PENALTY: i64 = 1_500;
/// Novelty awarded to action signatures never executed before.
pub const NOVELTY_UNSEEN_BP: u32 = 10_000;
/// Novelty for a retest after an inconclusive outcome (same action,
/// different attempt at the question).
pub const NOVELTY_RETEST_BP: u32 = 4_000;

/// One agent-generated candidate, pre-policy. All fields bounded by the
/// same rules as proposals (budgets are minimal-viable by construction:
/// [`CandidateExperiment::minimal_budget`] refuses to inflate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateExperiment {
    /// Candidate id (`cand:<cycle>:<n>`).
    pub id: String,
    /// Hypothesis under test.
    pub hypothesis_id: String,
    /// Hypothesis text (duplicate detection hashes this).
    pub hypothesis_text: String,
    /// What the experiment would do.
    pub action: ProposedAction,
    /// Minimal amount that still exercises the path (0 for read-only).
    pub amount_wei: u64,
    /// Claimed lane.
    pub risk: ExperimentRiskClass,
    /// Declared commitment.
    pub commitment: ResourceCommitment,
    /// Minimal-viable budget for exactly this action.
    pub budget: ExperimentBudget,
    /// Agent-estimated information value if it runs (0..=10000 bp).
    /// A claim, not a fact — scored, then judged by outcomes.
    pub expected_gain_bp: u32,
    /// Why this candidate exists (post-mortem record).
    pub reason: String,
}

/// Score breakdown: every term is inspectable post-mortem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// Agent-claimed information value (bp).
    pub gain_bp: i64,
    /// Novelty vs executed history (bp).
    pub novelty_bp: i64,
    /// Current uncertainty about the hypothesis (bp): the more uncertain,
    /// the more valuable the test.
    pub uncertainty_bp: i64,
    /// Cost penalty: amount share of the cycle budget (bp, negative or zero).
    pub cost_penalty_bp: i64,
    /// Risk penalty (0 or [`TESTNET_RISK_PENALTY`], negative or zero).
    pub risk_penalty_bp: i64,
    /// Total score.
    pub total: i64,
}

/// A scored candidate ready for ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredCandidate {
    /// Candidate.
    pub candidate: CandidateExperiment,
    /// Breakdown.
    pub breakdown: ScoreBreakdown,
}

/// One decision cycle's budget and execution guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleState {
    /// Cycle id.
    pub cycle_id: String,
    /// Max total wei across the whole cycle (all experiments).
    pub max_total_wei: u64,
    /// Wei already spent this cycle.
    pub spent_wei: u64,
    /// Experiment already executed this cycle (one-per-cycle rule).
    pub executed: Option<String>,
}

impl CycleState {
    /// Fresh cycle with an explicit cap.
    #[must_use]
    pub fn new(cycle_id: &str, max_total_wei: u64) -> Self {
        Self {
            cycle_id: cycle_id.to_string(),
            max_total_wei,
            spent_wei: 0,
            executed: None,
        }
    }

    /// Wei still available.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.max_total_wei.saturating_sub(self.spent_wei)
    }
}

/// Why a candidate was filtered before scoring. Invalid is a verdict,
/// not an error — the agent simply must not run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRejection {
    /// Same action signature already submitted/confirmed (replay).
    DuplicateAction {
        /// Candidate id.
        id: String,
    },
    /// Hypothesis already supported by a past experiment (useless repeat).
    RepetitiveHypothesis {
        /// Candidate id.
        id: String,
    },
    /// Amount exceeds the cycle's remaining budget.
    BudgetExhausted {
        /// Candidate id.
        id: String,
    },
    /// Score below [`MIN_EXECUTABLE_SCORE`] (teaches nothing).
    LowInformation {
        /// Candidate id.
        id: String,
        /// Score reached.
        score: i64,
    },
    /// A selection already executed this cycle.
    CycleSpent {
        /// Cycle id.
        cycle_id: String,
    },
}

/// Canonical action signature for duplicate/replay detection:
/// kind + asset + amount + destination. Same signature twice is a replay,
/// regardless of which hypothesis text wraps it.
#[must_use]
pub fn action_signature(action: &ProposedAction) -> String {
    match action {
        ProposedAction::TestnetTransfer {
            asset,
            destination,
            amount_wei,
        } => format!(
            "testnet_transfer:{}:{amount_wei}:{destination}",
            asset.name()
        ),
        other => format!("{}:local", other.kind_name()),
    }
}

/// Novelty of a candidate against executed history and curiosity:
/// unseen → full; seen-but-inconclusive → retest value; seen-and-decided
/// → zero (and the caller rejects it as duplicate/repetitive first).
#[must_use]
pub fn novelty_bp(
    candidate: &CandidateExperiment,
    store: &ExperimentStore,
    curiosity: &CuriosityState,
) -> u32 {
    let sig = action_signature(&candidate.action);
    let mut seen = false;
    let mut inconclusive_only = true;
    for rec in store.records() {
        let rec_sig = match rec.status {
            crate::store::ExperimentStatus::Submitted { .. }
            | crate::store::ExperimentStatus::Confirmed { .. } => {
                // Reconstruct the signature from the durable record.
                format!(
                    "testnet_transfer:{}:{}:{}",
                    rec.asset.name(),
                    rec.amount_wei,
                    rec.destination
                )
            }
            _ => continue,
        };
        if rec_sig == sig {
            seen = true;
            // Any decided outcome (success/fail → hypothesis judged) kills
            // retest value; inconclusive keeps it.
            match curiosity.last_outcome(&candidate.hypothesis_id) {
                Some(crate::evidence::ExperimentOutcome::Inconclusive) | None => {}
                _ => inconclusive_only = false,
            }
        }
    }
    if !seen {
        NOVELTY_UNSEEN_BP
    } else if inconclusive_only {
        NOVELTY_RETEST_BP
    } else {
        0
    }
}

/// The deterministic score: gain + novelty + uncertainty − cost − risk.
/// Pure integer math; same inputs always give the same total.
#[must_use]
pub fn score_candidate(
    candidate: &CandidateExperiment,
    store: &ExperimentStore,
    curiosity: &CuriosityState,
    cycle: &CycleState,
) -> ScoreBreakdown {
    let gain_bp = i64::from(candidate.expected_gain_bp.min(10_000));
    let novelty_bp = i64::from(novelty_bp(candidate, store, curiosity));
    let uncertainty_bp = i64::from(curiosity.uncertainty_bp(&candidate.hypothesis_id));
    let cost_penalty_bp = if cycle.max_total_wei == 0 {
        10_000
    } else {
        -((candidate.amount_wei.saturating_mul(10_000) / cycle.max_total_wei.max(1)) as i64)
            .min(10_000)
    };
    let risk_penalty_bp = if candidate.risk == ExperimentRiskClass::TestnetEconomic {
        -TESTNET_RISK_PENALTY
    } else {
        0
    };
    let total = gain_bp + novelty_bp + uncertainty_bp + cost_penalty_bp + risk_penalty_bp;
    ScoreBreakdown {
        gain_bp,
        novelty_bp,
        uncertainty_bp,
        cost_penalty_bp,
        risk_penalty_bp,
        total,
    }
}

/// The agent's decision: the best VALID candidate, or the reason nothing
/// may run. Validity (anti-loop) before ranking; ranking before thrift:
/// highest score wins, ties break toward the cheaper experiment, then id.
///
/// Carries the full `ExperimentDecision` payload required by the spec:
/// `proposal_id`, `expected_information_gain`, `estimated_cost`, `risk`,
/// `confidence`, `novelty`, `reason`, `selected_action`.
pub struct ExperimentDecision {
    /// Proposal id (`prop:<candidate_id>`).
    pub proposal_id: String,
    /// Agent-claimed information value (bp).
    pub expected_information_gain: u32,
    /// Estimated cost in wei.
    pub estimated_cost: u64,
    /// Risk class of the chosen action.
    pub risk: ExperimentRiskClass,
    /// Confidence in the hypothesis (bp) at decision time.
    pub confidence: u32,
    /// Novelty score (bp).
    pub novelty: u32,
    /// Why this candidate was selected (post-mortem record).
    pub reason: String,
    /// The action the executor must run.
    pub selected_action: ProposedAction,
}

impl ExperimentDecision {
    /// Build an `ExperimentDecision` from a scored winner and its
    /// context. The `proposal_id` is derived from the candidate id;
    /// `confidence` is read from the curiosity state for this hypothesis.
    #[must_use]
    pub fn build(
        candidate: &CandidateExperiment,
        breakdown: &ScoreBreakdown,
        curiosity: &CuriosityState,
        _cycle_id: &str,
    ) -> Self {
        Self {
            proposal_id: format!("prop:{}", candidate.id),
            expected_information_gain: candidate.expected_gain_bp,
            estimated_cost: candidate.amount_wei,
            risk: candidate.risk,
            confidence: curiosity.confidence_bp(&candidate.hypothesis_id),
            novelty: breakdown.novelty_bp as u32,
            reason: candidate.reason.clone(),
            selected_action: candidate.action.clone(),
        }
    }
}

/// The agent's decision: the best VALID candidate, or the reason nothing
/// may run. Validity (anti-loop) before ranking; ranking before thrift:
/// highest score wins, ties break toward the cheaper experiment, then id.
pub fn select_experiment(
    candidates: &[CandidateExperiment],
    store: &ExperimentStore,
    curiosity: &CuriosityState,
    cycle: &CycleState,
) -> Result<ExperimentDecision, CandidateRejection> {
    if let Some(done) = &cycle.executed {
        let _ = done;
        return Err(CandidateRejection::CycleSpent {
            cycle_id: cycle.cycle_id.clone(),
        });
    }
    let mut best: Option<ScoredCandidate> = None;
    let mut first_rejection: Option<CandidateRejection> = None;
    for c in candidates {
        // Anti-loop filter, cheapest checks first.
        if is_duplicate_action(c, store) {
            first_rejection.get_or_insert(CandidateRejection::DuplicateAction { id: c.id.clone() });
            continue;
        }
        if curiosity.is_supported(&c.hypothesis_id) {
            first_rejection
                .get_or_insert(CandidateRejection::RepetitiveHypothesis { id: c.id.clone() });
            continue;
        }
        if c.amount_wei > cycle.remaining() {
            first_rejection.get_or_insert(CandidateRejection::BudgetExhausted { id: c.id.clone() });
            continue;
        }
        let breakdown = score_candidate(c, store, curiosity, cycle);
        if breakdown.total < MIN_EXECUTABLE_SCORE {
            first_rejection.get_or_insert(CandidateRejection::LowInformation {
                id: c.id.clone(),
                score: breakdown.total,
            });
            continue;
        }
        let scored = ScoredCandidate {
            candidate: c.clone(),
            breakdown,
        };
        best = Some(match best {
            None => scored,
            Some(b)
                if scored.breakdown.total > b.breakdown.total
                    || (scored.breakdown.total == b.breakdown.total
                        && (scored.candidate.amount_wei, scored.candidate.id.clone())
                            < (b.candidate.amount_wei, b.candidate.id.clone())) =>
            {
                scored
            }
            Some(b) => b,
        });
    }
    best.map(|s| ExperimentDecision::build(&s.candidate, &s.breakdown, curiosity, &cycle.cycle_id))
        .ok_or_else(|| {
            first_rejection.unwrap_or(CandidateRejection::LowInformation {
                id: String::new(),
                score: 0,
            })
        })
}

/// True when the exact action already submitted/confirmed (replay guard).
fn is_duplicate_action(candidate: &CandidateExperiment, store: &ExperimentStore) -> bool {
    let sig = action_signature(&candidate.action);
    store.records().any(|rec| {
        matches!(
            rec.status,
            crate::store::ExperimentStatus::Submitted { .. }
                | crate::store::ExperimentStatus::Confirmed { .. }
        ) && format!(
            "testnet_transfer:{}:{}:{}",
            rec.asset.name(),
            rec.amount_wei,
            rec.destination
        ) == sig
    })
}

/// Generate the v0.3 candidate set from one real observation and an
/// already-formulated hypothesis.
///
/// Three deterministic rules (mechanical enumeration — the CHOICE is the
/// agent's score, computed in [`select_experiment`]):
/// R1 micro-probe: smallest viable transfer testing confirmation;
/// R2 replication: the previously-seen amount (usually rejected live as
///     duplicate — the anti-loop proving itself);
/// R3 read-only: zero-cost observation check (low gain, always valid).
///
/// `question` and `hypothesis` are produced by [`generate_question`]
/// and [`generate_hypothesis`] before this call — the agent's reasoning
/// chain, not operator hand-off.
pub fn generate_candidates(
    cycle_id: &str,
    observation_id: &str,
    question: &str,
    hypothesis_id: &str,
    hypothesis_text: &str,
    operator_destination: &str,
    now_unix: u64,
) -> Vec<CandidateExperiment> {
    let micro_budget = minimal_budget(
        "budget micro-probe",
        500,
        operator_destination,
        crate::budget::TestnetAsset::Xegld,
        now_unix,
    );
    let replicate_budget = minimal_budget(
        "budget replicate",
        1_000,
        operator_destination,
        crate::budget::TestnetAsset::Xegld,
        now_unix,
    );
    vec![
        CandidateExperiment {
            id: format!("{cycle_id}:micro-probe"),
            hypothesis_id: hypothesis_id.to_string(),
            hypothesis_text: hypothesis_text.to_string(),
            action: ProposedAction::TestnetTransfer {
                asset: TestnetAsset::Xegld,
                destination: operator_destination.to_string(),
                amount_wei: 500,
            },
            amount_wei: 500,
            risk: ExperimentRiskClass::TestnetEconomic,
            commitment: ResourceCommitment::Cr,
            budget: micro_budget,
            expected_gain_bp: 6_000,
            reason: format!(
                "R1 micro-probe: cheapest viable confirmation test for {observation_id} (q: {question})"
            ),
        },
        CandidateExperiment {
            id: format!("{cycle_id}:replicate-1000"),
            hypothesis_id: hypothesis_id.to_string(),
            hypothesis_text: hypothesis_text.to_string(),
            action: ProposedAction::TestnetTransfer {
                asset: TestnetAsset::Xegld,
                destination: operator_destination.to_string(),
                amount_wei: 1_000,
            },
            amount_wei: 1_000,
            risk: ExperimentRiskClass::TestnetEconomic,
            commitment: ResourceCommitment::Cr,
            budget: replicate_budget,
            expected_gain_bp: 3_000,
            reason: format!("R2 replication: same hypothesis, double check (q: {question})"),
        },
        CandidateExperiment {
            id: format!("{cycle_id}:observe-supply"),
            hypothesis_id: hypothesis_id.to_string(),
            hypothesis_text: hypothesis_text.to_string(),
            action: ProposedAction::Observe {
                source: "world".to_string(),
                query: question.to_string(),
            },
            amount_wei: 0,
            risk: ExperimentRiskClass::ReadOnly,
            commitment: ResourceCommitment::None,
            budget: minimal_readonly_budget(now_unix),
            expected_gain_bp: 1_000,
            reason: format!(
                "R3 read-only: zero-cost baseline for {observation_id} (q: {question})"
            ),
        },
    ]
}

/// Detect the highest-uncertainty domain from the agent's curiosity state.
/// Returns the hypothesis id with maximal `uncertainty_bp`.
/// Deterministic: max by (uncertainty desc, hypothesis_id asc).
pub fn detect_uncertainty(curiosity: &CuriosityState) -> String {
    curiosity
        .to_json()
        .ok()
        .and_then(|j| {
            serde_json::from_str::<serde_json::Value>(&j)
                .ok()
                .and_then(|v| v.get("beliefs").and_then(|b| b.as_object()).cloned())
        })
        .and_then(|beliefs| {
            beliefs
                .into_iter()
                .map(|(hid, b)| {
                    let unc = b
                        .get("uncertainty_bp")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(10_000);
                    (hid, unc)
                })
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(hid, _)| hid)
        })
        .unwrap_or_else(|| "hyp:uninitialized".to_string())
}

/// Generate a deterministic question from an observation and the
/// highest-uncertainty domain detected by [`detect_uncertainty`].
///
/// The question is a bounded string (≤256 chars) that identifies
/// WHAT the agent should investigate next. Deterministic given the
/// same (observation_text, highest_uncertainty_domain) pair.
pub fn generate_question(observation_text: &str, highest_uncertainty_domain: &str) -> String {
    let question = format!(
        "does_{observation_text}_{highest_uncertainty_domain}_hold",
        observation_text = observation_text.chars().take(40).collect::<String>(),
        highest_uncertainty_domain = highest_uncertainty_domain
    );
    if question.len() > 256 {
        question[..256].to_string()
    } else {
        question
    }
}

/// Generate a deterministic hypothesis from a question.
///
/// Returns `(hypothesis_id, hypothesis_text)`. Deterministic: the
/// `hypothesis_id` is a stable derivation (`hyp:hash-of-question`),
/// and `hypothesis_text` is a bounded human-readable description.
pub fn generate_hypothesis(question: &str) -> (String, String) {
    let hypothesis_id = format!("hyp:{}", deterministic_hash(question));
    let hypothesis_text = format!(
        "Investigating whether '{}' holds under current network conditions.",
        question.chars().take(80).collect::<String>()
    );
    (hypothesis_id, hypothesis_text)
}

/// Deterministic hex hash helper (pure string derivation).
fn deterministic_hash(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Minimal-viable budget for exactly one transfer: amount == cap, one
/// action, one retry, 1h expiry, single asset + single destination.
/// Anything larger is operator inflation, not agent choice.
fn minimal_budget(
    id: &str,
    amount_wei: u64,
    destination: &str,
    asset: TestnetAsset,
    now_unix: u64,
) -> ExperimentBudget {
    ExperimentBudget {
        id: id.to_string(),
        max_amount_wei: amount_wei,
        max_gas: 60_000,
        max_actions: 1,
        max_retries: 1,
        expiry_unix: now_unix + 3_600,
        allowed_assets: vec![asset],
        allowed_destinations: vec![destination.to_string()],
    }
}

fn minimal_readonly_budget(now_unix: u64) -> ExperimentBudget {
    ExperimentBudget {
        id: "budget readonly".to_string(),
        max_amount_wei: 1,
        max_gas: 1,
        max_actions: 1,
        max_retries: 0,
        expiry_unix: now_unix + 3_600,
        allowed_assets: vec![TestnetAsset::Xegld],
        allowed_destinations: vec!["world".to_string()],
    }
}

/// Post-mortem selection record: candidates, scores, winner, reason.
/// Sealed into evidence so the agent's decision is reconstructable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRecord {
    /// Cycle id.
    pub cycle_id: String,
    /// Every candidate considered with its score (or rejection).
    pub scored: Vec<ScoredSummary>,
    /// Winner id.
    pub selected_id: String,
    /// Winner's reason string.
    pub reason: String,
}

/// One candidate's post-mortem line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoredSummary {
    /// Candidate id.
    pub id: String,
    /// Score total, or rejection name when filtered.
    pub score_or_rejection: String,
}

impl SelectionRecord {
    /// Build from a selection round (winner + all candidates rescored).
    #[must_use]
    pub fn build(
        cycle_id: &str,
        candidates: &[CandidateExperiment],
        store: &ExperimentStore,
        curiosity: &CuriosityState,
        cycle: &CycleState,
        winner: &ExperimentDecision,
    ) -> Self {
        let scored = candidates
            .iter()
            .map(|c| {
                let line = if c.id == winner.proposal_id.trim_start_matches("prop:") {
                    format!(
                        "score={}",
                        (winner.expected_information_gain + winner.novelty) as i64
                    )
                } else if is_duplicate_action(c, store) {
                    "rejected=duplicate_action".to_string()
                } else if curiosity.is_supported(&c.hypothesis_id) {
                    "rejected=repetitive_hypothesis".to_string()
                } else if c.amount_wei > cycle.remaining() {
                    "rejected=budget_exhausted".to_string()
                } else {
                    let b = score_candidate(c, store, curiosity, cycle);
                    if b.total < MIN_EXECUTABLE_SCORE {
                        format!("rejected=low_information({})", b.total)
                    } else {
                        format!("score={}", b.total)
                    }
                };
                ScoredSummary {
                    id: c.id.clone(),
                    score_or_rejection: line,
                }
            })
            .collect();
        Self {
            cycle_id: cycle_id.to_string(),
            scored,
            selected_id: winner.proposal_id.trim_start_matches("prop:").to_string(),
            reason: winner.reason.clone(),
        }
    }
}
