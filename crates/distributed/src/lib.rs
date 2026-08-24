//! Distributed inference for DecentraAI (M9)
//!
//! This crate provides distributed inference capabilities, enabling:
//! - Worker discovery and registration with real-time capacity reporting
//! - Intelligent request routing based on worker capacity, latency, and throughput
//! - Automatic fallback to alternative workers when primary workers fail
//! - Reputation-based compensation for worker contributions
//!
//! # Architecture
//!
//! The distributed inference system consists of three main components:
//!
//! 1. **Worker Manager**: Manages worker registration, heartbeats, and capacity updates
//! 2. **Request Router**: Routes inference requests to the best available worker
//! 3. **Fallback Handler**: Manages retry logic with fallback workers
//!
//! # Example Usage
//!
//! ```no_run
//! use decentraai_distributed::{DistributedInference, InferenceConfig, WorkerManager};
//! use decentraai_p2p::P2PNode;
//! use decentraai_identity::Identity;
//! use std::path::Path;
//! use std::sync::Arc;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let identity = Identity::load(Path::new("identity/key.pem"))?;
//! let p2p_node = P2PNode::new(&identity, 1024 * 1024, 1024 * 1024 * 100, None)?;
//!
//! let config = InferenceConfig::default();
//! let worker_manager = Arc::new(WorkerManager::new(p2p_node.local_peer_id(), config.clone()));
//! let mut distributed = DistributedInference::new(p2p_node, config, Some(worker_manager), None)?;
//!
//! // Start worker discovery
//! distributed.start_worker_discovery().await?;
//!
//! // Route an inference request
//! // let request = InferRequest::new("model-hash".to_string(), "prompt".to_string(), 100);
//! // let response = distributed.route_request(request).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use decentraai_compute::{Admission, AdmitReason, ReservationLedger};

/// Default RAM estimate (MiB) for a workload whose model footprint is not yet
/// known (e.g. a provisioned model before its download completes). Matches
/// the coordinator's `PROVISION_DEFAULT_RAM_MB`.
const DEFAULT_EST_RAM_MB: u64 = 1024;
/// Absolute floor of free RAM/VRAM the worker keeps for itself when deciding
/// whether an inbound request fits (M15). Mirrors the coordinator's
/// `CapabilityMatcher` defaults so both ends agree.
const MIN_FREE_RAM_MB: u64 = 1024;
const MIN_FREE_VRAM_MB: u64 = 512;

/// Human-readable message for an admission rejection (M15).
fn describe_admit_reason(reason: AdmitReason) -> String {
    match reason {
        AdmitReason::InsufficientRam {
            available,
            required,
        } => {
            format!("{available} MiB free RAM below the required {required} MiB")
        }
        AdmitReason::InsufficientVram {
            available,
            required,
        } => {
            format!("{available} MiB free VRAM below the required {required} MiB")
        }
    }
}

pub mod agent_memory;
pub mod agent_messenger;
pub mod agent_orchestrator;
pub mod agent_runtime;
pub mod agents;
pub mod benchmark_datasets;
pub mod benchmark_manager;
pub mod breaker;
pub mod compute;
pub mod config;
pub mod embedding;
pub mod evidence_manager;
pub mod fallback;
pub mod intelligence_loop;
pub mod knowledge_runtime;
pub mod memory_propagator;
pub mod model_performance;
pub mod p2p_handler;
pub mod probe;
pub mod queue;
pub mod rate_limit;
pub mod replay;
pub mod retrieval_manager;
pub mod router;
pub mod session;
pub mod tool_calling;
pub mod tracker;
pub mod worker;

pub use compute::{
    ComputeAdvertisement, ComputeManager, ComputeMetricsReport, ContributionProfile,
    ContributionRow, ExecutedPlan, ExecutionAttribution, LivePerf, MeasuredContribution,
    RuntimeMetrics, WorkerMetricRow, build_advertisement, execution_statistics, short_node_id,
};
pub use config::InferenceConfig;
pub use error::DistributedError;
pub use fallback::FallbackHandler;
pub use p2p_handler::DistributedP2PHandler;
pub use queue::{QueueProcessResult, QueuedRequest, RequestQueueManager, WorkerRequestQueue};
pub use router::RequestRouter;
pub use tracker::RequestTracker;
pub use worker::WorkerManager;

/// Re-export protocol types for convenience
pub use decentraai_protocol::{
    InferErrorCode, InferMessage, InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement,
    WorkerStatus,
};

/// Builds a serving backend for a downloaded model file (M14 on-demand
/// provisioning). Returns the opaque engine handle — kept alive for the
/// worker session so the subprocess (e.g. llama-server) is not reaped —
/// plus the OpenAI-compatible backend that talks to it. The handle is
/// `dyn Any` because the engine type lives in the node CLI crate (which
/// depends on both `decentraai-distributed` and `decentraai-runtime`);
/// keeping it untyped avoids an orphan-rule impl or a runtime dependency
/// here just to hold a process alive.
pub type ProvisionFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = anyhow::Result<(
                    Box<dyn std::any::Any + Send>,
                    decentraai_inference_adapter::OpenAiCompatibleBackend,
                )>,
            > + Send,
    >,
>;
pub type ProvisioningFactory = Arc<dyn Fn(PathBuf) -> ProvisionFuture + Send + Sync>;

/// Policy + I/O paths a worker needs to fetch models on demand (M14).
///
/// A worker that carries this (via [`DistributedInference::register_worker_backend`])
/// answers a workload for a model it does not yet hold by downloading it
/// from the requester through the verified-transfer pipeline, indexing it in
/// the local registry, loading it into an inference engine, and only then
/// serving the request. The coordinator must have routed the request knowing
/// the worker advertises `can_provision` (see the compute matcher).
/// Backends loaded for on-demand-provisioned models, keyed by model hash.
/// The engine handle keeps the subprocess alive for the worker session; the
/// backend streams inference through it. All dropped together on shutdown.
pub type ProvisionedBackends = Arc<
    tokio::sync::Mutex<
        HashMap<
            String,
            (
                Box<dyn std::any::Any + Send>,
                decentraai_inference_adapter::OpenAiCompatibleBackend,
            ),
        >,
    >,
>;

/// Everything a provisioning task needs to reach the network and the worker
/// loop. Bundled so [`provision_on_demand`] stays under clippy's argument cap.
#[derive(Clone)]
pub struct Provisioner {
    p2p: decentraai_p2p::P2PNode,
    queue_mgr: Arc<RequestQueueManager>,
    worker_manager: Arc<WorkerManager>,
    provisioned: ProvisionedBackends,
    semaphore: Arc<tokio::sync::Semaphore>,
    /// Worker-side reservation ledger, passed so provisioned workloads share
    /// the same queue accounting when their requests are served.
    reservations: Arc<std::sync::Mutex<ReservationLedger>>,
    /// Live compute metrics (M16), so provisioned requests also report
    /// real throughput/latency into the node's advertisements.
    metrics: Option<std::sync::Arc<crate::compute::RuntimeMetrics>>,
}

#[derive(Clone)]
pub struct ProvisioningConfig {
    /// Node data dir; the model lands in `<data_dir>/models` and staging in
    /// `<data_dir>/staging`.
    pub data_dir: PathBuf,
    /// `db/registry.json` path to index the provisioned model into.
    pub registry_path: PathBuf,
    /// `db/reputation.json` path for the verified-transfer reputation store.
    pub reputation_path: PathBuf,
    /// Cap on simultaneous on-demand downloads (from `sharing` config).
    pub max_concurrent_downloads: usize,
    /// Reputation ban parameters passed through to the transfer pipeline.
    pub max_invalid_chunks: u8,
    pub ban_duration: std::time::Duration,
    /// Loads a downloaded model into an engine and returns a serving backend.
    pub backend_factory: ProvisioningFactory,
}

