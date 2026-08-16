//! Compute sharing coordinator (M12).
//!
//! Bridges the pure [`decentraai_compute`] scheduler into the async
//! distributed layer. The coordinator aggregates `ComputeAdvertisement`
//! frames received from peers, selects a worker for each workload, and
//! books/releases resource reservations. A node that wants to offer its
//! own GPU builds its advertisement from the real system probe and
//! broadcasts it on the announce interval.
//!
//! The compute path coexists with the legacy `WorkerAnnouncement`
//! discovery: new compute peers advertise hardware; the legacy path keeps
//! serving nodes that have not opted in to compute sharing yet.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use libp2p::PeerId;
use tokio::sync::Mutex;

use decentraai_compute::{CapabilityMatcher, ComputeRegistry, ComputeScheduler, Placement};
pub use decentraai_compute::{
    ComputeAdvertisement, ComputeAvailability, ComputeCapability, GpuSpec, ResourceReservation,
    ServedModel, WorkerHealth, WorkloadRequirements,
};

use decentraai_system_probe::{GpuProbeStatus, GpuSnapshot, SystemSnapshot};

const MIB: u64 = 1024 * 1024;

/// Conservative RAM footprint (MiB) assumed for a workload whose model is
/// not yet on any worker, used only to make provisioning-capable workers
/// schedulable before the model lands (M14).
const PROVISION_DEFAULT_RAM_MB: u64 = 1024;

/// Interval between local compute advertisement broadcasts.
pub const DEFAULT_ADVERTISEMENT_INTERVAL_MS: u64 = 5_000;
/// Heartbeat gap after which a peer's advertisement is treated as stale.
pub const DEFAULT_STALE_AFTER_MS: u64 = 30_000;

/// The compute engine identifier this node runs (matching what the
/// advertisement reports).
pub const ENGINE_LLAMA_SERVER: &str = "llama_server";

/// Leaf snapshot of live per-node performance placed into advertisements so
/// the coordinator's scheduler weighs real throughput/latency/queue load when
/// picking a worker (M16). Mirrors the time-varying fields of
/// [`ComputeAvailability`] and is produced by [`RuntimeMetrics::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LivePerf {
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
}

