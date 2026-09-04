//! v0.3 autonomous selection: scoring, choice, anti-loop, learning feedback.
//!
//! Proves the agent's decision is deterministic (same inputs → same
//! winner), thrifty (ties go cheaper), loop-safe (duplicates, replays,
//! repeats, exhaustion and spent cycles are all rejected), and adaptive
//! (learning changes the NEXT decision).

use decentraai_proposal::{
    AttemptInfo, CandidateExperiment, CandidateRejection, CuriosityState, CycleState,
    ExperimentOutcome, ExperimentProposal, ExperimentRiskClass, ExperimentStore, ProposedAction,
    ResourceCommitment, TestnetAsset, generate_candidates, score_candidate, select_experiment,
};

const NOW: u64 = 1_780_000_000;
const DEST: &str = "erd1operatordestination00000000000000000000";

fn micro_candidate(id: &str, gain: u32, amount: u64) -> CandidateExperiment {
    CandidateExperiment {
        id: id.to_string(),
        hypothesis_id: format!("hyp:{id}"),
        hypothesis_text: format!("hypothesis for {id}"),
        action: ProposedAction::TestnetTransfer {
            asset: TestnetAsset::Xegld,
            destination: DEST.to_string(),
            amount_wei: amount,
        },
        amount_wei: amount,
        risk: ExperimentRiskClass::TestnetEconomic,
        commitment: ResourceCommitment::Cr,
        budget: decentraai_proposal::ExperimentBudget {
            id: format!("budget:{id}"),
            max_amount_wei: amount,
            max_gas: 60_000,
            max_actions: 1,
            max_retries: 1,
            expiry_unix: NOW + 3_600,
            allowed_assets: vec![TestnetAsset::Xegld],
            allowed_destinations: vec![DEST.to_string()],
        },
        expected_gain_bp: gain,
        reason: format!("test candidate {id}"),
    }
}

fn cycle() -> CycleState {
    CycleState::new("cycle:test", 10_000)
}

fn seed_submitted(store: &mut ExperimentStore, sig_amount: u64, dest: &str) {
    let proposal = ExperimentProposal {
        version: 1,
        id: "prop:seed".to_string(),
        idea_id: "idea:seed".to_string(),
        risk: ExperimentRiskClass::TestnetEconomic,
        commitment: ResourceCommitment::Cr,
        budget: None,
        steps: vec![],
        created_by: "t".to_string(),
    };
    store.record_attempt(
        "exp:seed",
        AttemptInfo {
            proposal: &proposal,
            budget_id: "b",
            asset: &TestnetAsset::Xegld,
            destination: dest,
            amount_wei: sig_amount,
            attempts_used: 1,
            now_ms: 1,
        },
    );
    store.mark_submitted("exp:seed", "tx:seed", sig_amount, 2);
    store.mark_confirmed("exp:seed", "tx:seed", 3);
}

// Scoring is deterministic: same inputs, same breakdown.
#[test]
fn scoring_deterministic() {
    let c = micro_candidate("a", 6_000, 500);
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let cy = cycle();
    assert_eq!(
        score_candidate(&c, &store, &curiosity, &cy),
        score_candidate(&c, &store, &curiosity, &cy)
    );
}

// Best candidate wins by total.
#[test]
fn best_candidate_selection() {
    let cs = vec![
        micro_candidate("low", 1_000, 500),
        micro_candidate("high", 9_000, 500),
        micro_candidate("mid", 5_000, 500),
    ];
    let w = select_experiment(
        &cs,
        &ExperimentStore::new(),
        &CuriosityState::new(),
        &cycle(),
    )
    .expect("selects");
    assert_eq!(w.candidate.id, "high");
}

// Ties break toward the cheaper experiment.
#[test]
fn minimum_cost_selection() {
    // A: 7000 + 10000 + 10000 − 2000 − 1500 = 23500 (amount 2000/10000).
    // B: 6000 + 10000 + 10000 − 1000 − 1500 = 23500 (amount 1000/10000).
    let a = micro_candidate("pricey", 7_000, 2_000);
    let b = micro_candidate("cheap", 6_000, 1_000);
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let cy = cycle();
    assert_eq!(
        score_candidate(&a, &store, &curiosity, &cy).total,
        score_candidate(&b, &store, &curiosity, &cy).total
    );
    let w = select_experiment(&[a, b], &store, &curiosity, &cy).expect("selects");
    assert_eq!(w.candidate.id, "cheap");
}

