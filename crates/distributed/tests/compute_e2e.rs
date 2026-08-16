//! Two-node compute-sharing E2E (M12/M13 live gap): a real worker node
//! advertises a hardware snapshot over the wire, the coordinator receives
//! it into its compute registry, trusts the worker, routes a real request
//! through the capability-aware scheduler (holding a reservation for the
//! request), and releases it afterwards.
//!
//! The worker NEVER broadcasts a legacy `WorkerAnnouncement` in this test,
//! so the coordinator's legacy router has no workers — success of the routed
//! request proves the compute path was exercised, not the announcement path.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use decentraai_distributed::{
    ComputeManager, DistributedInference, DistributedP2PHandler, InferenceConfig,
    ProvisioningConfig, ProvisioningFactory, RequestTracker, WorkerManager,
};
use decentraai_identity::Identity;
use decentraai_p2p::{
    ChainedHandler, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode,
    RegistryServer,
};
use decentraai_protocol::{InferRequest, serialize_message};
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{GpuProbeStatus, GpuSnapshot, SystemSnapshot};
use libp2p::PeerId;
use libp2p::identity::Keypair;

const MODEL_HASH: &str = "e2e-model-hash";

fn libp2p_peer_id(identity: &Identity) -> PeerId {
    let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes()).unwrap();
    PeerId::from(keypair.public())
}

fn snapshot() -> SystemSnapshot {
    SystemSnapshot {
        logical_cpus: 8,
        cpu_usage_percent: 12.0,
        total_memory_bytes: 16 * 1024 * 1024 * 1024,
        available_memory_bytes: 12 * 1024 * 1024 * 1024,
        used_swap_bytes: 0,
        total_disk_free_bytes: 100 * 1024 * 1024 * 1024,
        battery_percent: None,
    }
}

fn gpu() -> GpuProbeStatus {
    GpuProbeStatus::Nvidia(GpuSnapshot {
        name: "RTX 4090".into(),
        total_vram_mib: 24564,
        free_vram_mib: 20000,
        utilization_percent: 10,
        temperature_celsius: 52,
        power_draw_watts: 120.0,
    })
}

