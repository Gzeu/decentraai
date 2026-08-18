//! Two-node agent-advertisement E2E (Collective Intelligence P1): a real
//! worker node advertises its logical agents (identity + semantic capability
//! claims + tools + policies) over the wire, signed with its Ed25519
//! identity; the coordinator receives it into its agent manager and can then
//! answer a unified semantic+physical match against the remote agent.
//!
//! This exercises the FULL wire path: P2P broadcast → generic RequestHandler
//! chain → SignedAgentAdvertisement deserialize → signature verify → agent
//! manager upsert. It does NOT touch inference — the agent layer is the
//! subject under test.

use std::sync::Arc;
use std::time::Duration;

use decentraai_agents::{
    AgentRecord, AgentRequirement, AgentState, ROLE_GENERALIST, ROLE_SPECIALIST, TOOL_KIND_HTTP,
    ToolDescriptor,
};
use decentraai_compute::CapabilityMatcher;
use decentraai_distributed::agents::AgentManager;
use decentraai_distributed::DistributedP2PHandler;
use decentraai_hub::capability::{CapabilityKind, Provenance};
use decentraai_identity::Identity;
use decentraai_p2p::{
    ChainedHandler, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, NetworkConfig, P2PNode,
};
use libp2p::PeerId;
use libp2p::identity::Keypair;


/// Builds an isolated test P2P node with mDNS disabled, so E2E tests running in
/// parallel on loopback never discover each other (each test dials its peers
/// explicitly). Returns `Result` so the existing `.unwrap()` calls stay valid.
fn test_node(
    identity: &Identity,
    max_msg: usize,
    max_chunk: usize,
    handler: Option<Arc<dyn decentraai_p2p::RequestHandler>>,
) -> anyhow::Result<P2PNode> {
    P2PNode::new_with_network(
        identity,
        max_msg,
        max_chunk,
        handler,
        NetworkConfig {
            lan_discovery: false,
            dht_enabled: false,
            relay_enabled: false,
            bootstrap_peers: vec![],
            max_connections: 8,
        },
    )
}

fn libp2p_peer_id(identity: &Identity) -> PeerId {
    let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes()).unwrap();
    PeerId::from(keypair.public())
}

fn ocr_agent(short_id: &str) -> AgentRecord {
    AgentRecord::new(format!("{short_id}:ocr"), "OCR", ROLE_SPECIALIST)
        .described("extracts and understands text from documents")
        .with_capability(CapabilityKind::Ocr, Provenance::Verified)
        .with_capability(CapabilityKind::DocumentUnderstanding, Provenance::Verified)
        .with_tool(ToolDescriptor::new("ocr.api", TOOL_KIND_HTTP).described("OCR API endpoint"))
}

fn generalist_agent(short_id: &str, model_hash: &str) -> AgentRecord {
    let mut rec = AgentRecord::new(format!("{short_id}:generalist"), "Generalist", ROLE_GENERALIST)
        .described("chat, reasoning and text generation on this node")
        .with_capability(CapabilityKind::Chat, Provenance::Inferred)
        .with_capability(CapabilityKind::Reasoning, Provenance::Inferred)
        .with_model(model_hash);
    rec.set_state(AgentState::Ready);
    rec
}

