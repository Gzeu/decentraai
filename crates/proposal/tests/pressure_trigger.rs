//! Golden M15: steady World → no trigger; small change → no trigger;
//! pressure threshold → trigger; duplicate tick → no duplicate;
//! restart → recovery; trigger → mission → evidence → learning → event.
//!
//! Pure and deterministic. Mirrors the node's tick-hook step for step:
//! snapshot → journal → pressure state → decision → (on Fire) the existing
//! autonomous loop would run as a bounded child (simulated here through
//! the same pure constructors the CLI uses).

use decentraai_proposal::{
    ActivityLedger, ConstructInput, CuriosityState, CycleState, ExperimentOutcome, ExperimentStore,
    HypothesisVerdict, ObservationSnapshot, PressureDecision, PressureState, PressureThresholds,
    ResearchActivity, ResearchGraph, ResearchJournal, ResearchTrace, WorldCursor, assign_lenses,
    compute_deltas, construct_multi_lens, diff_world, economy_note, evaluate_outcome,
    evaluate_pressure, extract_signals, generate_hypothesis, generate_question, open_families,
    parse_world_view, refuted_total, select_experiment,
};

fn world(tick: u64, events: usize, mission: Option<&str>, minted: u64) -> serde_json::Value {
    let evts: Vec<serde_json::Value> = (0..events)
        .map(|i| serde_json::json!({"kind": "e", "tick": tick.saturating_sub(i as u64)}))
        .collect();
    serde_json::json!({
        "tick": tick,
        "entities": [{"id": "agent-a", "location_id": "research-lab-main"},
                     {"id": "agent-b", "location_id": "research-lab-main"}],
        "events": evts,
        "mission_task_id": mission,
        "treasury_minted": minted,
        "treasury_burned": 0,
        "locations": [
            {"id": "research-lab-main", "capacity": 20,
             "services": {"research": 20, "inference": 5}}
        ]
    })
}

fn cfg() -> PressureThresholds {
    PressureThresholds {
        cooldown_ticks: 0, // golden drives ticks manually; cooldown tested separately
        ..PressureThresholds::default()
    }
}

#[test]
fn steady_world_never_triggers() {
    let journal = ResearchJournal::new();
    let (_, s1) = evaluate_pressure(
        &parse_world_view(&world(10, 2, None, 0)).unwrap(),
        &journal,
        &PressureState::default(),
        &cfg(),
    );
    // Identical world, later tick but below every threshold: skip.
    let v = parse_world_view(&world(11, 2, None, 0)).unwrap();
    let (d, _) = evaluate_pressure(&v, &journal, &s1, &cfg());
    assert!(matches!(d, PressureDecision::Skip { .. }), "{d:?}");
}

#[test]
fn small_change_alone_does_not_trigger() {
    let journal = ResearchJournal::new();
    let (_, s1) = evaluate_pressure(
        &parse_world_view(&world(10, 2, None, 0)).unwrap(),
        &journal,
        &PressureState::default(),
        &cfg(),
    );
    // One new event = 1 point < 2: skip, pressure remembered vs baseline.
    let (d, s2) = evaluate_pressure(
        &parse_world_view(&world(11, 3, None, 0)).unwrap(),
        &journal,
        &s1,
        &cfg(),
    );
    assert!(matches!(d, PressureDecision::Skip { .. }), "{d:?}");
    assert_eq!(s2.total_triggers, 0);
}

#[test]
fn pressure_threshold_fires_once_then_cools_down() {
    let journal = ResearchJournal::new();
    let (_, s1) = evaluate_pressure(
        &parse_world_view(&world(10, 2, None, 0)).unwrap(),
        &journal,
        &PressureState::default(),
        &cfg(),
    );
    // Drift (10→20) + new events (2→5): 2 points → Fire.
    let v = parse_world_view(&world(20, 5, None, 0)).unwrap();
    let (d, s2) = evaluate_pressure(&v, &journal, &s1, &cfg());
    let signals = match d {
        PressureDecision::Fire { points, signals } => {
            assert!(points >= 2);
            assert_eq!(s2.total_triggers, 1);
            assert_eq!(s2.base_tick, 20);
            signals
        }
        PressureDecision::Skip { reason } => panic!("must fire: {reason}"),
    };
    assert!(signals.len() >= 2);
    // Same tick evaluated again: duplicate → Skip (no double trigger).
    let (d2, _) = evaluate_pressure(&v, &journal, &s2, &cfg());
    assert!(matches!(d2, PressureDecision::Skip { .. }), "{d2:?}");
}

