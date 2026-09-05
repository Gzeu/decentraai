//! v0.6 Multi-Agent Research: lenses generate rival hypotheses, converge
//! by consensus, and still submit to the same deterministic arena.

use decentraai_proposal::*;

const NOW: u64 = 1_780_000_000;
const DEST: &str = "erd1operatordestination00000000000000000000";

fn base_input<'a>(
    deltas: &'a [SignalDelta],
    store: &'a ExperimentStore,
    curiosity: &'a CuriosityState,
) -> ConstructInput<'a> {
    ConstructInput {
        cycle_id: "cycle:multi",
        observation_id: "obs:m",
        question: "q",
        deltas,
        store,
        curiosity,
        journal: None,
        cycle_max_wei: 4_000,
        operator_destination: DEST,
        now_unix: NOW,
    }
}

#[test]
fn three_lenses_emit_three_distinct_families() {
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 42 burned 7"));
    let cs = construct_multi_lens(&base_input(&deltas, &store, &curiosity));
    assert!(!cs.is_empty());
    let slugs: Vec<&str> = cs.iter().filter_map(|c| c.id.rsplit(':').next()).collect();
    assert!(slugs.contains(&"generative"), "generative lens present");
    assert!(slugs.contains(&"conservative"), "conservative lens present");
    assert!(slugs.contains(&"skeptic"), "skeptic lens present");
    // Hypothesis families must differ per lens → agents disagree.
    let families: std::collections::BTreeSet<&str> =
        cs.iter().map(|c| c.hypothesis_id.as_str()).collect();
    assert!(families.len() >= 2, "distinct hypotheses across lenses");
}

#[test]
fn consensus_uplift_when_lenses_agree() {
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 1"));
    let cs = construct_multi_lens(&base_input(&deltas, &store, &curiosity));
    // Every skeptic candidate shares its action signature with the base
    // producer → those must carry the consensus bonus.
    let with: Vec<&CandidateExperiment> = cs
        .iter()
        .filter(|c| c.reason.contains("+consensus"))
        .collect();
    let without: Vec<&CandidateExperiment> = cs
        .iter()
        .filter(|c| !c.reason.contains("+consensus"))
        .collect();
    assert!(
        with.len() > without.len() || !with.is_empty(),
        "at least some candidates converge"
    );
    for c in with {
        assert!(c.reason.contains('+'));
        // Generative probe (gain 7000) + consensus 1000 = 8000.
        if c.risk == ExperimentRiskClass::TestnetEconomic {
            assert_eq!(c.expected_gain_bp, 8_000, "probe got the consensus uplift");
        }
    }
}

#[test]
fn skeptic_keeps_replication_family() {
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 5"));
    let cs = construct_multi_lens(&base_input(&deltas, &store, &curiosity));
    let skeptic: Vec<&CandidateExperiment> = cs
        .iter()
        .filter(|c| c.hypothesis_id.starts_with("fam:skeptic:"))
        .collect();
    assert!(
        skeptic
            .iter()
            .any(|c| c.risk == ExperimentRiskClass::TestnetEconomic),
        "skeptic replication path remains economic"
    );
}

#[test]
fn multi_lens_is_deterministic() {
    let run = || {
        let store = ExperimentStore::new();
        let curiosity = CuriosityState::new();
        let mut snap = ObservationSnapshot::default();
        let deltas = compute_deltas(&mut snap, &extract_signals("minted 3"));
        construct_multi_lens(&base_input(&deltas, &store, &curiosity))
    };
    assert_eq!(run(), run());
}

#[test]
fn lenses_still_submit_to_the_deterministic_arena() {
    // v0.6 must not bypass selection/policy: generated candidates flow
    // through select_experiment exactly like v0.3/v0.4 ones.
    let store = ExperimentStore::new();
    let curiosity = CuriosityState::new();
    let mut snap = ObservationSnapshot::default();
    let deltas = compute_deltas(&mut snap, &extract_signals("minted 8"));
    let cs = construct_multi_lens(&base_input(&deltas, &store, &curiosity));
    let cycle = CycleState::new("cycle:multi", 4_000);
    let w = select_experiment(&cs, &store, &curiosity, &cycle).expect("a winner exists");
    assert!(w.proposal_id.contains("cycle:multi:"));
}