/// Builds a node with its own agent manager and a signed broadcast path.
async fn build_node(
    identity: &Identity,
    short_id: &str,
    model_hash: &str,
) -> (Arc<AgentManager>, P2PNode, libp2p::Multiaddr) {
    let peer = libp2p_peer_id(identity);
    let mut manager = AgentManager::new(peer, format!("node-{short_id}"));
    manager.set_signing_key(identity.signing_key_bytes());
    manager.set_local_agents(vec![generalist_agent(short_id, model_hash), ocr_agent(short_id)]);
    let manager = Arc::new(manager);

    let mut handler = DistributedP2PHandler::new();
    handler.set_agent_manager(manager.clone());
    let chained = ChainedHandler::new().add_handler(Arc::new(handler));
    let node = test_node(
        identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained)),
    )
    .unwrap();
    let addr = node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    (manager, node, addr)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_node_exchange_signed_agent_advertisements() {
    let coordinator_identity = Identity::generate();
    let worker_identity = Identity::generate();
    let coordinator_peer = libp2p_peer_id(&coordinator_identity);

    let (coordinator_agents, coordinator_node, _coordinator_addr) =
        build_node(&coordinator_identity, "coord", "model-a").await;
    let (worker_agents, worker_node, worker_addr) =
        build_node(&worker_identity, "worker", "model-b").await;

    // The coordinator dials the worker (mDNS is not exercised in a loopback
    // test; a real node discovers peers via mDNS and dials them).
    coordinator_node.dial(&worker_addr.to_string()).await.unwrap();

    // The worker broadcasts its signed agent advertisement. Re-announce until
    // the dialed connection settles and the coordinator's agent manager sees
    // it (broadcast only reaches connected peers).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let wire = worker_agents.advertisement_wire_bytes().unwrap();
        worker_node.announce(wire);
        if coordinator_agents.remote_peer_count() > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the agent advertisement to propagate"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let view = coordinator_agents.view();
    let remote: Vec<_> = view.iter().filter(|v| v.remote).collect();
    assert!(
        remote.len() >= 2,
        "coordinator must see the worker's two logical agents, got {}: {:?}",
        remote.len(),
        remote.iter().map(|v| v.record.agent_id.as_str()).collect::<Vec<_>>()
    );

    // The worker's agents arrive with their full capability shape.
    let ocr = remote
        .iter()
        .find(|v| v.record.role == ROLE_SPECIALIST)
        .expect("worker's OCR specialist agent must be visible");
    assert!(ocr.record.has_capability(CapabilityKind::Ocr));
    assert!(ocr.record.has_tool("ocr.api"));
    assert_eq!(ocr.node_name, "node-worker");

    // The coordinator's own agents stay local (never marked remote).
    let local = view.iter().filter(|v| !v.remote).collect::<Vec<_>>();
    assert_eq!(local.len(), 2, "coordinator advertises its own two agents");

    // The worker can also see the coordinator's agents (bidirectional). The
    // coordinator broadcasts its own advertisement the same way.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let wire = coordinator_agents.advertisement_wire_bytes().unwrap();
        coordinator_node.announce(wire);
        if worker_agents
            .view()
            .iter()
            .any(|v| v.remote && v.record.role == ROLE_GENERALIST)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the coordinator's agent advertisement to propagate"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Cleanup: drop the nodes so the swarm tasks end.
    drop(coordinator_node);
    drop(worker_node);
    let _ = coordinator_peer;
}

#[tokio::test(flavor = "multi_thread")]
async fn forged_agent_advertisement_is_rejected() {
    // A forged advertisement (signed by a different identity) must be
    // dropped by the receiver — the anti-spoof invariant of the protocol.
    let victim_identity = Identity::generate();
    let attacker_identity = Identity::generate();
    let victim_peer = libp2p_peer_id(&victim_identity);

    let victim_manager = Arc::new(AgentManager::new(victim_peer, "victim".into()));
    let mut handler = DistributedP2PHandler::new();
    handler.set_agent_manager(victim_manager.clone());
    let chained = ChainedHandler::new().add_handler(Arc::new(handler));
    let victim_node = test_node(
        &victim_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained)),
    )
    .unwrap();
    let victim_addr = victim_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    // The attacker builds a fake "victim" advertisement signed with its OWN
    // key — the receiver must reject it because the signer key does not map
    // to the embedded peer id.
    let fake_adv = decentraai_agents::AgentAdvertisement::new(
        victim_peer,
        "victim",
        vec![generalist_agent("v", "model-x")],
    );
    let fake_bytes = serde_json::to_vec(&fake_adv).unwrap();
    let signed = decentraai_protocol::sign_agent_advertisement(
        &attacker_identity.signing_key_bytes(),
        &fake_bytes,
    );
    let wire = serde_json::to_vec(&signed).unwrap();

    let attacker_node = test_node(
        &attacker_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    attacker_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    attacker_node.dial(&victim_addr.to_string()).await.unwrap();
    attacker_node.announce(wire);

    // The victim must NOT record a remote agent whose peer is the forged
    // advertisement's peer (the attacker's signature does not map to it).
    // We check the specific peer rather than the global remote count so the
    // test stays deterministic even if an unrelated node's advertisement
    // (from an orphaned node in another E2E test) happens to arrive.
    wait_for(Duration::from_secs(3)).await;
    let forged_peer_recorded = victim_manager
        .view()
        .iter()
        .any(|v| v.remote && v.peer_id == victim_peer);
    assert!(
        !forged_peer_recorded,
        "forged agent advertisement must be rejected at the signature gate"
    );

    drop(attacker_node);
    drop(victim_node);
}