/// Module for error types
mod error {
    use decentraai_protocol::InferErrorCode;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum DistributedError {
        #[error("No workers available for model {0}")]
        NoWorkersAvailable(String),

        #[error("All workers failed for request {0}")]
        AllWorkersFailed(String),

        /// The worker answered `InferFailed { retryable: true }` — it did NOT
        /// execute the request (a pre-execution refusal, e.g. model not
        /// loaded yet) and explicitly asked for a safe retry. Unlike
        /// `AllWorkersFailed` (outcome unknown), re-sending to another worker
        /// is idempotency-safe because no generation happened.
        #[error("Worker reported a retryable failure: {0}")]
        WorkerRetryable(String),

        #[error("Request timeout after {0}ms")]
        RequestTimeout(u64),

        #[error("Worker {0} rejected request: {1}")]
        WorkerRejected(String, String),

        #[error("P2P communication error: {0}")]
        P2PError(#[from] anyhow::Error),

        #[error("Serialization error: {0}")]
        SerializationError(String),

        #[error("Worker {0} is not trusted")]
        UntrustedWorker(String),
    }

    impl DistributedError {
        /// Idempotency-safe retry policy: only transport/connection-level
        /// failures are retried. A definite worker rejection or a cancelled
        /// request must never be re-issued, because re-sending would
        /// duplicate non-idempotent work (re-generation, double token/KV
        /// accounting). A connection that never completed is safe to retry —
        /// Whether re-sending this logical request to a DIFFERENT worker is
        /// idempotency-safe. Inference is non-idempotent (each generation is
        /// unique), so we must never re-execute a request whose outcome is
        /// unknown. A `RequestTimeout` means the worker MAY have executed
        /// (accepted and generated) while the response was lost or too slow —
        /// re-sending to another worker duplicates the generation and its
        /// token/KV accounting. Only a `P2PError` at the initial
        /// request/response exchange is treated as likely-not-delivered
        /// (connection refused before the worker started), so it remains
        /// retryable. This gives at-most-once semantics for ambiguous
        /// outcomes; clients wanting a fresh generation send a new
        /// `request_id`/nonce.
        pub fn is_retryable(&self) -> bool {
            matches!(
                self,
                // A connection that never completed is safe to retry.
                DistributedError::P2PError(_)
                    // The worker explicitly refused BEFORE executing and asked
                    // for a safe retry (InferFailed.retryable=true) — no
                    // generation happened, so re-sending is idempotency-safe.
                    | DistributedError::WorkerRetryable(_)
            )
        }

        /// Stable machine-readable classification of this failure (M10
        /// Phase-1). Lets `/metrics`, logs and clients categorize an error
        /// without parsing the human-readable string. Maps directly to an
        /// [`InferErrorCode`] for populating `InferFailed` frames.
        pub fn code(&self) -> InferErrorCode {
            match self {
                DistributedError::NoWorkersAvailable(_) => InferErrorCode::NoWorkers,
                DistributedError::AllWorkersFailed(_) => InferErrorCode::AllWorkersFailed,
                DistributedError::WorkerRetryable(_) => InferErrorCode::RetryableWorker,
                DistributedError::RequestTimeout(_) => InferErrorCode::Timeout,
                DistributedError::WorkerRejected(_, _) => InferErrorCode::Rejected,
                DistributedError::P2PError(_) => InferErrorCode::Transport,
                DistributedError::SerializationError(_) => InferErrorCode::Serialization,
                DistributedError::UntrustedWorker(_) => InferErrorCode::Untrusted,
            }
        }
    }

    impl From<&DistributedError> for InferErrorCode {
        fn from(e: &DistributedError) -> Self {
            e.code()
        }
    }
}

/// The outcome of one request in a batch dispatch (Next-Gen adaptive fan-out).
/// Preserves the request id + chosen worker provenance so the caller can trace
/// each independent request end-to-end.
#[derive(Debug)]
pub struct BatchRequestOutcome {
    /// The request id (caller-supplied provenance).
    pub request_id: String,
    /// The worker that actually served it (empty on failure — the request did
    /// not complete; see `result` for the honest error).
    pub worker: String,
    /// The full result (Ok with measured usage / Err with the reason). Failed
    /// requests earn no credit and are safe to retry by the caller with a new
    /// request id (never auto-retried here to preserve idempotency).
    pub result: Result<InferResponse, DistributedError>,
}

/// Main distributed inference coordinator
///
/// Combines worker discovery, request routing, and fallback handling
/// into a single interface for distributed inference.
pub struct DistributedInference {
    p2p_node: decentraai_p2p::P2PNode,
    worker_manager: Arc<WorkerManager>,
    request_router: RequestRouter,
    fallback_handler: FallbackHandler,
    queue_manager: Arc<RequestQueueManager>,
    compute_manager: Option<Arc<crate::compute::ComputeManager>>,
    config: InferenceConfig,
    /// Security-log directory. When set, each routed request records a
    /// per-request audit event (`routed`/`inference_completed`) capturing the
    /// request id, worker, model hash and outcome (M10). None disables audit.
    logs_dir: Option<PathBuf>,
    /// Coordinator signing key bytes (P1). When set, outbound ``InferRequest``s
    /// are signed so workers can authenticate them and reject spoofs / unsigned
    /// traffic. `None` sends unsigned (legacy) frames.
    signing_key: Option<[u8; 32]>,
    /// Monotonic per-worker-outbound nonce source (P4). Each signed request
    /// gets a unique nonce so the worker's replay guard never sees a duplicate
    /// for this coordinator.
    outbound_nonce: std::sync::atomic::AtomicU64,
}

impl Clone for DistributedInference {
    fn clone(&self) -> Self {
        Self {
            p2p_node: self.p2p_node.clone(),
            worker_manager: self.worker_manager.clone(),
            request_router: self.request_router.clone(),
            fallback_handler: self.fallback_handler.clone(),
            queue_manager: self.queue_manager.clone(),
            compute_manager: self.compute_manager.clone(),
            config: self.config.clone(),
            logs_dir: self.logs_dir.clone(),
            signing_key: self.signing_key,
            outbound_nonce: std::sync::atomic::AtomicU64::new(
                self.outbound_nonce
                    .load(std::sync::atomic::Ordering::SeqCst),
            ),
        }
    }
}

impl DistributedInference {
    /// Creates a new distributed inference coordinator
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `config` - Distributed inference configuration
    /// * `worker_manager` - Optional worker manager to use (if None, a new one will be created)
    pub fn new(
        p2p_node: decentraai_p2p::P2PNode,
        config: InferenceConfig,
        worker_manager: Option<Arc<WorkerManager>>,
        tracker: Option<Arc<RequestTracker>>,
    ) -> anyhow::Result<Self> {
        let worker_manager = worker_manager.unwrap_or_else(|| {
            Arc::new(WorkerManager::new(p2p_node.local_peer_id(), config.clone()))
        });
        let request_router = RequestRouter::new(p2p_node.local_peer_id(), tracker.clone());
        let fallback_handler = FallbackHandler::new(config.max_retries);
        let queue_manager = Arc::new(RequestQueueManager::new(
            config.max_queue_depth as usize,
            std::time::Duration::from_millis(config.request_timeout_ms),
        ));

        Ok(Self {
            p2p_node,
            worker_manager,
            request_router,
            fallback_handler,
            queue_manager,
            compute_manager: None,
            config,
            logs_dir: None,
            signing_key: None,
            // Nonces MUST NOT restart from 0: a coordinator reboot would
            // re-issue nonces the workers' replay guards still remember,
            // rejecting every outbound request as a replay until both sides
            // restarted. Seeding from wall-clock epoch keeps new nonces
            // strictly ahead of anything previously seen across restarts.
            outbound_nonce: std::sync::atomic::AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(1),
            ),
        })
    }

    /// Sets the coordinator's signing key bytes (P1). With it set, every
    /// outbound routed request is Ed25519-signed so workers can authenticate
    /// it and reject spoofed/unsigned traffic.
    pub fn set_signing_identity(&mut self, signing_key: [u8; 32]) {
        self.signing_key = Some(signing_key);
    }

    /// Sets the security-log directory so routing records per-request audit
    /// events (M10). Pass `None` to keep routing silent.
    pub fn set_logs_dir(&mut self, logs_dir: Option<PathBuf>) {
        self.logs_dir = logs_dir;
    }

    /// Best-effort per-request audit event (M10): request id, session, worker,
    /// model hash and outcome, plus resource attribution when the worker
    /// reported measured usage. Never breaks the routing flow on a write
    /// error; prompts and outputs are never audit material.
    fn audit_routed(
        &self,
        request: &InferRequest,
        worker: &libp2p::PeerId,
        ok: bool,
        tokens_used: Option<u32>,
        processing_time_ms: Option<u32>,
        attempt: u32,
    ) {
        if let Some(logs_dir) = &self.logs_dir {
            decentraai_audit::record_best_effort(
                logs_dir,
                if ok {
                    "inference_completed"
                } else {
                    "inference_failed"
                },
                serde_json::json!({
                    "request_id": request.request_id.to_string(),
                    "session_id": request.session_id.clone().unwrap_or_default(),
                    "trace_id": request.trace_id,
                    "worker_id": worker.to_string(),
                    "model_hash": request.model_hash,
                    "status": if ok { "completed" } else { "failed" },
                    "attempt": attempt,
                    // Part 9/17 attribution: real measured usage (None on
                    // transport failure), never invented.
                    "tokens_used": tokens_used,
                    "processing_time_ms": processing_time_ms,
                }),
            );
        }
    }

    /// Ed25519-signs an outbound infer request with this node's signing key (P1).
    /// No-op when no signing key is set (legacy unsigned traffic).
    fn sign_request(&self, request: &mut InferRequest) {
        if let Some(key) = &self.signing_key {
            // P4: give each outbound request a unique nonce so the receiving
            // worker's replay guard never confuses it with a replayed frame.
            request.nonce = self
                .outbound_nonce
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            decentraai_protocol::sign_infer_request_with_key(key, request);
        }
    }

    /// Attaches a [`crate::compute::ComputeManager`] so routing can select
    /// workers from the capability-aware compute registry (hardware + model
    /// matching + resource reservations). The legacy announcement-based
    /// routing remains as a fallback.
    pub fn set_compute_manager(&mut self, compute_manager: Arc<crate::compute::ComputeManager>) {
        self.compute_manager = Some(compute_manager);
    }

    /// Returns the attached compute manager, if any.
    pub fn compute_manager(&self) -> Option<&Arc<crate::compute::ComputeManager>> {
        self.compute_manager.as_ref()
    }

    /// Returns a reference to the underlying P2P node
    pub fn p2p_node(&self) -> &decentraai_p2p::P2PNode {
        &self.p2p_node
    }

    /// Returns a reference to the worker manager
    pub fn worker_manager(&self) -> &WorkerManager {
        &self.worker_manager
    }

    /// Returns a mutable reference to the worker manager
    pub fn worker_manager_mut(&mut self) -> &mut WorkerManager {
        Arc::get_mut(&mut self.worker_manager).expect("WorkerManager is shared elsewhere")
    }

    /// Returns a clone of the Arc-wrapped worker manager
    pub fn worker_manager_arc(&self) -> Arc<WorkerManager> {
        self.worker_manager.clone()
    }

    /// Returns a reference to the request router
    pub fn request_router(&self) -> &RequestRouter {
        &self.request_router
    }

    /// Returns a reference to the fallback handler
    pub fn fallback_handler(&self) -> &FallbackHandler {
        &self.fallback_handler
    }

    /// Returns a reference to the queue manager
    pub fn queue_manager(&self) -> &RequestQueueManager {
        &self.queue_manager
    }

    /// Starts the worker discovery process
    ///
    /// This spawns background tasks for:
    /// - Broadcasting worker announcements (if this node is a worker)
    /// - Listening for worker announcements from peers
    /// - Managing worker heartbeat and stale detection
    pub async fn start_worker_discovery(&mut self) -> anyhow::Result<()> {
        let manager = self.worker_manager.clone();
        manager
            .start_discovery(&self.p2p_node, self.config.announcement_interval_ms)
            .await
    }

    /// Registers this node as a worker with the given capabilities
    ///
    /// # Arguments
    ///
    /// * `node_name` - Human-readable name for this worker
    /// * `loaded_models` - List of model hashes this worker can serve
    /// * `initial_capacity` - Initial available capacity (0.0 - 1.0)
    pub fn register_as_worker(
        &mut self,
        node_name: String,
        loaded_models: Vec<String>,
        initial_capacity: f32,
    ) -> anyhow::Result<()> {
        self.worker_manager
            .register_as_worker(node_name, loaded_models, initial_capacity)
    }