// Duplicate action (same sig as a confirmed experiment) is rejected.
#[test]
fn duplicate_rejection() {
    let mut store = ExperimentStore::new();
    seed_submitted(&mut store, 500, DEST);
    let c = micro_candidate("dup", 9_000, 500);
    assert!(matches!(
        select_experiment(&[c], &store, &CuriosityState::new(), &cycle()),
        Err(CandidateRejection::DuplicateAction { .. })
    ));
}

// Same action replay under a NEW hypothesis is still a replay.
#[test]
fn replay_rejection() {
    let mut store = ExperimentStore::new();
    seed_submitted(&mut store, 500, DEST);
    let mut c = micro_candidate("replay", 9_000, 500);
    c.hypothesis_id = "hyp:brand-new-question".to_string();
    assert!(matches!(
        select_experiment(&[c], &store, &CuriosityState::new(), &cycle()),
        Err(CandidateRejection::DuplicateAction { .. })
    ));
}

// Supported hypotheses are repetitive: no useless repeats.
#[test]
fn repetitive_hypothesis_rejection() {
    let mut curiosity = CuriosityState::new();
    curiosity.update("hyp:cheap", ExperimentOutcome::Success);
    let c = micro_candidate("cheap", 9_000, 500);
    assert!(matches!(
        select_experiment(&[c], &ExperimentStore::new(), &curiosity, &cycle()),
        Err(CandidateRejection::RepetitiveHypothesis { .. })
    ));
}

// Amount beyond the cycle remainder is exhausted, not scored.
#[test]
fn budget_exhaustion() {
    let mut cy = cycle();
    cy.spent_wei = 9_900;
    let c = micro_candidate("too-big", 9_000, 500);
    assert!(matches!(
        select_experiment(&[c], &ExperimentStore::new(), &CuriosityState::new(), &cy),
        Err(CandidateRejection::BudgetExhausted { .. })
    ));
}

// One experiment per cycle: a spent cycle selects nothing.
#[test]
fn one_experiment_per_cycle() {
    let mut cy = cycle();
    cy.executed = Some("exp:done".to_string());
    let c = micro_candidate("late", 9_000, 500);
    assert!(matches!(
        select_experiment(&[c], &ExperimentStore::new(), &CuriosityState::new(), &cy),
        Err(CandidateRejection::CycleSpent { .. })
    ));
}

// Learning changes the next decision: A wins, succeeds, then B wins.
#[test]
fn learning_changes_next_decision() {
    let a = micro_candidate("first", 9_000, 500);
    let b = micro_candidate("second", 5_000, 500);
    let store = ExperimentStore::new();
    let mut curiosity = CuriosityState::new();
    let cy = cycle();
    let w1 = select_experiment(&[a.clone(), b.clone()], &store, &curiosity, &cy).expect("first");
    assert_eq!(w1.candidate.id, "first");
    // A succeeds → supported → repetitive → next round picks B.
    curiosity.update(&a.hypothesis_id, ExperimentOutcome::Success);
    let w2 = select_experiment(&[a, b], &store, &curiosity, &cy).expect("second");
    assert_eq!(w2.candidate.id, "second");
}

// Inconclusive keeps curiosity high: uncertainty still rewards retests.
#[test]
fn uncertainty_update() {
    let mut curiosity = CuriosityState::new();
    assert_eq!(curiosity.uncertainty_bp("h"), 10_000);
    curiosity.update("h", ExperimentOutcome::Success);
    assert_eq!(curiosity.uncertainty_bp("h"), 2_000);
    assert_eq!(curiosity.confidence_bp("h"), 8_000);
    let mut c2 = CuriosityState::new();
    c2.update("h", ExperimentOutcome::Inconclusive);
    assert!(c2.uncertainty_bp("h") >= 8_000);
}

// Generator emits the three v0.3 rules with minimal budgets.
#[test]
fn generator_rules_and_minimal_budgets() {
    let cs = generate_candidates("cycle:g", "obs:1", DEST, NOW);
    assert_eq!(cs.len(), 3);
    for c in &cs {
        assert!(c.budget.max_amount_wei >= c.amount_wei);
        assert!(c.budget.max_actions >= 1);
    }
    let micro = cs.iter().find(|c| c.id.ends_with("micro-probe")).unwrap();
    assert_eq!(micro.amount_wei, 500);
    assert_eq!(
        micro.budget.max_amount_wei, 500,
        "minimal viable, not inflated"
    );
}