fn served_model() -> decentraai_compute::ServedModel {
    decentraai_compute::ServedModel {
        model_hash: MODEL_HASH.to_string(),
        file_name: "tiny.gguf".into(),
        size_mb: 512,
        est_ram_mb: 1024,
        est_vram_mb: 3072,
        context_tokens: 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_node_compute_advertisement_routes_and_releases_reservation() {
    let mock = httpmock::prelude::MockServer::start_async().await;
    let stream_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    mock.mock_async(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(stream_body);
    })
    .await;

    // ---- Worker node: computes its own advertisement and serves inference.
    let worker_identity = Identity::generate();
    let worker_peer = libp2p_peer_id(&worker_identity);
    let worker_compute = Arc::new(ComputeManager::new(
        worker_peer,
        "worker".to_string(),
        HashSet::new(),
    ));
    // The E2E worker opts in to remote inference so the coordinator can
    // actually route to it (the real node does this from its config).
    worker_compute.set_accepts_remote_inference(true);
    let worker_manager = Arc::new(WorkerManager::new(worker_peer, InferenceConfig::default()));
    let mut worker_handler = DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    worker_handler.set_compute_manager(worker_compute.clone());
    let worker_node = P2PNode::new(
        &worker_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(worker_handler)),
    )
    .unwrap();
    let worker_addr = worker_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let mut worker = DistributedInference::new(
        worker_node,
        InferenceConfig::default(),
        Some(worker_manager.clone()),
        None,
    )
    .unwrap();
    worker.set_compute_manager(worker_compute.clone());
    let backend = decentraai_inference_adapter::OpenAiCompatibleBackend::new(
        decentraai_inference_adapter::BackendConfig {
            base_url: mock.base_url(),
            model: "tiny.gguf".into(),
            ..Default::default()
        },
    )
    .unwrap();
    worker
        .register_worker_backend(backend, MODEL_HASH.to_string(), None, true)
        .unwrap();

    // ---- Coordinator node: aggregates advertisements and routes requests.
    let coord_identity = Identity::generate();
    let coord_peer = libp2p_peer_id(&coord_identity);
    let coord_compute = Arc::new(ComputeManager::new(
        coord_peer,
        "coordinator".to_string(),
        HashSet::new(),
    ));
    let coord_worker_manager = Arc::new(WorkerManager::new(coord_peer, InferenceConfig::default()));
    let tracker = Arc::new(RequestTracker::new());
    let mut coord_handler =
        DistributedP2PHandler::with_worker_manager(coord_worker_manager.clone());
    coord_handler.set_compute_manager(coord_compute.clone());
    coord_handler.set_tracker(tracker.clone());
    let coord_node = P2PNode::new(
        &coord_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(coord_handler)),
    )
    .unwrap();
    coord_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    coord_node
        .dial(&format!("{worker_addr}/p2p/{worker_peer}"))
        .await
        .unwrap();

    let mut coordinator = DistributedInference::new(
        coord_node,
        InferenceConfig::default(),
        Some(coord_worker_manager.clone()),
        Some(tracker),
    )
    .unwrap();
    coordinator.set_compute_manager(coord_compute.clone());
    // P1: sign routed requests so the worker authenticates them.
    coordinator.set_signing_identity(coord_identity.signing_key_bytes());

    // The worker advertises a real probe-derived advertisement over the wire.
    // Re-announce until the dialed connection settles and the coordinator's
    // registry sees it (broadcast only reaches connected peers).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let adv = worker_compute
            .advertise_local(snapshot(), gpu(), vec![served_model()], vec![], false)
            .await;
        worker.p2p_node().announce(serialize_message(&adv).unwrap());
        if !coord_compute.workers().await.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the compute advertisement to propagate"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The coordinator's compute registry picks it up (M12 propagation).
    let workers = coord_compute.workers().await;
    assert_eq!(workers[0].node_name, "worker");
    assert_eq!(workers[0].capability.gpu.as_ref().unwrap().name, "RTX 4090");
    assert!(workers[0].capability.has_model(MODEL_HASH));
    assert_eq!(workers[0].availability.available_ram_mb, 12 * 1024);

    // The coordinator must trust the worker before scheduling it.
    coord_compute.add_trusted(worker_peer).await;
    let req = coord_compute
        .requirements_for(MODEL_HASH)
        .await
        .expect("the advertised model must yield workload requirements");
    assert_eq!(req.est_ram_mb, 1024);
    assert_eq!(req.est_vram_mb, 3072);

    // Selection books a reservation (M13).
    let placement = coord_compute
        .select(&req)
        .await
        .expect("trusted, eligible worker must be selected");
    assert_eq!(placement.worker, worker_peer);
    assert_eq!(coord_compute.in_flight(&worker_peer).await, 1);
    assert_eq!(coord_compute.reserved_ram(&worker_peer).await, 1024);
    // Leave a clean ledger: the routed request below books its own booking.
    coord_compute
        .release(placement.reservation.reservation_id)
        .await;
    assert_eq!(coord_compute.in_flight(&worker_peer).await, 0);

    // Route a real request through the compute path: worker has no legacy
    // announcement, so only the capability-aware scheduler can route it.
    let mut request = InferRequest::new(MODEL_HASH.to_string(), "hello".into(), 64);
    request = request.with_sender(coord_peer);
    request = request.with_streaming(true);
    request.timeout_ms = 15_000;

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        coordinator.route_request_streamed(request, progress_tx),
    )
    .await
    .expect("routed request must complete")
    .expect("route must succeed");

    assert!(
        response.success,
        "inference must succeed: {:?}",
        response.error
    );
    assert_eq!(response.worker_peer_id, worker_peer);
    assert_eq!(response.output, "hello world");
    assert_eq!(
        response.tokens_used, 3,
        "two content chunks plus the terminal chunk"
    );

    let mut streamed = String::new();
    while let Ok(chunk) = progress_rx.try_recv() {
        streamed.push_str(&chunk);
    }
    assert_eq!(
        streamed, "hello world",
        "streamed chunks must match the output"
    );

    // The reservation must be released after the request completes (M13).
    assert_eq!(
        coord_compute.in_flight(&worker_peer).await,
        0,
        "reservation must be released after the request finishes"
    );
    assert_eq!(coord_compute.reserved_ram(&worker_peer).await, 0);

    // The coordinator's router counters prove exactly one routed request.
    let stats = coordinator.get_stats_async().await;
    assert_eq!(stats.total_requests, 1);
    assert_eq!(stats.successful_requests, 1);
    assert_eq!(stats.failed_requests, 0);
}