    /// Registers a backend to serve inference on this node and installs the
    /// P2P on_infer handler that accepts requests, enqueues them, and streams
    /// progress back to the requester. This method must be called after the
    /// DistributedInference is constructed and takes ownership of the backend.
    ///
    /// The request lifecycle guarantees exactly ONE terminal event per request:
    /// either an `InferResponse` (success) or an `InferFailed` (cancellation or
    /// backend error) — never both, never zero (queue-full is answered
    /// immediately with `InferFailed`).
    pub fn register_worker_backend(
        &mut self,
        backend: decentraai_inference_adapter::OpenAiCompatibleBackend,
        model_hash: String,
        provisioning: Option<ProvisioningConfig>,
        allow_remote: bool,
    ) -> anyhow::Result<()> {
        use decentraai_protocol::{InferMessage, InferResponse, serialize_message};

        let local_peer = self.p2p_node.local_peer_id();
        // Bounded inbound pipeline (data-plane hardening): the worker loop
        // serves one queued request at a time, so an abusive sender could
        // otherwise flood the unbounded channel and enqueue without bound.
        // Capacity is derived from `max_queue_depth` so the number of
        // requests in the pipeline (buffered + being served) never exceeds
        // the configured depth. The producer uses `try_send` and rejects the
        // request when the pipe is already full (see the on_infer handler).
        let inbound_cap = self.config.max_queue_depth.max(1) as usize;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(
            decentraai_protocol::InferRequest,
            Option<decentraai_compute::ResourceReservation>,
        )>(inbound_cap);
        let tx_clone = tx.clone();
        let p2p_clone = self.p2p_node.clone();
        let queue_mgr = self.queue_manager.clone();
        let backend_clone = backend.clone();
        let worker_manager = self.worker_manager.clone();
        let model_hash_clone = model_hash.clone();
        let compute_for_closure = self.compute_manager.clone();
        // M16: real compute metrics. The worker's streaming task records
        // measured tokens/sec and latency; the queue path keeps depth current.
        let compute_metrics = self.compute_manager.as_ref().map(|c| c.runtime_metrics());

        // Worker-side reservation enforcement (M15): the worker keeps its own
        // ledger of in-flight workloads booked against the capacity it
        // advertised, and refuses to serve a request whose footprint would
        // exceed the remaining headroom — even if a buggy or malicious
        // coordinator sent more work than it booked. The TTL is a safety net;
        // reservations are released explicitly on the terminal event.
        let local_reservations: Arc<std::sync::Mutex<ReservationLedger>> =
            Arc::new(std::sync::Mutex::new(ReservationLedger::new(
                std::time::Duration::from_secs(300),
                8,
            )));
        let reservations_closure = local_reservations.clone();
        let reservations_for_task = local_reservations.clone();

        // Engines spawned for on-demand-provisioned models, keyed by model
        // hash. Kept for the worker session so the subprocesses stay alive;
        // they are reaped when the node drops.
        let provisioned: ProvisionedBackends = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let provisioned_clone = provisioned.clone();
        let provisioning_clone = provisioning.clone();
        let provision_semaphore = Arc::new(tokio::sync::Semaphore::new(
            provisioning
                .as_ref()
                .map(|p| p.max_concurrent_downloads)
                .unwrap_or(0)
                .max(1),
        ));
        let semaphore_clone = provision_semaphore.clone();

        // Map inbound InferCancel frames to the queue manager so in-flight
        // requests are marked cancelled and the streaming loop aborts.
        let cancel_queue = self.queue_manager.clone();
        self.p2p_node.set_on_cancel_request(move |request_id| {
            let cancel_queue = cancel_queue.clone();
            tokio::spawn(async move {
                let _ = cancel_queue.cancel_request(request_id).await;
            });
        });

        // Clones reserved for the on_infer closure so the background worker
        // loop below keeps its own untouched copies.
        let p2p_for_closure = p2p_clone.clone();
        let queue_for_closure = queue_mgr.clone();
        let wm_for_closure = worker_manager.clone();
        let metrics_for_infer = compute_metrics.clone();
        // P4: replay guard for inbound requests, keyed by the authenticated
        // sender peer. TTL wraps the request timeout so a captured frame can't
        // be replayed after the window; capacity bounds memory per peer.
        let replay: Arc<std::sync::Mutex<crate::replay::ReplayGuard>> =
            Arc::new(std::sync::Mutex::new(crate::replay::ReplayGuard::new(
                std::time::Duration::from_secs(300),
                4096,
            )));
        let replay_for_closure = replay.clone();
        // H1: per-peer sliding-window rate limit on the worker path, protecting
        // the engine from an abusive/anomalous coordinator regardless of how
        // many requests it sends. Window = 60s, burst = configured via a
        // module constant (self._peer_limit below).
        let peer_limit = 120usize;
        let peer_limiter: Arc<std::sync::Mutex<crate::rate_limit::PeerRateLimiter>> = Arc::new(
            std::sync::Mutex::new(crate::rate_limit::PeerRateLimiter::new(
                std::time::Duration::from_secs(60),
                peer_limit,
                peer_limit * 2,
            )),
        );
        let peer_limiter_for_closure = peer_limiter.clone();
        let logs_dir_for_infer = self.logs_dir.clone();

        // Register sync on_infer handler that enqueues the request and returns Accept
        self.p2p_node.set_on_infer_request(
            move |peer: libp2p::PeerId, req: decentraai_protocol::InferRequest| -> anyhow::Result<Vec<u8>> {
                // P1/P2: verify the request is signed by the authenticated
                // connected peer before accepting work. `peer` is the real
                // Noise-authenticated PeerId; `req.sender_peer_id` is payload
                // and never trusted. A failed signature or an unsigned frame
                // when signing is required is answered terminal (never
                // executed, never retried).
                if !decentraai_protocol::verify_infer_request_signature(&peer, &req).is_ok() {
                    let resp = InferResponse {
                        request_id: req.request_id,
                        trace_id: req.trace_id.clone(),
                        worker_peer_id: local_peer,
                        completed_at: chrono::Utc::now().to_rfc3339(),
                        output: "".to_string(),
                        tokens_used: 0,
                        processing_time_ms: 0,
                        success: false,
                        error: Some("unauthenticated inference request: bad or missing signature".to_string()),
                    };
                    return serialize_message(&InferMessage::InferResponse(resp));
                }
                // Remote-sharing opt-in gate (config
                // `inference.allow_remote_inference`): the node only accepts
                // inference routed from *remote* peers when the operator has
                // explicitly enabled it. Local requests (`peer` == ourselves,
                // i.e. the node serving its own CLI ask) are never gated. The
                // rejection is terminal and non-retryable so a coordinator that
                // raced with a config change never re-sends a request the
                // worker will keep refusing.
                if !allow_remote && peer != local_peer {
                    let resp = InferResponse {
                        request_id: req.request_id,
                        trace_id: req.trace_id.clone(),
                        worker_peer_id: local_peer,
                        completed_at: chrono::Utc::now().to_rfc3339(),
                        output: "".to_string(),
                        tokens_used: 0,
                        processing_time_ms: 0,
                        success: false,
                        error: Some(
                            "worker is not accepting remote inference (inference.allow_remote_inference is false)"
                                .to_string(),
                        ),
                    };
                    return serialize_message(&InferMessage::InferResponse(resp));
                }
                // P4: replay protection — only *signed* requests (which we just
                // verified) consult the guard. Replaying an already-seen nonce
                // from the same authenticated sender is rejected before
                // admission/queue/backend, so output is never duplicated.
                if let Ok(mut replay_guard) = replay_for_closure.lock() {
                    if replay_guard.check_and_mark(
                        &peer,
                        req.nonce,
                        std::time::Instant::now(),
                    ) == crate::replay::ReplayCheck::Rejected
                    {
                        let resp = InferResponse {
                            request_id: req.request_id,
                            trace_id: req.trace_id.clone(),
                            worker_peer_id: local_peer,
                            completed_at: chrono::Utc::now().to_rfc3339(),
                            output: "".to_string(),
                            tokens_used: 0,
                            processing_time_ms: 0,
                            success: false,
                            error: Some(format!(
                                "replay detected: nonce {} already processed for this peer",
                                req.nonce
                            )),
                        };
                        if let Some(logs_dir) = &logs_dir_for_infer {
                            decentraai_audit::record_best_effort(
                                logs_dir,
                                "replay_rejected",
                                serde_json::json!({
                                    "request_id": req.request_id.to_string(),
                                    "peer": peer.to_string(),
                                    "nonce": req.nonce,
                                }),
                            );
                        }
                        return serialize_message(&InferMessage::InferResponse(resp));
                    }
                }
                // H1: per-peer sliding-window limit. An abusive/anomalous peer
                // that exceeds its burst is answered terminal (never executed),
                // protecting the engine and queue from overload.
                let within_burst = peer_limiter_for_closure
                    .lock()
                    .map(|mut l| l.allow(&peer, std::time::Instant::now()))
                    .unwrap_or(true);
                if !within_burst {
                    let resp = InferResponse {
                        request_id: req.request_id,
                        trace_id: req.trace_id.clone(),
                        worker_peer_id: local_peer,
                        completed_at: chrono::Utc::now().to_rfc3339(),
                        output: "".to_string(),
                        tokens_used: 0,
                        processing_time_ms: 0,
                        success: false,
                        error: Some("rate limited: peer exceeded the requests/minute budget".to_string()),
                    };
                    if let Some(logs_dir) = &logs_dir_for_infer {
                        decentraai_audit::record_best_effort(
                            logs_dir,
                            "peer_rate_limited",
                            serde_json::json!({
                                "request_id": req.request_id.to_string(),
                                "peer": peer.to_string(),
                                "limit_per_minute": peer_limit,
                            }),
                        );
                    }
                    return serialize_message(&InferMessage::InferResponse(resp));
                }
                // Only accept requests for the configured model.
                if req.model_hash != model_hash_clone {
                    // On-demand provisioning (M14): a workload for a model we do
                    // not hold yet is answered with InferAccepted immediately
                    // (the requester keeps waiting on the tracker) while a
                    // background task downloads, verifies, and serves the model.
                    if let Some(prov) = &provisioning_clone {
                        let accepted = serialize_message(&InferMessage::InferAccepted {
                            request_id: req.request_id,
                            worker_peer_id: local_peer,
                            estimated_wait_ms: 10,
                        })?;
                        let prov = prov.clone();
                        let ctx = Provisioner {
                            p2p: p2p_for_closure.clone(),
                            queue_mgr: queue_for_closure.clone(),
                            worker_manager: wm_for_closure.clone(),
                            provisioned: provisioned_clone.clone(),
                            semaphore: semaphore_clone.clone(),
                            reservations: reservations_closure.clone(),
                            metrics: metrics_for_infer.clone(),
                        };
                        tokio::spawn(async move {
                            provision_on_demand(&prov, ctx, req).await;
                        });
                        return Ok(accepted);
                    }

                    let resp = InferResponse {
                        request_id: req.request_id,
                        trace_id: req.trace_id.clone(),
                        worker_peer_id: local_peer,
                        completed_at: chrono::Utc::now().to_rfc3339(),
                        output: "".to_string(),
                        tokens_used: 0,
                        processing_time_ms: 0,
                        success: false,
                        error: Some("Model not available on this worker".to_string()),
                    };
                    return serialize_message(&InferMessage::InferResponse(resp));
                }

                // Worker-side admission gate (M15): book a local reservation for
                // this workload and refuse to serve it when the request's model
                // footprint would exceed the free capacity this node advertised.
                // Skip the gate when no advertisement has been broadcast yet or no
                // compute manager is attached (nothing was committed to the mesh).
                let reservation = {
                    let mut ledger = reservations_closure
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let (avail_ram, avail_vram, est_ram, est_vram) = match compute_for_closure
                        .as_ref()
                        .and_then(|c| c.last_local_advertisement_sync())
                    {
                        Some(ad) => {
                            // The model we are asked to serve is our *active*
                            // model (`req.model_hash == model_hash_clone` was
                            // already enforced above), so its weights are
                            // resident and already reflected in the live
                            // `available_ram_mb` probe. Charging `est_ram_mb`
                            // again double-counts them and rejects requests
                            // for the very model the engine is running — e.g.
                            // Llama-3.2-1B (1216 MiB est) + min-free 1024 =
                            // 2240 MiB on a worker with ~1992 MiB free. The
                            // marginal cost of a new request on a resident
                            // model is the KV/context working set only.
                            let est_ram = {
                                let e = ad.capability.request_ram_mb(&req.model_hash);
                                // Defensive: an unknown hash costs the default
                                // full-load estimate. In practice the hash was
                                // already validated against the active model
                                // above, so this branch is unreachable.
                                if e == 0 { DEFAULT_EST_RAM_MB } else { e }
                            };
                            let est_vram = ad
                                .capability
                                .model(&req.model_hash)
                                .map_or(0, |m| m.est_vram_mb);
                            (
                                ad.availability.available_ram_mb,
                                ad.availability.available_vram_mb,
                                est_ram,
                                est_vram,
                            )
                        }
                        None => (0, None, 0, 0),
                    };
                    if avail_ram > 0 {
                        let capacity = Admission {
                            available_ram_mb: avail_ram,
                            available_vram_mb: avail_vram,
                            min_free_ram_mb: MIN_FREE_RAM_MB,
                            min_free_vram_mb: MIN_FREE_VRAM_MB,
                        };
                        if let Err(reason) = ledger.admit(&local_peer, capacity, est_ram, est_vram)
                        {
                            let failed = InferMessage::InferFailed {
                                request_id: req.request_id,
                                worker_peer_id: local_peer,
                                error: format!(
                                    "worker has insufficient free capacity: {}",
                                    describe_admit_reason(reason)
                                ),
                                retryable: true,
                                code: Some(InferErrorCode::Capacity),
                            };
                            return serialize_message(&failed);
                        }
                        ledger.reserve(local_peer, est_ram, est_vram)
                    } else {
                        None
                    }
                };

                // Send to processing channel (background task will enqueue).
                // The channel is bounded (see `inbound_cap` above), so an
                // abusive sender cannot push unlimited queued work: when the
                // pipe is already full we reject the request here (a single
                // terminal `InferFailed`) and release the capacity booked at
                // admission rather than letting it linger until the TTL. A
                // channel that is closed means the worker is shutting down.
                if let Err(send_err) = tx_clone.try_send((req.clone(), reservation)) {
                    let shutting_down = matches!(
                        send_err,
                        tokio::sync::mpsc::error::TrySendError::Closed(_)
                    );
                    let (_, reservation) = send_err.into_inner();
                    if let Some(res) = &reservation {
                        reservations_closure
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .release(res.reservation_id);
                    }
                    let failed = InferMessage::InferFailed {
                        request_id: req.request_id,
                        worker_peer_id: local_peer,
                        error: if shutting_down {
                            "worker is shutting down".to_string()
                        } else {
                            "worker queue is full (backpressure)".to_string()
                        },
                        retryable: !shutting_down,
                        code: Some(InferErrorCode::Capacity),
                    };
                    return serialize_message(&failed);
                }

                // Respond with InferAccepted echoing the ORIGINAL request id so the
                // requester can correlate subsequent progress frames.
                let msg = InferMessage::InferAccepted {
                    request_id: req.request_id,
                    worker_peer_id: local_peer,
                    estimated_wait_ms: 10,
                };
                serialize_message(&msg)
            },
        );

        // Spawn background task to process queued requests and stream
        tokio::spawn(async move {
            // Worker-side bookings keyed by request id, mirroring the queue so
            // a request swept by the timeout sweep can release the capacity
            // booked at admission (M15). The reservation accompanies the request
            // through the inbound channel but is not stored in the queue, so we
            // keep it here until the request is either served or swept.
            let mut pending_bookings: HashMap<
                uuid::Uuid,
                Option<decentraai_compute::ResourceReservation>,
            > = HashMap::new();
            // Periodic worker-side timeout sweep. A request may sit in the
            // worker's own queue (e.g. while an earlier request streams out) and
            // must be timed out by the worker itself, not only by the requester
            // (M10 audit: the queue timeout helpers were defined but never wired
            // into the serving path). Every swept request is answered with a
            // single terminal `InferFailed` and its reservation is released.
            let mut sweep = tokio::time::interval(std::time::Duration::from_millis(250));

            loop {
                let inbound = tokio::select! {
                    item = rx.recv() => item,
                    _ = sweep.tick() => {
                        for swept in queue_mgr.cleanup_timed_out().await {
                            if let Some(res) = pending_bookings
                                .remove(&swept.request_id)
                                .flatten()
                            {
                                reservations_for_task
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .release(res.reservation_id);
                            }
                            if let Some(m) = &compute_metrics {
                                m.record_failure();
                            }
                            if let Ok(bytes) = serialize_message(&InferMessage::InferFailed {
                                request_id: swept.request_id,
                                worker_peer_id: local_peer,
                                error: "request timed out waiting in worker queue".to_string(),
                                retryable: true,
                                code: Some(InferErrorCode::Timeout),
                            }) {
                                let _ = p2p_clone
                                    .request(swept.request.sender_peer_id, bytes)
                                    .await;
                            }
                        }
                        continue;
                    }
                };
                let Some((req, reservation)) = inbound else {
                    break;
                };

                // Queue the request; a full queue is answered immediately so the
                // requester is never left hanging. The local reservation booked
                // at admission is released — the workload never ran.
                if !queue_mgr.queue_request(req.clone(), local_peer).await {
                    if let Some(res) = &reservation {
                        reservations_for_task
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .release(res.reservation_id);
                    }
                    if let Some(m) = &compute_metrics {
                        m.record_failure();
                    }
                    if let Ok(bytes) = serialize_message(&InferMessage::InferFailed {
                        request_id: req.request_id,
                        worker_peer_id: local_peer,
                        error: "worker queue is full".to_string(),
                        retryable: true,
                        code: Some(InferErrorCode::Capacity),
                    }) {
                        let _ = p2p_clone.request(req.sender_peer_id, bytes).await;
                    }
                    continue;
                }

                // Hold the booking until the request is served or swept; it is
                // now owned by the queue (indexed by request id).
                pending_bookings.insert(req.request_id, reservation);

                if let Some(m) = &compute_metrics {
                    m.set_queue_depth(queue_mgr.queue_depth(&local_peer).await as u32);
                }

                // Dequeue and process the request to a single terminal event.
                if let Some(queued) = queue_mgr.dequeue_request(&local_peer).await {
                    // The reservation lives on with the streaming task.
                    let reservation = pending_bookings.remove(&queued.request_id).flatten();
                    if let Some(m) = &compute_metrics {
                        m.set_queue_depth(queue_mgr.queue_depth(&local_peer).await as u32);
                    }
                    stream_request_to_terminal(
                        WorkerStreamCtx {
                            backend: &backend_clone,
                            p2p: &p2p_clone,
                            queue: &queue_mgr,
                            worker_manager: &worker_manager,
                            reservations: &reservations_for_task,
                            metrics: compute_metrics.as_ref(),
                        },
                        reservation,
                        queued,
                    )
                    .await;
                }
            }
        });

        Ok(())
    }