/// Live performance metrics captured from the *real* inference path (M16).
///
/// Written by the worker's streaming task as each request reaches a terminal
/// event, and by the on_infer/queue paths as the queue depth changes. Only
/// atomics, so both the synchronous `on_infer` callback and the async
/// streaming task can update it without a lock. The EWMA keeps `snapshot()`
/// immune to single slow/fast outliers so scheduler rankings stay stable.
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    tokens_per_second: AtomicU32,
    current_latency_ms: AtomicU32,
    queue_depth: AtomicU32,
    requests_completed: AtomicU64,
    requests_failed: AtomicU64,
    tokens_total: AtomicU64,
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Smooths one measured sample into the running estimate (M16).
    fn ewma(prev: u32, measured: u32) -> u32 {
        (prev as f32 * 0.8 + measured as f32 * 0.2).round() as u32
    }

    /// Records one completed request: tokens/sec derived from its true token
    /// count and wall time, plus its true latency.
    pub fn record_completion(&self, tokens: u64, latency_ms: u64) {
        let secs = (latency_ms.max(1) as f64) / 1000.0;
        let tps = (tokens as f64 / secs) as u32;
        let prev_tps = self.tokens_per_second.load(Ordering::Relaxed);
        self.tokens_per_second
            .store(Self::ewma(prev_tps, tps), Ordering::Relaxed);

        let prev_lat = self.current_latency_ms.load(Ordering::Relaxed);
        let lat = latency_ms.min(u32::MAX as u64) as u32;
        self.current_latency_ms
            .store(Self::ewma(prev_lat, lat), Ordering::Relaxed);

        self.requests_completed.fetch_add(1, Ordering::Relaxed);
        self.tokens_total.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Records one failed request (does not move the perf EWMA).
    pub fn record_failure(&self) {
        self.requests_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Reflects the current worker queue depth.
    pub fn set_queue_depth(&self, depth: u32) {
        self.queue_depth.store(depth, Ordering::Relaxed);
    }

    /// Live perf values for this node's advertisements.
    pub fn snapshot(&self) -> LivePerf {
        LivePerf {
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            tokens_per_second: self.tokens_per_second.load(Ordering::Relaxed),
            current_latency_ms: self.current_latency_ms.load(Ordering::Relaxed),
        }
    }

    /// Lifetime request/token totals for observability.
    pub fn totals(&self) -> (u64, u64, u64) {
        (
            self.requests_completed.load(Ordering::Relaxed),
            self.requests_failed.load(Ordering::Relaxed),
            self.tokens_total.load(Ordering::Relaxed),
        )
    }
}

/// Per-worker cumulative contribution ledger (M17).
///
/// The coordinator records how long each worker has been online (from the
/// heartbeat interval between advertisements) and how many requests it
/// actually served to a verified, terminal completion. This is the raw
/// material for the pure [`decentraai_compute::contribution_score`] /
/// `suggest_tier` engine, and is exposed in the metrics report so the
/// dashboard and API can show why a worker earned its tier.
#[derive(Debug, Default)]
struct ContributionTracker {
    /// Last time this peer advertised, used to accrue online seconds.
    last_announce: Option<Instant>,
    profile: decentraai_compute::ContributionProfile,
}

pub use decentraai_compute::ContributionProfile;

impl ContributionTracker {
    /// Accrues online time and refreshes hardware from a fresh advertisement.
    fn observe(&mut self, adv: &ComputeAdvertisement) {
        let now = Instant::now();
        if let Some(prev) = self.last_announce {
            let gap = now.duration_since(prev).as_secs();
            // Cap accrued online time at one gap so wildly stale
            // advertisements (clock drift, offline windows) can't inflate
            // the score. `saturating_sub` defends against same-tick updates.
            self.profile.online_seconds = self
                .profile
                .online_seconds
                .saturating_add(gap.clamp(0, 3600));
        }
        self.last_announce = Some(now);
        self.profile.cpu_cores = adv.capability.cpu_cores;
        self.profile.ram_mb = adv.capability.ram_mb;
        self.profile.vram_mb = adv.capability.gpu.as_ref().map(|g| g.vram_mb).unwrap_or(0);
    }
}

/// Pure builder: turns a real hardware probe into a `ComputeAdvertisement`.
///
/// Kept as a free function so unit tests can drive it with synthetic
/// snapshots and GPU states without touching `nvidia-smi` or sysinfo. The
/// argument list is long by design (each maps to one advertisement field);
/// building a struct would only obscure the field correspondence.
#[allow(clippy::too_many_arguments)]
pub fn build_advertisement(
    local_peer: PeerId,
    node_name: &str,
    engine: &str,
    snapshot: SystemSnapshot,
    gpu: GpuProbeStatus,
    served_models: Vec<ServedModel>,
    can_provision: bool,
    accepts_remote: bool,
    announced_at_ms: u64,
    perf: LivePerf,
) -> ComputeAdvertisement {
    let (gpu_spec, free_vram_mib, gpu_temp, gpu_util) = match &gpu {
        GpuProbeStatus::Nvidia(info) => (
            Some(GpuSpec {
                name: info.name.clone(),
                vram_mb: info.total_vram_mib * MIB / MIB,
                driver: "nvidia".into(),
            }),
            Some(info.free_vram_mib),
            Some(info.temperature_celsius),
            Some(info.utilization_percent),
        ),
        GpuProbeStatus::Unavailable(_) => (None, None, None, None),
    };

    let load_percent = (snapshot.cpu_usage_percent.clamp(0.0, 100.0)) as u8;

    ComputeAdvertisement {
        peer_id: local_peer,
        node_name: node_name.to_string(),
        capability: ComputeCapability {
            cpu_cores: snapshot.logical_cpus.max(1) as u16,
            ram_mb: snapshot.total_memory_bytes / MIB,
            gpu: gpu_spec,
            engine: engine.to_string(),
            served_models,
            can_provision,
            available_models: vec![],
        },
        availability: ComputeAvailability {
            available_ram_mb: snapshot.available_memory_bytes / MIB,
            available_vram_mb: free_vram_mib,
            load_percent,
            queue_depth: perf.queue_depth,
            tokens_per_second: perf.tokens_per_second,
            current_latency_ms: perf.current_latency_ms,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: gpu_temp,
            gpu_utilization_percent: gpu_util,
        },
        announced_at_ms,
        accepts_remote_inference: accepts_remote,
        node_id: short_node_id(&local_peer),
        node_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Compact, stable human-readable node identifier derived from the peer id
/// (e.g. `dca-8f2a3c`). libp2p ed25519 peer ids serialize as base58 beginning
/// with `12D3KooW`; the next six characters are taken as the indicator. The
/// same derivation is mirrored client-side in the dashboard as a fallback for
/// workers that predate the `node_id` advertisement field.
pub fn short_node_id(peer: &PeerId) -> String {
    let s = peer.to_string();
    let body = s.strip_prefix("12D3KooW").unwrap_or(&s);
    let head = &body[..body.len().min(6)];
    format!("dca-{head}")
}

/// Coordinator-side compute manager.
pub struct ComputeManager {
    local_peer: PeerId,
    node_name: String,
    engine: String,
    advertisement_interval_ms: u64,
    scheduler: Mutex<ComputeScheduler>,
    /// The most recent advertisement this node built and broadcast. The
    /// worker's on-demand admission gate (M15) enforces against this exact
    /// snapshot, so it must be readable synchronously from the P2P on_infer
    /// callback. Held by a std mutex with no await under lock.
    last_local_ad: std::sync::Mutex<Option<ComputeAdvertisement>>,
    /// Live performance metrics captured from real inference (M16). Shared
    /// so the worker's streaming task and the periodic advertiser both see
    /// the same throughput/latency/queue state.
    metrics: std::sync::Arc<RuntimeMetrics>,
    /// Per-worker contribution ledger (M17): accrued online time and served
    /// request counts, feeding the tier-suggestion engine.
    contribution: std::sync::Mutex<BTreeMap<PeerId, ContributionTracker>>,
    /// Live, coordinator-side network graph (M19): measured RTT / bandwidth
    /// to each peer, fed by a periodic `InferPing` probe and read by the
    /// execution planner to weight reach cost.
    network: std::sync::Mutex<decentraai_fabric::NetworkGraph>,
    /// Coordinator-side KV-cache / session accounting (M20): which worker
    /// holds each conversation's KV prefix and the honest per-worker KV
    /// occupancy derived from real routed requests + advertised `n_ctx`.
    sessions: std::sync::Mutex<crate::session::SessionAccount>,
    /// Bounded, newest-first history of executed plans (M23): real planner
    /// decisions + placements + outcomes surfaced by the dashboard EXECUTION
    /// view. `None` until `record_execution` is called.
    recent_executions: std::sync::Mutex<VecDeque<ExecutedPlan>>,
    /// Optional append-only JSON-lines history file (`db/executions.jsonl`).
    /// When set, every `record_execution` appends a best-effort line so the
    /// fabric keeps execution history across restarts; `load_execution_history`
    /// replays it back into `recent_executions` on startup. `None` keeps
    /// execution history in-memory only (default).
    executions_path: std::sync::Mutex<Option<std::path::PathBuf>>,
    /// Bounded, newest-first full autonomous execution decisions (M23 Full
    /// Autonomy): candidates, constraints, score, selected worker, KV affinity,
    /// engine capability, expected mode, fallback and lifecycle trace — the
    /// explainable decision, surfaced by the control plane.
    recent_decisions: std::sync::Mutex<VecDeque<decentraai_fabric::ExecutionDecision>>,
    /// Optional Ed25519 signing key (P3). When set, `advertise_local` emits a
    /// signed [`decentraai_protocol::SignedComputeAdvertisement`] so recipients
    /// can authenticate that the advertisement genuinely came from the node
    /// that claims it (anti-spoof). `None` broadcasts unsigned (legacy).
    signing_key: Option<[u8; 32]>,
    /// Per-worker circuit breaker (P5). Open workers are filtered out of the
    /// planner feed so a consistently failing worker is not re-selected (and
    /// no reservation is booked on it) until its cooldown elapses.
    breaker: std::sync::Mutex<crate::breaker::CircuitBreaker>,
    /// Whether this node accepts inference routed from remote peers
    /// (config `inference.allow_remote_inference`). Advertised honestly so
    /// coordinators never schedule a remote worker that would reject the
    /// request; the local node always accepts its own work regardless.
    /// Atomic so the shared `Arc<ComputeManager>` can flip it at runtime.
    accepts_remote_inference: std::sync::atomic::AtomicBool,
    /// Local model registry path (`db/registry.json`), when known, so the
    /// coordinator can resolve persisted capability claims for a model to give
    /// a real capability-requirement verdict. `None` = no registry wired yet
    /// (honest: claims resolution degrades to UNKNOWN).
    registry_path: std::sync::Mutex<Option<std::path::PathBuf>>,
    /// This node's DecentraAI build version (from CARGO_PKG_VERSION). The
    /// coordinator uses it to classify fabric peers as CURRENT / OUTDATED /
    /// UNKNOWN for the node-lifecycle view.
    node_version: String,
}

impl ComputeManager {
    /// Creates a manager with a fresh scheduler over the given trusted set.
    pub fn new(local_peer: PeerId, node_name: String, trusted: HashSet<PeerId>) -> Self {
        let registry =
            ComputeRegistry::new(std::time::Duration::from_millis(DEFAULT_STALE_AFTER_MS));
        let ledger =
            decentraai_compute::ReservationLedger::new(std::time::Duration::from_secs(60), 4);
        let mut scheduler =
            ComputeScheduler::new(registry, ledger, CapabilityMatcher::default(), trusted);
        // The coordinator's own peer id exempts local work from the remote
        // opt-in gate: a node always serves its own requests.
        scheduler.set_local_peer(local_peer);
        Self {
            local_peer,
            node_name,
            engine: ENGINE_LLAMA_SERVER.to_string(),
            advertisement_interval_ms: DEFAULT_ADVERTISEMENT_INTERVAL_MS,
            scheduler: Mutex::new(scheduler),
            last_local_ad: std::sync::Mutex::new(None),
            metrics: std::sync::Arc::new(RuntimeMetrics::new()),
            contribution: std::sync::Mutex::new(BTreeMap::new()),
            network: std::sync::Mutex::new(decentraai_fabric::NetworkGraph::new()),
            sessions: std::sync::Mutex::new(crate::session::SessionAccount::new()),
            recent_executions: std::sync::Mutex::new(VecDeque::new()),
            executions_path: std::sync::Mutex::new(None),
            recent_decisions: std::sync::Mutex::new(VecDeque::new()),
            signing_key: None,
            breaker: std::sync::Mutex::new(crate::breaker::CircuitBreaker::new(
                crate::breaker::BreakerConfig::default(),
            )),
            accepts_remote_inference: std::sync::atomic::AtomicBool::new(false),
            registry_path: std::sync::Mutex::new(None),
            node_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// This node's DecentraAI build version.
    pub fn node_version(&self) -> &str {
        &self.node_version
    }

    /// Sets the node's Ed25519 signing key (P3) so `advertise_local` emits
    /// signed advertisements that recipients can authenticate.
    pub fn set_signing_key(&mut self, signing_key: [u8; 32]) {
        self.signing_key = Some(signing_key);
    }

    /// Sets whether this node accepts inference routed from remote peers
    /// (config `inference.allow_remote_inference`). The value is advertised
    /// in every heartbeat so coordinators only schedule remote workers that
    /// will actually serve the request. Local work is always accepted.
    pub fn set_accepts_remote_inference(&self, accepts: bool) {
        self.accepts_remote_inference
            .store(accepts, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    pub fn advertisement_interval_ms(&self) -> u64 {
        self.advertisement_interval_ms
    }

    pub fn set_advertisement_interval_ms(&mut self, ms: u64) {
        self.advertisement_interval_ms = ms;
    }

    /// Sets the advertised engine-kind string (M22). Defaults to
    /// [`ENGINE_LLAMA_SERVER`]; recording llama-server runs regardless. Call
    /// this only when the config explicitly selects an alternative engine so
    /// the node honestly advertises what it actually runs and coordinators'
    /// planners can reason engine-aware. The value uses the engine fabric's
    /// wire strings (`vllm`, `sglang`, `ollama`, `openai-compatible`).
    pub fn set_engine(&mut self, engine: &str) {
        self.engine = engine.to_string();
    }

    /// Marks `peer` as trusted (eligible to run workloads).
    pub async fn add_trusted(&self, peer: PeerId) {
        self.scheduler.lock().await.add_trusted(peer);
    }

    /// Enables persistent execution history (Part 17/22). Every subsequent
    /// `record_execution` appends a JSON-lines entry to `path`
    /// (`db/executions.jsonl`) best-effort, and any history already in the
    /// file is replayed into the in-memory ring so a restarted coordinator
    /// keeps its execution trail. Pass `None` to keep history in-memory only.
    pub fn set_executions_path(&self, path: Option<std::path::PathBuf>) {
        let mut slot = self.executions_path.lock().unwrap();
        *slot = path.clone();
        if let Some(p) = path {
            self.replay_execution_history(&p);
        }
    }

    /// Replays `db/executions.jsonl` into the in-memory ring (oldest first,
    /// capped at the ring bound), so history survives coordinator restarts.
    /// Best-effort: a missing/corrupt file only logs and leaves the ring as is.
    fn replay_execution_history(&self, path: &std::path::Path) {
        const MAX_EXECUTIONS: usize = 128;
        let Ok(contents) = std::fs::read_to_string(path) else {
            return; // fresh install: no history yet
        };
        let mut ring = self.recent_executions.lock().unwrap();
        for line in contents.lines() {
            if let Ok(rec) = serde_json::from_str::<ExecutedPlan>(line) {
                ring.push_back(rec);
            }
        }
        while ring.len() > MAX_EXECUTIONS {
            ring.pop_front();
        }
    }

    /// Appends one execution record to the history file, best-effort (never
    /// breaks the routing flow on a write error).
    fn persist_execution(&self, rec: &ExecutedPlan) {
        let path = self.executions_path.lock().unwrap().clone();
        let Some(path) = path else { return };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(error = %e, "failed to create executions history dir");
                return;
            }
        }
        let Ok(json) = serde_json::to_string(rec) else { return };
        use std::io::Write;
        let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "failed to open executions history file");
                return;
            }
        };
        if let Err(e) = writeln!(f, "{json}") {
            tracing::warn!(error = %e, "failed to append executions history");
        }
    }

    /// Removes `peer` from the trusted set (no longer eligible to run
    /// workloads). Mirror of [`ComputeManager::add_trusted`]; used by the
    /// control-plane worker approve/revoke endpoints.
    pub async fn remove_trusted(&self, peer: &PeerId) {
        self.scheduler.lock().await.remove_trusted(peer);
    }

    /// Whether `peer` is trusted to run workloads.
    pub async fn is_trusted(&self, peer: &PeerId) -> bool {
        self.scheduler.lock().await.is_trusted(peer)
    }

    /// Records the latest advertisement received from a peer.
    pub async fn process_advertisement(&self, adv: ComputeAdvertisement) {
        self.scheduler.lock().await.upsert(adv.clone());
        self.contribution
            .lock()
            .unwrap()
            .entry(adv.peer_id)
            .or_default()
            .observe(&adv);
    }

    /// Accounts one routing outcome for `peer`: a verified completion or a
    /// failure. The coordinator calls this from the request path so the
    /// contribution ledger reflects real served compute, feeding the
    /// tier-suggestion engine (M17).
    pub async fn record_outcome(&self, peer: &PeerId, verified: bool) {
        if let Some(tracker) = self.contribution.lock().unwrap().get_mut(peer) {
            if verified {
                tracker.profile.verified_requests =
                    tracker.profile.verified_requests.saturating_add(1);
            } else {
                tracker.profile.failed_requests = tracker.profile.failed_requests.saturating_add(1);
            }
        }
    }

    /// Records a retryable routing failure for `peer` (P5), possibly tripping
    /// the circuit breaker so the worker is omitted from planning until its
    /// cooldown elapses.
    pub fn record_breaker_failure(&self, peer: &PeerId) {
        self.breaker
            .lock()
            .unwrap()
            .record_failure(peer, std::time::Instant::now());
    }

    /// Records a routing success for `peer` (P5), resetting its failure run.
    pub fn record_breaker_success(&self, peer: &PeerId) {
        self.breaker
            .lock()
            .unwrap()
            .record_success(peer, std::time::Instant::now());
    }

    /// Whether a request may currently be routed to `peer` (P5).
    pub fn breaker_allows(&self, peer: &PeerId) -> bool {
        self.breaker
            .lock()
            .unwrap()
            .allow(peer, std::time::Instant::now())
    }

    /// Marks a peer offline (stale heartbeat or explicit disconnect).
    pub async fn mark_offline(&self, peer: &PeerId) {
        self.scheduler.lock().await.mark_offline(peer);
    }

    /// Worker-fabric health maintenance (M24). Runs the coordinator's
    /// resilient-lifecycle pass:
    /// 1. expire stale reservations (booked on workers that vanished),
    /// 2. flip stale (no-heartbeat) workers to `Offline`, returning their ids,
    /// 3. evict workers that stay offline past `grace`, returning removed
    ///    records for audit.
    ///
    /// This is the coordinator-side half of "detect → remove → recover":
    /// evicted workers that heartbeated again would have been re-added by a
    /// fresh advertisement on rejoin (automatic recovery).
    pub async fn reap_unhealthy(
        &self,
        grace: std::time::Duration,
    ) -> (usize, Vec<(PeerId, String)>) {
        self.scheduler.lock().await.reap_offline(grace)
    }

    /// Records a measured round-trip time to `peer` (M19). Written by the
    /// periodic `InferPing` network probe; read by the execution planner for
    /// reach-cost-aware selection.
    pub fn record_rtt(&self, peer: &PeerId, rtt_us: u64, bandwidth_mbps: u32) {
        let mut graph = self.network.lock().unwrap();
        let prior = graph.get(&peer.to_string());
        let measured_rtt_us = if rtt_us > 0 {
            Some(rtt_us as u32)
        } else {
            None
        };
        let link = decentraai_fabric::LinkMetrics::prior(prior.locality, measured_rtt_us);
        if bandwidth_mbps > 0 {
            let link = decentraai_fabric::LinkMetrics {
                bandwidth_mbps,
                ..link
            };
            graph.set(&peer.to_string(), link.refresh());
        } else {
            graph.set(&peer.to_string(), link);
        }
    }

    /// The current network graph snapshot (coordinator-centric link metrics).
    pub fn network_graph(&self) -> decentraai_fabric::NetworkGraph {
        self.network.lock().unwrap().clone()
    }

    /// Records where a session's KV prefix lives after a routed request
    /// completed (M20). `tokens_used` is the real input+output tokens the
    /// worker reported; `capacity` is the worker's advertised `n_ctx` for the
    /// model (0 = unknown). This is honest coordinator-side accounting — no
    /// fabricated engine telemetry.
    pub async fn record_session_usage(
        &self,
        session_id: &str,
        worker: &PeerId,
        model_hash: &str,
        tokens_used: u32,
    ) {
        // Real advertised KV capacity (n_ctx) for the model on that worker, or
        // 0 when unknown/unadvertised.
        let capacity = self
            .scheduler
            .lock()
            .await
            .registry()
            .get(worker)
            .and_then(|adv| adv.capability.model(model_hash))
            .map(|m| m.context_tokens)
            .unwrap_or(0);
        self.sessions.lock().unwrap().record(
            session_id,
            *worker,
            model_hash,
            tokens_used,
            capacity,
        );
    }

    /// The worker holding a session's KV prefix, if any (M20 continuation
    /// affinity). `None` for unknown/stale sessions → deterministic fallback.
    pub fn session_residency(&self, session_id: &str) -> Option<PeerId> {
        self.sessions.lock().unwrap().residency(session_id)
    }

    /// Derives the honest KV-cache state the planner should use for `worker`
    /// serving `model_hash` (M20): `Partial { used, capacity }` when the
    /// worker advertises a real `n_ctx` and the coordinator has accounted
    /// resident sessions, otherwise the conservative `Empty`/`Unknown`.
    pub fn kv_state_for(
        &self,
        worker: &PeerId,
        model_hash: &str,
    ) -> decentraai_fabric::KVCacheState {
        use decentraai_fabric::KVCacheState;
        let used = self
            .sessions
            .lock()
            .unwrap()
            .worker_kv_used(worker, model_hash);
        match used {
            Some((used_tokens, capacity)) if capacity > 0 => {
                if used_tokens >= capacity {
                    KVCacheState::Full
                } else {
                    KVCacheState::Partial {
                        used: used_tokens.min(capacity),
                        capacity,
                    }
                }
            }
            // No advertised capacity: unbounded from the account's view.
            _ => KVCacheState::Empty,
        }
    }

    /// Drops all KV/session accounting for `worker` (called when a worker is
    /// evicted/offline so stale residency never steers routing to a dead
    /// node). Returns the number of sessions removed.
    pub fn drop_worker_sessions(&self, worker: &PeerId) -> usize {
        self.sessions.lock().unwrap().drop_worker(worker)
    }

    /// Number of coordinator-tracked sessions (observability).
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Snapshot of every coordinator-tracked KV/session (M20): session id →
    /// worker residency + model + accounted tokens + capacity. Real state,
    /// never fabricated; serde-friendly for the dashboard/API.
    pub fn sessions(&self) -> serde_json::Value {
        let snap = self.sessions.lock().unwrap().snapshot();
        serde_json::json!({
            "sessions_active": snap.len(),
            "sessions": snap.into_iter().map(|(id, kv)| serde_json::json!({
                "session_id": id,
                "worker": kv.worker.to_string(),
                "model_hash": kv.model_hash,
                "tokens_used": kv.tokens_used,
                "capacity": kv.capacity,
                "kv_headroom": if kv.capacity > 0 {
                    Some(kv.tokens_used.min(kv.capacity))
                } else { None },
            })).collect::<Vec<_>>(),
        })
    }

    /// Records an executed plan (M23): the real planner decision + placement +
    /// outcome, surfaced by the dashboard EXECUTION view. Bounded to the most
    /// recent 128 executions so the buffer cannot grow unbounded.
    pub fn record_execution(
        &self,
        request_id: &str,
        plan: &decentraai_fabric::ExecutionPlan,
        placement: &Placement,
        continuation: Option<String>,
        outcome: &str,
        attribution: ExecutionAttribution,
    ) {
        const MAX_EXECUTIONS: usize = 128;
        let mut ring = self.recent_executions.lock().unwrap();
        // Real network + KV reasons from the coordinator's live state at the
        // moment this decision was made (M19/M20).
        let link = self
            .network
            .lock()
            .unwrap()
            .get(&placement.worker.to_string());
        let rtt_ms = link.rtt_us / 1000;
        let kv_headroom = {
            use decentraai_fabric::KVCacheState;
            match self.kv_state_for(&placement.worker, &plan.model_hash) {
                KVCacheState::Partial { used, capacity } => format!("{used}/{capacity}"),
                KVCacheState::Empty => "unbounded (no n_ctx advertised)".to_string(),
                KVCacheState::Full => "full".to_string(),
                KVCacheState::Unknown => "unknown".to_string(),
            }
        };
        let (est_ram_mb, est_vram_mb) = plan.reservation_budget();
        let (is_continuation, prefix_worker) = (
            continuation.is_some(),
            continuation.map(|p| p.to_string()),
        );
        ring.push_back(ExecutedPlan {
            request_id: request_id.to_string(),
            plan_id: plan.plan_id.clone(),
            model_hash: plan.model_hash.clone(),
            selected_worker: placement.worker.to_string(),
            score: placement.confidence,
            stages: plan.stage_count(),
            reservation_id: placement.reservation.reservation_id.to_string(),
            is_continuation,
            prefix_worker,
            network_rtt_ms: rtt_ms,
            kv_headroom,
            outcome: outcome.to_string(),
            reasoning: "fabric planner single-stage placement (network+KV+capability aware)"
                .to_string(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            tokens_used: attribution.tokens_used,
            processing_time_ms: attribution.processing_time_ms,
            attempt: attribution.attempt,
            est_ram_mb,
            est_vram_mb,
        });
        let rec = ring.back().unwrap().clone();
        while ring.len() > MAX_EXECUTIONS {
            ring.pop_front();
        }
        drop(ring);
        // Part 17/22: persist best-effort so the execution trail survives
        // restarts. Never blocks or breaks the routing flow.
        self.persist_execution(&rec);
    }

    /// Snapshot of recent executed plans, newest-first (M23).
    pub fn executions(&self) -> Vec<ExecutedPlan> {
        self.recent_executions
            .lock()
            .unwrap()
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    /// Snapshot of the newest-full autonomous execution decisions (M23 Full
    /// Autonomy), newest-first, for the control plane.
    pub fn decisions(&self) -> Vec<decentraai_fabric::ExecutionDecision> {
        self.recent_decisions
            .lock()
            .unwrap()
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    /// Builds and records the full autonomous execution decision for a request
    /// (DISCOVER → CLASSIFY → CANDIDATES → CONSTRAINTS → SCORE → SELECT),
    /// reusing the same live fabric state the planner consumes. The decision is
    /// explainable (candidates, constraints, score, selected worker, KV
    /// affinity, engine capability, expected mode, fallback, trace) and stored
    /// for the control plane. Returns `()`; never affects routing.
    ///
    /// The decision is recorded with the same *live* planner the real routing
    /// path uses — the measured network graph (M19) and expert registry (M21) —
    /// so its network cost and score reflect genuine runtime state, then held
    /// in the bounded ring. The coordinator later calls [`Self::finalize_decision`]
    /// to correlate it with the reservation/plan and the observed outcome and
    /// to append the Reserved → Executing → Completed/Failed → Released trace.
    pub async fn record_decision(
        &self,
        request_id: &str,
        req: &WorkloadRequirements,
        prompt_tokens: u32,
        session_id: Option<&str>,
        priority: u8,
        streaming: bool,
    ) {
        use decentraai_fabric::decision;
        const MAX_DECISIONS: usize = 64;
        let facts = self.fabric_facts(&req.model_hash).await;
        if facts.is_empty() {
            return;
        }
        let (is_continuation, prefix_resident_on) = match session_id {
            Some(sid) => match self.session_residency(sid) {
                Some(w) => (true, Some(w.to_string())),
                None => (false, None),
            },
            None => (false, None),
        };
        let rfacts = decentraai_fabric::RequestFacts {
            model_hash: req.model_hash.clone(),
            est_ram_mb: req.est_ram_mb,
            est_vram_mb: req.est_vram_mb,
            context: decentraai_fabric::ContextProfile {
                prompt_tokens,
                max_output_tokens: req.max_tokens,
                is_continuation,
                prefix_resident_on,
            },
            transfer_mib: 0,
            local_peer: Some(self.local_peer.to_string()),
            priority,
            // Capability requirement: carried from the request (if any) and
            // resolved against the local registry's persisted claims so the
            // decision records a REAL verdict (VERIFIED/INFERRED/MISSING) when
            // data exists, honest UNKNOWN otherwise. Routing is unchanged.
            required_capability: req.required_capability.clone(),
            capability_claims: self.capability_claims_for_model(&req.model_hash),
        };
        // Mirror the planner the real routing path builds, so the recorded
        // decision shares the live network graph (M19) and objective weights.
        let planner = decentraai_fabric::ExecutionPlanner {
            network: self.network.lock().unwrap().clone(),
            allow_multi_stage: true,
            ..Default::default()
        };
        let decision = decision::evaluate(&planner, request_id, &rfacts, &facts, streaming, false);
        let mut ring = self.recent_decisions.lock().unwrap();
        ring.push_back(decision);
        while ring.len() > MAX_DECISIONS {
            ring.pop_front();
        }
    }

    /// Correlates the stored autonomous [`ExecutionDecision`] for `request_id`
    /// with the actual reservation, plan and observed outcome, and appends the
    /// lifecycle events (Reserved → Executing → Completed/Failed → Released) so
    /// the control plane renders a live trace rather than just the initial
    /// intent. Safe, bounded observability only — never affects routing.
    pub fn finalize_decision(
        &self,
        request_id: &str,
        selected_worker: &str,
        plan_id: &str,
        reservation_id: &str,
        ok: bool,
    ) {
        let mut ring = self.recent_decisions.lock().unwrap();
        let Some(d) = ring.iter_mut().find(|d| d.request_id == request_id) else {
            return;
        };
        d.selected_worker = Some(selected_worker.to_string());
        if let Some(plan) = d.plan.as_mut() {
            plan.plan_id = plan_id.to_string();
        }
        d.reservation_id = Some(reservation_id.to_string());
        d.outcome = Some(if ok { "succeeded".into() } else { "failed".into() });
        d.trace.push(decentraai_fabric::ExecutionEvent::Reserved {
            worker: Some(selected_worker.to_string()),
        });
        d.trace.push(decentraai_fabric::ExecutionEvent::Executing {
            worker: Some(selected_worker.to_string()),
        });
        if ok {
            d.trace.push(decentraai_fabric::ExecutionEvent::Completed { ok: true });
        } else {
            d.trace
                .push(decentraai_fabric::ExecutionEvent::Failed { cause: "execution_error".into(), retryable: false });
        }
        d.trace.push(decentraai_fabric::ExecutionEvent::Released {
            worker: Some(selected_worker.to_string()),
        });
    }

    /// Snapshot of live workers, newest-advertisement first.
    pub async fn workers(&self) -> Vec<ComputeAdvertisement> {
        self.scheduler.lock().await.registry().list()
    }

    /// Number of workloads currently booked on `peer` (reservations held).
    pub async fn in_flight(&self, peer: &PeerId) -> usize {
        self.scheduler.lock().await.ledger().in_flight(peer)
    }

    /// RAM (MiB) currently booked on `peer` by outstanding reservations.
    pub async fn reserved_ram(&self, peer: &PeerId) -> u64 {
        self.scheduler.lock().await.ledger().reserved_ram(peer)
    }

    /// Selects the best eligible worker and books a reservation.
    pub async fn select(&self, req: &WorkloadRequirements) -> Option<Placement> {
        self.scheduler.lock().await.select(req, Instant::now())
    }

    /// Planner over the live worker registry (M18 net) → fabric planner (M23).
    ///
    /// Builds a fabric [`WorkerFacts`] set from the current advertisements for
    /// the given `model_hash` and runs the autonomous [`ExecutionPlanner`] to
    /// choose the best worker. This is the *integration point* for the
    /// execution-fabric: how worker ordering accounts for engine capability
    /// (M22), network cost (M19) and KV state (M20) before capacity is
    /// enforced by the scheduler.
    pub async fn fabric_facts(&self, model_hash: &str) -> Vec<decentraai_fabric::WorkerFacts> {
        let scheduler = self.scheduler.lock().await;
        // P5: an open (tripped) worker is omitted entirely, so the planner
        // never selects it and no reservation is booked on it until cooldown.
        let now = std::time::Instant::now();
        let breaker = self.breaker.lock().unwrap();
        scheduler
            .registry()
            .list()
            .into_iter()
            .filter(|adv| breaker.allow(&adv.peer_id, now))
            .map(|adv| {
                let a = &adv.availability;
                let cap = &adv.capability;
                decentraai_fabric::WorkerFacts {
                    peer_id: adv.peer_id.to_string(),
                    trusted: scheduler.is_trusted(&adv.peer_id),
                    healthy: a.healthy(),
                    engine: decentraai_fabric::EngineKind::parse(&cap.engine),
                    tokens_per_second: a.tokens_per_second,
                    latency_ms: a.current_latency_ms,
                    // Honest perf provenance (Phase N): any nonzero advertised
                    // perf means a real measured completion fed the EWMA;
                    // zero means never measured (estimated/unknown). Pure
                    // provenance — the score formula is unchanged.
                    perf_measured: a.tokens_per_second > 0 || a.current_latency_ms > 0,
                    queue_depth: a.queue_depth,
                    load_percent: a.load_percent,
                    available_ram_mb: a.available_ram_mb,
                    available_vram_mb: a.available_vram_mb.unwrap_or(0),
                    serves_model: cap.serves_or_provisions(model_hash),
                    available_models: cap.available_models.clone(),
                    capabilities: decentraai_fabric::EngineKind::parse(&cap.engine)
                        .advertised_capabilities(),
                    kv: self.kv_state_for(&adv.peer_id, model_hash),
                }
            })
            .collect()
    }

    /// Number of live, eligible workers for `model_hash` (trusted + healthy +
    /// serves the model), from the current registry across the local scheduler
    /// and the P2P advertisement view. Used as the *real* `eligible_after_primary`
    /// input to `decentraai_fabric::adapt` so a retry/replan decision reflects
    /// actual remaining capacity — never a fabricated count.
    pub async fn eligible_worker_count(&self, model_hash: &str) -> usize {
        let scheduler = self.scheduler.lock().await;
        let now = std::time::Instant::now();
        let breaker = self.breaker.lock().unwrap();
        scheduler
            .registry()
            .list()
            .into_iter()
            .filter(|adv| breaker.allow(&adv.peer_id, now))
            .filter(|adv| {
                scheduler.is_trusted(&adv.peer_id)
                    && adv.availability.healthy()
                    && adv.capability.serves_or_provisions(model_hash)
            })
            .count()
    }

    /// Builds an `ExecutionPlan` for `req` using the fabric planner and books
    /// the reservation on the planned worker. Returns the plan and the
    /// reserved placement, or `None` when the fabric finds no eligible worker.
    ///
    /// `prompt_tokens` is the caller's estimated prompt length (KV-aware
    /// routing, M20). This replaces the pure scheduler `select` as the
    /// coordinator's first choice: the planner computes *whom*
    /// (engine/network/KV-aware), the scheduler then enforces capacity via
    /// `reserve_worker` (M18). If the planner's top worker cannot be reserved
    /// (became full / dropped / is the local node), we fall back to the plain
    /// scheduler `select` so a request is never stranded by planner optimism.
    pub async fn plan_and_reserve(
        &self,
        req: &WorkloadRequirements,
        prompt_tokens: u32,
        session_id: Option<&str>,
        priority: u8,
    ) -> Option<(decentraai_fabric::ExecutionPlan, Placement)> {
        let facts = self.fabric_facts(&req.model_hash).await;
        if facts.is_empty() {
            return None;
        }

        let mut experts = decentraai_fabric::ExpertRegistry::new();
        // M21: if any worker advertises expert-level routing for the model,
        // record its shard so the planner may split. Today no engine reports
        // `expert_routing`, so this stays empty and the router honestly returns
        // a whole-model decision — the abstraction is live, not mocked.
        for f in &facts {
            if f.capabilities.expert_routing && f.serves_model {
                experts.record(
                    &req.model_hash,
                    &f.peer_id,
                    decentraai_fabric::ExpertShard {
                        experts: Vec::new(),
                        routing_capable: true,
                        coverage: 1.0,
                    },
                );
            }
        }
        let planner = decentraai_fabric::ExecutionPlanner {
            network: self.network.lock().unwrap().clone(),
            experts,
            allow_multi_stage: true,
            ..Default::default()
        };

        // M20: continuation affinity. If this session already ran on a worker
        // (its KV prefix is resident there), mark the bundle as a continuation
        // and tell the planner where the prefix lives so it can steer back to
        // that worker (cache locality). Unknown/stale sessions fall back to a
        // plain cold routing decision (deterministic, no dead-worker steer).
        let (is_continuation, prefix_resident_on) = match session_id {
            Some(sid) => match self.session_residency(sid) {
                Some(w) => (true, Some(w.to_string())),
                None => (false, None),
            },
            None => (false, None),
        };
        let rfacts = decentraai_fabric::RequestFacts {
            model_hash: req.model_hash.clone(),
            est_ram_mb: req.est_ram_mb,
            est_vram_mb: req.est_vram_mb,
            context: decentraai_fabric::ContextProfile {
                prompt_tokens,
                max_output_tokens: req.max_tokens,
                is_continuation,
                prefix_resident_on,
            },
            transfer_mib: 0,
            local_peer: Some(self.local_peer.to_string()),
            priority,
            // Capability requirement carried from the request, resolved against
            // the local registry's persisted claims so the planner records a
            // REAL verdict when data exists (honest UNKNOWN otherwise).
            required_capability: req.required_capability.clone(),
            capability_claims: self.capability_claims_for_model(&req.model_hash),
        };

        let result = planner.plan(&rfacts, &facts);
        let workers = result.plan.workers();
        let first = workers.first()?;

        // M20 observability: surface the KV-aware inputs that shaped the
        // planner decision — continuation affinity and every eligible worker's
        // derived KV-cache state (from real n_ctx + accounted usage).
        {
            use decentraai_fabric::KVCacheState;
            let kv_view: Vec<String> = facts
                .iter()
                .map(|f| {
                    let s = match &f.kv {
                        KVCacheState::Empty => "empty".to_string(),
                        KVCacheState::Full => "full".to_string(),
                        KVCacheState::Partial { used, capacity } => {
                            format!("{used}/{capacity}")
                        }
                        KVCacheState::Unknown => "unknown".to_string(),
                    };
                    format!("{}:{}", &f.peer_id[..f.peer_id.len().min(12)], s)
                })
                .collect();
            tracing::info!(
                session_id = session_id.unwrap_or(""),
                is_continuation,
                prefix_worker = rfacts.context.prefix_resident_on.as_deref().unwrap_or(""),
                kv_states = ?kv_view,
                elidable_sessions = self.session_count(),
                "M20 KV-aware planner inputs"
            );
        }

        // The planner may pick the local node's self-advertisement; the
        // coordinator never schedules a remote request onto itself via P2P.
        let peer: libp2p::PeerId = match first.parse() {
            Ok(p) if p != self.local_peer => p,
            _ => return self.select_pub_remote(req).await.map(|p| (result.plan, p)),
        };

        let placement = self
            .scheduler
            .lock()
            .await
            .reserve_worker(&peer, req, Instant::now());
        match placement {
            Some(p) => Some((result.plan, p)),
            // Planner's top worker is full/unreservable → scheduler fallback.
            None => self.select_pub_remote(req).await.map(|p| (result.plan, p)),
        }
    }

    /// DRY-RUN planning preview: builds the same `ExecutionPlan` the
    /// coordinator would use for `model_hash` (via `fabric_facts` + the fabric
    /// planner) WITHOUT reserving any worker or sending any request. Returns
    /// the chosen plan + its selected worker + estimated cost, or `None` when
    /// the fabric finds no eligible worker. Read-only; never mutates state.
    ///
    /// This is the "what would the router do?" preview used by the confirmed
    /// mutation path's dry-run mode — it lets an operator see exactly what
    /// would be reserved/routed before actually executing.
    pub async fn plan_preview(
        &self,
        model_hash: &str,
        prompt_tokens: u32,
        session_id: Option<&str>,
        priority: u8,
    ) -> Option<(decentraai_fabric::ExecutionPlan, String, u32)> {
        let req = self.requirements_for(model_hash).await?;
        let facts = self.fabric_facts(model_hash).await;
        if facts.is_empty() {
            return None;
        }
        let (is_continuation, prefix_resident_on) = match session_id {
            Some(sid) => match self.session_residency(sid) {
                Some(w) => (true, Some(w.to_string())),
                None => (false, None),
            },
            None => (false, None),
        };
        let rfacts = decentraai_fabric::RequestFacts {
            model_hash: req.model_hash.clone(),
            est_ram_mb: req.est_ram_mb,
            est_vram_mb: req.est_vram_mb,
            context: decentraai_fabric::ContextProfile {
                prompt_tokens,
                max_output_tokens: req.max_tokens,
                is_continuation,
                prefix_resident_on,
            },
            transfer_mib: 0,
            local_peer: Some(self.local_peer.to_string()),
            priority,
            required_capability: req.required_capability.clone(),
            capability_claims: self.capability_claims_for_model(model_hash),
        };
        let planner = decentraai_fabric::ExecutionPlanner {
            network: self.network.lock().unwrap().clone(),
            allow_multi_stage: true,
            ..Default::default()
        };
        let result = planner.plan(&rfacts, &facts);
        let worker = result.plan.workers().first()?.clone();
        Some((result.plan, worker, result.estimated_ms))
    }

    /// The scheduler's best placement that is NOT this coordinator
    /// (a routed request must never be sent to the local node over P2P).
    async fn select_pub_remote(&self, req: &WorkloadRequirements) -> Option<Placement> {
        let mut scheduler = self.scheduler.lock().await;
        let best = scheduler.select(req, Instant::now());
        match best {
            Some(p) if p.worker != self.local_peer => Some(p),
            _ => None,
        }
    }

    /// Releases a reservation (call on workload completion or failure).
    pub async fn release(&self, reservation_id: uuid::Uuid) {
        self.scheduler.lock().await.release(reservation_id);
    }

    /// Derives a `WorkloadRequirements` for `model_hash` from the union of
    /// what workers advertise they serve (taking the largest RAM/VRAM
    /// footprint so the coordinator never under-reserves). Returns `None`
    /// when no known worker serves the model — the compute path cannot
    /// schedule it.
    ///
    /// When no worker serves the model but at least one trusted worker
    /// advertises `can_provision`, the requirements are still returned so
    /// the scheduler can route to a worker that will fetch the model on
    /// demand (M14). Until the model is provisioned nobody knows its real
    /// footprint, so a conservative default is used; the worker re-advertises
    /// the true model footprint after it downloads it.
    pub async fn requirements_for(&self, model_hash: &str) -> Option<WorkloadRequirements> {
        let workers = self.scheduler.lock().await.registry().list();
        let mut ram: u64 = 0;
        let mut vram: u64 = 0;
        let mut can_provision = false;
        for adv in &workers {
            if let Some(model) = adv.capability.model(model_hash) {
                ram = ram.max(model.est_ram_mb);
                vram = vram.max(model.est_vram_mb);
            }
            if adv.capability.can_provision {
                can_provision = true;
            }
        }
        if ram == 0 && vram == 0 {
            if !can_provision {
                return None;
            }
            ram = PROVISION_DEFAULT_RAM_MB;
        }
        Some(WorkloadRequirements::new(model_hash.to_string(), ram, vram))
    }

    /// Sets the local policy: when true, the scheduler may route workloads
    /// to workers that will fetch the model on demand instead of only to
    /// workers that already serve it (M14).
    pub async fn set_allow_provisioning(&self, allow: bool) {
        self.scheduler.lock().await.set_allow_provisioning(allow);
    }

    /// Builds this node's own advertisement from a real probe and records it
    /// locally (so the coordinator can schedule to itself when appropriate).
    pub async fn advertise_local(
        &self,
        snapshot: SystemSnapshot,
        gpu: GpuProbeStatus,
        served_models: Vec<ServedModel>,
        available_models: Vec<ServedModel>,
        can_provision: bool,
    ) -> ComputeAdvertisement {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut adv = build_advertisement(
            self.local_peer,
            &self.node_name,
            &self.engine,
            snapshot,
            gpu,
            served_models,
            can_provision,
            self.accepts_remote_inference
                .load(std::sync::atomic::Ordering::Relaxed),
            now,
            self.metrics.snapshot(),
        );
        adv.capability.available_models = available_models;
        self.scheduler.lock().await.upsert(adv.clone());
        *self.last_local_ad.lock().unwrap() = Some(adv.clone());
        adv
    }

    /// The wire form of an advertisement to broadcast: a signed
    /// [`decentraai_protocol::SignedComputeAdvertisement`] when a signing key is
    /// set (P3), otherwise the raw advertisement (legacy). Recipients verify the
    /// signed envelope and reject spoofed advertisements.
    pub fn advertisement_wire_bytes(&self, adv: &ComputeAdvertisement) -> anyhow::Result<Vec<u8>> {
        use decentraai_protocol::{sign_compute_advertisement, serialize_message};
        let raw = serialize_message(adv)?;
        if let Some(key) = &self.signing_key {
            let signed = sign_compute_advertisement(key, &raw);
            serialize_message(&signed)
        } else {
            Ok(raw)
        }
    }

    /// The advertisement this node most recently built and broadcast (the
    /// capacity it committed to the network). Synchronous, for the worker's
    /// on_infer admission gate. `None` until the first `advertise_local`.
    pub fn last_local_advertisement_sync(&self) -> Option<ComputeAdvertisement> {
        self.last_local_ad.lock().unwrap().clone()
    }

    /// Refreshes this node's on-disk model set after a model install/removal
    /// (Issue #26 §25): re-scans the local registry, rebuilds `available_models`
    /// with fresh BLAKE3 hashes, and re-advertises so coordinators see the new
    /// fabric reality without a node restart. `served_models` is preserved from
    /// the previous local advertisement (a pull does not load anything).
    ///
    /// The caller is expected to broadcast the returned advertisement (the
    /// periodic broadcaster picks it up on the next heartbeat as well).
    pub async fn refresh_local_models(
        &self,
        registry_path: &std::path::Path,
        context_tokens: u32,
    ) -> anyhow::Result<ComputeAdvertisement> {        use decentraai_registry::ModelRegistry;
        use std::io::Read;

        let gpu_present = matches!(
            decentraai_system_probe::probe_gpu(),
            decentraai_system_probe::GpuProbeStatus::Nvidia(_)
        );
        let mut available_models = Vec::new();
        if registry_path.exists() {
            let registry = ModelRegistry::load(registry_path)?;
            for record in registry.models.values() {
                let file = std::path::Path::new(&record.canonical_path);
                // Hash streamingly (BLAKE3), never loading the whole file.
                let mut hasher = blake3::Hasher::new();
                let mut f = match std::fs::File::open(file) {
                    Ok(f) => f,
                    Err(_) => continue, // gone from disk; registry scan prunes it
                };
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = f.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                let model_hash = hasher.finalize().to_hex().to_string();
                let size_mb = (record.size_bytes / (1024 * 1024)).max(1);
                available_models.push(ServedModel {
                    model_hash,
                    file_name: record.relative_path.clone(),
                    size_mb,
                    // NOTE: this is the GPU-offload *working-set* RAM estimate
                    // (a worker offloading the weights holds only a fraction
                    // in system RAM), NOT the full-load footprint from
                    // decentraai_compute::estimate_ram_mb. Do not swap the two
                    // without validating placement on hardware — they model
                    // different load modes (GPU offload vs CPU full-load).
                    est_ram_mb: size_mb / 4 + 1024,
                    est_vram_mb: if gpu_present { size_mb } else { 0 },
                    context_tokens,
                });
            }
        }
        let served_models = self
            .last_local_advertisement_sync()
            .map(|a| a.capability.served_models)
            .unwrap_or_default();
        let snapshot = decentraai_system_probe::SystemSnapshot::collect();
        let gpu = decentraai_system_probe::probe_gpu();
        let can_provision = self
            .last_local_advertisement_sync()
            .map(|a| a.capability.can_provision)
            .unwrap_or(false);
        let adv = self
            .advertise_local(
                snapshot,
                gpu,
                served_models,
                available_models,
                can_provision,
            )
            .await;
        Ok(adv)
    }

    /// Records the local registry path so the coordinator can resolve persisted
    /// capability claims for a model (best-effort; `None` keeps UNKNOWN).
    pub fn set_registry_path(&self, path: std::path::PathBuf) {
        *self.registry_path.lock().unwrap() = Some(path);
    }

    /// Resolves persisted capability claims for a model identified by its
    /// BLAKE3 `model_hash`. The registry stores records keyed by relative path
    /// (which ends in a file name), so we first map the hash → file name via
    /// the last local advertisement's served/available models, then look up the
    /// registry. Returns `(capability snake_case, provenance)` pairs, or empty
    /// when the hash/file is unknown, the registry is unavailable, or the model
    /// has no claims (honest: empty = UNKNOWN, never fabricated).
    fn capability_claims_for_model(
        &self,
        model_hash: &str,
    ) -> Vec<(String, String)> {
        let Some(adv) = self.last_local_advertisement_sync() else {
            return Vec::new();
        };
        let file_name = adv
            .capability
            .served_models
            .iter()
            .chain(adv.capability.available_models.iter())
            .find(|m| m.model_hash == model_hash)
            .map(|m| m.file_name.as_str());
        let Some(file_name) = file_name else {
            return Vec::new();
        };
        self.capability_claims_for_file(file_name)
    }

    /// Resolves persisted capability claims for a model file name (the registry
    /// stores records keyed by relative path, which ends in the file name).
    fn capability_claims_for_file(
        &self,
        file_name: &str,
    ) -> Vec<(String, String)> {
        let Some(path) = self.registry_path.lock().unwrap().clone() else {
            return Vec::new();
        };
        let Ok(registry) = decentraai_registry::ModelRegistry::load(&path) else {
            return Vec::new();
        };
        let Some(record) = registry
            .models
            .values()
            .find(|r| r.relative_path.ends_with(file_name))
        else {
            return Vec::new();
        };
        record
            .capability_claims
            .iter()
            .map(|c| (c.capability.clone(), c.provenance.clone()))
            .collect()
    }

    /// Shared handle to the node's live perf metrics. The worker's streaming
    /// task records real completions into it so subsequent advertisements
    /// (and therefore coordinator scheduling) reflect measured throughput and
    /// latency (M16).
    pub fn runtime_metrics(&self) -> std::sync::Arc<RuntimeMetrics> {
        self.metrics.clone()
    }
}
/// Convenience: derive a `GpuSpec` and free-VRAM from a `GpuSnapshot`.
pub fn gpu_from_snapshot(info: &GpuSnapshot) -> (Option<GpuSpec>, Option<u64>) {
    (
        Some(GpuSpec {
            name: info.name.clone(),
            vram_mb: info.total_vram_mib,
            driver: "nvidia".into(),
        }),
        Some(info.free_vram_mib),
    )
}

/// One worker row for the compute metrics report (M16). Built entirely from
/// the real [`ComputeManager`] state: the last advertisement's live
/// availability plus the scheduler's reservation bookkeeping. The static
/// identity + resource fields (CPU/RAM/GPU/engine/models) come from the
/// advertised capability — never invented.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerMetricRow {
    pub peer_id: String,
    pub node_name: String,
    /// Compact stable identifier (`dca-…`) from the advertisement; empty when
    /// the remote predates the field (the dashboard falls back to deriving it
    /// from the peer id).
    pub node_id: String,
    pub status: String,
    /// Whether the coordinator trusts this peer to run workloads (from the
    /// scheduler's live trust set, not derived from advertisements).
    pub trusted: bool,
    /// Whether the worker is currently reachable (not offline/stale in the
    /// compute registry). Real state: an `Offline` advertisement means the
    /// node stopped heartbeating.
    pub reachable: bool,
    /// Synthetic yet honest count of connection trouble: 1 when this worker
    /// is offline/stale in the registry (its last heartbeat lapsed), else 0.
    /// Derived from the same real registry state as `status`, never invented.
    pub connection_errors: u64,
    pub load_percent: u8,
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
    /// Whether `tokens_per_second`/`current_latency_ms` reflect real measured
    /// completions (EWMA) vs an estimated/zero baseline. Honest provenance;
    /// never affects scheduling.
    pub perf_measured: bool,
    pub available_ram_mb: u64,
    pub available_vram_mb: Option<u64>,
    pub in_flight: usize,
    pub reserved_ram_mb: u64,
    // ---- Static identity + resources from the advertised capability ----
    /// Logical CPU cores this node advertises.
    pub cpu_cores: u16,
    /// Total host RAM in MiB.
    pub ram_mb: u64,
    /// GPU name (None = CPU-only node).
    pub gpu_name: Option<String>,
    /// Total VRAM in MiB (None = CPU-only node).
    pub gpu_vram_mb: Option<u64>,
    /// Inference engine, e.g. "llama_server".
    pub engine: String,
    /// Models this node serves, with their real KV context window.
    pub served_models: Vec<MetricServedModel>,
    /// Models this node has on disk (registry) but is not serving right now —
    /// the honest "could serve" set. Distinct from `served_models`: a worker
    /// can swap its engine on request, so the coordinator (and dashboard)
    /// must be able to see what it COULD run, not just what is loaded.
    pub available_models: Vec<MetricServedModel>,
    /// Seconds since this worker's last heartbeat (registry staleness).
    pub last_seen_secs: u64,
    /// Whether the node accepts inference routed from remote peers (its
    /// advertised `accepts_remote_inference` — the honest remote-sharing
    /// opt-in). Local work is always accepted regardless.
    pub accepts_remote_inference: bool,
}

/// Compact serde view of one served model (per-node, M16/M20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricServedModel {
    pub file_name: String,
    pub size_mb: u64,
    /// Content hash of the GGUF artifact. Lets the coordinator map a
    /// dashboard-visible file name onto the exact artifact a worker serves,
    /// so chat routing can build a real `InferRequest` for a remote model.
    pub model_hash: String,
    /// Real KV-cache context window this worker allocates for the model
    /// (`--ctx-size`); 0 = unknown.
    pub context_tokens: u32,
}

/// One worker's contribution row for the metrics report (M17): the raw
/// measurements the tier engine consumes plus the resulting score and
/// suggested tier, so the admin/dashboard can see *why* a worker earned its
/// tier.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContributionRow {
    pub peer_id: String,
    pub node_name: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub vram_mb: u64,
    pub online_seconds: u64,
    pub verified_requests: u64,
    pub failed_requests: u64,
    pub score: f64,
    pub suggested_tier: u8,
    /// Reputation-dampened contribution credits (M9-9): zero for idle or
    /// complete-failure workers, scaled by quality and clean-service ratio.
    pub reward_tokens: u64,
}

/// Coordinator-side view of the whole mesh for observability (M16).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComputeMetricsReport {
    pub workers: Vec<WorkerMetricRow>,
    pub contributions: Vec<ContributionRow>,
    pub local_peer: String,
    pub local_perf: LivePerfSnapshot,
    pub totals: TotalsSnapshot,
}

/// Serializable snapshot of the local node's live perf.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LivePerfSnapshot {
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
}