#[test]
fn cooldown_separates_triggers() {
    let cfg_cd = PressureThresholds {
        fire_threshold: 1,
        cooldown_ticks: 10,
        min_tick_drift: 1,
        ..PressureThresholds::default()
    };
    let journal = ResearchJournal::new();
    let (_, s1) = evaluate_pressure(
        &parse_world_view(&world(10, 0, None, 0)).unwrap(),
        &journal,
        &PressureState::default(),
        &cfg_cd,
    );
    let (_, s2) = evaluate_pressure(
        &parse_world_view(&world(11, 0, None, 0)).unwrap(),
        &journal,
        &s1,
        &cfg_cd,
    );
    assert_eq!(s2.total_triggers, 1);
    // Next tick still inside cooldown: skip even with motion.
    let (d, _) = evaluate_pressure(
        &parse_world_view(&world(12, 4, None, 0)).unwrap(),
        &journal,
        &s2,
        &cfg_cd,
    );
    assert!(matches!(d, PressureDecision::Skip { .. }), "{d:?}");
}

#[test]
fn restart_recovers_identical_decisions() {
    let journal = ResearchJournal::new();
    let (_, s1) = evaluate_pressure(
        &parse_world_view(&world(10, 2, None, 0)).unwrap(),
        &journal,
        &PressureState::default(),
        &cfg(),
    );
    // Serialize the trigger state (the node's atomic file), reload, decide.
    let reloaded = PressureState::from_json(&s1.to_json().unwrap()).unwrap();
    let v = parse_world_view(&world(30, 9, None, 4)).unwrap();
    let (d_live, _) = evaluate_pressure(&v, &journal, &s1, &cfg());
    let (d_restart, _) = evaluate_pressure(&v, &journal, &reloaded, &cfg());
    assert_eq!(d_live, d_restart);
    assert!(matches!(d_restart, PressureDecision::Fire { .. }));
}