    /// Routes an inference request to the best available worker
    ///
    /// This will:
    /// 1. Select the best worker (capability-aware when a compute manager is
    ///    attached, otherwise the legacy capacity scheduler)
    /// 2. Send the request to that worker
    /// 3. Handle the response or trigger fallback on failure
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request to route
    ///
    /// # Returns
    ///
    /// The inference response from a worker, or an error if all workers fail
    pub async fn route_request(
        &self,
        request: InferRequest,
    ) -> Result<InferResponse, DistributedError> {
        // P1 signing happens inside route_request_inner.
        self.route_request_inner(request, None).await
    }

    /// Routes a request, preferring the given worker on the first attempt
    /// (Next-Gen batch pinning). Honors a batch allocation by reserving the
    /// exact worker when it is still eligible; if it is not (dropped, unhealthy,
    /// full, untrusted, local node), falls back to normal planning. Retries
    /// re-plan freely (the preferred worker may have failed), so pinning never
    /// strands a request or violates capacity/trust/KV.
    pub async fn route_request_on(
        &self,
        preferred: &libp2p::PeerId,
        request: InferRequest,
    ) -> Result<InferResponse, DistributedError> {
        self.route_request_inner(request, Some(preferred)).await
    }

    /// Shared single-request routing. `preferred` (optional) is the batch
    /// allocation's worker to pin on the first attempt; retries re-plan.
    async fn route_request_inner(
        &self,
        mut request: InferRequest,
        preferred: Option<&libp2p::PeerId>,
    ) -> Result<InferResponse, DistributedError> {
        // P1: sign outbound requests so workers can authenticate them.
        self.sign_request(&mut request);
        // Capability-aware compute path: pick a worker that serves the model
        // and has RAM/VRAM headroom, and hold a reservation for the duration.
        // When the compute path cannot even build requirements for the model
        // (no registry data), fall through to the legacy announcement-based
        // router. If the compute path DID attempt the request, its own retry
        // budget + is_retryable() policy is authoritative — a legacy re-send
        // would violate at-most-once (see the boundary after the loop).
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                let prompt_tokens = prompt_token_estimate(&request.prompt);
                let base_backoff_ms = 200u64;
                // M23 Full Autonomy: record the explainable autonomous decision
                // (candidates, constraints, score, KV affinity, engine cap,
                // expected mode, fallback, trace) for the control plane.
                compute
                    .record_decision(
                        &request.request_id.to_string(),
                        &req,
                        prompt_tokens,
                        request.session_id.as_deref(),
                        request.priority,
                        request.stream,
                    )
                    .await;
                let mut last_error = None;
                for attempt in 0u32..=self.config.max_retries {
                    // Batch pinning: on the first attempt, prefer the allocated
                    // worker (plan_and_reserve_on reserves it only if still
                    // eligible, else falls back to normal planning). Retries
                    // re-plan freely — the pinned worker may have failed.
                    let planned = if attempt == 0 {
                        match preferred {
                            Some(p) => {
                                compute
                                    .plan_and_reserve_on(
                                        p,
                                        &req,
                                        prompt_tokens,
                                        request.session_id.as_deref(),
                                        request.priority,
                                    )
                                    .await
                            }
                            None => {
                                compute
                                    .plan_and_reserve(
                                        &req,
                                        prompt_tokens,
                                        request.session_id.as_deref(),
                                        request.priority,
                                    )
                                    .await
                            }
                        }
                    } else {
                        compute
                            .plan_and_reserve(
                                &req,
                                prompt_tokens,
                                request.session_id.as_deref(),
                                request.priority,
                            )
                            .await
                    };
                    let Some((plan, placement, mut trace)) = planned else {
                        break;
                    };
                    // Complete the decision trace's runtime half (observe-only):
                    // the actual reserved worker + reservation + outcome.
                    trace.request_id = request.request_id.to_string();
                    trace.reserved_worker = Some(placement.worker.to_string());
                    trace.reservation_id = Some(placement.reservation.reservation_id.to_string());
                    trace.attempt = attempt;
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    tracing::info!(
                        request_id = %request.request_id,
                        session_id = request.session_id.as_deref().unwrap_or(""),
                        model_hash = %request.model_hash,
                        worker_peer_id = %placement.worker,
                        reservation_id = %placement.reservation.reservation_id,
                        plan_id = %plan.plan_id,
                        stages = %plan.stage_count(),
                        attempt,
                        "fabric planner selected worker"
                    );
                    let result = self
                        .request_router
                        .send_request(&self.p2p_node, request.clone(), task_placement)
                        .await;
                    // Release the booking whether or not the request succeeded.
                    compute.release(placement.reservation.reservation_id).await;
                    // M23: record the executed planner decision for the
                    // dashboard EXECUTION view (real state).
                    let cont = request
                        .session_id
                        .as_deref()
                        .and_then(|s| compute.session_residency(s));
                    compute.record_execution(
                        &request.request_id.to_string(),
                        &plan,
                        &placement,
                        cont.map(|p| p.to_string()),
                        if result.is_ok() {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        ExecutionAttribution {
                            tokens_used: result.as_ref().ok().map(|resp| resp.tokens_used),
                            processing_time_ms: result
                                .as_ref()
                                .ok()
                                .map(|resp| resp.processing_time_ms),
                            attempt,
                        },
                    );
                    // Decision trace: complete the outcome and persist the full
                    // request → candidates → rejection → scoring → selected →
                    // reservation → outcome record (observe-only, deterministic).
                    trace.outcome = if result.is_ok() {
                        "succeeded".to_string()
                    } else {
                        "failed".to_string()
                    };
                    compute.record_selection_trace(trace);
                    // M23 Full Autonomy: correlate the recorded autonomous
                    // decision with this reservation/plan/outcome and append
                    // the Reserved → Executing → Completed/Failed → Released
                    // lifecycle trace for the control plane.
                    compute.finalize_decision(
                        &request.request_id.to_string(),
                        &placement.worker.to_string(),
                        &plan.plan_id,
                        &placement.reservation.reservation_id.to_string(),
                        result.is_ok(),
                    );
                    // M20: record the session's KV prefix residency from the
                    // real tokens the worker reported.
                    if let Some(session_id) = &request.session_id {
                        if let Ok(resp) = &result {
                            compute
                                .record_session_usage(
                                    session_id,
                                    &placement.worker,
                                    &request.model_hash,
                                    resp.tokens_used,
                                )
                                .await;
                        }
                    }
                    // M17: account the routed outcome for contribution.
                    compute
                        .record_outcome(&placement.worker, result.is_ok())
                        .await;
                    // Compute Contribution & Quota — Q1: credit the real
                    // measured work (exactly once, deduped by request_id) so
                    // the worker earns quota for what it actually served. Only
                    // a verified completion with measured usage is credited;
                    // failures, timeouts and transport errors earn nothing.
                    if let Ok(resp) = &result {
                        compute.record_credited_contribution(
                            &placement.worker,
                            &request.request_id.to_string(),
                            true,
                            Some(resp.tokens_used),
                            Some(resp.processing_time_ms),
                        );
                        // Verified-compute-economy: strict "no credit without
                        // evidence" path. Credits ONLY when a valid SelectionTrace
                        // exists (recorded above) — proving the worker was eligible
                        // and the reservation succeeded. Idempotent on request_id.
                        compute.record_evidence_credit(
                            &request.request_id.to_string(),
                            &placement.worker,
                            resp.tokens_used,
                            resp.processing_time_ms,
                        );
                    }
                    // P5: feed the per-worker circuit breaker — a success
                    // resets the run; only a retryable failure counts toward
                    // tripping (rejections/cancellations are not the worker's
                    // fault and never trip it).
                    if result.is_ok() {
                        compute.record_breaker_success(&placement.worker);
                    } else if result
                        .as_ref()
                        .err()
                        .map(|e| e.is_retryable())
                        .unwrap_or(false)
                    {
                        compute.record_breaker_failure(&placement.worker);
                    }
                    // M10: per-request audit event tying request, worker and
                    // model hash to the observed outcome, with resource
                    // attribution from the real worker response.
                    self.audit_routed(
                        &request,
                        &placement.worker,
                        result.is_ok(),
                        result.as_ref().ok().map(|r| r.tokens_used),
                        result.as_ref().ok().map(|r| r.processing_time_ms),
                        attempt,
                    );
                    let success = result.is_ok();
                    if success {
                        return result;
                    }
                    last_error = result.err();
                    let err = last_error.as_ref().unwrap();
                    // M23 Full Autonomy (OBSERVE → ADAPT): decide, from real
                    // state, whether to retry/re-plan or abort. Safety-bound by
                    // `decentraai_fabric::adapt` — never retry after emitting
                    // tokens, never re-send a definitive rejection/cancellation.
                    let is_continuation = request
                        .session_id
                        .as_deref()
                        .map(|s| compute.session_residency(s).is_some())
                        .unwrap_or(false);
                    let budget_remaining = self.config.max_retries.saturating_sub(attempt);
                    // Real remaining capacity from the live registry — than a
                    // fabricated count holds. Any worker still trusted, healthy
                    // and able to serve the model (minus the one we just tried)
                    // is a genuine retry target.
                    let eligible_after_primary = {
                        let total = compute.eligible_worker_count(&request.model_hash).await;
                        total.saturating_sub(1) // exclude the attempt just made
                    };
                    let adaptation = decentraai_fabric::adapt(
                        success,
                        err.is_retryable(),
                        false, // non-streamed send is not a client cancellation
                        0,     // transport failure before any output was delivered
                        eligible_after_primary,
                        budget_remaining,
                        is_continuation,
                    );
                    let should_retry = matches!(
                        adaptation,
                        decentraai_fabric::Adaptation::Retry
                            | decentraai_fabric::Adaptation::Replan
                    );
                    if should_retry && attempt < self.config.max_retries {
                        tracing::warn!(
                            request_id = %request.request_id,
                            worker_peer_id = %placement.worker,
                            error = %err,
                            attempt,
                            adaptation = ?adaptation,
                            "autonomous adapt: replan / retry on a fresh worker"
                        );
                        self.fallback_handler
                            .wait_backoff(attempt, base_backoff_ms)
                            .await;
                        continue;
                    }
                    tracing::warn!(
                        worker_peer_id = %placement.worker,
                        error = %err,
                        attempt,
                        adaptation = ?adaptation,
                        "no safe retry remains; aborting compute path"
                    );
                    break;
                }
                // At-most-once boundary (review finding): a fall-through to
                // the legacy announcement router is a FINAL re-send. It is
                // safe only when the last compute failure provably executed
                // nothing (retryable transport / worker retryable refusal).
                // Ambiguous outcomes — RequestTimeout (the worker MAY have
                // generated while the response was lost), definitive
                // rejections, no workers — must abort: re-sending would
                // duplicate non-idempotent work and bypass is_retryable().
                if let Some(err) = last_error {
                    if !err.is_retryable() {
                        tracing::warn!(
                            error = %err,
                            "compute path exhausted with ambiguous outcome; aborting (no legacy re-send)"
                        );
                        return Err(err);
                    }
                    tracing::warn!(
                        error = %err,
                        "compute path exhausted with retryable failure; final legacy fallback"
                    );
                }
            }
        }