/// Serializable lifetime totals.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TotalsSnapshot {
    pub requests_completed: u64,
    pub requests_failed: u64,
    pub tokens_total: u64,
}

/// A recorded execution decision + its outcome, surfaced by the dashboard's
/// EXECUTION view (M23/M24). Pure real state captured from the live planner
/// and the routing result — never mocked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutedPlan {
    pub request_id: String,
    pub plan_id: String,
    pub model_hash: String,
    /// The selected worker (PeerId).
    pub selected_worker: String,
    /// Planner confidence in this placement (0..1).
    pub score: f32,
    /// Number of execution stages in the plan.
    pub stages: usize,
    /// Reservation id held for this request.
    pub reservation_id: String,
    /// Whether the request is a KV continuation of an existing session.
    pub is_continuation: bool,
    /// Worker holding the session's KV prefix, if a continuation.
    pub prefix_worker: Option<String>,
    /// Measured RTT to the selected worker (ms), from the M19 network graph.
    pub network_rtt_ms: u32,
    /// KV-cache headroom of the selected worker at decision time (e.g.
    /// "500/2048"), from real n_ctx + accounted usage.
    pub kv_headroom: String,
    /// Outcome: succeeded / failed / in flight.
    pub outcome: String,
    /// Human-readable planner reasoning for selecting this worker.
    pub reasoning: String,
    /// Wall-clock timestamp (unix seconds) when the decision was recorded.
    pub ts: u64,
    /// ---- Resource attribution (Part 9/17): honest measured usage ----
    /// Total tokens the worker reported for this request (real `usage`),
    /// `None` when no worker response was received (e.g. transport failure).
    pub tokens_used: Option<u32>,
    /// Wall-clock processing time the worker measured, ms, excluding queue
    /// time (real `processing_time_ms`); `None` on transport failure.
    pub processing_time_ms: Option<u32>,
    /// Which retry attempt produced this outcome (0 = first placement).
    pub attempt: u32,
    /// Total RAM/VRAM budget the plan reserved across its stages (MiB),
    /// from `ExecutionPlan::reservation_budget` — the attribution baseline
    /// before measured usage replaces it.
    pub est_ram_mb: u64,
    pub est_vram_mb: u64,
}

