//! v0.4 construction: the agent BUILDS experiments from signals, deltas
//! and learning — it does not pick among fixed rules.

use decentraai_proposal::*;

const NOW: u64 = 1_780_000_000;
const DEST: &str = "erd1operatordestination00000000000000000000";

fn seed_confirmed(store: &mut ExperimentStore, amount: u64) {
    let proposal = ExperimentProposal {
        version: 1,
        id: format!("prop:seed-{amount}"),
        idea_id: format!("idea:{amount}"),
        risk: ExperimentRiskClass::TestnetEconomic,
        commitment: ResourceCommitment::Cr,
        budget: None,
        steps: vec![],
        created_by: "t".to_string(),
    };
    store.record_attempt(
        &format!("exp:seed-{amount}"),
        AttemptInfo {
            proposal: &proposal,
            budget_id: "b",
            asset: &TestnetAsset::Xegld,
            destination: DEST,
            amount_wei: amount,
            attempts_used: 1,
            now_ms: 1,
        },
    );
    store.mark_submitted(&format!("exp:seed-{amount}"), "tx:seed", amount, 2);
    store.mark_confirmed(&format!("exp:seed-{amount}"), "tx:seed", 3);
}

#[test]
fn signal_extractor_deterministic_and_bounded() {
    let a = extract_signals("treasury minted 5250, burned 225, tick 313");
    let b = extract_signals("treasury minted 5250, burned 225, tick 313");
    assert_eq!(a, b);
    assert_eq!(a.len(), 3);
    assert_eq!(a[0].key, "minted");
    assert_eq!(a[0].value, 5_250);
}

#[test]
fn deltas_sort_by_magnitude_then_key() {
    let mut snap = ObservationSnapshot::default();
    let first = extract_signals("minted 100 burned 10");
    let _ = compute_deltas(&mut snap, &first);
    let second = extract_signals("minted 100 burned 60");
    let deltas = compute_deltas(&mut snap, &second);
    assert_eq!(deltas[0].key, "burned");
    assert_eq!(deltas[0].delta, 50);
    assert_eq!(deltas[1].delta, 0);
}

#[test]
fn construction_builds_novel_probe_and_scaling_from_learning() {
    let mut store = ExperimentStore::new();
    seed_confirmed(&mut store, 500);
    seed_confirmed(&mut store, 1_000);
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(
        &mut snap,
        &extract_signals("treasury minted 5250 burned 225"),
    );
    let cs = construct_candidates(&ConstructInput {
        cycle_id: "cycle:v4",
        observation_id: "obs:x",
        question: "why-did-minted-change",
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: None,
        cycle_max_wei: 4_000,
        operator_destination: DEST,
        now_unix: NOW,
    });
    // Smallest unseen grid amount (100 and 250 are unseen; grid order
    // picks 100 first) + scale probe 2000 (2× largest confirmed 1000)
    // + delta observe probe.
    let ids: Vec<&str> = cs.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"cycle:v4:probe-100"), "novel probe: {ids:?}");
    assert!(
        ids.contains(&"cycle:v4:scale-2000"),
        "learning-driven scaling: {ids:?}"
    );
    assert!(
        ids.contains(&"cycle:v4:observe-minted"),
        "delta observer: {ids:?}"
    );
    // NO duplicate of the confirmed 500/1000 amounts may appear.
    assert!(!ids.contains(&"cycle:v4:probe-500"));
    assert!(!ids.contains(&"cycle:v4:probe-1000"));
    // Every candidate budget is minimal-viable.
    for c in &cs {
        assert!(c.budget.max_amount_wei >= c.amount_wei);
    }
}

#[test]
fn construction_respects_cycle_budget() {
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 5"));
    let cs = construct_candidates(&ConstructInput {
        cycle_id: "cycle:v4",
        observation_id: "obs:x",
        question: "q",
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: None,
        cycle_max_wei: 100, // max 50 wei → below the 100-wei grid floor
        operator_destination: DEST,
        now_unix: NOW,
    });
    assert!(
        cs.iter().all(|c| c.amount_wei == 0),
        "no economic candidate under a 100-wei ceiling"
    );
}

#[test]
fn family_closes_after_learning_supported() {
    // Learning: mark a transfer-health family member Supported.
    let mut curiosity = CuriosityState::new();
    let q = "why-did-minted-change";
    // Reproduce the family hash by probing which hypothesis id the
    // constructor emits, then mark it — the NEXT construction must
    // produce NO economic candidate for that family.
    let store = ExperimentStore::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 9"));
    let first = construct_candidates(&ConstructInput {
        cycle_id: "cycle:1",
        observation_id: "obs",
        question: q,
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: None,
        cycle_max_wei: 4_000,
        operator_destination: DEST,
        now_unix: NOW,
    });
    let probe = first
        .iter()
        .find(|c| c.hypothesis_id.contains("transfer-health"))
        .expect("probe exists");
    curiosity.update(&probe.hypothesis_id, ExperimentOutcome::Success);

    let second = construct_candidates(&ConstructInput {
        cycle_id: "cycle:2",
        observation_id: "obs",
        question: q,
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: None,
        cycle_max_wei: 4_000,
        operator_destination: DEST,
        now_unix: NOW,
    });
    assert!(
        second
            .iter()
            .all(|c| !c.hypothesis_id.contains("transfer-health")),
        "closed family yields no new economic candidates"
    );
    // But the delta family is still open → read-only candidate remains.
    assert!(
        second
            .iter()
            .any(|c| c.risk == ExperimentRiskClass::ReadOnly),
        "delta family still constructible"
    );
}

#[test]
fn construction_is_deterministic() {
    let run = || {
        let store = ExperimentStore::new();
        let curiosity = CuriosityState::new();
        let mut snap = ObservationSnapshot::default();
        let deltas = compute_deltas(&mut snap, &extract_signals("minted 42 burned 7"));
        construct_candidates(&ConstructInput {
            cycle_id: "cycle:d",
            observation_id: "obs",
            question: "q-why",
            deltas: &deltas,
            store: &store,
            curiosity: &curiosity,
            journal: None,
            cycle_max_wei: 4_000,
            operator_destination: DEST,
            now_unix: NOW,
        })
    };
    assert_eq!(run(), run());
}

#[test]
fn constructed_candidates_pass_through_selection() {
    let mut store = ExperimentStore::new();
    seed_confirmed(&mut store, 500);
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 42"));
    let cs = construct_candidates(&ConstructInput {
        cycle_id: "cycle:v4",
        observation_id: "obs",
        question: "q",
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: None,
        cycle_max_wei: 4_000,
        operator_destination: DEST,
        now_unix: NOW,
    });
    let cycle = CycleState::new("cycle:v4", 4_000);
    let winner = select_experiment(&cs, &store, &curiosity, &cycle).expect("selection works");
    assert!(
        winner.proposal_id.starts_with("prop:cycle:v4:"),
        "constructed winner flows through selection: {}",
        winner.proposal_id
    );
}
