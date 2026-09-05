//! Golden test: the full v0.1 sandbox chain plus the four economic denials.
//!
//! Chain under test:
//! observation → question → hypothesis → AgentIdea → proposal →
//! policy validation → sandbox execution → evidence → outcome → learning.
//!
//! And the invariant: `EconomicStateMutation`, `FundTransfer`,
//! `SignerChange`, `DCAIMint` → DENIED, always, by policy — with the
//! executor boundary as a second, independently tested gate.

use decentraai_proposal::{
    AgentIdea, DenyAllEconomicAuthorization, DenyReason, EvidenceLog, ExecutionMode,
    ExperimentEvidence, ExperimentOutcome, Hypothesis, Observation, PolicyDecision,
    ResearchQuestion, ResourceCommitment, decide, derive_learnings, execute, parse_proposal,
};

const NOW: u64 = 1_787_000_000_000;

fn chain_fixture() -> (Observation, ResearchQuestion, Hypothesis, AgentIdea, String) {
    let obs = Observation {
        id: "obs:tick-218-supply".to_string(),
        text: "treasury minted 2660, burned 130, supply 3109 at tick 218".to_string(),
        source: "world".to_string(),
        observed_at_ms: NOW,
    };
    let question = ResearchQuestion {
        id: "q:supply-drift".to_string(),
        text: "does supply drift stay within band between ticks?".to_string(),
        observation_id: obs.id.clone(),
    };
    let hypothesis = Hypothesis {
        id: "hyp:supply-drift-band".to_string(),
        text: "supply drift between consecutive ticks stays within 2%".to_string(),
        question_id: question.id.clone(),
    };
    let idea = AgentIdea {
        id: "idea:supply-drift-1".to_string(),
        hypothesis_id: hypothesis.id.clone(),
        summary: "observe two ticks, simulate drift, record finding".to_string(),
        proposer: "agent:observer-1".to_string(),
    };
    let proposal_json = serde_json::json!({
        "version": 1,
        "id": "prop:supply-drift-1",
        "idea_id": idea.id,
        "risk": "sandbox",
        "commitment": "none",
        "created_by": idea.proposer,
        "steps": [
            {"id": "s1", "rationale": "read last supply",
             "action": {"kind": "observe", "source": "world", "query": "treasury supply"}},
            {"id": "s2", "rationale": "simulate drift",
             "action": {"kind": "simulate", "scenario": "supply-drift", "steps": 10}},
            {"id": "s3", "rationale": "record outcome",
             "action": {"kind": "record_finding", "text": "drift within band"}}
        ]
    })
    .to_string();
    (obs, question, hypothesis, idea, proposal_json)
}

#[test]
fn golden_sandbox_chain_end_to_end() {
    let (obs, question, hypothesis, idea, proposal_json) = chain_fixture();

    // Chain linkage: every stage references its parent.
    assert_eq!(question.observation_id, obs.id);
    assert_eq!(hypothesis.question_id, question.id);
    assert_eq!(idea.hypothesis_id, hypothesis.id);

    // Proposal → policy: allowed in the sandbox lane.
    let proposal = parse_proposal(&proposal_json).expect("valid sandbox proposal");
    assert_eq!(proposal.idea_id, idea.id);
    let decision = decide(&proposal, &DenyAllEconomicAuthorization, NOW);
    assert_eq!(
        decision,
        PolicyDecision::Allow {
            mode: ExecutionMode::Sandbox
        }
    );

    // Execution: three deterministic results, simulated measurement labeled.
    let report = execute(&proposal, &decision, NOW).expect("sandbox executes");
    assert_eq!(report.results.len(), 3);
    let sim = &report.results[1];
    assert_eq!(sim.action_kind, "simulate");
    assert!(sim.measurement.expect("simulate measures").1);

    // Evidence: sealed, chained, verified.
    let evidence = ExperimentEvidence::seal(&report, ExperimentOutcome::Success, NOW, None);
    assert!(evidence.verify_seal());
    let mut log = EvidenceLog::new();
    log.append(evidence).expect("genesis appends");
    assert!(log.verify_chain());

    // Learning: derived from evidence, success counted.
    let learnings = derive_learnings(log.entries());
    assert_eq!(learnings.len(), 1);
    assert_eq!(learnings[0].proposal_id, proposal.id);
    assert_eq!(learnings[0].success_bp, 10_000);
    assert_eq!(learnings[0].evidence_ids.len(), 1);
}

/// Each economic action kind is denied by policy with its own typed reason.
#[test]
fn economic_actions_denied() {
    let cases = [
        (
            "economic_state_mutation",
            serde_json::json!({"kind": "economic_state_mutation", "detail": "flip flag"}),
        ),
        (
            "fund_transfer",
            serde_json::json!({"kind": "fund_transfer", "detail": "1 Cr to X"}),
        ),
        (
            "signer_change",
            serde_json::json!({"kind": "signer_change", "detail": "rotate signer"}),
        ),
        (
            "dcai_mint",
            serde_json::json!({"kind": "dcai_mint", "detail": "mint 1 DCAI"}),
        ),
    ];
    for (kind, action) in cases {
        let proposal_json = serde_json::json!({
            "version": 1,
            "id": format!("prop:evil-{kind}"),
            "idea_id": "idea:x",
            "risk": "sandbox",
            "commitment": "none",
            "created_by": "agent:rogue",
            "steps": [
                {"id": "s1", "rationale": "smuggle economic effect",
                 "action": action}
            ]
        })
        .to_string();
        // Structurally valid (so the denial is explicit, not a parse error)…
        let proposal = parse_proposal(&proposal_json).expect("parses structurally");
        // …but denied by policy with the exact action reason.
        assert_eq!(
            decide(&proposal, &DenyAllEconomicAuthorization, NOW),
            PolicyDecision::Deny {
                reason: DenyReason::EconomicAction { index: 0, kind }
            },
            "{kind} must be DENIED"
        );
    }
}

/// Every real commitment is denied through the economic seam, and live
/// risk is denied structurally — no path to the economy in v0.1.
#[test]
fn commitments_and_live_risk_denied() {
    let (_, _, _, _, base) = chain_fixture();
    let mut v: serde_json::Value = serde_json::from_str(&base).unwrap();

    for commitment in ["cr", "dcai", "escrow"] {
        v["commitment"] = serde_json::json!(commitment);
        let proposal = parse_proposal(&v.to_string()).expect("parses structurally");
        match decide(&proposal, &DenyAllEconomicAuthorization, NOW) {
            PolicyDecision::Deny {
                reason: DenyReason::EconomicCommitment { .. },
            } => {}
            other => panic!("commitment {commitment} must be DENIED, got {other:?}"),
        }
    }

    v["commitment"] = serde_json::json!("none");
    v["risk"] = serde_json::json!("economic");
    let proposal = parse_proposal(&v.to_string()).expect("parses structurally");
    assert_eq!(
        decide(&proposal, &DenyAllEconomicAuthorization, NOW),
        PolicyDecision::Deny {
            reason: DenyReason::EconomicRiskClass
        }
    );

    // Sanity: the None commitment the golden path uses is the wire value.
    assert_eq!(
        serde_json::to_value(ResourceCommitment::None).unwrap(),
        serde_json::json!("none")
    );
}