/// Deterministic historical statistics derived from real measured execution
/// history (Phase N — Historical Intelligence). No ML, no synthetic benchmarks:
/// every number comes from the `ExecutedPlan` records' measured fields
/// (`tokens_used`, `processing_time_ms`, `outcome`, `ts`, `model_hash`,
/// `selected_worker`, `attempt`). Missing measurements (`None`) are simply
/// excluded from the aggregation they would feed — never treated as zero.
///
/// Returns a serde-friendly object. Pure; no I/O.
pub fn execution_statistics(history: &[ExecutedPlan]) -> serde_json::Value {
    let total = history.len();
    let succeeded = history.iter().filter(|p| p.outcome == "succeeded").count();
    let failed = history.iter().filter(|p| p.outcome == "failed").count();

    // Measured throughput (tokens/sec) and latency (ms) — only from records
    // that actually reported both usage and processing time.
    let measured: Vec<(f64, f64)> = history
        .iter()
        .filter_map(|p| match (p.tokens_used, p.processing_time_ms) {
            (Some(tokens), Some(ms)) if ms > 0 => Some((f64::from(tokens), f64::from(ms))),
            _ => None,
        })
        .collect();
    let measured_count = measured.len();
    let total_tokens_measured: u64 = measured.iter().map(|(t, _)| *t as u64).sum();
    let avg_tokens_per_sec = if measured_count > 0 {
        measured.iter().map(|(t, ms)| t / (ms / 1000.0)).sum::<f64>() / measured_count as f64
    } else {
        0.0
    };
    let avg_latency_ms = if measured_count > 0 {
        measured.iter().map(|(_, ms)| ms).sum::<f64>() / measured_count as f64
    } else {
        0.0
    };

    // Per-model outcomes (deterministic key order).
    let mut per_model: Vec<(String, usize, usize, usize)> = Vec::new(); // (model, total, succ, fail)
    for p in history {
        if let Some(e) = per_model.iter_mut().find(|(m, _, _, _)| *m == p.model_hash) {
            e.1 += 1;
            if p.outcome == "succeeded" {
                e.2 += 1;
            } else if p.outcome == "failed" {
                e.3 += 1;
            }
        } else {
            let (s, f) = if p.outcome == "succeeded" {
                (1, 0)
            } else if p.outcome == "failed" {
                (0, 1)
            } else {
                (0, 0)
            };
            per_model.push((p.model_hash.clone(), 1, s, f));
        }
    }
    per_model.sort_by(|a, b| a.0.cmp(&b.0));

    // Per-worker outcomes (deterministic key order).
    let mut per_worker: Vec<(String, usize, usize, usize)> = Vec::new();
    for p in history {
        if let Some(e) = per_worker.iter_mut().find(|(w, _, _, _)| *w == p.selected_worker) {
            e.1 += 1;
            if p.outcome == "succeeded" {
                e.2 += 1;
            } else if p.outcome == "failed" {
                e.3 += 1;
            }
        } else {
            let (s, f) = if p.outcome == "succeeded" {
                (1, 0)
            } else if p.outcome == "failed" {
                (0, 1)
            } else {
                (0, 0)
            };
            per_worker.push((p.selected_worker.clone(), 1, s, f));
        }
    }
    per_worker.sort_by(|a, b| a.0.cmp(&b.0));

    // Retry statistics: how many records were a retry (attempt > 0).
    let retries = history.iter().filter(|p| p.attempt > 0).count();

    serde_json::json!({
        "records": total,
        "outcomes": { "succeeded": succeeded, "failed": failed, "other": total.saturating_sub(succeeded + failed) },
        "measured": {
            "records": measured_count,
            "total_tokens": total_tokens_measured,
            "avg_tokens_per_sec": avg_tokens_per_sec,
            "avg_latency_ms": avg_latency_ms,
        },
        "retries": retries,
        "per_model": per_model.into_iter().map(|(m, t, s, f)| serde_json::json!({
            "model": m, "total": t, "succeeded": s, "failed": f,
        })).collect::<Vec<_>>(),
        "per_worker": per_worker.into_iter().map(|(w, t, s, f)| serde_json::json!({
            "worker": w, "total": t, "succeeded": s, "failed": f,
        })).collect::<Vec<_>>(),
        "note": "deterministic statistics from measured execution history; no synthetic data",
    })
}