        // Legacy path: get the current worker list from the manager (async,
        // never blocks the runtime).
        let workers = self.worker_manager.get_workers().await;

        // Select the best worker for this request
        let placement = self
            .request_router
            .select_worker(&request, &workers)
            .await?;

        // Send the request and handle the response
        self.request_router
            .send_request(&self.p2p_node, request, placement)
            .await
    }

    /// Like [`route_request`](Self::route_request) but streams each received
    /// `InferProgress` chunk into `progress` as it arrives.
    pub async fn route_request_streamed(
        &self,
        mut request: InferRequest,
        progress: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<InferResponse, DistributedError> {
        // P1: sign outbound requests so workers can authenticate them.
        self.sign_request(&mut request);
        self.route_request_streamed_inner(request, progress, None)
            .await
    }

    /// Like [`route_request_streamed`](Self::route_request_streamed) but prefers
    /// the given worker on the first attempt (Next-Gen batch pinning): it
    /// reserves exactly `preferred` when still eligible (via
    /// `plan_and_reserve_on`), otherwise falls back to normal planning. Uses
    /// the streamed send path, which completes reliably over real LANs even
    /// under high latency.
    pub async fn route_request_streamed_on(
        &self,
        preferred: &libp2p::PeerId,
        mut request: InferRequest,
        progress: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<InferResponse, DistributedError> {
        // P1: sign outbound requests so workers can authenticate them.
        self.sign_request(&mut request);
        self.route_request_streamed_inner(request, progress, Some(preferred))
            .await
    }

    /// Shared streamed-routing logic. `preferred` (optional) pins the first
    /// attempt to a batch-allocated worker; retries re-plan freely.
    async fn route_request_streamed_inner(
        &self,
        mut request: InferRequest,
        progress: tokio::sync::mpsc::UnboundedSender<String>,
        preferred: Option<&libp2p::PeerId>,
    ) -> Result<InferResponse, DistributedError> {
        // P1: sign outbound requests so workers can authenticate them.
        self.sign_request(&mut request);
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                let prompt_tokens = prompt_token_estimate(&request.prompt);
                // M23 Full Autonomy: record the explainable autonomous decision
                // (candidates, constraints, score, KV affinity, engine cap,
                // expected mode, fallback, trace) for the control plane.
                compute
                    .record_decision(
                        &request.request_id.to_string(),
                        &req,
                        prompt_tokens,
                        request.session_id.as_deref(),
                        request.priority,
                        request.stream,
                    )
                    .await;
                if let Some((plan, placement, mut trace)) = match preferred {
                    Some(p) => {
                        compute
                            .plan_and_reserve_on(
                                p,
                                &req,
                                prompt_tokens,
                                request.session_id.as_deref(),
                                request.priority,
                            )
                            .await
                    }
                    None => {
                        compute
                            .plan_and_reserve(
                                &req,
                                prompt_tokens,
                                request.session_id.as_deref(),
                                request.priority,
                            )
                            .await
                    }
                } {
                    // Complete the decision trace's runtime half (observe-only).
                    trace.request_id = request.request_id.to_string();
                    trace.reserved_worker = Some(placement.worker.to_string());
                    trace.reservation_id = Some(placement.reservation.reservation_id.to_string());
                    trace.attempt = 0;
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    tracing::info!(
                        request_id = %request.request_id,
                        session_id = request.session_id.as_deref().unwrap_or(""),
                        model_hash = %request.model_hash,
                        worker_peer_id = %placement.worker,
                        reservation_id = %placement.reservation.reservation_id,
                        plan_id = %plan.plan_id,
                        stages = %plan.stage_count(),
                        "fabric planner selected worker"
                    );
                    let result = self
                        .request_router
                        .send_request_streamed(
                            &self.p2p_node,
                            request.clone(),
                            task_placement,
                            progress.clone(),
                        )
                        .await;
                    compute.release(placement.reservation.reservation_id).await;
                    // M23: record the executed planner decision for the
                    // dashboard EXECUTION view (real state).
                    let cont = request
                        .session_id
                        .as_deref()
                        .and_then(|s| compute.session_residency(s));
                    compute.record_execution(
                        &request.request_id.to_string(),
                        &plan,
                        &placement,
                        cont.map(|p| p.to_string()),
                        if result.is_ok() {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        ExecutionAttribution {
                            tokens_used: result.as_ref().ok().map(|resp| resp.tokens_used),
                            processing_time_ms: result
                                .as_ref()
                                .ok()
                                .map(|resp| resp.processing_time_ms),
                            // The streaming path is intentionally single-attempt
                            // (retrying mid-stream would duplicate partial output).
                            attempt: 0,
                        },
                    );
                    // Decision trace: complete the outcome and persist the full
                    // record (observe-only, deterministic).
                    trace.outcome = if result.is_ok() {
                        "succeeded".to_string()
                    } else {
                        "failed".to_string()
                    };
                    compute.record_selection_trace(trace);
                    // M23 Full Autonomy: correlate the recorded autonomous
                    // decision with this reservation/plan/outcome and append
                    // the Reserved → Executing → Completed/Failed → Released
                    // lifecycle trace for the control plane.
                    compute.finalize_decision(
                        &request.request_id.to_string(),
                        &placement.worker.to_string(),
                        &plan.plan_id,
                        &placement.reservation.reservation_id.to_string(),
                        result.is_ok(),
                    );
                    // M20: record the session's KV prefix residency from the
                    // real tokens the worker reported.
                    if let Some(session_id) = &request.session_id {
                        if let Ok(resp) = &result {
                            compute
                                .record_session_usage(
                                    session_id,
                                    &placement.worker,
                                    &request.model_hash,
                                    resp.tokens_used,
                                )
                                .await;
                        }
                    }
                    compute
                        .record_outcome(&placement.worker, result.is_ok())
                        .await;
                    // Compute Contribution & Quota — Q1: credit the real
                    // measured work for a streamed completion, exactly once by
                    // request_id. Streaming is single-attempt, so no mid-stream
                    // retry can double-credit; only a verified completion with
                    // measured usage earns quota.
                    if let Ok(resp) = &result {
                        compute.record_credited_contribution(
                            &placement.worker,
                            &request.request_id.to_string(),
                            true,
                            Some(resp.tokens_used),
                            Some(resp.processing_time_ms),
                        );
                        // Verified-compute-economy: strict "no credit without
                        // evidence" path (streaming). Credits ONLY when a valid
                        // SelectionTrace exists — proving the worker was eligible
                        // and the reservation succeeded. Idempotent on request_id.
                        compute.record_evidence_credit(
                            &request.request_id.to_string(),
                            &placement.worker,
                            resp.tokens_used,
                            resp.processing_time_ms,
                        );
                    }
                    // P5: feed the per-worker circuit breaker (streaming path).
                    if result.is_ok() {
                        compute.record_breaker_success(&placement.worker);
                    } else if result
                        .as_ref()
                        .err()
                        .map(|e| e.is_retryable())
                        .unwrap_or(false)
                    {
                        compute.record_breaker_failure(&placement.worker);
                    }
                    // M10: per-request audit event (streaming path).
                    self.audit_routed(
                        &request,
                        &placement.worker,
                        result.is_ok(),
                        result.as_ref().ok().map(|r| r.tokens_used),
                        result.as_ref().ok().map(|r| r.processing_time_ms),
                        0,
                    );
                    if result.is_ok() {
                        return result;
                    }
                    tracing::warn!(
                        worker_peer_id = %placement.worker,
                        error = %result.as_ref().err().unwrap(),
                        "fabric-planned worker failed; falling back to legacy router"
                    );
                }
            }
        }

        let workers = self.worker_manager.get_workers().await;
        let placement = self
            .request_router
            .select_worker(&request, &workers)
            .await?;
        self.request_router
            .send_request_streamed(&self.p2p_node, request, placement, progress)
            .await
    }

    /// Produces a deterministic, adaptive batch allocation for a set of
    /// **independent, same-model** requests (Next-Gen adaptive fan-out).
    /// Reuses the pure [`decentraai_fabric::allocate_batch`] over the LIVE
    /// fabric facts for the batch's model, so the allocation reflects real
    /// capacity/load/KV residency at this instant.
    ///
    /// Each entry in `requests` is `(request_id, InferRequest)`. All requests
    /// must target the same `model_hash` (a realistic independent-request
    /// batch); a request with a different model is returned as a non-eligible
    /// assignment so the caller fails it honestly. Returns `None` when no
    /// compute manager is attached (no fabric to allocate over).
    ///
    /// This is the **planner boundary**: it says *which* worker each
    /// independent request should go to. It never splits a single generation.
    /// The actual dispatch reuses the existing single-request
    /// `route_request` path (which is authoritative for capacity/reservation/
    /// retry/quota/KV/recovery).
    pub async fn plan_batch(
        &self,
        requests: &[(String, InferRequest)],
    ) -> Option<decentraai_fabric::BatchAllocation> {
        let compute = self.compute_manager.as_ref()?;
        if requests.is_empty() {
            // Empty batch -> empty allocation (honest).
            return Some(decentraai_fabric::BatchAllocation {
                assignments: Vec::new(),
                worker_shares: std::collections::BTreeMap::new(),
            });
        }

        // Build RequestFacts for every request, then allocate PER MODEL so a
        // mixed-model batch is distributed correctly (each model's requests use
        // the fabric facts for THAT model — a model served only by one node is
        // never pinned to a worker that does not serve it). Deterministic: group
        // by model hash, allocate each group with its own live facts, merge.
        let mut by_model: std::collections::BTreeMap<
            String,
            Vec<(String, decentraai_fabric::RequestFacts)>,
        > = std::collections::BTreeMap::new();
        for (request_id, req) in requests {
            let is_continuation = req.session_id.is_some();
            let prefix = req
                .session_id
                .as_deref()
                .and_then(|s| compute.session_residency(s))
                .map(|p| p.to_string());
            by_model.entry(req.model_hash.clone()).or_default().push((
                request_id.clone(),
                decentraai_fabric::RequestFacts {
                    model_hash: req.model_hash.clone(),
                    est_ram_mb: 512,
                    est_vram_mb: 0,
                    context: decentraai_fabric::ContextProfile {
                        prompt_tokens: prompt_token_estimate(&req.prompt),
                        max_output_tokens: req.max_tokens,
                        is_continuation,
                        prefix_resident_on: prefix,
                    },
                    transfer_mib: 0,
                    local_peer: Some(self.p2p_node.local_peer_id().to_string()),
                    priority: req.priority,
                    required_capability: None,
                    capability_claims: Vec::new(),
                },
            ));
        }

        let mut assignments = Vec::new();
        let mut worker_shares: std::collections::BTreeMap<String, f64> =
            std::collections::BTreeMap::new();
        for (model, group) in by_model {
            let facts = compute.fabric_facts(&model).await;
            let alloc = decentraai_fabric::allocate_batch(
                &group,
                &facts,
                &decentraai_fabric::PlannerConfig::default(),
            );
            assignments.extend(alloc.assignments);
            for (peer, share) in alloc.worker_shares {
                worker_shares.insert(peer, share);
            }
        }
        // Preserve the caller's original request order.
        let original: std::collections::HashMap<String, usize> = requests
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (id.clone(), i))
            .collect();
        assignments.sort_by_key(|a| original.get(&a.request_id).copied().unwrap_or(usize::MAX));
        Some(decentraai_fabric::BatchAllocation {
            assignments,
            worker_shares,
        })
    }

    /// Dispatches a batch of **independent** requests through the existing
    /// single-request routing path, honoring the batch allocation's worker
    /// preference (via [`route_request_on`]) so each request is pinned to its
    /// allocated worker on the first attempt. The single-request path remains
    /// authoritative for capacity, reservation, retry, quota, KV affinity,
    /// recovery and audit; if the allocated worker is no longer eligible, the
    /// request safely falls back to normal planning.
    ///
    /// This is the **operational** boundary: it runs real independent requests,
    /// collects per-request results with provenance, and never invents capacity
    /// or splits a generation.
    ///
    /// Returns one outcome per input request (same order), with the request id,
    /// the chosen worker, and the result (or an honest error).
    pub async fn route_batch(
        &self,
        requests: Vec<(String, InferRequest)>,
    ) -> Vec<BatchRequestOutcome> {
        // Compute the batch allocation over the live fabric (deterministic).
        let alloc = self.plan_batch(&requests).await;
        let assignments: std::collections::HashMap<String, String> = alloc
            .as_ref()
            .map(|a| {
                a.assignments
                    .iter()
                    .map(|x| (x.request_id.clone(), x.worker.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut out = Vec::with_capacity(requests.len());
        for (request_id, request) in requests {
            // Pin to the allocated worker if it is eligible (non-empty and
            // eligible in the allocation); otherwise route normally.
            let allocated = assignments.get(&request_id).filter(|w| !w.is_empty());
            let preferred: Option<libp2p::PeerId> = allocated.and_then(|w| w.parse().ok());
            // Use the streamed send path (drops progress): over a real LAN the
            // streaming request/response completes reliably even under high
            // latency, whereas the non-streamed send can time out waiting for a
            // buffered final response. Matches the proven chat-remote path.
            let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let mut req = request.clone();
            req = req.with_streaming(true);
            let result = match preferred {
                Some(p) => self.route_request_streamed_on(&p, req, progress_tx).await,
                None => self.route_request_streamed(req, progress_tx).await,
            };
            let _ = progress_rx; // drained; output carried in the response
            let worker = match &result {
                Ok(r) => r.worker_peer_id.to_string(),
                Err(_) => String::new(),
            };
            out.push(BatchRequestOutcome {
                request_id,
                worker,
                result,
            });
        }
        out
    }

    /// Cancels an in-flight request on a worker by sending an `InferCancel`
    /// frame. The worker aborts generation and replies with
    /// `InferFailed(cancelled)`, which the router reports as an error to the
    /// caller that is still awaiting the request.
    pub async fn cancel_request(
        &self,
        worker_peer_id: libp2p::PeerId,
        request_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        use decentraai_protocol::{InferMessage, serialize_message};
        let msg = InferMessage::InferCancel {
            request_id,
            reason: "cancelled by coordinator".to_string(),
        };
        let bytes = serialize_message(&msg)?;
        self.p2p_node.request(worker_peer_id, bytes).await?;
        Ok(())
    }

    /// Broadcasts this worker's current status to all connected peers
    pub async fn broadcast_worker_status(&self) -> anyhow::Result<()> {
        self.worker_manager.broadcast_status(&self.p2p_node).await
    }

    /// Updates the capacity of the local worker
    ///
    /// Call this when the worker's capacity changes (e.g., after loading/unloading models)
    ///
    /// # Arguments
    ///
    /// * `available_capacity` - New available capacity (0.0 - 1.0)
    /// * `queue_depth` - Current queue depth
    /// * `tokens_per_second` - Current throughput
    /// * `current_latency_ms` - Current latency estimate
    pub fn update_local_capacity(
        &mut self,
        available_capacity: f32,
        queue_depth: u32,
        tokens_per_second: u32,
        current_latency_ms: u32,
    ) -> anyhow::Result<()> {
        self.worker_manager.update_local_capacity(
            available_capacity,
            queue_depth,
            tokens_per_second,
            current_latency_ms,
        )
    }

    /// Gets statistics about the distributed inference system (synchronous)
    /// Note: queued_requests will be 0 as it requires async access
    pub fn get_stats(&self) -> DistributedStats {
        DistributedStats {
            worker_count: self.worker_manager.worker_count_sync(),
            local_worker_registered: self.worker_manager.is_registered_sync(),
            pending_requests: self.request_router.pending_requests_sync(),
            total_requests: self.request_router.total_requests_sync(),
            successful_requests: self.request_router.successful_requests_sync(),
            failed_requests: self.request_router.failed_requests_sync(),
            queued_requests: 0, // Async access needed for accurate count
        }
    }

    /// Gets statistics about the distributed inference system (async)
    /// Includes queue information which requires async access. Uses async
    /// locks throughout so it is safe to call from inside the runtime.
    pub async fn get_stats_async(&self) -> DistributedStats {
        DistributedStats {
            worker_count: self.worker_manager.worker_count().await,
            local_worker_registered: self.worker_manager.is_registered().await,
            pending_requests: self.request_router.pending_requests().await,
            total_requests: self.request_router.total_requests().await,
            successful_requests: self.request_router.successful_requests().await,
            failed_requests: self.request_router.failed_requests().await,
            queued_requests: self.queue_manager.total_queued().await,
        }
    }

    /// Shuts down the underlying P2P node
    pub fn shutdown(self) {
        self.p2p_node.shutdown();
    }
}

/// Drives one queued request through the backend stream to exactly ONE
/// terminal event. Normal completion sends a single `InferResponse` with the
/// accumulated output, token count and wall time; cancellation and backend
/// errors each send a single `InferFailed` (retryable=false for cancellation,
/// retryable=true for transient backend failures). The queue is always
/// `complete_request`ed so the worker's in-flight set stays consistent.
///
/// Bundles the worker-side handles (M15/M16) so the free function stays under
/// clippy's argument cap.
struct WorkerStreamCtx<'a> {
    backend: &'a decentraai_inference_adapter::OpenAiCompatibleBackend,
    p2p: &'a decentraai_p2p::P2PNode,
    queue: &'a RequestQueueManager,
    worker_manager: &'a WorkerManager,
    reservations: &'a Arc<std::sync::Mutex<ReservationLedger>>,
    metrics: Option<&'a std::sync::Arc<crate::compute::RuntimeMetrics>>,
}

async fn stream_request_to_terminal(
    ctx: WorkerStreamCtx<'_>,
    reservation: Option<decentraai_compute::ResourceReservation>,
    queued: QueuedRequest,
) {
    use decentraai_inference_adapter::{BackendRequest, InferenceBackend};
    use decentraai_protocol::{InferMessage, InferProgress, InferResponse};

    let WorkerStreamCtx {
        backend,
        p2p,
        queue,
        worker_manager,
        reservations,
        metrics,
    } = &ctx;

    let request_id = queued.request_id;
    let sender = queued.request.sender_peer_id;
    let trace_id = queued.request.trace_id.clone();
    let local_peer = p2p.local_peer_id();
    let started = std::time::Instant::now();

    // H8: correlate every downstream log line for this request. The span is
    // entered for the whole function so stdout/std of the worker carry
    // request_id/trace_id (and show up as structured fields with --log-format
    // json), making requests traceable end-to-end.
    let _span = tracing::info_span!(
        "infer_request",
        request_id = %request_id,
        trace_id = %trace_id,
        sender = %sender,
    );
    let _entered = _span.enter();

    // Worker-side reservation release (M15): every terminal path below must
    // free the RAM/VRAM booked at admission so capacity can be reused.
    let release_reservation =
        |reservations: &Arc<std::sync::Mutex<ReservationLedger>>,
         reservation: &Option<decentraai_compute::ResourceReservation>| {
            if let Some(res) = reservation {
                reservations
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .release(res.reservation_id);
            }
        };

    // Mark the worker busy for the duration of the request.
    let _ = worker_manager.update_local_capacity(0.0, 0, 0, 0);

    let backend_req = BackendRequest {
        request_id: request_id.to_string(),
        prompt: queued.request.prompt.clone(),
        max_tokens: queued.request.max_tokens,
        temperature: queued.request.temperature,
        top_p: queued.request.top_p,
    };

    let stream = match backend.stream(backend_req).await {
        Ok(stream) => stream,
        Err(e) => {
            release_reservation(reservations, &reservation);
            if let Some(m) = metrics {
                m.record_failure();
            }
            send_infer(
                p2p,
                sender,
                InferMessage::InferFailed {
                    request_id,
                    worker_peer_id: local_peer,
                    error: format!("backend error: {e}"),
                    retryable: true,
                    code: Some(InferErrorCode::Engine),
                },
            )
            .await;
            let _ = queue.complete_request(request_id).await;
            let _ = worker_manager.update_local_capacity(1.0, 0, 50, 100);
            return;
        }
    };

    let mut stream = stream;
    let mut seq: u64 = 0;
    let mut output = String::new();
    let mut terminal_sent = false;

    while let Some(next) = next_stream_item(&mut stream, queue, request_id).await {
        match next {
            NextItem::Chunk(Ok(chunk)) => {
                seq += 1;
                output.push_str(&chunk.text);
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferProgress(InferProgress {
                        request_id,
                        worker_peer_id: local_peer,
                        tokens_generated: seq as u32,
                        partial_output: chunk.text.clone(),
                        percent_complete: 0.0,
                    }),
                )
                .await;
            }
            NextItem::Chunk(Err(e)) => {
                terminal_sent = true;
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferFailed {
                        request_id,
                        worker_peer_id: local_peer,
                        error: format!("backend error: {e}"),
                        retryable: true,
                        code: Some(InferErrorCode::Engine),
                    },
                )
                .await;
                break;
            }
            NextItem::Cancelled => {
                terminal_sent = true;
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferFailed {
                        request_id,
                        worker_peer_id: local_peer,
                        error: "cancelled".to_string(),
                        retryable: false,
                        code: Some(InferErrorCode::Cancelled),
                    },
                )
                .await;
                break;
            }
            NextItem::End => break, // normal completion, terminal below
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if !terminal_sent {
        if let Some(m) = metrics {
            m.record_completion(seq, elapsed_ms);
        }
        send_infer(
            p2p,
            sender,
            InferMessage::InferResponse(InferResponse {
                request_id,
                trace_id,
                worker_peer_id: local_peer,
                completed_at: chrono::Utc::now().to_rfc3339(),
                output,
                tokens_used: seq as u32,
                processing_time_ms: elapsed_ms as u32,
                success: true,
                error: None,
            }),
        )
        .await;
    } else if let Some(m) = metrics {
        m.record_failure();
    }

    let _ = queue.complete_request(request_id).await;
    if let Some(m) = metrics {
        m.set_queue_depth(queue.queue_depth(&local_peer).await as u32);
    }
    release_reservation(reservations, &reservation);
    let _ = worker_manager.update_local_capacity(1.0, 0, 50, 100);
}

/// Sends one InferMessage frame to `sender` via the P2P request/response
/// channel. Errors are logged, never fatal: a dropped requester must not
/// take down the worker loop.
/// Serves a workload for a model the worker does not yet hold (M14).
///
/// Downloads `req.model_hash` from the requester through the verified
/// transfer pipeline (per-chunk BLAKE3 + Merkle root + atomic rename),
/// indexes the verified model into the local registry, loads it into an
/// inference engine, and only then streams the request to a terminal
/// event. Any failure is reported as a terminal `InferFailed`; this
/// function never panics or takes down the node.
async fn provision_on_demand(prov: &ProvisioningConfig, ctx: Provisioner, req: InferRequest) {
    use decentraai_protocol::{InferMessage, InferProgress};

    let p2p = &ctx.p2p;
    let local_peer = p2p.local_peer_id();
    let sender = req.sender_peer_id;
    let model_hash = req.model_hash.clone();
    let request_id = req.request_id;

    // Already provisioned earlier in this session? Serve straight away.
    let cached = ctx
        .provisioned
        .lock()
        .await
        .get(&model_hash)
        .map(|(_, b)| b.clone());
    if let Some(backend) = cached {
        serve_provisioned_request(&backend, &ctx, local_peer, req).await;
        return;
    }

    // Bound concurrent downloads (config `sharing.max_concurrent_downloads`).
    let permit = match ctx.semaphore.clone().acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => return, // semaphore closed during shutdown
    };

    // Keepalive: an empty progress frame resets the requester's wait clock
    // so a slow transfer does not trip the coordinator timeout mid-download.
    send_infer(
        p2p,
        sender,
        InferMessage::InferProgress(InferProgress {
            request_id,
            worker_peer_id: local_peer,
            tokens_generated: 0,
            partial_output: String::new(),
            percent_complete: 0.0,
        }),
    )
    .await;

    // Download + verify through the existing transfer pipeline.
    let mut reputation = match decentraai_p2p::reputation::ReputationStore::load(
        &prov.reputation_path,
        prov.max_invalid_chunks,
        prov.ban_duration,
    ) {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!(error = %e, "failed to open reputation store for provisioning");
            return;
        }
    };
    let model_path = match decentraai_p2p::transfer::download(
        p2p,
        sender,
        &model_hash,
        &prov.data_dir,
        &mut reputation,
    )
    .await
    {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!(peer = %sender, error = %e, "on-demand model provisioning failed");
            send_infer(
                p2p,
                sender,
                InferMessage::InferFailed {
                    request_id,
                    worker_peer_id: local_peer,
                    error: format!("model provisioning failed: {e}"),
                    retryable: false,
                    code: Some(InferErrorCode::Engine),
                },
            )
            .await;
            return;
        }
    };

    // Index the verified model into the local registry, creating the
    // registry file on first provisioning (a fresh node has none yet).
    let models_dir = prov.data_dir.join("models");
    match decentraai_registry::ModelRegistry::load(&prov.registry_path) {
        Ok(mut registry) => {
            let _ = registry.scan_directory(&models_dir);
            let _ = registry.save(&prov.registry_path);
        }
        Err(_) => {
            if let Ok(mut registry) = decentraai_registry::ModelRegistry::new(models_dir.clone()) {
                if let Some(parent) = prov.registry_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = registry.scan_directory(&models_dir);
                let _ = registry.save(&prov.registry_path);
            }
        }
    }

    // Load it into an engine, keeping the engine alive for the session.
    let (engine, backend) = match (prov.backend_factory)(model_path).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(error = %e, "failed to start engine for provisioned model");
            send_infer(
                p2p,
                sender,
                InferMessage::InferFailed {
                    request_id,
                    worker_peer_id: local_peer,
                    error: format!("provisioned engine failed to start: {e}"),
                    retryable: true,
                    code: Some(InferErrorCode::Engine),
                },
            )
            .await;
            return;
        }
    };
    {
        let mut map = ctx.provisioned.lock().await;
        let entry = map
            .entry(model_hash)
            .or_insert_with(|| (engine, backend.clone()));
        let backend = entry.1.clone();
        drop(map);
        serve_provisioned_request(&backend, &ctx, local_peer, req).await;
    }
    drop(permit);
}