#[tokio::test(flavor = "multi_thread")]
async fn unified_matcher_answers_semantic_match_against_remote_agent() {
    // A coordinator with an OCR requirement can match it against a remote
    // agent's semantic claims — the seam that P3 delegation will use.
    let worker_identity = Identity::generate();
    let worker_peer = libp2p_peer_id(&worker_identity);
    let worker_agent = ocr_agent("w").with_model("model-x");

    let mut wl = decentraai_compute::WorkloadRequirements::new("model-x".into(), 256, 0);
    wl.required_capability = Some("ocr".into());
    let requirement = AgentRequirement::new(
        vec![decentraai_hub::requirements::CapabilityRequirement {
            capability: CapabilityKind::Ocr,
            evidence: decentraai_hub::requirements::EvidenceLevel::Verified,
        }],
        Some(wl.clone()),
    );

    // The hosting node advertises capacity for the required model.
    let adv = decentraai_compute::ComputeAdvertisement {
        peer_id: worker_peer,
        node_name: "worker".into(),
        capability: decentraai_compute::ComputeCapability {
            cpu_cores: 8,
            ram_mb: 16 * 1024,
            gpu: None,
            engine: "llama_server".into(),
            served_models: vec![decentraai_compute::ServedModel {
                model_hash: "model-x".into(),
                file_name: "m.gguf".into(),
                size_mb: 512,
                est_ram_mb: 256,
                est_vram_mb: 0,
                context_tokens: 0,
            }],
            can_provision: false,
            available_models: vec![],
        },
        availability: decentraai_compute::ComputeAvailability {
            available_ram_mb: 8 * 1024,
            available_vram_mb: None,
            load_percent: 10,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 40,
            status: decentraai_compute::WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        },
        announced_at_ms: 1_700_000_000_000,
        accepts_remote_inference: true,
        node_id: "dca-w".into(),
        node_version: "1.0.0".into(),
    };

    let ledger = decentraai_compute::ReservationLedger::new(Duration::from_secs(60), 4);
    let outcome = decentraai_agents::match_agent(
        &worker_agent,
        &adv,
        &requirement,
        &CapabilityMatcher::default(),
        &ledger,
        true,
        None,
    );
    assert_eq!(outcome, decentraai_agents::AgentMatchOutcome::Eligible);

    // Negative control: a generalist without OCR must be rejected on the
    // semantic gate, even though the physical node can run the model.
    let generalist = generalist_agent("w", "model-x");
    let outcome = decentraai_agents::match_agent(
        &generalist,
        &adv,
        &requirement,
        &CapabilityMatcher::default(),
        &ledger,
        true,
        None,
    );
    assert!(
        matches!(
            outcome,
            decentraai_agents::AgentMatchOutcome::Rejected(
                decentraai_agents::AgentMatchReason::SemanticMissing { .. }
            )
        ),
        "generalist without OCR must fail the semantic gate: {outcome:?}"
    );
}