/// Measured usage a worker reports back for one request (Part 9/17). Kept
/// as a single struct so `record_execution` stays under the argument budget
/// and callers pass attribution in one place. All fields are honest: `None`
/// on transport failure (no worker response), never invented.
#[derive(Debug, Clone, Default)]
pub struct ExecutionAttribution {
    /// Total tokens the worker reported (input + output).
    pub tokens_used: Option<u32>,
    /// Wall-clock processing time measured by the worker, ms, excluding
    /// queue time.
    pub processing_time_ms: Option<u32>,
    /// Which retry attempt produced the outcome (0 = first placement).
    pub attempt: u32,
}

impl ComputeManager {
    /// Builds a serde-friendly snapshot of the mesh for the metrics API /
    /// dashboard (M16).
    pub async fn metrics_report(&self) -> ComputeMetricsReport {
        let workers = self.scheduler.lock().await.registry().list();
        let now = std::time::Instant::now();
        let mut rows = Vec::with_capacity(workers.len());
        for adv in &workers {
            let offline = adv.availability.status == WorkerHealth::Offline;
            let last_seen_secs = self
                .scheduler
                .lock()
                .await
                .registry()
                .last_seen_secs(&adv.peer_id, now)
                .unwrap_or(0);
            rows.push(WorkerMetricRow {
                peer_id: adv.peer_id.to_string(),
                node_name: adv.node_name.clone(),
                node_id: adv.node_id.clone(),
                status: format!("{:?}", adv.availability.status),
                trusted: self.is_trusted(&adv.peer_id).await,
                reachable: !offline,
                connection_errors: u64::from(offline),
                load_percent: adv.availability.load_percent,
                queue_depth: adv.availability.queue_depth,
                tokens_per_second: adv.availability.tokens_per_second,
                current_latency_ms: adv.availability.current_latency_ms,
                perf_measured: adv.availability.tokens_per_second > 0
                    || adv.availability.current_latency_ms > 0,
                available_ram_mb: adv.availability.available_ram_mb,
                available_vram_mb: adv.availability.available_vram_mb,
                in_flight: self.in_flight(&adv.peer_id).await,
                reserved_ram_mb: self.reserved_ram(&adv.peer_id).await,
                cpu_cores: adv.capability.cpu_cores,
                ram_mb: adv.capability.ram_mb,
                gpu_name: adv.capability.gpu.as_ref().map(|g| g.name.clone()),
                gpu_vram_mb: adv.capability.gpu.as_ref().map(|g| g.vram_mb),
                engine: adv.capability.engine.clone(),
                served_models: adv
                    .capability
                    .served_models
                    .iter()
                    .map(|m| MetricServedModel {
                        file_name: m.file_name.clone(),
                        size_mb: m.size_mb,
                        model_hash: m.model_hash.clone(),
                        context_tokens: m.context_tokens,
                    })
                    .collect(),
                available_models: adv
                    .capability
                    .available_models
                    .iter()
                    .map(|m| MetricServedModel {
                        file_name: m.file_name.clone(),
                        size_mb: m.size_mb,
                        model_hash: m.model_hash.clone(),
                        context_tokens: m.context_tokens,
                    })
                    .collect(),
                last_seen_secs,
                accepts_remote_inference: adv.accepts_remote_inference,
            });
        }
        let contributions = self.contribution_report_locked(workers).await;
        let perf = self.metrics.snapshot();
        let (completed, failed, tokens) = self.metrics.totals();
        ComputeMetricsReport {
            workers: rows,
            contributions,
            local_peer: self.local_peer.to_string(),
            local_perf: LivePerfSnapshot {
                queue_depth: perf.queue_depth,
                tokens_per_second: perf.tokens_per_second,
                current_latency_ms: perf.current_latency_ms,
            },
            totals: TotalsSnapshot {
                requests_completed: completed,
                requests_failed: failed,
                tokens_total: tokens,
            },
        }
    }

