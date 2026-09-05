//! Golden end-to-end: World event → autonomous decision → World mission
//! → evidence → learning → new World event.
//!
//! Pure and deterministic (no network, no wall-clock, no randomness).
//! Mirrors the operator (`node-cli autonomous-cycle --world-url`) step for
//! step: cursor gate → question → multi-lens construction → selection →
//! mission projection → verdict inference → curiosity/journal/graph/activity
//! seal → cursor advance → next tick re-gates on the NEW World event.

use decentraai_proposal::{
    ActivityLedger, CuriosityState, CycleState, ExperimentOutcome, HypothesisVerdict,
    ResearchActivity, ResearchGraph, ResearchJournal, ResearchTrace, WorldCursor, assign_lenses,
    compute_deltas, construct_multi_lens, diff_world, economy_note, evaluate_outcome,
    extract_signals, generate_hypothesis, generate_question, parse_world_view, select_experiment,
    ConstructInput, ExperimentStore, ObservationSnapshot,
};
use std::collections::BTreeMap;

fn snapshot_a() -> serde_json::Value {
    serde_json::json!({
        "tick": 100,
        "entities": [{"id": "agent-b"}, {"id": "agent-a"}],
        "events": [{"kind": "research_asked", "tick": 100}],
        "mission_task_id": null,
        "treasury_minted": 10,
        "treasury_burned": 1,
        "locations": [
            {"id": "research-lab-main", "services": {"research": 20, "inference": 5}},
            {"id": "forge-workshop", "services": {"coding": 25}}
        ]
    })
}

fn snapshot_b() -> serde_json::Value {
    // The sealed mission from cycle 1 is now a REAL World event + mission
    // pointer: this is the "→ new World event" half of the golden chain.
    serde_json::json!({
        "tick": 101,
        "entities": [{"id": "agent-b"}, {"id": "agent-a"}],
        "events": [
            {"kind": "research_asked", "tick": 100},
            {"kind": "task_published", "tick": 101}
        ],
        "mission_task_id": "task-test-1",
        "treasury_minted": 10,
        "treasury_burned": 1,
        "locations": [
            {"id": "research-lab-main", "services": {"research": 20, "inference": 5}},
            {"id": "forge-workshop", "services": {"coding": 25}}
        ]
    })
}

fn obs_text(view_tick: u64, entities: usize, events: usize, mission: u64, minted: u64, burned: u64) -> String {
    format!(
        "world tick {view_tick} entities {entities} events {events} mission {mission} minted {minted} burned {burned}"
    )
}