/// Polls `cond` until it is true or the deadline passes, then asserts it.
/// Sleeps for a fixed time (for negative assertions we must let the wire
/// path a chance to run before asserting nothing happened).
async fn wait_for(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_exchange_agent_messages_over_the_transport() {
    use decentraai_agents::{AgentMessage, MessageKind};
    use decentraai_distributed::agent_messenger::AgentMessenger;

    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let peer_b = libp2p_peer_id(&identity_b);

    // Node B: messenger + handler that drains inbound frames into its inbox.
    // The node stays alive so A can dial and deliver to it.
    let (messenger_b, node_b, node_b_addr) = {
        let mut handler = DistributedP2PHandler::new();
        let messenger_b = Arc::new(AgentMessenger::new(
            test_node(
                &identity_b,
                DEFAULT_MAX_MESSAGE_BYTES,
                DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
                None,
            )
            .unwrap(),
        ));
        handler.set_messenger(messenger_b.clone());
        let chained = ChainedHandler::new().add_handler(Arc::new(handler));
        let node_b = test_node(
            &identity_b,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            Some(Arc::new(chained)),
        )
        .unwrap();
        let addr = node_b.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
        (messenger_b, node_b, addr)
    };

    // Node A: messenger sends a message to B.
    let node_a = test_node(
        &identity_a,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    node_a.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    node_a.dial(&node_b_addr.to_string()).await.unwrap();
    let messenger_a = AgentMessenger::new(node_a.clone());

    // Re-send until the transport delivers (request only reaches connected
    // peers; the dialed connection needs a beat to settle).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let message = AgentMessage::new("msg-1", "a:research", "b:ocr", MessageKind::Delegate)
        .with_nonce(7)
        .with_created_at_ms(1_700_000_000_000)
        .with_task("t-42");
    loop {
        // Best-effort send: failures before the connection settles are retried.
        let _ = messenger_a.send(peer_b, message.clone()).await;
        if messenger_b.has_pending("b:ocr") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the agent message to be delivered"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let received = messenger_b.pop("b:ocr").expect("inbox must hold the message");
    assert_eq!(received.message_id, message.message_id);
    assert_eq!(received.kind, MessageKind::Delegate);
    assert_eq!(received.task_id.as_deref(), Some("t-42"));
    assert_eq!(received.from_agent, "a:research");
    assert_eq!(received.nonce, 7);

    drop(messenger_a);
    drop(messenger_b);
    drop(node_a);
    drop(node_b);
}

#[tokio::test(flavor = "multi_thread")]
async fn orchestrator_delegates_a_workflow_to_a_remote_agent() {
    use decentraai_agents::{
        AgentAdvertisement, AgentRecord, AgentTask, AgentState, DelegationVerdict, ROLE_SPECIALIST,
        TaskVerification,
    };
    use decentraai_distributed::agent_messenger::AgentMessenger;
    use decentraai_distributed::agent_orchestrator::AgentOrchestrator;
    use decentraai_hub::capability::{CapabilityKind, Provenance};
    use decentraai_hub::requirements::EvidenceLevel;

    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let peer_a = libp2p_peer_id(&identity_a);
    let peer_b = libp2p_peer_id(&identity_b);

    // Node B: a messenger with a minimal "agent runtime" that answers any
    // Delegate with a Reply (task → {"ocr_text": "…"} object).
    let messenger_b = Arc::new(AgentMessenger::new(
        test_node(
            &identity_b,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap(),
    ));
    let mut handler_b = DistributedP2PHandler::new();
    handler_b.set_messenger(messenger_b.clone());
    let chained_b = ChainedHandler::new().add_handler(Arc::new(handler_b));
    let node_b = test_node(
        &identity_b,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_b)),
    )
    .unwrap();
    let node_b_addr = node_b.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    // Re-point the messenger's transport to the handler-bearing node so its
    // send() rides the connected peer.
    messenger_b.set_transport(node_b.clone());

    // B's agent runtime: a production AgentRuntime that executes Delegates
    // and replies to the delegating peer (via the message's from_peer).
    let mut agent_runtime = decentraai_distributed::agent_runtime::AgentRuntime::new(
        "b:ocr",
        messenger_b.clone(),
    );
    agent_runtime.with_executor(|_task, _inputs| async move {
        Ok(serde_json::json!({ "ocr_text": "parsed text" }))
    });
    let _runtime_task = tokio::spawn(async move { agent_runtime.run_forever().await });

    // Node A: orchestrator with a messenger riding the SAME node that dials
    // B, so both the outgoing Delegate and the inbound Reply flow through the
    // connected peer's handler.
    let messenger_a = Arc::new(AgentMessenger::new(
        test_node(
            &identity_a,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap(),
    ));
    let mut handler_a = DistributedP2PHandler::new();
    handler_a.set_messenger(messenger_a.clone());
    let chained_a = ChainedHandler::new().add_handler(Arc::new(handler_a));
    let node_a = test_node(
        &identity_a,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_a)),
    )
    .unwrap();
    node_a.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    node_a.dial(&node_b_addr.to_string()).await.unwrap();
    messenger_a.set_transport(node_a.clone());

    // The coordinator's agent manager knows B advertises an OCR agent.
    let agent_manager = decentraai_distributed::agents::AgentManager::new(peer_a, "coord".into());
    let mut ocr = AgentRecord::new("b:ocr", "OCR", ROLE_SPECIALIST)
        .with_capability(CapabilityKind::Ocr, Provenance::Verified)
        .with_model("m");
    ocr.set_state(AgentState::Ready);
    agent_manager.process_advertisement(AgentAdvertisement::new(
        peer_b,
        "node-b",
        vec![ocr],
    ));
    let agent_manager = Arc::new(agent_manager);

    let mut orchestrator = AgentOrchestrator::new(messenger_a.clone(), agent_manager.clone(), peer_a);
    orchestrator.with_delegate_timeout(Duration::from_secs(10));

    // A master task that needs OCR and an object output, self-check verified.
    let mut task = AgentTask::new("t-master")
        .require_capability(CapabilityKind::Ocr, EvidenceLevel::Verified)
        .verified_by(TaskVerification::SelfCheck);
    task.output_schema = Some(r#"{"type":"object"}"#.into());

    // Let the dialed connection settle, then orchestrate.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let outcome = orchestrator.orchestrate(&task).await;

    assert_eq!(
        outcome.verdict,
        DelegationVerdict::Completed,
        "orchestration must complete: {:?}",
        outcome.result.stages
    );
    // Synthesis received the OCR output and returned an object.
    assert!(
        outcome.result.final_output.is_some(),
        "final output must be produced"
    );
    let synth_stage = outcome
        .result
        .stages
        .iter()
        .find(|s| s.stage_id == "synthesis")
        .expect("synthesis stage present");
    assert!(synth_stage.verified, "synthesis output verified");
    assert!(
        synth_stage.output.is_some(),
        "synthesis output present after remote delegation"
    );

    drop(messenger_a);
    drop(messenger_b);
    drop(node_a);
    drop(node_b);
}
/// P9 collective workflow end-to-end: the `research_report_template`
/// (Research → Finance → Documents → Synthesis, Critic-verified) instantiated
/// into a plan and executed by delegating every stage to a remote agent,
/// verifying per hop. Proves the full template → plan → delegate → verify →
/// collect path on real nodes.
#[tokio::test(flavor = "multi_thread")]
async fn orchestrator_runs_research_report_workflow_on_remote_agent() {
    use decentraai_agents::{
        AgentAdvertisement, AgentRecord, AgentState, DelegationVerdict, ROLE_GENERALIST,
        research_report_template,
    };
    use decentraai_distributed::agent_messenger::AgentMessenger;
    use decentraai_distributed::agent_orchestrator::AgentOrchestrator;
    use decentraai_hub::capability::{CapabilityKind, Provenance};

    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let peer_a = libp2p_peer_id(&identity_a);
    let peer_b = libp2p_peer_id(&identity_b);

    // Node B: messenger + a production AgentRuntime answering every Delegate
    // with a per-stage object (honestly distinguishable by task id).
    let messenger_b = Arc::new(AgentMessenger::new(
        test_node(
            &identity_b,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap(),
    ));
    let mut handler_b = DistributedP2PHandler::new();
    handler_b.set_messenger(messenger_b.clone());
    let chained_b = ChainedHandler::new().add_handler(Arc::new(handler_b));
    let node_b = test_node(
        &identity_b,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_b)),
    )
    .unwrap();
    let node_b_addr = node_b.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    messenger_b.set_transport(node_b.clone());

    let mut agent_runtime = decentraai_distributed::agent_runtime::AgentRuntime::new(
        "b:generalist",
        messenger_b.clone(),
    );
    agent_runtime.with_executor(|task, _inputs| {
        let stage = task.task_id.clone();
        async move { Ok(serde_json::json!({ "stage": stage, "ok": true })) }
    });
    let _runtime_task = tokio::spawn(async move { agent_runtime.run_forever().await });

    // Node A: orchestrator.
    let messenger_a = Arc::new(AgentMessenger::new(
        test_node(
            &identity_a,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap(),
    ));
    let mut handler_a = DistributedP2PHandler::new();
    handler_a.set_messenger(messenger_a.clone());
    let chained_a = ChainedHandler::new().add_handler(Arc::new(handler_a));
    let node_a = test_node(
        &identity_a,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_a)),
    )
    .unwrap();
    node_a.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    node_a.dial(&node_b_addr.to_string()).await.unwrap();
    messenger_a.set_transport(node_a.clone());

    // The remote agent advertises the capabilities the research_report
    // template requires (reasoning, document understanding, chat).
    let mut generalist = AgentRecord::new("b:generalist", "Generalist", ROLE_GENERALIST)
        .with_capability(CapabilityKind::Reasoning, Provenance::Inferred)
        .with_capability(CapabilityKind::DocumentUnderstanding, Provenance::Inferred)
        .with_capability(CapabilityKind::Chat, Provenance::Inferred);
    generalist.set_state(AgentState::Ready);
    let agent_manager = decentraai_distributed::agents::AgentManager::new(peer_a, "coord".into());
    agent_manager.process_advertisement(AgentAdvertisement::new(
        peer_b,
        "node-b",
        vec![generalist],
    ));
    let agent_manager = Arc::new(agent_manager);

    let mut orchestrator = AgentOrchestrator::new(messenger_a.clone(), agent_manager.clone(), peer_a);
    orchestrator.with_delegate_timeout(Duration::from_secs(15));

    // Instantiate the research-report workflow from the P9 template.
    let master_task = decentraai_agents::AgentTask::new("report-master");
    let plan = research_report_template()
        .instantiate(&master_task, "plan-report", 1_700_000_000_000)
        .expect("template instantiates into a valid plan");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let outcome = orchestrator.orchestrate_plan(&plan, None).await;

    assert_eq!(
        outcome.verdict,
        DelegationVerdict::Completed,
        "workflow must complete: {:?}",
        outcome.result.stages
    );
    // The template has research + finance + documents + synthesis.
    assert_eq!(
        outcome.result.stages.len(),
        4,
        "research + finance + documents + synthesis"
    );
    let synth = outcome
        .result
        .stages
        .iter()
        .find(|s| s.stage_id == "synthesis")
        .expect("synthesis stage present");
    assert!(synth.verified, "synthesis output verified");
    assert!(
        outcome.result.final_output.is_some(),
        "workflow produces a final output"
    );

    drop(messenger_a);
    drop(messenger_b);
    drop(node_a);
    drop(node_b);
}