    /// Coordinator-side contribution snapshot (M17): every known worker's
    /// accrued online time and served requests, scored through the pure tier
    /// engine. Kept separate from raw scheduling so the API can surface both
    /// "how free is it" (metrics) and "what has it earned" (contribution).
    pub async fn contribution_report(&self) -> Vec<ContributionRow> {
        let workers = self.scheduler.lock().await.registry().list();
        self.contribution_report_locked(workers).await
    }

    /// Builds contribution rows from a live worker list. Needs the node names
    /// (advertisements) to pair with the per-peer ledger held on
    /// `self.contribution`.
    async fn contribution_report_locked(
        &self,
        workers: Vec<ComputeAdvertisement>,
    ) -> Vec<ContributionRow> {
        use decentraai_compute::{contribution_score, reward_tokens, suggest_tier, RewardPolicy};
        let ledger = self.contribution.lock().unwrap();
        let mut rows = Vec::with_capacity(workers.len());
        for adv in workers {
            let profile = ledger.get(&adv.peer_id).map(|t| t.profile);
            let profile = profile.unwrap_or_default();
            rows.push(ContributionRow {
                peer_id: adv.peer_id.to_string(),
                node_name: adv.node_name.clone(),
                cpu_cores: profile.cpu_cores,
                ram_mb: profile.ram_mb,
                vram_mb: profile.vram_mb,
                online_seconds: profile.online_seconds,
                verified_requests: profile.verified_requests,
                failed_requests: profile.failed_requests,
                score: contribution_score(&profile),
                suggested_tier: suggest_tier(&profile),
                reward_tokens: reward_tokens(&profile, &RewardPolicy::default()),
            });
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_compute::ServedModel;

    fn peer() -> PeerId {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    #[test]
    fn short_node_id_is_stable_and_compact() {
        // Same key -> same indicator, every time (the identity a fabric
        // participant is known by). Format: `dca-` + 6 chars.
        let p = peer();
        let id = short_node_id(&p);
        assert!(id.starts_with("dca-"), "id must carry the dca- prefix: {id}");
        assert_eq!(id.len(), 10, "dca-xxxxxx is exactly 10 chars: {id}");
        assert_eq!(short_node_id(&p), id, "must be deterministic per peer");
        // Distinct peers should almost never collide in the first 6 chars.
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..64 {
            distinct.insert(short_node_id(&peer()));
        }
        assert!(distinct.len() >= 60, "6 hex chars must spread well: {}", distinct.len());
    }

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            logical_cpus: 8,
            cpu_usage_percent: 25.0,
            total_memory_bytes: 32 * 1024 * 1024 * 1024,
            available_memory_bytes: 16 * 1024 * 1024 * 1024,
            used_swap_bytes: 0,
            total_disk_free_bytes: 200 * 1024 * 1024 * 1024,
        }
    }

    fn gpu() -> GpuProbeStatus {
        GpuProbeStatus::Nvidia(GpuSnapshot {
            name: "RTX 4090".into(),
            total_vram_mib: 24564,
            free_vram_mib: 20000,
            utilization_percent: 10,
            temperature_celsius: 55,
            power_draw_watts: 150.0,
        })
    }

    fn model() -> ServedModel {
        ServedModel {
            model_hash: "abc".into(),
            file_name: "model.gguf".into(),
            size_mb: 2048,
            est_ram_mb: 256,
            est_vram_mb: 3072,
            context_tokens: 0,
        }
    }

    #[test]
    fn builds_real_advertisement_from_probe() {
        let p = peer();
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            1_700_000_000_000,
            LivePerf::default(),
        );
        assert_eq!(adv.peer_id, p);
        assert_eq!(adv.node_name, "gpu-rig");
        assert_eq!(adv.capability.cpu_cores, 8);
        assert_eq!(adv.capability.ram_mb, 32 * 1024);
        assert_eq!(adv.availability.available_ram_mb, 16 * 1024);
        assert_eq!(adv.availability.available_vram_mb, Some(20000));
        assert_eq!(adv.availability.load_percent, 25);
        assert!(adv.capability.has_model("abc"));
        assert!(!adv.capability.can_provision);
        let spec = adv.capability.gpu.unwrap();
        assert_eq!(spec.name, "RTX 4090");
        assert_eq!(spec.vram_mb, 24564);
    }