/// Queues a request and streams it through an already-running backend to a
/// terminal event. Mirrors the bound-model worker loop so provisioned
/// models behave exactly like the worker's own model.
async fn serve_provisioned_request(
    backend: &decentraai_inference_adapter::OpenAiCompatibleBackend,
    ctx: &Provisioner,
    local_peer: libp2p::PeerId,
    req: InferRequest,
) {
    use decentraai_protocol::InferMessage;

    let p2p = &ctx.p2p;
    if !ctx.queue_mgr.queue_request(req.clone(), local_peer).await {
        if let Ok(bytes) = decentraai_protocol::serialize_message(&InferMessage::InferFailed {
            request_id: req.request_id,
            worker_peer_id: local_peer,
            error: "worker queue is full".to_string(),
            retryable: true,
            code: Some(InferErrorCode::Capacity),
        }) {
            let _ = p2p.request(req.sender_peer_id, bytes).await;
        }
        return;
    }
    if let Some(queued) = ctx.queue_mgr.dequeue_request(&local_peer).await {
        stream_request_to_terminal(
            WorkerStreamCtx {
                backend,
                p2p,
                queue: &ctx.queue_mgr,
                worker_manager: &ctx.worker_manager,
                reservations: &ctx.reservations,
                metrics: ctx.metrics.as_ref(),
            },
            None,
            queued,
        )
        .await;
    }
}

