//! v0.3 autonomous selection: scoring, choice, anti-loop, learning feedback.
//!
//! Proves the agent's decision is deterministic (same inputs → same
//! winner), thrifty (ties go cheaper), loop-safe (duplicates, replays,
//! repeats, exhaustion and spent cycles are all rejected), and adaptive
//! (learning changes the NEXT decision).

use decentraai_proposal::{
    AttemptInfo, CandidateExperiment, CandidateRejection, CuriosityState, CycleState,
    ExperimentOutcome, ExperimentProposal, ExperimentRiskClass, ExperimentStore, HypothesisVerdict,
    ProposedAction, ResourceCommitment, SuccessCriterion, TestnetAsset, detect_uncertainty,
    evaluate_outcome, generate_candidates, generate_hypothesis, generate_question, score_candidate,
    select_experiment,
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
        criterion: decentraai_proposal::SuccessCriterion::TxConfirmation,
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
    assert_eq!(w.proposal_id, "prop:high");
    assert_eq!(w.expected_information_gain, 9_000);
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
    assert_eq!(w.proposal_id, "prop:cheap");
    assert_eq!(w.estimated_cost, 1_000);
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
    assert_eq!(w1.proposal_id, "prop:first");
    // A succeeds → supported → repetitive → next round picks B.
    curiosity.update(&a.hypothesis_id, ExperimentOutcome::Success);
    let w2 = select_experiment(&[a, b], &store, &curiosity, &cy).expect("second");
    assert_eq!(w2.proposal_id, "prop:second");
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
    let cs = generate_candidates(
        "cycle:g",
        "obs:1",
        "does_transfer_work",
        "hyp:transfer",
        "Transfer works",
        DEST,
        NOW,
    );
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

#[test]
fn question_and_hypothesis_generation_deterministic() {
    let mut curiosity = CuriosityState::new();
    curiosity.update("hyp:known", ExperimentOutcome::Success);
    let unc = curiosity.detect_uncertainty();
    assert_eq!(unc, "hyp:known"); // only entry

    let empty_curiosity = CuriosityState::new();
    let unc_default = detect_uncertainty(&empty_curiosity);
    assert_eq!(unc_default, "hyp:uninitialized");

    let q1 = generate_question("treasury burned 187", &unc_default);
    let q2 = generate_question("treasury burned 187", &unc_default);
    assert_eq!(q1, q2);

    let (h_id1, h_text1) = generate_hypothesis(&q1);
    let (h_id2, h_text2) = generate_hypothesis(&q2);
    assert_eq!(h_id1, h_id2);
    assert_eq!(h_text1, h_text2);
    assert!(h_id1.starts_with("hyp:"));
}

// GOLDEN AUTONOMOUS CHOICE: A (cheap, low gain), B (medium cost, high
// gain), C (high cost, high risk). The agent MUST choose B — then, after
// learning marks B's hypothesis supported, the next decision MUST move
// to a different candidate. This proves selection is the agent's own.
#[test]
fn golden_autonomous_choice() {
    // A: observe, cost 0, gain 1000, risk ReadOnly (no risk penalty).
    let a = CandidateExperiment {
        id: "cand:a-cheap-observe".to_string(),
        hypothesis_id: "hyp:a-observability".to_string(),
        hypothesis_text: "observable".to_string(),
        action: ProposedAction::Observe {
            source: "world".to_string(),
            query: "q".to_string(),
        },
        criterion: SuccessCriterion::ObservationContains {
            needle: "x".to_string(),
        },
        amount_wei: 0,
        risk: ExperimentRiskClass::ReadOnly,
        commitment: ResourceCommitment::None,
        budget: decentraai_proposal::ExperimentBudget {
            id: "budget:a".to_string(),
            max_amount_wei: 0,
            max_gas: 0,
            max_actions: 1,
            max_retries: 0,
            expiry_unix: NOW + 3_600,
            allowed_assets: vec![TestnetAsset::Xegld],
            allowed_destinations: vec![DEST.to_string()],
        },
        expected_gain_bp: 1_000,
        reason: "A: cheap low-gain".to_string(),
    };
    // B: transfer 500, gain 6000, Testnet risk.
    let b = micro_candidate("b-medium-high-gain", 6_000, 500);
    // C: transfer 2500, gain 5000, Testnet risk (cost pressure).
    let c = micro_candidate("c-pricey-risky", 5_000, 2_500);
    let store = ExperimentStore::new();
    let mut curiosity = CuriosityState::new();
    let cy = CycleState::new("cycle:golden", 10_000);

    // Scores: A = 1k+10k+10k-0-0 = 21000; B = 6k+10k+10k-500-1500 = 24000;
    // C = 5k+10k+10k-2500-1500 = 21000. B wins (medium cost, high gain).
    let w1 = select_experiment(&[a.clone(), b.clone(), c.clone()], &store, &curiosity, &cy)
        .expect("first pick");
    assert_eq!(
        w1.proposal_id, "prop:b-medium-high-gain",
        "agent must pick B (best information-per-cost), got {}",
        w1.proposal_id
    );

    // B's hypothesis confirmed → supported → repetitive → rejected next.
    curiosity.update(&b.hypothesis_id, ExperimentOutcome::Success);
    let w2 =
        select_experiment(&[a, b, c], &store, &curiosity, &cy).expect("learning flips decision");
    assert_ne!(
        w2.proposal_id, "prop:b-medium-high-gain",
        "supported hypothesis must not be re-run"
    );
    assert_eq!(
        w2.proposal_id, "prop:cand:a-cheap-observe",
        "tie at 21000 → cheaper (free observe) wins over the pricey one"
    );
}

// Verdict inference: execution success ≠ hypothesis supported; the
// criterion decides, never the operator.
#[test]
fn verdict_inferred_not_declared() {
    // Tx confirmed success → Supported.
    assert_eq!(
        evaluate_outcome(
            &SuccessCriterion::TxConfirmation,
            Some("success"),
            "anything"
        ),
        HypothesisVerdict::Supported
    );
    // Tx failed → Refuted (execution DID run, hypothesis disproven).
    assert_eq!(
        evaluate_outcome(&SuccessCriterion::TxConfirmation, Some("fail"), "anything"),
        HypothesisVerdict::Refuted
    );
    // No tx (read-only lane) on a tx criterion → Inconclusive.
    assert_eq!(
        evaluate_outcome(&SuccessCriterion::TxConfirmation, None, "anything"),
        HypothesisVerdict::Inconclusive
    );
    // Observation needle present → Supported; absent → Inconclusive.
    let crit = SuccessCriterion::ObservationContains {
        needle: "minted".to_string(),
    };
    assert_eq!(
        evaluate_outcome(&crit, None, "treasury minted 4530"),
        HypothesisVerdict::Supported
    );
    assert_eq!(
        evaluate_outcome(&crit, None, "treasury empty"),
        HypothesisVerdict::Inconclusive
    );
}

// Candidates carry THREE DISTINCT hypotheses (not one, three actions).
#[test]
fn generator_emits_three_distinct_hypotheses() {
    let cs = generate_candidates(
        "cycle:g",
        "obs:1",
        "does_x_hold",
        "hyp:root",
        "root hyp",
        DEST,
        NOW,
    );
    assert_eq!(cs.len(), 3);
    let mut ids: Vec<&str> = cs.iter().map(|c| c.hypothesis_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each candidate must test its own hypothesis");
}