    #[tokio::test]
    async fn set_engine_advertises_and_parses_real_kind() {
        let p = peer();
        let mut manager = ComputeManager::new(p, "n".into(), HashSet::from([p]));
        // Default is llama-server until an engine is explicitly selected.
        manager
            .advertise_local(snapshot(), gpu(), vec![model()], vec![], false)
            .await;
        assert_eq!(
            manager.last_local_advertisement_sync().unwrap().capability.engine,
            ENGINE_LLAMA_SERVER
        );

        // Selecting vLLM changes the advertised engine and the fabric facts
        // parse it back to the honest kind (M22).
        manager.set_engine("vllm");
        manager
            .advertise_local(snapshot(), gpu(), vec![model()], vec![], false)
            .await;
        let adv = manager.last_local_advertisement_sync().unwrap();
        assert_eq!(adv.capability.engine, "vllm");
        let facts = manager.fabric_facts("abc").await;
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].engine, decentraai_fabric::EngineKind::Vllm);
    }

    #[tokio::test]
    async fn available_models_flow_into_advertisement_and_facts() {
        // Regression for the fabric model view: the `available_models` channel
        // (models on disk, not currently loaded) must reach the advertisement
        // and the planner's WorkerFacts, so the coordinator can discover what a
        // worker COULD serve, not just what it has loaded.
        let p = peer();
        let manager = ComputeManager::new(p, "n".into(), HashSet::from([p]));
        let on_disk = vec![decentraai_compute::ServedModel {
            model_hash: "hash-b".into(),
            file_name: "other-model.gguf".into(),
            size_mb: 500,
            est_ram_mb: 1200,
            est_vram_mb: 0,
            context_tokens: 0,
        }];
        manager
            .advertise_local(snapshot(), gpu(), vec![model()], on_disk, false)
            .await;
        let adv = manager.last_local_advertisement_sync().unwrap();
        assert_eq!(adv.capability.available_models.len(), 1);
        assert_eq!(adv.capability.available_models[0].file_name, "other-model.gguf");
        assert_eq!(adv.capability.served_models.len(), 1, "served set unchanged");

        // The planner facts carry the full on-disk collection too.
        let facts = manager.fabric_facts("abc").await;
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].available_models.len(), 1);
        assert_eq!(facts[0].available_models[0].file_name, "other-model.gguf");
    }

    #[tokio::test]
    async fn fabric_facts_records_perf_measured_from_advertised_perf() {
        // Phase N perf provenance: `fabric_facts` must set `perf_measured` to
        // true when the advertisement carries nonzero advertised perf (a real
        // completion fed the EWMA) and false when it is zero (never measured).
        let local = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::new());

        // Never-measured worker: all-zero perf -> estimated.
        let fresh_peer = peer();
        let fresh = build_advertisement(
            fresh_peer,
            "fresh",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(fresh).await;
        let facts = manager.fabric_facts("abc").await;
        let f = facts.iter().find(|f| f.peer_id == fresh_peer.to_string()).unwrap();
        assert!(!f.perf_measured, "zero advertised perf must be ESTIMATED");
        assert_eq!(f.tokens_per_second, 0);
        assert_eq!(f.latency_ms, 0);

        // Measured worker: nonzero advertised perf -> measured.
        let busy_peer = peer();
        let busy = build_advertisement(
            busy_peer,
            "busy",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf {
                queue_depth: 2,
                tokens_per_second: 180,
                current_latency_ms: 50,
            },
        );
        manager.process_advertisement(busy).await;
        let facts = manager.fabric_facts("abc").await;
        let f = facts.iter().find(|f| f.peer_id == busy_peer.to_string()).unwrap();
        assert!(f.perf_measured, "nonzero advertised perf must be MEASURED");
        assert_eq!(f.tokens_per_second, 180);
        assert_eq!(f.latency_ms, 50);

        // Latency-only measurement (tokens still zero) is also measured.
        let lat_peer = peer();
        let lat = build_advertisement(
            lat_peer,
            "lat",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf {
                queue_depth: 0,
                tokens_per_second: 0,
                current_latency_ms: 90,
            },
        );
        manager.process_advertisement(lat).await;
        let facts = manager.fabric_facts("abc").await;
        let f = facts.iter().find(|f| f.peer_id == lat_peer.to_string()).unwrap();
        assert!(f.perf_measured, "latency-only measurement must count as measured");
    }

    #[tokio::test]
    async fn refresh_local_models_readvertises_after_install() {
        // Issue #26 §25: a model installed through the Hub must reach the
        // fabric without a node restart. refresh_local_models re-scans the
        // registry, rebuilds available_models and re-advertises.
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("fresh.gguf"), b"not a real gguf").unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        {
            let mut reg = decentraai_registry::ModelRegistry::new(models_dir.clone()).unwrap();
            reg.scan_directory(&models_dir).unwrap();
            reg.save(&registry_path).unwrap();
        }

        let p = peer();
        let manager = ComputeManager::new(p, "n".into(), HashSet::from([p]));
        // Before any install there is no local advertisement.
        assert!(manager.last_local_advertisement_sync().is_none());

        let adv = manager.refresh_local_models(&registry_path, 4096).await.unwrap();
        assert_eq!(adv.capability.available_models.len(), 1);
        assert_eq!(adv.capability.available_models[0].file_name, "fresh.gguf");
        assert_eq!(adv.capability.available_models[0].context_tokens, 4096);
        assert!(manager.last_local_advertisement_sync().is_some());

        // A second refresh after adding another file picks it up without
        // losing the earlier one.
        std::fs::write(models_dir.join("second.gguf"), b"another").unwrap();
        let mut reg = decentraai_registry::ModelRegistry::load(&registry_path).unwrap();
        reg.scan_directory(&models_dir).unwrap();
        reg.save(&registry_path).unwrap();
        let adv2 = manager.refresh_local_models(&registry_path, 4096).await.unwrap();
        let names: Vec<_> = adv2
            .capability
            .available_models
            .iter()
            .map(|m| m.file_name.as_str())
            .collect();
        assert!(names.contains(&"fresh.gguf"));
        assert!(names.contains(&"second.gguf"));
    }

    #[test]
    fn builds_cpu_only_advertisement_when_gpu_missing() {
        let p = peer();
        let adv = build_advertisement(
            p,
            "cpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            GpuProbeStatus::Unavailable("nvidia-smi not found".into()),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        assert!(adv.capability.gpu.is_none());
        assert_eq!(adv.availability.available_vram_mb, None);
    }

    #[tokio::test]
    async fn selects_and_releases_via_manager() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let placement = manager.select(&req).await.expect("eligible worker");
        assert_eq!(placement.worker, p);
        assert!(
            manager.select(&req).await.is_some(),
            "only one reservation tracked per select"
        );
        manager.release(placement.reservation.reservation_id).await;
        assert!(manager.select(&req).await.is_some());
    }

    #[tokio::test]
    async fn plan_and_reserve_builds_executed_plan() {
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::from([worker]));
        let adv = build_advertisement(
            worker,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;
        manager.record_rtt(&worker, 2_000, 1_000);

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let (plan, placement) = manager
            .plan_and_reserve(&req, 200, None, 0)
            .await
            .expect("fabric planner finds the worker");
        assert_eq!(
            plan.stage_count(),
            1,
            "single GPU worker -> single-stage plan"
        );
        assert_eq!(placement.worker, worker, "planner books the remote worker");
        assert_eq!(manager.in_flight(&worker).await, 1);

        manager.release(placement.reservation.reservation_id).await;
        assert_eq!(
            manager.in_flight(&worker).await,
            0,
            "release frees the booking"
        );
    }

    #[tokio::test]
    async fn plan_preview_plans_without_reserving() {
        // DRY-RUN: plan_preview builds the same plan the coordinator would use
        // but must NOT hold any reservation (in_flight stays 0).
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::from([worker]));
        let adv = build_advertisement(
            worker,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;
        manager.record_rtt(&worker, 2_000, 1_000);

        let (plan, chosen_worker, est_ms) = manager
            .plan_preview("abc", 200, None, 0)
            .await
            .expect("preview finds the worker");
        assert_eq!(plan.stage_count(), 1);
        assert_eq!(chosen_worker, worker.to_string());
        assert!(est_ms > 0);
        // Crucially: no reservation is held by a dry-run preview.
        assert_eq!(
            manager.in_flight(&worker).await,
            0,
            "plan_preview must not reserve"
        );

        // No known model -> None (honest).
        assert!(manager.plan_preview("missing", 10, None, 0).await.is_none());
    }
    #[tokio::test]
    async fn plan_and_reserve_never_selects_local_node() {
        let local = peer();
        // Only the local node advertises; the planner must not self-schedule.
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::from([local]));
        let adv = build_advertisement(
            local,
            "self",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        assert!(
            manager.plan_and_reserve(&req, 10, None, 0).await.is_none(),
            "coordinator must not route a remote request to itself"
        );
    }

    #[tokio::test]
    async fn circuit_breaker_omits_a_tripped_worker_from_planning() {
        let local = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::new());
        let worker = peer();
        manager.add_trusted(worker).await;
        let adv = build_advertisement(
            worker,
            "wedged",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        // Before tripping, the worker is routable.
        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        assert!(manager.plan_and_reserve(&req, 10, None, 0).await.is_some());
        assert_eq!(manager.fabric_facts("abc").await.len(), 1);

        // Trip the breaker: repeated retryable failures open it.
        let cfg = crate::breaker::BreakerConfig::default();
        for _ in 0..cfg.threshold {
            manager.record_breaker_failure(&worker);
        }
        // An open worker is omitted from the planner feed and never reserved.
        assert!(!manager.breaker_allows(&worker));
        assert_eq!(manager.fabric_facts("abc").await.len(), 0);
        assert!(
            manager.plan_and_reserve(&req, 10, None, 0).await.is_none(),
            "must not reserve/open a trip request on an open worker"
        );

        // A success re-opens eligibility.
        manager.record_breaker_success(&worker);
        assert!(manager.breaker_allows(&worker));
    }

    #[tokio::test]
    async fn continuation_is_steered_back_to_prefix_worker() {
        let local = peer();
        let worker_a = peer();
        let worker_b = peer();
        let manager = ComputeManager::new(
            local,
            "coordinator".into(),
            HashSet::from([worker_a, worker_b]),
        );
        // Both workers serve the same model with a real 2048-token context.
        let ctx_model = ServedModel {
            context_tokens: 2048,
            ..model()
        };
        let adv_a = build_advertisement(
            worker_a,
            "a",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![ctx_model.clone()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        let adv_b = build_advertisement(
            worker_b,
            "b",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![ctx_model.clone()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv_a).await;
        manager.process_advertisement(adv_b).await;

        // Cold request with a session id -> arbitrary (a or b).
        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let cold = manager
            .plan_and_reserve(&req, 100, Some("s1"), 0)
            .await
            .expect("workers eligible");
        let (plan_cold, placement_cold) = cold;
        assert_eq!(plan_cold.stage_count(), 1);

        // Account the session as resident on that worker (real tokens_used).
        manager
            .record_session_usage("s1", &placement_cold.worker, "abc", 200)
            .await;

        // Continuation with the same session must be steered back to that
        // same worker (cache locality), deterministically.
        let cont = manager
            .plan_and_reserve(&req, 50, Some("s1"), 0)
            .await
            .expect("continuation routable");
        assert_eq!(
            cont.1.worker, placement_cold.worker,
            "continuation reuses prefix worker"
        );
        assert_eq!(cont.0.workers(), vec![placement_cold.worker.to_string()]);
    }

    #[tokio::test]
    async fn kv_state_reflects_accounted_usage() {
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::from([worker]));
        let ctx_model = ServedModel {
            context_tokens: 2048,
            ..model()
        };
        let adv = build_advertisement(
            worker,
            "w",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![ctx_model],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        use decentraai_fabric::KVCacheState;
        // No sessions yet -> empty (unbounded, no known usage).
        assert_eq!(manager.kv_state_for(&worker, "abc"), KVCacheState::Empty);

        manager
            .record_session_usage("s1", &worker, "abc", 300)
            .await;
        manager
            .record_session_usage("s2", &worker, "abc", 500)
            .await;
        // 800 of 2048 used.
        assert_eq!(
            manager.kv_state_for(&worker, "abc"),
            KVCacheState::Partial {
                used: 800,
                capacity: 2048
            }
        );

        // A request sized to need more headroom than remains should not be
        // placed on a nearly-full worker when capacity is known (the fabric
        // planner only books it if it can accommodate via headroom).
        manager
            .record_session_usage("s3", &worker, "abc", 2000)
            .await;
        assert_eq!(manager.kv_state_for(&worker, "abc"), KVCacheState::Full);
    }

    #[tokio::test]
    async fn unknown_session_falls_back_to_cold_routing() {
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "coordinator".into(), HashSet::from([worker]));
        let ctx_model = ServedModel {
            context_tokens: 2048,
            ..model()
        };
        manager
            .process_advertisement(build_advertisement(
                worker,
                "w",
                ENGINE_LLAMA_SERVER,
                snapshot(),
                gpu(),
                vec![ctx_model],
                false,
                true,
                0,
                LivePerf::default(),
            ))
            .await;

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        // Unknown session -> not a continuation; must still route to the
        // (only) eligible worker deterministically.
        let (plan, placement) = manager
            .plan_and_reserve(&req, 100, Some("never-seen"), 0)
            .await
            .expect("routes");
        assert_eq!(placement.worker, worker);
        assert_eq!(plan.workers(), vec![worker.to_string()]);
    }

    #[tokio::test]
    async fn requirements_for_accepts_provisioning_workers() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![],
            true,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        let req = manager
            .requirements_for("zzz-not-served")
            .await
            .expect("provisioning worker is schedulable");
        assert_eq!(req.model_hash, "zzz-not-served");
        assert!(req.est_ram_mb > 0);

        // With no provisioning-capable worker, unknown models are not routable.
        let p2 = peer();
        let manager2 = ComputeManager::new(p2, "coordinator".into(), HashSet::from([p2]));
        let adv2 = build_advertisement(
            p2,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![],
            false,
            true,
            0,
            LivePerf::default(),
        );
        manager2.process_advertisement(adv2).await;
        assert!(manager2.requirements_for("zzz-not-served").await.is_none());
    }

    #[tokio::test]
    async fn contribution_report_scores_real_accounting() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let worker = peer();
        let adv = build_advertisement(
            worker,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![],
            true,
            true,
            0,
            LivePerf::default(),
        );
        manager.process_advertisement(adv).await;

        // Zero work: guest, zero score.
        let report = manager.contribution_report().await;
        let row = report
            .iter()
            .find(|r| r.peer_id == worker.to_string())
            .expect("worker appears in contribution report");
        assert_eq!(row.suggested_tier, 1);
        assert_eq!(row.verified_requests, 0);

        // Servings accrue and push the tier up.
        for _ in 0..5000 {
            manager.record_outcome(&worker, true).await;
        }
        manager.record_outcome(&worker, false).await; // one failure shouldn't tank it
        let report = manager.contribution_report().await;
        let row = report
            .iter()
            .find(|r| r.peer_id == worker.to_string())
            .unwrap();
        assert_eq!(row.verified_requests, 5000);
        assert_eq!(row.failed_requests, 1);
        assert!(row.score > 0.0);
        assert!(
            row.suggested_tier >= 2,
            "served worker should be a contributor+"
        );
    }

    #[test]
    fn runtime_metrics_ewma_tracks_throughput_and_latency() {
        let m = RuntimeMetrics::new();
        // 100 tokens in 1000ms = 100 tps; 100 tokens in 100ms = 1000 tps.
        m.record_completion(100, 1000);
        m.record_completion(100, 100);
        let snap = m.snapshot();
        assert!(
            snap.tokens_per_second > 100,
            "EWMA should raise TPS, got {}",
            snap.tokens_per_second
        );
        assert!(snap.current_latency_ms > 100 && snap.current_latency_ms < 1000);
        m.set_queue_depth(4);
        assert_eq!(m.snapshot().queue_depth, 4);
        let (completed, failed, tokens) = m.totals();
        assert_eq!(completed, 2);
        assert_eq!(failed, 0);
        assert_eq!(tokens, 200);
        m.record_failure();
        assert_eq!(m.totals().1, 1);
    }

    #[test]
    fn advertisement_carries_live_perf_metrics() {
        let p = peer();
        let perf = LivePerf {
            queue_depth: 3,
            tokens_per_second: 150,
            current_latency_ms: 40,
        };
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            perf,
        );
        assert_eq!(adv.availability.queue_depth, 3);
        assert_eq!(adv.availability.tokens_per_second, 150);
        assert_eq!(adv.availability.current_latency_ms, 40);
    }

    #[tokio::test]
    async fn metrics_report_reflects_registry_and_bookings() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf {
                queue_depth: 2,
                tokens_per_second: 75,
                current_latency_ms: 60,
            },
        );
        manager.process_advertisement(adv).await;
        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let placement = manager.select(&req).await.expect("eligible");
        let report = manager.metrics_report().await;
        assert_eq!(report.workers.len(), 1);
        assert_eq!(report.workers[0].queue_depth, 2);
        assert_eq!(report.workers[0].in_flight, 1);
        assert!(report.workers[0].reserved_ram_mb >= 256);
        manager.release(placement.reservation.reservation_id).await;
    }

    #[tokio::test]
    async fn metrics_report_separates_served_from_available_models() {
        // Part 3/17: the compute metrics view must expose what each worker has
        // on disk (available_models) next to what it serves, so the dashboard
        // and model picker can discover any model a worker COULD run.
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let mut adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        );
        adv.capability.available_models = vec![decentraai_compute::ServedModel {
            model_hash: "on-disk-hash".into(),
            file_name: "on-disk.gguf".into(),
            size_mb: 700,
            est_ram_mb: 1400,
            est_vram_mb: 0,
            context_tokens: 0,
        }];
        manager.process_advertisement(adv).await;
        let report = manager.metrics_report().await;
        let row = &report.workers[0];
        assert_eq!(row.served_models.len(), 1, "served model still listed");
        assert_eq!(row.served_models[0].file_name, model().file_name);
        assert_eq!(row.available_models.len(), 1);
        assert_eq!(row.available_models[0].file_name, "on-disk.gguf");
        assert_eq!(row.available_models[0].model_hash, "on-disk-hash");
    }

    #[tokio::test]
    async fn trusted_reachesable_fields_track_add_and_remove() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::new());
        let worker = peer();
        manager.process_advertisement(build_advertisement(
            worker,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        ))
        .await;

        // New worker is not trusted and reachable (heartbeat fresh).
        assert!(!manager.is_trusted(&worker).await);
        let before = manager.metrics_report().await;
        let row = before.workers.iter().find(|r| r.peer_id == worker.to_string()).unwrap();
        assert!(!row.trusted, "must start untrusted");
        assert!(row.reachable, "fresh advertisement is reachable");
        assert_eq!(row.connection_errors, 0);

        // Approving flips the reported trust field and the is_trusted gate.
        manager.add_trusted(worker).await;
        assert!(manager.is_trusted(&worker).await);
        let after = manager.metrics_report().await;
        let row = after.workers.iter().find(|r| r.peer_id == worker.to_string()).unwrap();
        assert!(row.trusted, "approve must mark trusted");

        // Revoking flips it back.
        manager.remove_trusted(&worker).await;
        assert!(!manager.is_trusted(&worker).await);
        let revoked = manager.metrics_report().await;
        let row = revoked.workers.iter().find(|r| r.peer_id == worker.to_string()).unwrap();
        assert!(!row.trusted, "revoke must clear trust");
    }

    #[tokio::test]
    async fn offline_worker_is_flagged_unreachable_with_connection_error() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let worker = peer();
        manager.process_advertisement(build_advertisement(
            worker,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            false,
            true,
            0,
            LivePerf::default(),
        ))
        .await;

        manager.mark_offline(&worker).await;
        let report = manager.metrics_report().await;
        let row = report.workers.iter().find(|r| r.peer_id == worker.to_string()).unwrap();
        assert!(!row.reachable, "offline worker is unreachable");
        assert_eq!(row.connection_errors, 1, "offline is surfaced as a connection error");
        assert_eq!(row.status, "Offline");
    }

    #[tokio::test]
    async fn record_execution_captures_network_and_kv_reasons() {
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "c".into(), HashSet::from([worker]));
        manager
            .process_advertisement(build_advertisement(
                worker,
                "w",
                ENGINE_LLAMA_SERVER,
                snapshot(),
                GpuProbeStatus::Unavailable("none".into()),
                vec![ServedModel {
                    context_tokens: 2048,
                    ..model()
                }],
                false,
                true,
                0,
                LivePerf::default(),
            ))
            .await;
        manager.record_rtt(&worker, 50_000, 1000); // 50ms RTT

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let (plan, placement) = manager
            .plan_and_reserve(&req, 100, None, 0)
            .await
            .expect("plan");
        let plan = plan.clone();
        // Account some session usage so KV headroom is a Partial value.
        manager
            .record_session_usage("s1", &worker, "abc", 500)
            .await;

        manager.record_execution(
            "r1",
            &plan,
            &placement,
            None,
            "succeeded",
            ExecutionAttribution {
                tokens_used: Some(321),
                processing_time_ms: Some(1234),
                attempt: 0,
            },
        );
        let recs = manager.executions();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].request_id, "r1");
        assert_eq!(recs[0].selected_worker, worker.to_string());
        assert_eq!(recs[0].network_rtt_ms, 50);
        assert_eq!(recs[0].kv_headroom, "500/2048");
        // Part 9/17 attribution: measured usage from the worker response is
        // recorded next to the planned reservation budget.
        assert_eq!(recs[0].tokens_used, Some(321));
        assert_eq!(recs[0].processing_time_ms, Some(1234));
        assert_eq!(recs[0].attempt, 0);
        assert!(recs[0].est_ram_mb >= 256, "reservation budget carried");
        assert_eq!(recs[0].est_vram_mb, 3072);
        // Ring buffer bounds.
        for i in 0..150 {
            manager.record_execution(
                &format!("r{i}"),
                &plan,
                &placement,
                None,
                "succeeded",
                ExecutionAttribution::default(),
            );
        }
        assert!(manager.executions().len() <= 128);
        // Continue the session so continuation reasons render in the record.
        let cont = manager
            .plan_and_reserve(&req, 50, Some("s1"), 0)
            .await
            .expect("cont");
        manager.record_execution(
            "c1",
            &cont.0,
            &cont.1,
            Some(worker.to_string()),
            "succeeded",
            ExecutionAttribution {
                tokens_used: Some(77),
                processing_time_ms: Some(888),
                attempt: 0,
            },
        );
        let recs = manager.executions();
        let c = recs.iter().find(|r| r.request_id == "c1").unwrap();
        assert!(c.is_continuation);
        assert_eq!(
            c.prefix_worker.as_deref(),
            Some(worker.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn execution_history_persists_across_manager_restart() {
        // Part 17/22: with a history file set, recorded executions are
        // appended as JSON lines and replayed into a fresh manager, so the
        // coordinator keeps its execution trail across restarts.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db/executions.jsonl");

        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "c".into(), HashSet::from([worker]));
        manager.set_executions_path(Some(path.clone()));
        manager
            .process_advertisement(build_advertisement(
                worker,
                "w",
                ENGINE_LLAMA_SERVER,
                snapshot(),
                GpuProbeStatus::Unavailable("none".into()),
                vec![model()],
                false,
                true,
                0,
                LivePerf::default(),
            ))
            .await;
        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let (plan, placement) = manager
            .plan_and_reserve(&req, 100, None, 0)
            .await
            .expect("plan");
        manager.record_execution(
            "persist-1",
            &plan,
            &placement,
            None,
            "succeeded",
            ExecutionAttribution {
                tokens_used: Some(99),
                processing_time_ms: Some(500),
                attempt: 1,
            },
        );

        // The history file exists and carries the record.
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("persist-1"), "history appended: {contents}");
        let first: ExecutedPlan = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(first.tokens_used, Some(99));

        // A fresh manager pointed at the same file replays the record.
        let restarted = ComputeManager::new(local, "c".into(), HashSet::from([worker]));
        restarted.set_executions_path(Some(path.clone()));
        let recs = restarted.executions();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].request_id, "persist-1");
        assert_eq!(recs[0].tokens_used, Some(99));
    }

    #[tokio::test]
    async fn record_decision_stores_an_explainable_autonomous_decision() {
        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "c".into(), HashSet::from([worker]));
        manager
            .process_advertisement(build_advertisement(
                worker,
                "w",
                ENGINE_LLAMA_SERVER,
                snapshot(),
                GpuProbeStatus::Unavailable("none".into()),
                vec![ServedModel {
                    context_tokens: 4096,
                    est_vram_mb: 0,
                    ..model()
                }],
                false,
                true,
                0,
                LivePerf::default(),
            ))
            .await;

        let req = manager
            .requirements_for("abc")
            .await
            .expect("the advertised model yields workload requirements");
        manager
            .record_decision("r1", &req, 100, None, 128, true)
            .await;

        let ds = manager.decisions();
        assert_eq!(ds.len(), 1, "decision recorded");
        let d = &ds[0];
        assert_eq!(d.request_id, "r1");
        // The eligible worker is selected and its constraints are satisfied.
        assert_eq!(d.selected_worker.as_deref(), Some(worker.to_string().as_str()));
        let cand = d
            .candidates
            .iter()
            .find(|c| c.peer_id == worker.to_string())
            .unwrap();
        assert!(cand.constraints.is_satisfied());
        assert!(cand.score.is_some(), "eligible candidate has a score");
        assert!(
            d.trace.iter().any(|e| matches!(
                e,
                decentraai_fabric::ExecutionEvent::Planned { .. }
            )),
            "trace includes the Planned event"
        );
        // Finalize correlates the decision with the reservation/plan/outcome
        // and appends the lifecycle trace (Reserved → Executing → Released).
        manager.finalize_decision("r1", &worker.to_string(), "plan-1", "res-1", true);
        let ds = manager.decisions();
        let d = &ds[0];
        assert_eq!(d.reservation_id.as_deref(), Some("res-1"));
        assert_eq!(d.outcome.as_deref(), Some("succeeded"));
        assert_eq!(d.plan.as_ref().map(|p| p.plan_id.as_str()), Some("plan-1"));
        assert!(d
            .trace
            .iter()
            .any(|e| matches!(e, decentraai_fabric::ExecutionEvent::Reserved { .. })));
        assert!(d
            .trace
            .iter()
            .any(|e| matches!(e, decentraai_fabric::ExecutionEvent::Completed { ok: true })));
        // Ring buffer bounds.
        for i in 0..70 {
            manager
                .record_decision(&format!("r{i}"), &req, 10, None, 0, false)
                .await;
        }
        assert!(manager.decisions().len() <= 64);
    }

    #[tokio::test]
    async fn record_decision_resolves_real_capability_verdict_from_registry() {
        // The coordinator resolves a requirement against the local registry's
        // persisted claims, so the decision carries a REAL verdict — not
        // UNKNOWN — when the data exists.
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("model.gguf"), b"GGUF magic").unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        {
            let mut reg =
                decentraai_registry::ModelRegistry::new(models_dir.clone()).unwrap();
            reg.scan_directory(&models_dir).unwrap();
            reg.set_capability_claims(
                "model.gguf",
                vec![decentraai_registry::CapabilityClaimRecord {
                    capability: "ocr".into(),
                    provenance: "verified".into(),
                }],
            )
            .unwrap();
            reg.save(&registry_path).unwrap();
        }

        let local = peer();
        let worker = peer();
        let manager = ComputeManager::new(local, "c".into(), HashSet::from([worker]));
        // Build the LOCAL advertisement from the registry (sets the hash and
        // the last_local_ad so claims resolution can map hash -> file name).
        manager.set_registry_path(registry_path.clone());
        let adv = manager
            .refresh_local_models(&registry_path, 4096)
            .await
            .unwrap();
        let model_hash = adv.capability.available_models[0].model_hash.clone();

        // A remote worker that serves the model, so the planner has an eligible
        // candidate to record a decision against.
        manager
            .process_advertisement(build_advertisement(
                worker,
                "w",
                ENGINE_LLAMA_SERVER,
                snapshot(),
                GpuProbeStatus::Unavailable("none".into()),
                vec![ServedModel {
                    model_hash: model_hash.clone(),
                    context_tokens: 4096,
                    est_vram_mb: 0,
                    ..model()
                }],
                false,
                true,
                0,
                LivePerf::default(),
            ))
            .await;

        let mut req = manager
            .requirements_for(&model_hash)
            .await
            .expect("advertised model yields requirements");
        req.required_capability = Some("ocr".to_string());
        manager
            .record_decision("r-cap", &req, 100, None, 128, true)
            .await;

        let d = &manager.decisions()[0];
        let view = d
            .capability_requirement
            .as_ref()
            .expect("requirement requested -> verdict present");
        // Real claim resolution: the registry says the model can do OCR with
        // verified evidence — so the decision records satisfied at VERIFIED.
        assert!(view.satisfied, "verified claim -> satisfied");
        assert_eq!(view.evidence, "VERIFIED");
    }

    #[test]
    fn execution_statistics_derives_deterministic_aggregates() {
        // Build a small synthetic history with real measured fields: two
        // succeeded records (with tokens+time), one failed, one retry.
        let rec = |id: &str, model: &str, worker: &str, outcome: &str, tokens: Option<u32>, ms: Option<u32>, attempt: u32| ExecutedPlan {
            request_id: id.to_string(),
            plan_id: "p".into(),
            model_hash: model.to_string(),
            selected_worker: worker.to_string(),
            score: 0.5,
            stages: 1,
            reservation_id: "r".into(),
            is_continuation: false,
            prefix_worker: None,
            network_rtt_ms: 10,
            kv_headroom: "1/1".into(),
            outcome: outcome.to_string(),
            reasoning: "".into(),
            ts: 1,
            tokens_used: tokens,
            processing_time_ms: ms,
            attempt,
            est_ram_mb: 100,
            est_vram_mb: 0,
        };

        let history = vec![
            rec("a", "m1", "w1", "succeeded", Some(100), Some(1000), 0),
            rec("b", "m1", "w1", "succeeded", Some(200), Some(2000), 1), // retry
            rec("c", "m2", "w2", "failed", None, None, 0),               // no measurement
        ];

        let s = execution_statistics(&history);
        assert_eq!(s["records"], 3);
        assert_eq!(s["outcomes"]["succeeded"], 2);
        assert_eq!(s["outcomes"]["failed"], 1);
        // Only the two measured records feed throughput/latency; the failed
        // record with None measurements is excluded, never treated as 0.
        assert_eq!(s["measured"]["records"], 2);
        assert_eq!(s["measured"]["total_tokens"], 300);
        assert!(s["measured"]["avg_tokens_per_sec"].as_f64().unwrap() > 0.0);
        assert_eq!(s["measured"]["avg_latency_ms"], 1500.0);
        assert_eq!(s["retries"], 1);
        // Per-model: m1 (2 total) and m2 (1 total), deterministic order.
        let per_model = s["per_model"].as_array().unwrap();
        assert_eq!(per_model.len(), 2);
        assert_eq!(per_model[0]["model"], "m1");
        assert_eq!(per_model[0]["total"], 2);
        assert_eq!(per_model[0]["succeeded"], 2);
        assert_eq!(per_model[1]["model"], "m2");
        // Per-worker: w1 (2) and w2 (1).
        let per_worker = s["per_worker"].as_array().unwrap();
        assert_eq!(per_worker.len(), 2);
        assert_eq!(per_worker[0]["worker"], "w1");
        assert_eq!(per_worker[0]["total"], 2);

        // Empty history -> all zeros, no panic.
        let e = execution_statistics(&[]);
        assert_eq!(e["records"], 0);
        assert_eq!(e["measured"]["records"], 0);
    }
}