async fn send_infer(p2p: &decentraai_p2p::P2PNode, sender: libp2p::PeerId, msg: InferMessage) {
    if let Ok(bytes) = decentraai_protocol::serialize_message(&msg) {
        if let Err(e) = p2p.request(sender, bytes).await {
            tracing::debug!(%sender, error = %e, "failed to deliver infer frame");
        }
    }
}

/// One unit of a streamed backend response, enriched with the cancellation
/// signal so the caller can distinguish stream end from an abort.
enum NextItem {
    Chunk(
        Result<
            decentraai_inference_adapter::StreamChunk,
            decentraai_inference_adapter::BackendError,
        >,
    ),
    End,
    Cancelled,
}

/// Awaits the next streamed chunk OR a cancellation signal, whichever fires
/// first. The cancellation poll makes aborts prompt even while the backend
/// is between chunks (token generation is CPU-bound and may stall briefly).
async fn next_stream_item<S>(
    stream: &mut S,
    queue: &RequestQueueManager,
    request_id: uuid::Uuid,
) -> Option<NextItem>
where
    S: futures::Stream<
            Item = Result<
                decentraai_inference_adapter::StreamChunk,
                decentraai_inference_adapter::BackendError,
            >,
        > + Unpin
        + Send,
{
    use futures::StreamExt;
    tokio::select! {
        item = stream.next() => match item {
            Some(r) => Some(NextItem::Chunk(r)),
            None => Some(NextItem::End),
        },
        _ = cancel_poll(queue, request_id) => Some(NextItem::Cancelled),
    }
}