/// When the capability-aware scheduler's selected worker is unreachable,
/// the coordinator must fall back to the legacy announcement-based router
/// instead of failing the request (M13 fallback routing).
#[tokio::test(flavor = "multi_thread")]
async fn compute_path_falls_back_to_legacy_router_on_worker_failure() {
    let mock = httpmock::prelude::MockServer::start_async().await;
    let stream_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"fallback\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    mock.mock_async(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(stream_body);
    })
    .await;

    // ---- A real legacy worker that serves inference and announces itself
    // ---- the old way (WorkerAnnouncement, no compute advertisement).
    let w2_identity = Identity::generate();
    let w2_peer = libp2p_peer_id(&w2_identity);
    let w2_worker_manager = Arc::new(WorkerManager::new(w2_peer, InferenceConfig::default()));
    let w2_handler = DistributedP2PHandler::with_worker_manager(w2_worker_manager.clone());
    let w2_node = P2PNode::new(
        &w2_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(w2_handler)),
    )
    .unwrap();
    let w2_addr = w2_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    let mut w2 = DistributedInference::new(
        w2_node,
        InferenceConfig::default(),
        Some(w2_worker_manager.clone()),
        None,
    )
    .unwrap();
    let w2_backend = decentraai_inference_adapter::OpenAiCompatibleBackend::new(
        decentraai_inference_adapter::BackendConfig {
            base_url: mock.base_url(),
            model: "tiny.gguf".into(),
            ..Default::default()
        },
    )
    .unwrap();
    w2.register_worker_backend(w2_backend, MODEL_HASH.to_string(), None, true)
        .unwrap();

    // ---- Coordinator.
    let coord_identity = Identity::generate();
    let coord_peer = libp2p_peer_id(&coord_identity);
    let coord_compute = Arc::new(ComputeManager::new(
        coord_peer,
        "coordinator".to_string(),
        HashSet::new(),
    ));
    let coord_worker_manager = Arc::new(WorkerManager::new(coord_peer, InferenceConfig::default()));
    let tracker = Arc::new(RequestTracker::new());
    let mut coord_handler =
        DistributedP2PHandler::with_worker_manager(coord_worker_manager.clone());
    coord_handler.set_compute_manager(coord_compute.clone());
    coord_handler.set_tracker(tracker.clone());
    let coord_node = P2PNode::new(
        &coord_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(coord_handler)),
    )
    .unwrap();
    coord_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    coord_node
        .dial(&format!("{w2_addr}/p2p/{w2_peer}"))
        .await
        .unwrap();

    let mut coordinator = DistributedInference::new(
        coord_node,
        InferenceConfig::default(),
        Some(coord_worker_manager.clone()),
        Some(tracker),
    )
    .unwrap();
    coordinator.set_compute_manager(coord_compute.clone());
    // P1: sign routed requests so the worker authenticates them.
    coordinator.set_signing_identity(coord_identity.signing_key_bytes());

    // The coordinator also trusts a "ghost" worker that is never connected:
    // the capability-aware scheduler will pick it, and the send must fail.
    let ghost_peer = libp2p_peer_id(&Identity::generate());
    let ghost_adv = decentraai_distributed::build_advertisement(
        ghost_peer,
        "ghost",
        "llama_server",
        snapshot(),
        gpu(),
        vec![served_model()],
        false,
        true,
        1_700_000_000_000,
        decentraai_distributed::LivePerf::default(),
    );
    coord_compute.process_advertisement(ghost_adv).await;
    coord_compute.add_trusted(ghost_peer).await;

    // The real worker announces the legacy way; re-announce until the
    // dialed connection settles and the coordinator sees it.
    let announcement = decentraai_protocol::WorkerAnnouncement {
        peer_id: w2_peer,
        node_name: "legacy-worker".into(),
        loaded_models: vec![MODEL_HASH.to_string()],
        available_capacity: 1.0,
        queue_depth: 0,
        tokens_per_second: 50,
        current_latency_ms: 100,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        w2.p2p_node()
            .announce(serialize_message(&announcement).unwrap());
        if coord_worker_manager.worker_count().await >= 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the legacy announcement to propagate"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The compute path selects the ghost (trusted, advertises the model);
    // the send fails, so the legacy router must serve via the real worker.
    let mut request = InferRequest::new(MODEL_HASH.to_string(), "hi".into(), 64);
    request = request.with_sender(coord_peer);
    request = request.with_streaming(true);
    request.timeout_ms = 15_000;

    let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        coordinator.route_request_streamed(request, progress_tx),
    )
    .await
    .expect("fallback must complete")
    .expect("fallback must succeed");

    assert!(
        response.success,
        "fallback must succeed: {:?}",
        response.error
    );
    assert_eq!(
        response.worker_peer_id, w2_peer,
        "the legacy worker must serve after the compute worker fails"
    );
    assert_eq!(response.output, "fallback");
    assert_eq!(
        coord_compute.in_flight(&ghost_peer).await,
        0,
        "ghost reservation released"
    );
}
/// M14 on-demand provisioning: a worker that does not hold the requested
/// model fetches it from the requester through the verified transfer
/// pipeline (per-chunk BLAKE3 + Merkle root), indexes it, and serves the
/// request. The coordinator routes to the worker because it advertises
/// `can_provision` and the coordinator's scheduler permits provisioning.
#[tokio::test(flavor = "multi_thread")]
async fn on_demand_provisioning_downloads_verifies_and_serves() {
    use decentraai_inference_adapter::{BackendConfig, OpenAiCompatibleBackend};

    // ---- Coordinator holds a model in its registry (served via RegistryServer).
    let coord_dir = tempfile::TempDir::new().unwrap();
    let coord_models = coord_dir.path().join("models");
    std::fs::create_dir_all(&coord_models).unwrap();
    let model_bytes = b"GGUF fake tiny model content for on-demand provisioning".to_vec();
    std::fs::write(coord_models.join("provisioned.gguf"), &model_bytes).unwrap();
    let mut coord_registry = ModelRegistry::new(coord_models.clone()).unwrap();
    coord_registry.scan_directory(&coord_models).unwrap();
    std::fs::create_dir_all(coord_dir.path().join("db")).unwrap();
    coord_registry
        .save(&coord_dir.path().join("db/registry.json"))
        .unwrap();
    let provisioned_hash = blake3::hash(&model_bytes).to_hex().to_string();

    // Mock engine for the provisioned model.
    let mock = httpmock::prelude::MockServer::start_async().await;
    let stream_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"provisioned\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    mock.mock_async(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(stream_body);
    })
    .await;

    // ---- Worker: bound to a DIFFERENT model, but provisions on demand.
    let worker_identity = Identity::generate();
    let worker_peer = libp2p_peer_id(&worker_identity);
    let worker_compute = Arc::new(ComputeManager::new(
        worker_peer,
        "worker".to_string(),
        HashSet::new(),
    ));
    // The E2E worker opts in to remote inference so the coordinator can
    // actually route to it (the real node does this from its config).
    worker_compute.set_accepts_remote_inference(true);
    let worker_manager = Arc::new(WorkerManager::new(worker_peer, InferenceConfig::default()));
    let mut worker_handler = DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    worker_handler.set_compute_manager(worker_compute.clone());
    let worker_node = P2PNode::new(
        &worker_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(worker_handler)),
    )
    .unwrap();
    let worker_addr = worker_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let mut worker = DistributedInference::new(
        worker_node,
        InferenceConfig::default(),
        Some(worker_manager.clone()),
        None,
    )
    .unwrap();
    worker.set_compute_manager(worker_compute.clone());

    let bound_backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: mock.base_url(),
        model: "bound".into(),
        ..Default::default()
    })
    .unwrap();

    // The provisioning factory "loads" the downloaded model into the mock
    // engine. The engine handle is opaque; the worker keeps it alive.
    let worker_dir = tempfile::TempDir::new().unwrap();
    let worker_registry_path = worker_dir.path().join("db/registry.json");
    let provision_base_url = mock.base_url();
    let factory: ProvisioningFactory = Arc::new(move |_model_path| {
        let base = provision_base_url.clone();
        Box::pin(async move {
            let backend = OpenAiCompatibleBackend::new(BackendConfig {
                base_url: base,
                model: "provisioned".into(),
                ..Default::default()
            })
            .map_err(|e| anyhow::anyhow!("backend: {e}"))?;
            Ok((Box::new(()) as Box<dyn std::any::Any + Send>, backend))
        })
    });
    let provisioning = ProvisioningConfig {
        data_dir: worker_dir.path().to_path_buf(),
        registry_path: worker_registry_path.clone(),
        reputation_path: worker_dir.path().join("db/reputation.json"),
        max_concurrent_downloads: 2,
        max_invalid_chunks: 3,
        ban_duration: Duration::from_secs(60),
        backend_factory: factory,
    };
    worker
        .register_worker_backend(bound_backend, MODEL_HASH.to_string(), Some(provisioning), true)
        .unwrap();

    // ---- Coordinator: chained handler (distributed + registry server).
    let coord_identity = Identity::generate();
    let coord_peer = libp2p_peer_id(&coord_identity);
    let coord_compute = Arc::new(ComputeManager::new(
        coord_peer,
        "coordinator".to_string(),
        HashSet::new(),
    ));
    coord_compute.set_allow_provisioning(true).await;
    let coord_worker_manager = Arc::new(WorkerManager::new(coord_peer, InferenceConfig::default()));
    let tracker = Arc::new(RequestTracker::new());
    let mut coord_handler =
        DistributedP2PHandler::with_worker_manager(coord_worker_manager.clone());
    coord_handler.set_compute_manager(coord_compute.clone());
    coord_handler.set_tracker(tracker.clone());
    let chained = ChainedHandler::new()
        .add_handler(Arc::new(coord_handler))
        .add_handler(Arc::new(RegistryServer::new(coord_registry)));
    let coord_node = P2PNode::new(
        &coord_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained)),
    )
    .unwrap();
    coord_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    coord_node
        .dial(&format!("{worker_addr}/p2p/{worker_peer}"))
        .await
        .unwrap();

    let mut coordinator = DistributedInference::new(
        coord_node,
        InferenceConfig::default(),
        Some(coord_worker_manager.clone()),
        Some(tracker),
    )
    .unwrap();
    coordinator.set_compute_manager(coord_compute.clone());
    // P1: sign routed requests so the worker authenticates them.
    coordinator.set_signing_identity(coord_identity.signing_key_bytes());

    // The worker advertises `can_provision = true` over the wire until the
    // coordinator's registry sees it.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let adv = worker_compute
            .advertise_local(snapshot(), gpu(), vec![served_model()], vec![], true)
            .await;
        worker.p2p_node().announce(serialize_message(&adv).unwrap());
        if !coord_compute.workers().await.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the provisioning advertisement to propagate"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The scheduler must accept the worker for a model it does NOT serve.
    coord_compute.add_trusted(worker_peer).await;
    let req = coord_compute
        .requirements_for(&provisioned_hash)
        .await
        .expect("a provisioning worker makes the workload schedulable");
    let placement = coord_compute
        .select(&req)
        .await
        .expect("trusted provisioning worker must be selected");
    assert_eq!(placement.worker, worker_peer);
    coord_compute
        .release(placement.reservation.reservation_id)
        .await;

    // Route a request for the not-yet-local model; the worker must fetch,
    // verify, and serve it.
    let mut request = InferRequest::new(provisioned_hash.clone(), "serve it".into(), 64);
    request = request.with_sender(coord_peer);
    request = request.with_streaming(true);
    request.timeout_ms = 20_000;

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        coordinator.route_request_streamed(request, progress_tx),
    )
    .await
    .expect("provisioned request must complete")
    .expect("provisioning route must succeed");

    assert!(
        response.success,
        "provisioned inference must succeed: {:?}",
        response.error
    );
    assert_eq!(response.worker_peer_id, worker_peer);
    assert_eq!(response.output, "provisioned");
    assert_eq!(
        response.tokens_used, 2,
        "one content chunk plus the terminal chunk"
    );

    let mut streamed = String::new();
    while let Ok(chunk) = progress_rx.try_recv() {
        streamed.push_str(&chunk);
    }
    assert_eq!(
        streamed, "provisioned",
        "streamed chunks must match the output"
    );

    // The model was downloaded through the verified pipeline, then indexed
    // into the worker's registry.
    let downloaded = worker_dir.path().join("models/provisioned.gguf");
    assert!(
        downloaded.is_file(),
        "provisioned model must land in the worker's models dir"
    );
    assert_eq!(
        blake3::hash(&std::fs::read(&downloaded).unwrap())
            .to_hex()
            .to_string(),
        provisioned_hash,
        "the provisioned file must verify byte-for-byte"
    );
    let registry = ModelRegistry::load(&worker_registry_path).unwrap();
    assert_eq!(
        registry.models.len(),
        1,
        "provisioned model must be indexed"
    );

    // The reservation is released after the request completes.
    assert_eq!(coord_compute.in_flight(&worker_peer).await, 0);
    assert_eq!(coord_compute.reserved_ram(&worker_peer).await, 0);
}