#[test]
fn golden_world_event_to_learning_to_new_world_event() {
    // ── Tick 1: fresh cursor always researches ──────────────────────────
    let cursor = WorldCursor::default();
    assert!(cursor.is_fresh());
    let view_a = parse_world_view(&snapshot_a()).unwrap();
    assert_eq!(view_a.tick, 100);
    assert_eq!(view_a.entity_ids, vec!["agent-a".to_string(), "agent-b".to_string()]);
    let delta_a = diff_world(&cursor, &view_a);
    assert!(delta_a.should_research, "fresh cursor must research: {}", delta_a.reason);

    // Observation shaped exactly like world_bridge::observation_from_world.
    let obs_id_a = format!("obs:world:{}", view_a.tick);
    let obs_text_a = obs_text(100, 2, 1, 0, 10, 1);

    // Reasoning chain: uncertainty → question → hypothesis.
    let mut curiosity = CuriosityState::new();
    let mut journal = ResearchJournal::new();
    let store = ExperimentStore::new();
    let mut snap_mem = ObservationSnapshot::default();
    let domain_a = curiosity.detect_uncertainty();
    let question_a = generate_question(&obs_text_a, &domain_a);
    let (hyp_a, _hyp_text_a) = generate_hypothesis(&question_a);
    assert!(!hyp_a.is_empty());

    // Multi-lens construction + deterministic selection.
    let deltas_a = compute_deltas(&mut snap_mem, &extract_signals(&obs_text_a));
    assert!(!deltas_a.is_empty());
    let cycle = CycleState::new("cycle:golden-001", 2_000);
    let candidates_a = construct_multi_lens(&ConstructInput {
        cycle_id: "cycle:golden-001",
        observation_id: &obs_id_a,
        question: &question_a,
        deltas: &deltas_a,
        store: &store,
        curiosity: &curiosity,
        journal: Some(&journal),
        cycle_max_wei: 2_000,
        operator_destination: "erd1goldenoperator",
        now_unix: 1_700_000_000,
    });
    assert!(!candidates_a.is_empty(), "agent must construct candidates");
    let winner_a = select_experiment(&candidates_a, &store, &curiosity, &cycle)
        .expect("agent must select a winner");
    // Determinism: same inputs → same winner.
    let winner_a2 = select_experiment(&candidates_a, &store, &curiosity, &cycle).unwrap();
    assert_eq!(winner_a.proposal_id, winner_a2.proposal_id);

    // Mission projection: the World would return a task id here
    // (409-keep is the None arm — also covered below).
    let mission_task: Option<String> = Some("task-test-1".to_string());

    // Verdict inference exactly like the operator's read-only lane for
    // read-only winners, or a confirmed testnet success for economic ones.
    let verdict_a = if winner_a.risk == decentraai_proposal::ExperimentRiskClass::ReadOnly {
        evaluate_outcome(&winner_a.criterion, None, &obs_text_a)
    } else {
        evaluate_outcome(&winner_a.criterion, Some("success"), &obs_text_a)
    };
    assert_eq!(
        verdict_a,
        HypothesisVerdict::Supported,
        "golden text contains the delta needle / tx confirms — first cycle supports"
    );
    let outcome_a = match verdict_a {
        HypothesisVerdict::Supported => ExperimentOutcome::Success,
        HypothesisVerdict::Refuted => ExperimentOutcome::Failed,
        HypothesisVerdict::Inconclusive => ExperimentOutcome::Inconclusive,
    };
    curiosity.update(&winner_a.hypothesis_id, outcome_a);
    assert!(curiosity.is_supported(&winner_a.hypothesis_id));
    journal.record(
        "cycle:golden-001",
        &winner_a.hypothesis_id,
        verdict_a,
        "ev:golden-001",
        1_700_000_000_000,
    );

    // Economy feedback: success settles trust upward; refutation would be
    // information gain, never an automatic failure.
    let note = economy_note(verdict_a);
    assert!(note.contains("trust+"));

    // Lens assignment reuses REAL entity ids — never invents agents.
    let lens_map: BTreeMap<String, String> = assign_lenses(&view_a.entity_ids);
    assert_eq!(lens_map.len(), 3);
    for eid in lens_map.values() {
        assert!(view_a.entity_ids.contains(eid), "no fake agents: {eid}");
    }

    // Activity lifecycle reflects the sealed verdict (Supported → Trading).
    let activity_state = ResearchActivity::Working.on_verdict(verdict_a);
    assert_eq!(activity_state, ResearchActivity::Trading);
    let mut ledger = ActivityLedger::new();
    for (lens, eid) in &lens_map {
        ledger.set(eid, activity_state, &format!("{lens}: golden research"), view_a.tick);
    }
    // Restart survival: serialize → reload → identical.
    let ledger_back = ActivityLedger::from_json(&ledger.to_json().unwrap()).unwrap();
    assert_eq!(ledger, ledger_back);

    // Research graph node seals the FULL chain with stable ids.
    let cheapest = view_a.cheapest_service.clone().expect("locations expose services");
    let mut graph = ResearchGraph::new();
    graph.append(ResearchTrace {
        trace_id: ResearchTrace::make_trace_id(view_a.tick, &obs_id_a),
        world_tick: view_a.tick,
        observation_id: obs_id_a.clone(),
        question: question_a.clone(),
        hypothesis_id: winner_a.hypothesis_id.clone(),
        proposal_id: winner_a.proposal_id.clone(),
        mission_task_id: mission_task.clone(),
        evidence_id: "ev:golden-001".to_string(),
        verdict: verdict_a,
        learning_summary: format!("{verdict_a:?}: {} | {note}", journal.research_report()),
        next_question: "preview".to_string(),
        lens_assignments: lens_map.clone(),
        activity: activity_state.slug().to_string(),
        provider: Some(format!("{}/{}", cheapest.location_id, cheapest.capability)),
        cost: Some(cheapest.price),
    });
    assert_eq!(graph.traces.len(), 1);
    let trace = &graph.traces[0];
    assert_eq!(trace.trace_id, ResearchTrace::make_trace_id(100, &obs_id_a));
    assert_eq!(trace.mission_task_id, Some("task-test-1".to_string()));
    assert_eq!(trace.provider, Some("research-lab-main/inference".to_string()));
    assert_eq!(trace.cost, Some(5));
    let graph_back = ResearchGraph::from_json(&graph.to_json().unwrap()).unwrap();
    assert_eq!(graph, graph_back);

    // Cursor advances (crash-recovery boundary) — reload-first identical.
    let cursor_1 = cursor.advanced(&view_a);
    assert_eq!(cursor_1.last_tick, 100);
    let cursor_back = WorldCursor::from_json(&cursor_1.to_json().unwrap()).unwrap();
    assert_eq!(cursor_1, cursor_back);

    // ── Tick 2: the sealed mission is a NEW World event ─────────────────
    let view_b = parse_world_view(&snapshot_b()).unwrap();
    let delta_b = diff_world(&cursor_1, &view_b);
    assert!(
        delta_b.should_research,
        "new mission event must re-trigger: {}",
        delta_b.reason
    );
    assert!(delta_b.mission_changed);
    assert!(delta_b.reason.contains("new-events"));

    // The next question builds on learning: the supported hypothesis is
    // closed per-hypothesis, so the agent must NOT repeat it.
    let obs_text_b = obs_text(101, 2, 2, 1, 10, 1);
    let domain_b = curiosity.detect_uncertainty();
    let question_b = generate_question(&obs_text_b, &domain_b);
    let deltas_b = compute_deltas(&mut snap_mem, &extract_signals(&obs_text_b));
    let candidates_b = construct_multi_lens(&ConstructInput {
        cycle_id: "cycle:golden-002",
        observation_id: "obs:world:101",
        question: &question_b,
        deltas: &deltas_b,
        store: &store,
        curiosity: &curiosity,
        journal: Some(&journal),
        cycle_max_wei: 2_000,
        operator_destination: "erd1goldenoperator",
        now_unix: 1_700_000_100,
    });
    assert!(!candidates_b.is_empty());
    let cycle_2 = CycleState::new("cycle:golden-002", 2_000);
    let winner_b = select_experiment(&candidates_b, &store, &curiosity, &cycle_2)
        .expect("second tick must select");
    assert_ne!(
        winner_b.hypothesis_id, winner_a.hypothesis_id,
        "supported hypothesis must not repeat — learning drives the next question"
    );

    // 409-keep arm: when the World keeps the existing mission, the trace
    // still seals (mission None) and the loop continues.
    let mut graph2 = graph.clone();
    graph2.append(ResearchTrace {
        trace_id: ResearchTrace::make_trace_id(101, "obs:world:101"),
        world_tick: 101,
        observation_id: "obs:world:101".to_string(),
        question: question_b,
        hypothesis_id: winner_b.hypothesis_id.clone(),
        proposal_id: winner_b.proposal_id.clone(),
        mission_task_id: None,
        evidence_id: "ev:golden-002".to_string(),
        verdict: HypothesisVerdict::Inconclusive,
        learning_summary: economy_note(HypothesisVerdict::Inconclusive).to_string(),
        next_question: "preview-2".to_string(),
        lens_assignments: assign_lenses(&view_b.entity_ids),
        activity: ResearchActivity::Exploring.slug().to_string(),
        provider: None,
        cost: None,
    });
    assert_eq!(graph2.traces.len(), 2);
    assert!(graph2.traces[1].trace_id.starts_with("trace:world:101:"));
}

#[test]
fn steady_world_skips_without_new_question() {
    // No change since the cursor → gate stays closed (no busy-looping).
    let view = parse_world_view(&snapshot_a()).unwrap();
    let cursor = WorldCursor::default().advanced(&view);
    let delta = diff_world(&cursor, &view);
    assert!(!delta.should_research);
    // …but treasury movement alone re-opens it (economy feedback path).
    let mut moved = view.clone();
    moved.minted += 1;
    let delta2 = diff_world(&cursor, &moved);
    assert!(delta2.should_research);
}