/// Polls the queue's cancellation flag until it is set.
async fn cancel_poll(queue: &RequestQueueManager, request_id: uuid::Uuid) {
    loop {
        if queue.is_cancelled(request_id).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Statistics about the distributed inference system
#[derive(Debug, Clone, serde::Serialize)]
pub struct DistributedStats {
    pub worker_count: usize,
    pub local_worker_registered: bool,
    pub pending_requests: u64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub queued_requests: usize,
}

/// Rough prompt-token estimate from character count. A true tokenizer lives in
/// the model engine; this is the conservative ~4 chars/token approximation the
/// KV-aware planner (M20) uses to steer long-context requests to KV-rich
/// workers. It need not be exact — only proportionate.
pub fn prompt_token_estimate(prompt: &str) -> u32 {
    (prompt.chars().count() as u32).div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distributed_stats_default() {
        let stats = DistributedStats {
            worker_count: 0,
            local_worker_registered: false,
            pending_requests: 0,
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            queued_requests: 0,
        };

        assert!(!stats.local_worker_registered);
        assert_eq!(stats.worker_count, 0);
    }

    #[test]
    fn error_retry_policy_is_idempotency_safe() {
        // Connection-level failures before delivery are retryable (the worker
        // almost certainly never started).
        assert!(DistributedError::P2PError(anyhow::anyhow!("conn refused")).is_retryable());
        // A RequestTimeout means the worker MAY have executed while the
        // response was lost/slow. Re-sending the same logical request to a
        // different worker would re-generate (non-idempotent) and double its
        // token/KV accounting, so it must NOT be retried (at-most-once).
        assert!(!DistributedError::RequestTimeout(1000).is_retryable());
        // A definitive rejection or cancellation must never be duplicated.
        assert!(!DistributedError::WorkerRejected("w".into(), "cancelled".into()).is_retryable());
        assert!(!DistributedError::NoWorkersAvailable("m".into()).is_retryable());
        assert!(!DistributedError::UntrustedWorker("w".into()).is_retryable());
        assert!(!DistributedError::AllWorkersFailed("w failed".into()).is_retryable());
        // The worker's EXPLICIT retryable signal (InferFailed.retryable=true,
        // refused before executing) is idempotency-safe to re-send.
        assert!(DistributedError::WorkerRetryable("model not loaded yet".into()).is_retryable());
    }

    #[test]
    fn worker_retryable_maps_to_stable_code() {
        use decentraai_protocol::InferErrorCode as Code;
        let e = DistributedError::WorkerRetryable("model not loaded yet".into());
        assert_eq!(e.code(), Code::RetryableWorker);
        assert_eq!(e.code().code(), "retryable_worker");
    }

    #[test]
    fn distributed_error_maps_to_stable_error_code() {
        use decentraai_protocol::InferErrorCode as Code;
        // Required category coverage (stable, non-retryable rejection).
        let rejected = DistributedError::WorkerRejected("w".into(), "busy".into());
        assert_eq!(rejected.code(), Code::Rejected);
        assert!(!rejected.is_retryable());
        assert_eq!(Code::from(&rejected), Code::Rejected);

        assert_eq!(DistributedError::RequestTimeout(1000).code(), Code::Timeout);
        assert_eq!(
            DistributedError::P2PError(anyhow::anyhow!("conn refused")).code(),
            Code::Transport
        );
        assert_eq!(
            DistributedError::NoWorkersAvailable("m".into()).code(),
            Code::NoWorkers
        );
        assert_eq!(
            DistributedError::AllWorkersFailed("r".into()).code(),
            Code::AllWorkersFailed
        );
        assert_eq!(
            DistributedError::SerializationError("bad".into()).code(),
            Code::Serialization
        );
        assert_eq!(
            DistributedError::UntrustedWorker("w".into()).code(),
            Code::Untrusted
        );
        // Token strings are stable for logging/metrics/clients.
        assert_eq!(rejected.code().code(), "rejected");
        assert_eq!(Code::Timeout.code(), "timeout");
        assert_eq!(Code::Transport.code(), "transport");
    }

    #[tokio::test]
    async fn per_request_audit_records_outcome_when_logs_dir_set() {
        use crate::{DistributedInference, InferenceConfig};
        use decentraai_p2p::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let identity = decentraai_identity::Identity::generate();
        let node = P2PNode::new(
            &identity,
            DEFAULT_MAX_MESSAGE_BYTES,
            DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap();
        let mut distributed =
            DistributedInference::new(node, InferenceConfig::default(), None, None).unwrap();
        distributed.set_logs_dir(Some(logs_dir.clone()));

        let worker = libp2p::PeerId::random();
        let mut req = InferRequest::new("modelhash".into(), "hi".into(), 32);
        req.request_id = uuid::Uuid::new_v4();
        req.trace_id = "tr_audit_test".into();
        req.session_id = Some("sess1".into());

        // Routing a completed request writes an inference_completed audit event
        // carrying request/worker/model-hash/status correlation fields (M10),
        // plus resource attribution (tokens/time/attempt) when known.
        distributed.audit_routed(&req, &worker, true, Some(42), Some(1337), 2);
        let line = std::fs::read_to_string(logs_dir.join("audit.jsonl")).unwrap();
        let event: serde_json::Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
        assert_eq!(event["event"], "inference_completed");
        assert_eq!(event["details"]["request_id"], req.request_id.to_string());
        assert_eq!(event["details"]["trace_id"], "tr_audit_test");
        assert_eq!(event["details"]["worker_id"], worker.to_string());
        assert_eq!(event["details"]["model_hash"], "modelhash");
        assert_eq!(event["details"]["status"], "completed");
        assert_eq!(event["details"]["tokens_used"], 42);
        assert_eq!(event["details"]["processing_time_ms"], 1337);
        assert_eq!(event["details"]["attempt"], 2);
    }
}
