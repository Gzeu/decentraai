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
        AdmitReason::InsufficientRam { available, required } => {
            format!("{available} MiB free RAM below the required {required} MiB")
        }
        AdmitReason::InsufficientVram { available, required } => {
            format!("{available} MiB free VRAM below the required {required} MiB")
        }
    }
}

pub mod compute;
pub mod config;
pub mod fallback;
pub mod p2p_handler;
pub mod queue;
pub mod router;
pub mod tracker;
pub mod worker;

pub use compute::{
    ComputeManager, ComputeMetricsReport, LivePerf, RuntimeMetrics, WorkerMetricRow,
    build_advertisement,
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
    InferMessage, InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement, WorkerStatus,
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
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum DistributedError {
        #[error("No workers available for model {0}")]
        NoWorkersAvailable(String),

        #[error("All workers failed for request {0}")]
        AllWorkersFailed(String),

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
        })
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
    ) -> anyhow::Result<()> {
        use decentraai_protocol::{
            InferMessage, InferResponse, serialize_message,
        };

        let local_peer = self.p2p_node.local_peer_id();
        let (tx, mut rx) =
            tokio::sync::mpsc::unbounded_channel::<(
                decentraai_protocol::InferRequest,
                Option<decentraai_compute::ResourceReservation>,
            )>();
        let tx_clone = tx.clone();
        let p2p_clone = self.p2p_node.clone();
        let queue_mgr = self.queue_manager.clone();
        let backend_clone = backend.clone();
        let worker_manager = self.worker_manager.clone();
        let model_hash_clone = model_hash.clone();
        let compute_for_closure = self.compute_manager.clone();
        // M16: real compute metrics. The worker's streaming task records
        // measured tokens/sec and latency; the queue path keeps depth current.
        let compute_metrics =
            self.compute_manager.as_ref().map(|c| c.runtime_metrics());

        // Worker-side reservation enforcement (M15): the worker keeps its own
        // ledger of in-flight workloads booked against the capacity it
        // advertised, and refuses to serve a request whose footprint would
        // exceed the remaining headroom — even if a buggy or malicious
        // coordinator sent more work than it booked. The TTL is a safety net;
        // reservations are released explicitly on the terminal event.
        let local_reservations: Arc<std::sync::Mutex<ReservationLedger>> = Arc::new(
            std::sync::Mutex::new(ReservationLedger::new(
                std::time::Duration::from_secs(300),
                8,
            )),
        );
        let reservations_closure = local_reservations.clone();
        let reservations_for_task = local_reservations.clone();

        // Engines spawned for on-demand-provisioned models, keyed by model
        // hash. Kept for the worker session so the subprocesses stay alive;
        // they are reaped when the node drops.
        let provisioned: ProvisionedBackends = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let provisioned_clone = provisioned.clone();
        let provisioning_clone = provisioning.clone();
        let provision_semaphore = Arc::new(tokio::sync::Semaphore::new(
            provisioning.as_ref().map(|p| p.max_concurrent_downloads).unwrap_or(0).max(1),
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

        // Register sync on_infer handler that enqueues the request and returns Accept
        self.p2p_node.set_on_infer_request(move |req: decentraai_protocol::InferRequest| -> anyhow::Result<Vec<u8>> {
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
                let mut ledger = reservations_closure.lock().unwrap_or_else(|e| e.into_inner());
                let (avail_ram, avail_vram, est_ram, est_vram) =
                    match compute_for_closure
                        .as_ref()
                        .and_then(|c| c.last_local_advertisement_sync())
                    {
                        Some(ad) => {
                            let est = ad
                                .capability
                                .served_models
                                .iter()
                                .find(|m| m.model_hash == req.model_hash)
                                .map(|m| (m.est_ram_mb, m.est_vram_mb))
                                .unwrap_or((DEFAULT_EST_RAM_MB, 0));
                            (
                                ad.availability.available_ram_mb,
                                ad.availability.available_vram_mb,
                                est.0,
                                est.1,
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
                    if let Err(reason) = ledger.admit(&local_peer, capacity, est_ram, est_vram) {
                        let failed = InferMessage::InferFailed {
                            request_id: req.request_id,
                            worker_peer_id: local_peer,
                            error: format!(
                                "worker has insufficient free capacity: {}",
                                describe_admit_reason(reason)
                            ),
                            retryable: true,
                        };
                        return serialize_message(&failed);
                    }
                    ledger.reserve(local_peer, est_ram, est_vram)
                } else {
                    None
                }
            };

            // Send to processing channel (background task will enqueue). If the
            // channel is gone the worker is shutting down; tell the requester.
            if tx_clone.send((req.clone(), reservation)).is_err() {
                let failed = InferMessage::InferFailed {
                    request_id: req.request_id,
                    worker_peer_id: local_peer,
                    error: "worker is shutting down".to_string(),
                    retryable: false,
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
        });

        // Spawn background task to process queued requests and stream
        tokio::spawn(async move {
            while let Some((req, reservation)) = rx.recv().await {
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
                    }) {
                        let _ = p2p_clone.request(req.sender_peer_id, bytes).await;
                    }
                    continue;
                }

                if let Some(m) = &compute_metrics {
                    m.set_queue_depth(queue_mgr.queue_depth(&local_peer).await as u32);
                }

                // Dequeue and process the request to a single terminal event.
                if let Some(queued) = queue_mgr.dequeue_request(&local_peer).await {
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
        // Capability-aware compute path: pick a worker that serves the model
        // and has RAM/VRAM headroom, and hold a reservation for the duration.
        // If the selected worker fails (offline, rejection, timeout), fall
        // through to the legacy announcement-based router instead of failing
        // the request.
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                if let Some(placement) = compute.select(&req).await {
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    tracing::info!(
                        request_id = %request.request_id,
                        model_hash = %request.model_hash,
                        worker_peer_id = %placement.worker,
                        reservation_id = %placement.reservation.reservation_id,
                        "capability-aware scheduler selected worker"
                    );
                    let result = self
                        .request_router
                        .send_request(&self.p2p_node, request.clone(), task_placement)
                        .await;
                    // Release the booking whether or not the request succeeded.
                    compute.release(placement.reservation.reservation_id).await;
                    if result.is_ok() {
                        return result;
                    }
                    tracing::warn!(
                        worker_peer_id = %placement.worker,
                        error = %result.as_ref().err().unwrap(),
                        "compute-selected worker failed; falling back to legacy router"
                    );
                }
            }
        }

        // Legacy path: get the current worker list from the manager (async,
        // never blocks the runtime).
        let workers = self.worker_manager.get_workers().await;

        // Select the best worker for this request
        let placement = self.request_router.select_worker(&request, &workers).await?;

        // Send the request and handle the response
        self.request_router
            .send_request(&self.p2p_node, request, placement)
            .await
    }

    /// Like [`route_request`](Self::route_request) but streams each received
    /// `InferProgress` chunk into `progress` as it arrives.
    pub async fn route_request_streamed(
        &self,
        request: InferRequest,
        progress: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<InferResponse, DistributedError> {
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                if let Some(placement) = compute.select(&req).await {
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    tracing::info!(
                        request_id = %request.request_id,
                        model_hash = %request.model_hash,
                        worker_peer_id = %placement.worker,
                        reservation_id = %placement.reservation.reservation_id,
                        "capability-aware scheduler selected worker"
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
                    if result.is_ok() {
                        return result;
                    }
                    tracing::warn!(
                        worker_peer_id = %placement.worker,
                        error = %result.as_ref().err().unwrap(),
                        "compute-selected worker failed; falling back to legacy router"
                    );
                }
            }
        }

        let workers = self.worker_manager.get_workers().await;
        let placement = self.request_router.select_worker(&request, &workers).await?;
        self.request_router
            .send_request_streamed(&self.p2p_node, request, placement, progress)
            .await
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

    // Worker-side reservation release (M15): every terminal path below must
    // free the RAM/VRAM booked at admission so capacity can be reused.
    let release_reservation = |reservations: &Arc<std::sync::Mutex<ReservationLedger>>,
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
async fn provision_on_demand(
    prov: &ProvisioningConfig,
    ctx: Provisioner,
    req: InferRequest,
) {
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
    Chunk(Result<decentraai_inference_adapter::StreamChunk, decentraai_inference_adapter::BackendError>),
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
    S: futures::Stream<Item = Result<decentraai_inference_adapter::StreamChunk, decentraai_inference_adapter::BackendError>>
        + Unpin
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
}