#[test]
fn golden_trigger_to_learning_to_next_pressure() {
    // Full M15 chain, pure: pressure → Fire → the EXISTING loop's
    // constructors (same calls the CLI makes) → mission → evidence →
    // verdict → learning → journal → next tick re-pressures on the event.
    let cfg_fire = PressureThresholds {
        fire_threshold: 1,
        cooldown_ticks: 0,
        ..PressureThresholds::default()
    };
    let mut journal = ResearchJournal::new();
    let mut curiosity = CuriosityState::new();
    let store = ExperimentStore::new();
    let mut snap_mem = ObservationSnapshot::default();

    // Tick 1: baseline.
    let v1 = parse_world_view(&world(50, 1, None, 0)).unwrap();
    let (_, s1) = evaluate_pressure(&v1, &journal, &PressureState::default(), &cfg_fire);

    // Tick 2: drift + events → Fire. The node would now spawn the loop
    // child (read-only/bounded, never live-testnet from the trigger path).
    let v2 = parse_world_view(&world(58, 4, None, 0)).unwrap();
    let (d2, s2) = evaluate_pressure(&v2, &journal, &s1, &cfg_fire);
    assert!(matches!(d2, PressureDecision::Fire { .. }), "{d2:?}");

    // The child's work, simulated with the real constructors:
    // WorldCursor gate → question → multi-lens → selection → verdict.
    let mut cursor = WorldCursor::default();
    let delta = diff_world(&cursor, &v2);
    assert!(delta.should_research);
    let obs_id = format!("obs:world:{}", v2.tick);
    let obs_text = format!(
        "world tick {} entities {} events {} mission 0 minted {} burned 0",
        v2.tick, v2.entity_count, v2.event_count, v2.minted
    );
    let domain = curiosity.detect_uncertainty();
    let question = generate_question(&obs_text, &domain);
    let (hyp_id, _) = generate_hypothesis(&question);
    assert!(!hyp_id.is_empty());
    let deltas = compute_deltas(&mut snap_mem, &extract_signals(&obs_text));
    let cycle = CycleState::new("cycle:trigger-golden", 2_000);
    let candidates = construct_multi_lens(&ConstructInput {
        cycle_id: "cycle:trigger-golden",
        observation_id: &obs_id,
        question: &question,
        deltas: &deltas,
        store: &store,
        curiosity: &curiosity,
        journal: Some(&journal),
        cycle_max_wei: 2_000,
        operator_destination: "erd1goldenoperator",
        now_unix: 1_700_000_000,
    });
    assert!(!candidates.is_empty());
    let winner = select_experiment(&candidates, &store, &curiosity, &cycle).unwrap();
    // Mission projection (the World returns a task; 409-keep is None).
    let mission_task: Option<String> = Some("task-trigger-1".to_string());
    let verdict = if winner.risk == decentraai_proposal::ExperimentRiskClass::ReadOnly {
        evaluate_outcome(&winner.criterion, None, &obs_text)
    } else {
        evaluate_outcome(&winner.criterion, Some("success"), &obs_text)
    };
    let outcome = match verdict {
        HypothesisVerdict::Supported => ExperimentOutcome::Success,
        HypothesisVerdict::Refuted => ExperimentOutcome::Failed,
        HypothesisVerdict::Inconclusive => ExperimentOutcome::Inconclusive,
    };
    curiosity.update(&winner.hypothesis_id, outcome);
    journal.record(
        "cycle:trigger-golden",
        &winner.hypothesis_id,
        verdict,
        "ev:t1",
        1,
    );
    assert_eq!(journal.entries.len(), 1);
    let tallied: u32 = journal
        .family_tallies()
        .values()
        .map(|(s, r, i)| s + r + i)
        .sum();
    assert_eq!(tallied, 1);
    // Refutation is pressure fuel, support is not: helpers stay honest.
    assert_eq!(
        refuted_total(&journal),
        u32::from(verdict == HypothesisVerdict::Refuted)
    );
    let _ = open_families(&journal);

    // Seal the graph node (stable ids, real lens entities, economy note).
    let lens_map = assign_lenses(&v2.entity_ids);
    let activity = ResearchActivity::Working.on_verdict(verdict);
    let mut ledger = ActivityLedger::new();
    for (lens, eid) in &lens_map {
        ledger.set(eid, activity, &format!("{lens}: trigger research"), v2.tick);
    }
    let mut graph = ResearchGraph::new();
    graph.append(ResearchTrace {
        trace_id: ResearchTrace::make_trace_id(v2.tick, &obs_id),
        world_tick: v2.tick,
        observation_id: obs_id.clone(),
        question: question.clone(),
        hypothesis_id: winner.hypothesis_id.clone(),
        proposal_id: winner.proposal_id.clone(),
        mission_task_id: mission_task.clone(),
        evidence_id: "ev:t1".to_string(),
        verdict,
        learning_summary: format!("{verdict:?} | {}", economy_note(verdict)),
        next_question: "preview".to_string(),
        lens_assignments: lens_map,
        activity: activity.slug().to_string(),
        provider: None,
        cost: None,
    });
    assert_eq!(graph.traces.len(), 1);
    cursor = cursor.advanced(&v2);

    // Tick 3: the sealed mission is a NEW World event — but it appeared
    // right after OUR trigger, so the node adopts it instead of duplicating.
    let v3 = parse_world_view(&world(59, 5, Some("task-trigger-1"), 0)).unwrap();
    let (d3, s3) = evaluate_pressure(&v3, &journal, &s2, &cfg_fire);
    assert!(matches!(d3, PressureDecision::Skip { .. }), "{d3:?}");
    if let PressureDecision::Skip { reason } = d3 {
        assert!(reason.contains("mission-adopted"), "{reason}");
    }
    assert_eq!(s3.base_mission, Some("task-trigger-1".to_string()));
    // A FOREIGN mission long after our trigger is still real pressure.
    let v4 = parse_world_view(&world(80, 9, Some("task-foreign-9"), 0)).unwrap();
    let (d4, _) = evaluate_pressure(&v4, &journal, &s3, &cfg_fire);
    assert!(matches!(d4, PressureDecision::Fire { .. }), "{d4:?}");
    // The loop cursor still sees the gain independently (two gates, one truth).
    assert!(diff_world(&cursor, &v3).should_research);
}