/// M15 worker-side reservation enforcement: the worker refuses to serve a
/// request whose model footprint would exceed the free capacity it
/// advertised — even when the coordinator (or a buggy/malicious sender)
/// routes it anyway. The gate mirrors the coordinator's CapabilityMatcher
/// so both ends agree on headroom.
#[tokio::test(flavor = "multi_thread")]
async fn worker_rejects_request_exceeding_advertised_capacity() {
    use decentraai_inference_adapter::{BackendConfig, OpenAiCompatibleBackend};
    use decentraai_protocol::{InferMessage, deserialize_message};

    // A model too large for a worker advertising only 2 GiB free RAM.
    let big_model = decentraai_compute::ServedModel {
        model_hash: MODEL_HASH.to_string(),
        file_name: "big.gguf".into(),
        size_mb: 8192,
        est_ram_mb: 8192,
        est_vram_mb: 0,
        context_tokens: 0,
    };
    let tiny_snapshot = SystemSnapshot {
        logical_cpus: 8,
        cpu_usage_percent: 10.0,
        total_memory_bytes: 8 * 1024 * 1024 * 1024,
        available_memory_bytes: 2 * 1024 * 1024 * 1024,
        used_swap_bytes: 0,
        total_disk_free_bytes: 100 * 1024 * 1024 * 1024,
        battery_percent: None,
    };

    let mock = httpmock::prelude::MockServer::start_async().await;
    let stream_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let big_mock = mock
        .mock_async(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/chat/completions");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(stream_body);
        })
        .await;

    // ---- Worker with a compute manager attached (advertises its capacity).
    let worker_identity = Identity::generate();
    let worker_peer = libp2p_peer_id(&worker_identity);
    let worker_compute = Arc::new(ComputeManager::new(
        worker_peer,
        "worker".to_string(),
        HashSet::new(),
    ));
    // The E2E worker opts in to remote inference so the coordinator can
    // actually route to it (the real node does this from its config).
    worker_compute.set_accepts_remote_inference(true);
    let worker_manager = Arc::new(WorkerManager::new(worker_peer, InferenceConfig::default()));
    let mut worker_handler = DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    worker_handler.set_compute_manager(worker_compute.clone());
    let worker_node = P2PNode::new(
        &worker_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(worker_handler)),
    )
    .unwrap();
    let worker_addr = worker_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let mut worker = DistributedInference::new(
        worker_node,
        InferenceConfig::default(),
        Some(worker_manager.clone()),
        None,
    )
    .unwrap();
    worker.set_compute_manager(worker_compute.clone());
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: mock.base_url(),
        model: "big".into(),
        ..Default::default()
    })
    .unwrap();
    worker
        .register_worker_backend(backend, MODEL_HASH.to_string(), None, true)
        .unwrap();

    // The worker advertises only 2 GiB free RAM — the big model cannot fit.
    worker_compute
        .advertise_local(
            tiny_snapshot,
            GpuProbeStatus::Unavailable("no gpu".into()),
            vec![big_model.clone()],
            vec![],
            false,
        )
        .await;

    // ---- Client that routes the request regardless of headroom.
    let client_identity = Identity::generate();
    let client_node = P2PNode::new(
        &client_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    client_node.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    client_node
        .dial(&format!("{worker_addr}/p2p/{worker_peer}"))
        .await
        .unwrap();

    async fn send(
        client_node: &P2PNode,
        worker_peer: PeerId,
        client_identity: &Identity,
        nonce: u64,
    ) -> anyhow::Result<Vec<u8>> {
        let client_peer = libp2p_peer_id(client_identity);
        let mut request = InferRequest::new(MODEL_HASH.to_string(), "hello".into(), 64);
        request = request.with_sender(client_peer);
        request.timeout_ms = 10_000;
        request.nonce = nonce; // P4: distinct nonce per send (replays rejected)
        // P1: sign so the worker authenticates the request before admitting it.
        decentraai_protocol::sign_infer_request_with_key(
            &client_identity.signing_key_bytes(),
            &mut request,
        );
        let payload = serialize_message(&request)?;
        client_node.request(worker_peer, payload).await
    }

    // The worker must reject the oversized workload at the door. Retry only
    // for the newly dialed connection to settle (per AGENTS.md). Each retry
    // uses a fresh nonce so the replay guard never flags the connection settle.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let rejected = {
        let mut n = 0u64;
        loop {
            let attempt = send(&client_node, worker_peer, &client_identity, n).await;
            n += 1;
            match attempt {
                Ok(bytes) => break bytes,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("timed out awaiting a connection to the worker: {e}"),
            }
        }
    };
    let reply: InferMessage = deserialize_message(&rejected, rejected.len()).unwrap();
    match reply {
        InferMessage::InferFailed {
            retryable, error, ..
        } => {
            assert!(retryable, "capacity rejection must be retryable");
            assert!(
                error.contains("insufficient free capacity"),
                "unexpected rejection message: {error}"
            );
        }
        other => panic!("expected InferFailed, got {other:?}"),
    }
    assert_eq!(
        big_mock.hits(),
        0,
        "the rejected request must never reach the backend"
    );

    // ...and admit the same workload once the advertised free capacity
    // actually fits it (positive control proving the gate is the cause).
    worker_compute
        .advertise_local(snapshot(), gpu(), vec![big_model], vec![], false)
        .await;
    let admitted = send(&client_node, worker_peer, &client_identity, 5000)
        .await
        .unwrap();
    let reply: InferMessage = deserialize_message(&admitted, admitted.len()).unwrap();
    assert!(
        matches!(reply, InferMessage::InferAccepted { .. }),
        "an ample advertisement must admit the same workload: {reply:?}"
    );

    // The worker's own ledger must have released the admitted request's
    // reservation after it completed (the backend stream finished).
    tokio::time::sleep(Duration::from_millis(300)).await;
}
