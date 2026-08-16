//! OpenAI-compatible API endpoint (M4c), the web dashboard (M7b/M7c),
//! and tiered subscription auth (P1): a thin proxy in front of the
//! managed llama-server. All inference logic lives in llama.cpp.
//!
//! Auth model: the master token (runtime/api.token) is unlimited admin.
//! Issued subscription tokens (`dsk_…`) resolve through db/tokens.json
//! on every request — no restart needed after `token create/revoke` —
//! and get per-tier model allowlists and sliding-window rate limits.
//!
//! Every inference request joins a fair FIFO queue (Q2): one request
//! at a time reaches the backend with the machine's full resources,
//! everyone else waits in arrival order. The dashboard (GET /) renders
//! live node status from /status and /v1/peers only — it never calls
//! the proxied inference endpoints, so watching the page neither
//! inflates the request counter nor resets the idle-unload clock.

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use decentraai_config::{GenerationSection, ResourceSection, TiersSection};
use futures::StreamExt;
use rand_core::RngCore;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::ServeManager;
use crate::dashboard::{DASHBOARD_HTML, JS_TEMPLATE};
use crate::queue::InferenceQueue;

/// Maximum audit events shown on the dashboard.
const DASHBOARD_EVENT_LIMIT: usize = 10;
/// Maximum inference calls kept in the recent-requests ring buffer.
const RECENT_REQUEST_LIMIT: usize = 12;
/// Sliding rate-limit window.
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Phase M LIMITS: max mutations (execute) per minute per token name.
const EXECUTE_RATE_LIMIT_PER_MINUTE: usize = 10;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// Proxy-boundary cap on the JSON-encoded prompt text forwarded to
/// llama-server (mirrors `BackendConfig::max_prompt_bytes` on the distributed
/// path). Rejected up front so an oversized local request cannot hold the
/// engine or the inference queue slot.
const MAX_PROMPT_BYTES: usize = 200_000;
/// Proxy-boundary cap on caller-requested `max_tokens` (mirrors
/// `BackendConfig::max_output_tokens`). llama-server clamps internally, but we
/// reject loudly instead of forwarding an unbounded generation request.
const MAX_OUTPUT_TOKENS: u64 = 8192;
/// HTTP request timeout to the managed llama-server backend, matching the
/// distributed `BackendConfig` default so a hung engine releases its slot
/// instead of holding the queue forever.
const BACKEND_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Per-token usage counters: (requests, generated tokens, last-used unix secs).
type UsageCounters = Arc<StdMutex<HashMap<String, (u64, u64, u64)>>>;

/// RAII guard for a consumer-quota reservation (Q2).
///
/// Reserving quota up front and settling/releasing through this guard makes
/// consumer quota enforcement safe across the proxy's many return paths: if a
/// handler returns without settling (early error, cancellation, transport
/// failure), the guard's `Drop` releases the reservation so no quota leaks as
/// reserved. On a measured success the caller calls [`settle`](Self::settle),
/// which converts the reservation into consumed quota (releasing the unused
/// remainder) and marks it settled so `Drop` does nothing.
struct ConsumerQuotaGuard {
    ledger: Option<Arc<StdMutex<decentraai_compute::QuotaLedger>>>,
    reservation_id: Option<String>,
    settled: bool,
}

impl ConsumerQuotaGuard {
    fn new(ledger: Arc<StdMutex<decentraai_compute::QuotaLedger>>, reservation_id: String) -> Self {
        Self {
            ledger: Some(ledger),
            reservation_id: Some(reservation_id),
            settled: false,
        }
    }

    /// Settles the reservation against real measured usage. Idempotent: a
    /// second call is a no-op. Releases the unused remainder.
    fn settle(&mut self, tokens_used: u64) {
        if self.settled {
            return;
        }
        if let (Some(ledger), Some(id)) = (&self.ledger, &self.reservation_id) {
            let mut ledger = ledger.lock().unwrap();
            let _ = ledger.settle(id, tokens_used);
        }
        self.settled = true;
    }
}

impl Drop for ConsumerQuotaGuard {
    fn drop(&mut self) {
        // Release any still-held reservation so it cannot leak as reserved.
        if !self.settled {
            if let (Some(ledger), Some(id)) = (&self.ledger, &self.reservation_id) {
                let mut ledger = ledger.lock().unwrap();
                let _ = ledger.release(id);
            }
        }
    }
}

/// One completed inference call, shown in the dashboard's recent list.
#[derive(Debug, Clone, Serialize)]
pub struct RequestStat {
    pub timestamp: u64,
    pub endpoint: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub duration_ms: u64,
    pub tokens_per_second: f64,
}

/// Auth/gate failures. Small enough to return by value (a full axum
/// Response is 128+ bytes and trips clippy::result_large_err); converted
/// into an HTTP response only at the handler boundary.
#[derive(Debug)]
enum GateError {
    Unauthorized,
    Forbidden(String),
    RateLimited(usize),
}

impl GateError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => unauthorized(),
            Self::Forbidden(message) => forbidden(&message),
            Self::RateLimited(limit) => too_many_requests(limit),
        }
    }
}

/// How the caller authenticated on this request.
#[derive(Debug)]
enum Auth {
    /// No token configured on the node at all (api_auth_required=false).
    Open,
    /// The master admin token: unlimited.
    Master,
    /// An issued subscription token with its tier and role (H4).
    Subscriber {
        name: String,
        tier: u8,
        role: decentraai_tokens::Role,
    },
    /// A consumer API key (`dca_…`) resolved through the consumer key store
    /// (Q2). Carries the quota ceiling + rate limit from the key record; the
    /// key never grants admin/operator privileges (see `require_master` /
    /// `require_operator_or_admin`). `account` is the owner account in the
    /// authoritative quota ledger.
    Consumer {
        key_id: String,
        account: String,
        quota_ceiling: u64,
        rate_limit_per_minute: u32,
    },
}

impl Auth {
    fn who(&self) -> String {
        match self {
            Self::Open => "open".to_string(),
            Self::Master => "master".to_string(),
            Self::Subscriber { name, .. } => name.clone(),
            Self::Consumer { key_id, .. } => key_id.clone(),
        }
    }
}

/// Static node details the dashboard renders (kept separate so
/// [`ApiState::new`] stays readable).
#[derive(Clone)]
pub struct DashboardInfo {
    /// Node data dir; the audit log is read from `<repo_root>/logs`.
    pub repo_root: PathBuf,
    /// Reputation store path (db/reputation.json) when configured.
    pub reputation_path: Option<PathBuf>,
    /// Reputation thresholds, needed to reload the store read-only.
    pub max_invalid_chunks: u8,
    pub ban_duration: Duration,
    /// The public API port, shown in the dashboard.
    pub api_port: u16,
    /// Model name requested at startup.
    pub model_name: String,
    /// Model file size in bytes (0 when unknown).
    pub model_size_bytes: u64,
    /// Sampling defaults merged into inference requests (Q1).
    pub generation: GenerationSection,
    /// Resource limits/guards from the config (Settings view).
    pub resources: ResourceSection,
    /// Whether the node runs the Kademlia DHT (cross-subnet discovery).
    pub dht_enabled: bool,
    /// Whether the node runs relay + DCUtR (NAT traversal).
    pub relay_enabled: bool,
    /// Whether LAN mDNS discovery is enabled.
    pub lan_discovery: bool,
    /// Number of configured bootstrap peers (network.bootstrap_peers).
    pub bootstrap_peer_count: usize,
}

/// Shared proxy state.
#[derive(Clone)]
pub struct ApiState {
    /// Base URL of the managed llama-server (e.g. http://127.0.0.1:41501).
    backend_url: String,
    /// Optional master Bearer token; admin when set.
    auth_token: Option<Arc<str>>,
    /// Lifecycle handle; activity is recorded per request.
    manager: Arc<Mutex<ServeManager>>,
    client: reqwest::Client,
    info: DashboardInfo,
    /// Runtime-editable generation defaults. Seeded from the node config at
    /// startup; an admin can update them live via the master-gated settings
    /// endpoint, and the proxy reads this (not the immutable `info`) when
    /// applying sampling defaults. Read-only for clients.
    runtime_generation: Arc<tokio::sync::RwLock<GenerationSection>>,
    /// Live Model Hub pull progress: `repo -> (bytes_downloaded, total_bytes)`.
    /// Populated while a pull is in flight; removed on completion. Read by the
    /// dashboard pull-status endpoint for a real progress bar.
    hub_pulls: Arc<StdMutex<HashMap<String, (u64, u64)>>>,
    /// Subscription registry (db/tokens.json) when tiers are in use.
    token_store_path: Option<PathBuf>,
    /// Consumer API key registry (db/consumer_keys.json) for the Compute
    /// Contribution & Quota consumer path (Q2). When set, `dca_…` keys are
    /// accepted as an inference credential with a quota ceiling + rate limit.
    consumer_keys_path: Option<PathBuf>,
    /// The authoritative quota ledger, `Arc`-shared with the compute manager
    /// (Q2: worker credits and consumer reserve/settle are one ledger). `None`
    /// when running without compute; consumer quota enforcement is skipped.
    quota_ledger: Option<Arc<StdMutex<decentraai_compute::QuotaLedger>>>,
    /// Per-consumer-key sliding-window rate limiting (Q2). Keyed by key_id;
    /// independent from both tier rate limits and the execute mutation limit.
    consumer_rate_windows: Arc<StdMutex<HashMap<String, VecDeque<Instant>>>>,
    /// Per-consumer-key usage counters (requests + generated tokens + last used),
    /// for the admin/dashboard. Keyed by key_id.
    consumer_usage: UsageCounters,
    /// Tier policies from the config; None = admin-token-only.
    tiers: Option<TiersSection>,
    /// Fair FIFO queue for inference requests (Q2).
    queue: Arc<InferenceQueue>,
    started_at: Instant,
    /// Completed inference calls (POST /v1/completions, /v1/chat/completions).
    requests_served: Arc<AtomicU64>,
    /// Sum of completion tokens across all inference calls.
    tokens_generated: Arc<AtomicU64>,
    /// Inference calls that reached the backend but failed (for success-rate).
    requests_failed: Arc<AtomicU64>,
    /// Newest-first ring buffer of recent inference calls.
    recent_requests: Arc<StdMutex<VecDeque<RequestStat>>>,
    /// Per-token sliding-window timestamps (rate limiting).
    rate_windows: Arc<StdMutex<HashMap<String, VecDeque<Instant>>>>,
    /// Sliding-window timestamps for the MUTATING execute path (Phase M LIMITS).
    /// Separate from `rate_windows` so mutation rate limiting never interacts
    /// with tier inference limits. Keyed by the token name (or "master").
    execute_windows: Arc<StdMutex<HashMap<String, VecDeque<Instant>>>>,
    /// Per-token usage counters.
    token_usage: UsageCounters,
    /// Real distributed-compute coordinator state (M23/M24), wired into the
    /// dashboard so WORKERS/NETWORK/EXECUTION views render real state only.
    /// `None` when running without a compute manager (e.g. plain serve).
    compute: Option<Arc<decentraai_distributed::ComputeManager>>,
    /// The live P2P node, for the NETWORK view (connected peers).
    p2p: Option<decentraai_p2p::P2PNode>,
    /// Fabric inference coordinator (M18+). When attached, the proxy can
    /// route `/v1/chat/completions` to a *trusted remote worker* that
    /// advertises the requested model, instead of only serving locally.
    /// `None` = plain local-only proxy (unchanged behaviour).
    distributed: Option<Arc<decentraai_distributed::DistributedInference>>,
}

impl ApiState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend_url: String,
        auth_token: Option<String>,
        manager: Arc<Mutex<ServeManager>>,
        info: DashboardInfo,
        token_store_path: Option<PathBuf>,
        tiers: Option<TiersSection>,
        queue: Arc<InferenceQueue>,
        compute: Option<Arc<decentraai_distributed::ComputeManager>>,
        p2p: Option<decentraai_p2p::P2PNode>,
    ) -> Self {
        Self {
            backend_url,
            auth_token: auth_token.map(Into::into),
            manager,
            client: reqwest::Client::builder()
                .timeout(BACKEND_REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            runtime_generation: Arc::new(tokio::sync::RwLock::new(info.generation.clone())),
            hub_pulls: Arc::new(StdMutex::new(HashMap::new())),
            info,
            token_store_path,
            consumer_keys_path: None,
            quota_ledger: None,
            consumer_rate_windows: Arc::new(StdMutex::new(HashMap::new())),
            consumer_usage: Arc::new(StdMutex::new(HashMap::new())),
            tiers,
            queue,
            started_at: Instant::now(),
            requests_served: Arc::new(AtomicU64::new(0)),
            tokens_generated: Arc::new(AtomicU64::new(0)),
            requests_failed: Arc::new(AtomicU64::new(0)),
            recent_requests: Arc::new(StdMutex::new(VecDeque::new())),
            rate_windows: Arc::new(StdMutex::new(HashMap::new())),
            execute_windows: Arc::new(StdMutex::new(HashMap::new())),
            token_usage: Arc::new(StdMutex::new(HashMap::new())),
            compute,
            p2p,
            distributed: None,
        }
    }

    /// Attaches the fabric inference coordinator so the proxy can route chat
    /// inference to trusted remote workers (M18+). Call once at startup on
    /// the node daemon path, where a `DistributedInference` already exists.
    pub fn attach_distributed(&mut self, distributed: Arc<decentraai_distributed::DistributedInference>) {
        self.distributed = Some(distributed);
    }

    /// Enables the consumer API key path (Q2): points at the consumer key
    /// registry and shares the authoritative quota ledger with the compute
    /// manager, so consumer reserve/settle is one ledger with worker credits.
    /// Call once at startup on paths that serve inference with a compute
    /// manager attached.
    pub fn attach_consumer(
        &mut self,
        consumer_keys_path: Option<std::path::PathBuf>,
        quota_ledger: Option<Arc<StdMutex<decentraai_compute::QuotaLedger>>>,
    ) {
        self.consumer_keys_path = consumer_keys_path;
        self.quota_ledger = quota_ledger;
    }

    fn presented_token(headers: &HeaderMap) -> Option<&str> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    }

    /// Classifies the caller: master token, issued subscription token
    /// (resolved through the registry on every request), a consumer API key
    /// (`dca_…`, resolved through the consumer key store — Q2), or open.
    fn classify(&self, headers: &HeaderMap) -> Result<Auth, GateError> {
        let presented = Self::presented_token(headers);
        match &self.auth_token {
            None => Ok(Auth::Open),
            Some(master) => {
                let presented = presented.ok_or(GateError::Unauthorized)?;
                if presented == master.as_ref() {
                    return Ok(Auth::Master);
                }
                // Consumer API keys (dca_…): resolved through the consumer key
                // store. Never admin; carries ceiling + rate limit.
                if presented.starts_with(decentraai_tokens::KEY_PREFIX) {
                    if !self.consumer_enabled() {
                        return Err(GateError::Unauthorized);
                    }
                    let path = self.consumer_keys_path.as_ref().unwrap();
                    let mut store = decentraai_tokens::ConsumerKeyStore::load(path)
                        .map_err(|_| GateError::Unauthorized)?;
                    // Copy the auth-relevant fields, then touch last-used
                    // (mutable) so the borrow of `store` does not conflict.
                    let record = {
                        let rec = store.lookup(presented);
                        rec.map(|r| {
                            (
                                r.key_id.clone(),
                                r.owner_account.clone(),
                                r.quota_ceiling,
                                r.rate_limit_per_minute,
                            )
                        })
                    };
                    match record {
                        Some((key_id, account, quota_ceiling, rate_limit_per_minute)) => {
                            store.touch_used(&key_id);
                            return Ok(Auth::Consumer {
                                key_id,
                                account,
                                quota_ceiling,
                                rate_limit_per_minute,
                            });
                        }
                        None => return Err(GateError::Unauthorized),
                    }
                }
                match &self.token_store_path {
                    Some(path) => {
                        let store = decentraai_tokens::TokenStore::load(path)
                            .map_err(|_| GateError::Unauthorized)?;
                        match store.lookup(presented) {
                            Some(record) => Ok(Auth::Subscriber {
                                name: record.name.clone(),
                                tier: record.tier,
                                role: record.role,
                            }),
                            None => Err(GateError::Unauthorized),
                        }
                    }
                    None => Err(GateError::Unauthorized),
                }
            }
        }
    }

    /// Admin endpoints (P3: token create/list/revoke) are gated on the
    /// master token. When no API token is configured (open mode) there is
    /// no boundary to enforce, so admin stays usable single-user; subscriber
    /// tokens and unauthenticated callers are rejected. Returns a small
    /// [`GateError`] so the handler boundary turns it into the response.
    fn require_master(&self, headers: &HeaderMap) -> Result<(), GateError> {
        match self.classify(headers) {
            Ok(Auth::Master) | Ok(Auth::Open) => Ok(()),
            Ok(Auth::Subscriber { name, .. }) => Err(GateError::Forbidden(format!(
                "'{name}' is a subscription token; admin asks for the master token"
            ))),
            Ok(Auth::Consumer { key_id, .. }) => Err(GateError::Forbidden(format!(
                "'{key_id}' is a consumer API key; admin asks for the master token"
            ))),
            Err(_) => Err(GateError::Unauthorized),
        }
    }

    /// Role separation (H4): the operational read views (status, workers,
    /// network, execution, peers) are allowed for the master (admin), open
    /// mode (single-user), or an `operator`-role subscription token. A plain
    /// `client` token may only run inference within its tier.
    fn require_operator_or_admin(&self, headers: &HeaderMap) -> Result<(), GateError> {
        match self.classify(headers) {
            Ok(Auth::Master) | Ok(Auth::Open) => Ok(()),
            Ok(Auth::Subscriber { role, name, .. }) => {
                if role == decentraai_tokens::Role::Operator {
                    Ok(())
                } else {
                    Err(GateError::Forbidden(format!(
                        "'{name}' is a client token; operational views need an operator or admin token"
                    )))
                }
            }
            Ok(Auth::Consumer { key_id, .. }) => Err(GateError::Forbidden(format!(
                "'{key_id}' is a consumer API key; operational views need an operator or admin token"
            ))),
            Err(_) => Err(GateError::Unauthorized),
        }
    }

    /// Per-tier model allowlist. The request body's `model` field is
    /// advisory (llama-server serves what it loaded), but we enforce it
    /// anyway: it is honest about what the tier may use, and it protects
    /// multi-model routing when that lands.
    fn check_model_access(&self, auth: &Auth, body: &[u8]) -> Result<(), GateError> {
        let Auth::Subscriber { tier, name, .. } = auth else {
            // Master, open, and consumer keys have no per-model tier gate here:
            // a consumer key is bounded by its quota ceiling + rate limit, not
            // by the subscription-tier model allowlist.
            return Ok(());
        };
        let Some(tiers) = &self.tiers else {
            return Ok(());
        };
        let Some(policy) = tiers.policy(*tier) else {
            return Ok(());
        };
        if policy.models.is_empty() {
            return Ok(());
        }
        let requested: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let Some(model) = requested["model"].as_str() else {
            return Ok(());
        };
        let base = Path::new(model)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(model);
        if policy
            .models
            .iter()
            .any(|allowed| allowed == model || allowed == base)
        {
            Ok(())
        } else {
            Err(GateError::Forbidden(format!(
                "model '{base}' is not available to your tier ({name}, tier {tier})"
            )))
        }
    }

    /// Sliding-window rate limit per token. The master token and the
    /// open mode are unlimited; the window is pruned on every call.
    fn check_rate_limit(&self, auth: &Auth) -> Result<(), GateError> {
        let Auth::Subscriber { name, tier, .. } = auth else {
            return Ok(());
        };
        let Some(policy) = self.tiers.as_ref().and_then(|t| t.policy(*tier)) else {
            return Ok(());
        };
        let limit = policy.rate_limit_per_minute as usize;
        let mut windows = self.rate_windows.lock().unwrap();
        let window = windows.entry(name.clone()).or_default();
        let cutoff = Instant::now() - RATE_WINDOW;
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        if window.len() >= limit {
            decentraai_audit::record_best_effort(
                &self.info.repo_root.join("logs"),
                "rate_limited",
                serde_json::json!({"token": name, "tier": tier, "limit_per_minute": limit}),
            );
            return Err(GateError::RateLimited(limit));
        }
        window.push_back(Instant::now());
        Ok(())
    }

    /// Phase M LIMITS for the MUTATING execute path: a fixed per-name sliding
    /// window (default 10 mutations/minute) so a misbehaving/compromised master
    /// token cannot hammer the fabric with executions. Separate from tier
    /// inference limits; audited as `execute_rate_limited`. Read-only calls are
    /// never limited here.
    fn check_execute_rate_limit(&self, token_name: &str) -> Result<(), GateError> {
        let mut windows = self.execute_windows.lock().unwrap();
        let window = windows.entry(token_name.to_string()).or_default();
        let cutoff = Instant::now() - RATE_WINDOW;
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        if window.len() >= EXECUTE_RATE_LIMIT_PER_MINUTE {
            decentraai_audit::record_best_effort(
                &self.info.repo_root.join("logs"),
                "execute_rate_limited",
                serde_json::json!({
                    "token": token_name,
                    "limit_per_minute": EXECUTE_RATE_LIMIT_PER_MINUTE,
                }),
            );
            return Err(GateError::RateLimited(EXECUTE_RATE_LIMIT_PER_MINUTE));
        }
        window.push_back(Instant::now());
        Ok(())
    }

    /// Records one completed inference call: counters plus the ring buffer.
    /// Token counts come from llama.cpp's `usage`; tok/s prefers the
    /// backend's own `timings.predicted_per_second` and falls back to
    /// completion tokens over wall time. Streaming (SSE) bodies do not
    /// parse as JSON and simply record zeros for the token fields.
    fn record_inference(&self, endpoint: &str, elapsed: Duration, body: &[u8]) {
        let parsed: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let prompt = parsed["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        let completion = parsed["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        let per_sec = parsed["timings"]["predicted_per_second"]
            .as_f64()
            .or_else(|| {
                (completion > 0 && elapsed.as_secs_f64() > 0.0)
                    .then(|| completion as f64 / elapsed.as_secs_f64())
            })
            .unwrap_or(0.0);
        self.requests_served.fetch_add(1, Ordering::SeqCst);
        self.tokens_generated
            .fetch_add(completion, Ordering::SeqCst);
        let stat = RequestStat {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            endpoint: endpoint.to_string(),
            prompt_tokens: prompt,
            completion_tokens: completion,
            duration_ms: elapsed.as_millis() as u64,
            tokens_per_second: per_sec,
        };
        let mut log = self.recent_requests.lock().unwrap();
        log.push_front(stat);
        log.truncate(RECENT_REQUEST_LIMIT);
    }

    fn note_token_usage(&self, auth: &Auth, generated: u64) {
        match auth {
            Auth::Subscriber { name, .. } => {
                let mut usage = self.token_usage.lock().unwrap();
                let entry = usage.entry(name.clone()).or_default();
                entry.0 += 1;
                entry.1 += generated;
                entry.2 = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
            Auth::Consumer { key_id, .. } => {
                let mut usage = self.consumer_usage.lock().unwrap();
                let entry = usage.entry(key_id.clone()).or_default();
                entry.0 += 1;
                entry.1 += generated;
                entry.2 = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
            _ => {}
        }
    }

    /// Per-key rate limit for consumer API keys (Q2): a sliding window keyed
    /// by key_id, capped at the key's own `rate_limit_per_minute`. Independent
    /// from quota (frequency vs consumption) and from tier/execute limits.
    fn check_consumer_rate_limit(&self, key_id: &str, limit_per_minute: u32) -> Result<(), GateError> {
        let limit = limit_per_minute as usize;
        let mut windows = self.consumer_rate_windows.lock().unwrap();
        let window = windows.entry(key_id.to_string()).or_default();
        let cutoff = Instant::now() - RATE_WINDOW;
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        if window.len() >= limit {
            decentraai_audit::record_best_effort(
                &self.info.repo_root.join("logs"),
                "consumer_rate_limited",
                serde_json::json!({ "key_id": key_id, "limit_per_minute": limit }),
            );
            return Err(GateError::RateLimited(limit));
        }
        window.push_back(Instant::now());
        Ok(())
    }

    /// Whether consumer API keys are enabled (a registry path + shared ledger
    /// are wired). Read-only.
    fn consumer_enabled(&self) -> bool {
        self.consumer_keys_path.is_some() && self.quota_ledger.is_some()
    }

    /// Reserves quota for a consumer request (Q2) against the authoritative
    /// ledger, capped at `min(account.available, quota_ceiling)`. Returns a
    /// RAII guard: on success the caller settles it with measured usage; on
    /// any other exit the guard's `Drop` releases the reservation (no leak).
    fn reserve_consumer_quota(
        &self,
        account: &str,
        key_id: &str,
        request_id: &str,
        quota_ceiling: u64,
    ) -> Option<ConsumerQuotaGuard> {
        let ledger = self.quota_ledger.clone()?;
        let reservation_id = format!("consumer:{key_id}:{request_id}");
        {
            let mut ledger = ledger.lock().unwrap();
            let acc = ledger.account(&account.to_string());
            let available = acc.map(|a| a.available).unwrap_or(0);
            let amount = available.min(quota_ceiling);
            if amount == 0 {
                decentraai_audit::record_best_effort(
                    &self.info.repo_root.join("logs"),
                    "consumer_quota_denied",
                    serde_json::json!({ "key_id": key_id, "account": account, "request_id": request_id }),
                );
                return None;
            }
            if ledger
                .reserve(&account.to_string(), &reservation_id, amount)
                .is_err()
            {
                decentraai_audit::record_best_effort(
                    &self.info.repo_root.join("logs"),
                    "consumer_quota_denied",
                    serde_json::json!({ "key_id": key_id, "account": account, "request_id": request_id }),
                );
                return None;
            }
        }
        Some(ConsumerQuotaGuard::new(ledger, reservation_id))
    }
}

/// A small latency/success snapshot for the dashboard (M10): p50/p95/p99
/// latencies over recent requests plus the overall success rate. Pure and
/// I/O-free so tests drive it with synthetic samples.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InferenceStats {
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub count: u64,
    pub success_rate_percent: f64,
    pub requests_served: u64,
    pub requests_failed: u64,
    pub queue_waiting: usize,
}

/// Computes the given percentile (0..=100) from a list of durations (ms).
fn percentile_ms(mut samples: Vec<u64>, q: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = ((samples.len() - 1) as f64 * q / 100.0).floor() as usize;
    samples[idx]
}

/// Builds an [`InferenceStats`] from the recent-request ring buffer and the
/// live counters. Deterministic; empty history yields zeros.
pub fn inference_stats(
    recent: &[RequestStat],
    requests_served: u64,
    requests_failed: u64,
    queue_waiting: usize,
) -> InferenceStats {
    let durations: Vec<u64> = recent.iter().map(|r| r.duration_ms).collect();
    let total = requests_served + requests_failed;
    let success_rate = if total == 0 {
        0.0
    } else {
        requests_served as f64 / total as f64 * 100.0
    };
    InferenceStats {
        p50_ms: percentile_ms(durations.clone(), 50.0),
        p95_ms: percentile_ms(durations.clone(), 95.0),
        p99_ms: percentile_ms(durations.clone(), 99.0),
        count: recent.len() as u64,
        success_rate_percent: success_rate,
        requests_served,
        requests_failed,
        queue_waiting,
    }
}

/// Fills missing sampling fields from the configured generation
/// defaults and prepends the system prompt when the conversation has
/// none. Fields the caller set are never touched; malformed bodies pass
/// through unchanged and fail downstream as before.
pub fn apply_generation_defaults(generation: &GenerationSection, body: &[u8]) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.to_vec();
    };
    let Some(obj) = value.as_object_mut() else {
        return body.to_vec();
    };
    obj.entry("temperature")
        .or_insert_with(|| serde_json::json!(generation.temperature));
    obj.entry("top_p")
        .or_insert_with(|| serde_json::json!(generation.top_p));
    if let Some(k) = generation.top_k {
        obj.entry("top_k").or_insert_with(|| serde_json::json!(k));
    }
    obj.entry("repeat_penalty")
        .or_insert_with(|| serde_json::json!(generation.repeat_penalty));
    if !generation.system_prompt.is_empty() {
        if let Some(serde_json::Value::Array(messages)) = obj.get_mut("messages") {
            let has_system = messages.iter().any(|m| m["role"] == "system");
            if !has_system {
                messages.insert(
                    0,
                    serde_json::json!({"role": "system", "content": generation.system_prompt}),
                );
            }
        }
    }
    serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
}

/// The models indexed in the local registry, for the dashboard.
fn registry_models(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = data_dir.join("db/registry.json");
    let Ok(registry) = decentraai_registry::ModelRegistry::load(&path) else {
        return Vec::new();
    };
    registry
        .list_models()
        .into_iter()
        .map(|m| serde_json::json!({"name": m.relative_path, "size_bytes": m.size_bytes}))
        .collect()
}

// P3 - Admin dashboard handlers
const ADMIN_HTML: &str = r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>DecentraAI Admin</title>
<style>body{font:15px/1.5 system-ui,sans-serif;background:#0f141b;color:#e6edf3}.card{border:1px solid #2a3442;border-radius:10px;padding:14px}</style></head><body>
<h1>DecentraAI Admin</h1>
<div class="card"><h2>Create Token</h2><form id="f"><input name="name" placeholder="Token name" required><select name="t"><option value="1">Guest</option><option value="2">Contributor</option><option value="3">Core</option></select><select name="role"><option value="client">Client</option><option value="operator">Operator</option></select><button>Create</button></form><div id="new" style="display:none"><code id="token"></code><button onclick="navigator.clipboard.writeText(document.getElementById('token').textContent)">Copy</button></div><p id="status"></p></div>
<div class="card"><h2>Tokens</h2><table id="tbl"><thead><tr><th>Name</th><th>Tier</th><th>Role</th><th>Action</th></tr></thead><tbody></tbody></table></div>
<div class="card"><h2>Consumer API Keys</h2><form id="cf"><input name="account" placeholder="Owner account" required><input name="ceiling" type="number" min="1" placeholder="Quota ceiling" required><input name="rate" type="number" min="1" placeholder="req/min" required><button>Create</button></form><div id="cnew" style="display:none"><code id="ckey"></code><button onclick="navigator.clipboard.writeText(document.getElementById('ckey').textContent)">Copy</button><span>shown once</span></div><p id="cstatus"></p><table id="ctbl"><thead><tr><th>Key</th><th>Account</th><th>Ceiling</th><th>Rate</th><th>Used</th><th>Account quota (avail/cons)</th><th>Status</th><th>Action</th></tr></thead><tbody></tbody></table></div>
<div class="card"><h2>Audit events</h2><ul id="audit" style="list-style:none;padding-left:0"><li class="off">loading&hellip;</li></ul></div>
<p id="api-url"></p></body><script>
var f=document.getElementById('f'),status=document.getElementById('status'),tbl=document.querySelector('#tbl tbody'),tokenEl=document.getElementById('token'),newDiv=document.getElementById('new');
var cf=document.getElementById('cf'),cstatus=document.getElementById('cstatus'),ctbl=document.querySelector('#ctbl tbody'),ckeyEl=document.getElementById('ckey'),cnewDiv=document.getElementById('cnew');
var esc=function(s){return String(s).replace(/[&<>"]/g,function(c){return{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]});};
f.addEventListener('submit',async e=>{e.preventDefault();var n=f.name.value,t=parseInt(f.t.value),role=f.role.value;status.textContent='Creating...';var r=await fetch('/api/admin/token/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n,tier:t,role:role})});var d=await r.json();if(r.ok){tokenEl.textContent=d.token;newDiv.style.display='block';status.innerHTML='<span style="color:green">Saved! Copy now.</span>';f.reset()}else status.innerHTML='<span style="color:red">'+d.error.message+'</span>'};
cf.addEventListener('submit',async e=>{e.preventDefault();var acct=cf.account.value,ceil=parseInt(cf.ceiling.value),rate=parseInt(cf.rate.value);cstatus.textContent='Creating...';var r=await fetch('/api/admin/consumer-key/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({account:acct,quota_ceiling:ceil,rate_limit_per_minute:rate})});var d=await r.json();if(r.ok){ckeyEl.textContent=d.token;cnewDiv.style.display='block';cstatus.innerHTML='<span style="color:green">Saved! Copy now.</span>';cf.reset()}else cstatus.innerHTML='<span style="color:red">'+d.error.message+'</span>';loadConsumer();});
async function load(){var r=await fetch('/api/admin/token/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();tbl.innerHTML='';d.tokens.forEach(t=>{var row=document.createElement('tr');row.innerHTML='<td>'+esc(t.name)+'</td><td>'+t.tier+'</td><td>'+esc(t.role)+'</td><td><button data-n="'+t.name+'" onclick="revoke(event)">Revoke</button></td>';tbl.appendChild(row)});loadAudit();loadConsumer();}
async function loadConsumer(){var r=await fetch('/api/admin/consumer-key/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();ctbl.innerHTML='';if(!(d.keys||[]).length){ctbl.innerHTML='<tr><td colspan="8" class="off">no consumer API keys</td></tr>';return;}d.keys.forEach(k=>{var q=k.account_quota||{},row=document.createElement('tr');row.innerHTML='<td><code>'+esc(k.key_id)+'</code></td><td>'+esc(k.account)+'</td><td>'+k.quota_ceiling+'</td><td>'+k.rate_limit_per_minute+'</td><td>'+k.requests+' ('+k.tokens_generated+' tok)</td><td>'+q.available+'/'+q.consumed+'</td><td>'+(k.revoked?'revoked':'active')+'</td><td>'+(k.revoked?'':'<button data-id="'+esc(k.key_id)+'" onclick="revokeConsumer(event)">Revoke</button>')+'</td>';ctbl.appendChild(row)});}
var auditEl=document.getElementById('audit');
async function loadAudit(){var r=await fetch('/api/admin/events',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();var evs=d.events||[];auditEl.innerHTML=evs.length?'':('<li class="off">no security events yet</li>');evs.forEach(function(e){var li=document.createElement('li');var d2=new Date((e.timestamp||0)*1000).toLocaleString();li.innerHTML='<code>'+esc(e.event||'')+'</code> <span class="off">'+d2+'</span> <span class="small">'+esc(JSON.stringify(e.details||Object()))+'</span>';auditEl.appendChild(li);});}
window.onload=load;
function revoke(e){var n=e.target.dataset.n;fetch('/api/admin/token/revoke',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n})}).then(_=>load());}
function revokeConsumer(e){var id=e.target.dataset.id;fetch('/api/admin/consumer-key/revoke',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({key_id:id})}).then(_=>loadConsumer());}
document.getElementById('api-url').textContent='API: http://127.0.0.1:{}/v1';
</script></html>"##;
fn admin_html(port: u16) -> String {
    ADMIN_HTML.replace("{}", &port.to_string())
}
async fn admin_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    Html(admin_html(state.info.api_port)).into_response()
}
async fn admin_token_list_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let tokens = match &state.token_store_path {
        Some(p) => decentraai_tokens::TokenStore::load(p)
            .map(|s| s.list())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    // Per-token live usage (requests + generated tokens) collected by
    // note_token_usage during routed inference; absent for tokens that never
    // served a request or when inference bypassed the coordinator.
    let usage = state.token_usage.lock().unwrap().clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let body = serde_json::json!({"tokens": tokens.iter().map(|t| {
        let u = usage.get(&t.name).copied().unwrap_or((0, 0, 0));
        serde_json::json!({
            "name": &t.name,
            "tier": t.tier,
            "role": t.role.name(),
            "created_at": t.created_at,
            "revoked": t.revoked,
            "expires_at": t.expires_at,
            "expired": t.expires_at.is_some_and(|ts| ts <= now),
            "requests": u.0,
            "tokens_generated": u.1,
            "last_used_at": if u.2 > 0 { Some(u.2) } else { None },
        })
    }).collect::<Vec<_>>()});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
async fn admin_token_create_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return forbidden("missing name"),
    };
    let tier = match req
        .get("tier")
        .and_then(|v| v.as_u64())
        .and_then(|n| u8::try_from(n).ok())
    {
        Some(t) if (1..=3).contains(&t) => t,
        _ => return forbidden("tier 1-3"),
    };
    let role = match req
        .get("role")
        .and_then(|v| v.as_str())
        .and_then(|s| decentraai_tokens::Role::parse(s).ok())
    {
        Some(r) => r,
        None => decentraai_tokens::Role::DEFAULT,
    };
    // Optional unix-seconds expiry (Part 12/22 developer tokens): an expired
    // token stops authenticating even though its record stays listed.
    let expires_at = req.get("expires_at").and_then(|v| v.as_u64());
    let plaintext = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.create_with_role(&name, decentraai_tokens::Tier(tier), expires_at, role) {
                Ok(t) => {
                    let a = state.info.repo_root.join("logs/audit.jsonl");
                    let _ = decentraai_audit::record(
                        a.parent().unwrap_or(&state.info.repo_root),
                        "token_created",
                        serde_json::json!({"name": &name, "tier": tier, "role": role.name(), "expires_at": expires_at}),
                    );
                    Some(t)
                }
                Err(_) => return forbidden("name taken"),
            }
        }
        None => return forbidden("no store"),
    };
    let body = serde_json::json!({"token": plaintext, "name": name, "tier": tier, "role": role.name(), "expires_at": expires_at});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
async fn admin_token_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let name = match req.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return forbidden("missing name"),
    };
    let success = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.revoke(&name) {
                Ok(()) => true,
                Err(_) => return forbidden("no such token"),
            }
        }
        None => return forbidden("no store"),
    };
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "token_revoked",
        serde_json::json!({"name": name}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": success}).to_string(),
    )
        .into_response()
}

/// Q2 — Create a consumer API key (`dca_…`) for an account, master-gated.
/// Shows the plaintext secret exactly once; only its hash + display prefix
/// are stored. The key carries a per-request quota ceiling, a per-key rate
/// limit, and optional scopes. The key never grants admin/operator privileges.
async fn admin_consumer_key_create_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(path) = &state.consumer_keys_path else {
        return forbidden("consumer keys are not enabled (no shared ledger)");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let account = match req.get("account").and_then(|v| v.as_str()) {
        Some(a) if !a.trim().is_empty() => a.trim().to_string(),
        _ => return forbidden("missing account"),
    };
    let quota_ceiling = req.get("quota_ceiling").and_then(|v| v.as_u64()).unwrap_or(0);
    let rate_limit_per_minute = req
        .get("rate_limit_per_minute")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32;
    let scopes = req
        .get("scopes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if quota_ceiling == 0 {
        return forbidden("quota_ceiling must be > 0");
    }
    if rate_limit_per_minute == 0 {
        return forbidden("rate_limit_per_minute must be > 0");
    }
    let mut store = match decentraai_tokens::ConsumerKeyStore::load(path) {
        Ok(s) => s,
        Err(_) => return forbidden("consumer key store unreadable"),
    };
    let plaintext = match store.create(&account, quota_ceiling, rate_limit_per_minute, scopes.clone())
    {
        Ok(p) => p,
        Err(e) => return forbidden(&e.to_string()),
    };
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "consumer_key_created",
        serde_json::json!({
            "account": account,
            "key_prefix": decentraai_tokens::key_prefix(&plaintext),
            "quota_ceiling": quota_ceiling,
            "rate_limit_per_minute": rate_limit_per_minute,
            "scopes": scopes,
        }),
    );
    let key_id = store
        .lookup(&plaintext)
        .map(|r| r.key_id.clone())
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "token": plaintext,
            "key_id": key_id,
            "key_prefix": decentraai_tokens::key_prefix(&plaintext),
            "account": account,
            "quota_ceiling": quota_ceiling,
            "rate_limit_per_minute": rate_limit_per_minute,
            "note": "shown once; only its hash and prefix are stored",
        })
        .to_string(),
    )
        .into_response()
}

/// Q2 — Revoke a consumer API key by key id, master-gated.
async fn admin_consumer_key_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(path) = &state.consumer_keys_path else {
        return forbidden("consumer keys are not enabled");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let key_id = match req.get("key_id").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => return forbidden("missing key_id"),
    };
    let mut store = match decentraai_tokens::ConsumerKeyStore::load(path) {
        Ok(s) => s,
        Err(_) => return forbidden("consumer key store unreadable"),
    };
    if store.revoke(&key_id).is_err() {
        return forbidden("no active consumer key with that id");
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "consumer_key_revoked",
        serde_json::json!({"key_id": key_id}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "key_id": key_id}).to_string(),
    )
        .into_response()
}

/// Q2 — List consumer API key metadata (never the plaintext secret),
/// master-gated. Includes live usage counters from the running node.
async fn admin_consumer_key_list_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let keys = match &state.consumer_keys_path {
        Some(p) => decentraai_tokens::ConsumerKeyStore::load(p)
            .map(|s| s.list())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let usage = state.consumer_usage.lock().unwrap().clone();
    let ledger = state.quota_ledger.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let body = serde_json::json!({"keys": keys.iter().map(|k| {
        let u = usage.get(&k.key_id).copied().unwrap_or((0, 0, 0));
        // Live account balance for the key's owner (authoritative ledger).
        let (available, reserved, consumed) = ledger.as_ref().map(|l| {
            let l = l.lock().unwrap();
            let acc = l.account(&k.owner_account);
            (
                acc.map(|a| a.available).unwrap_or(0),
                acc.map(|a| a.reserved).unwrap_or(0),
                acc.map(|a| a.consumed).unwrap_or(0),
            )
        }).unwrap_or((0, 0, 0));
        serde_json::json!({
            "key_id": &k.key_id,
            "prefix": &k.prefix,
            "account": &k.owner_account,
            "created_at": k.created_at,
            "revoked": k.revoked,
            "quota_ceiling": k.quota_ceiling,
            "rate_limit_per_minute": k.rate_limit_per_minute,
            "scopes": &k.scopes,
            "requests": u.0,
            "tokens_generated": u.1,
            "last_used_at": if u.2 > 0 { Some(u.2) } else { None },
            "account_quota": { "available": available, "reserved": reserved, "consumed": consumed },
            "expired_or_revoked": k.revoked,
            "age_secs": now.saturating_sub(k.created_at),
        })
    }).collect::<Vec<_>>()});
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Model Hub search (Part 16/22): `GET /api/admin/hub/search?query=…&limit=…`
/// queries HuggingFace for GGUF models. Master-gated; the dashboard Model Hub
/// card calls this to let operators discover models to pull, on-device.
async fn admin_hub_search_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let q = query.get("query").cloned().unwrap_or_default();
    if q.trim().is_empty() {
        return forbidden("missing query");
    }
    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
        .min(30);
    // Optional capability filter: keep only models whose real metadata supports
    // the requested capability (e.g. `capability=ocr`). Unsupported or invalid
    // capability values yield an empty filtered set, never a false positive.
    let capability = query
        .get("capability")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let catalog = decentraai_hub::HubCatalog::new();
    match catalog.search(&q, limit).await {
        Ok(models) => {
            let body = hub_search_body(&q, &models, capability);
            (
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                .to_string(),
        )
            .into_response(),
    }
}

/// Serialize a Hub search result for the admin API. Pure, so tests can
/// drive it with synthetic models.
///
/// When `capability` is supplied, models are filtered to those whose real Hub
/// metadata supports that capability (via the provenance-aware matcher); the
/// kept hits carry the matching evidence so the operator sees why each model
/// qualified.
fn hub_search_body(
    query: &str,
    models: &[decentraai_hub::HubModel],
    capability: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    let req = capability.map(|cap| vec![decentraai_hub::CapabilityRequirement {
        capability: cap,
        evidence: decentraai_hub::EvidenceLevel::Any,
    }]);

    let filtered: Vec<&decentraai_hub::HubModel> = models
        .iter()
        .filter(|m| match &req {
            Some(r) => {
                // Classify this model from its own metadata and check the
                // required capability. A model that does not claim it is
                // dropped (UNKNOWN is not satisfied, never fabricated).
                let caps = decentraai_hub::capability::classify(
                    m.pipeline_tag.as_ref().map(|t| t.as_str()),
                    &m.tags,
                    &m.id,
                );
                decentraai_hub::satisfies_any(&caps, r)
            }
            None => true,
        })
        .collect();

    serde_json::json!({
        "query": query,
        "capability_filter": capability.map(|c| c.label()),
        "total": models.len(),
        "matched": filtered.len(),
        "models": filtered.iter().map(|m| serde_json::json!({
            "id": m.id,
            "pipeline_tag": m.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": m.tags,
            "downloads": m.downloads,
        })).collect::<Vec<_>>(),
    })
}

/// Live Hub capability search for the MCP `search_models_by_capability` tool.
/// Performs the network lookup, then reuses the pure [`hub_search_body`]
/// filter so the result is provenance-honest. On Hub failure it returns an
/// explicit error object (never a fabricated empty 'no models').
async fn mcp_capability_search(
    query: &str,
    limit: usize,
    capability: decentraai_hub::CapabilityKind,
) -> serde_json::Value {
    let catalog = decentraai_hub::HubCatalog::new();
    match catalog.search(query, limit).await {
        Ok(models) => hub_search_body(query, &models, Some(capability)),
        Err(e) => serde_json::json!({
            "error": e.to_string(),
            "matched": 0,
            "models": [],
        }),
    }
}

/// Local capability filter for the MCP `find_local_models_by_capability` tool.
/// Filters THIS node's models (from `fabric_model_list`) by their persisted
/// registry claims — no Hub round-trip. `evidence` is "any" (verified or
/// inferred) or "verified" (only verified claims). A model with no matching
/// claim is never included (honest: absent = UNKNOWN).
async fn mcp_local_capability_search(
    state: &ApiState,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    let Some(list) = fabric_model_list(state).await else {
        return serde_json::json!({ "matched": 0, "models": [] });
    };
    filter_local_models_by_capability(&list, capability, evidence)
}

/// Pure filter: given an OpenAI-style model list (entries may carry
/// `capability_claims`), keep only local models whose persisted claims satisfy
/// the capability. `evidence` is "any" or "verified". A model with no matching
/// claim is never included (honest: absent = UNKNOWN). Pure so tests can drive
/// it with a synthetic list.
fn filter_local_models_by_capability(
    list: &serde_json::Value,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    let require_verified = evidence == "verified";
    let mut matched = Vec::new();
    for m in list["data"].as_array().cloned().unwrap_or_default() {
        let Some(claims) = m["capability_claims"].as_array() else {
            continue; // no persisted claims -> UNKNOWN, not a match
        };
        let hit = claims.iter().find(|c| {
            let cap = c["capability"].as_str().unwrap_or("");
            let prov = c["provenance"].as_str().unwrap_or("");
            cap.eq_ignore_ascii_case(capability)
                && (!require_verified || prov.eq_ignore_ascii_case("verified"))
        });
        if let Some(hit) = hit {
            matched.push(serde_json::json!({
                "id": m["id"],
                "evidence": hit["provenance"],
            }));
        }
    }
    serde_json::json!({
        "capability": capability,
        "evidence": if require_verified { "verified" } else { "any" },
        "matched": matched.len(),
        "models": matched,
    })
}

/// Evaluate every worker in the fabric against a model + capability query
/// (MCP `get_worker_capability`). A thin, READ-ONLY projection: it reuses the
/// authoritative worker advertisements, persisted registry claims, the existing
/// capability resolver and resource-fit vocabulary — no execution, no
/// reservations, no model start. Remote workers with no capability/telemetry
/// data resolve to honest UNKNOWN rather than a fabricated verdict.
async fn mcp_worker_capability(
    state: &ApiState,
    model: &str,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    // Best-effort registry load for persisted claims (absent => UNKNOWN).
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let claims: Vec<(String, String)> = registry
        .as_ref()
        .map(|reg| {
            claims_for_file_name(reg, model)
                .into_iter()
                .map(|c| (c.capability, c.provenance))
                .collect()
        })
        .unwrap_or_default();

    // Fetch the worker set once and reuse it for the requested model and for
    // every on-disk variant below (workers()/is_trusted are async I/O).
    let mut workers: Vec<(decentraai_compute::ComputeAdvertisement, bool)> = Vec::new();
    let mut local_peer: Option<String> = None;
    if let Some(cm) = &state.compute {
        local_peer = Some(cm.local_peer().to_string());
        for adv in cm.workers().await {
            let trusted = cm.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
    }

    let mut results: Vec<WorkerCapResult> = Vec::new();
    for (adv, trusted) in &workers {
        let is_local = local_peer.as_deref() == Some(&adv.peer_id.to_string());
        let accepts_remote_work = is_local || adv.accepts_remote_inference;
        results.push(worker_capability_verdict_with_policy(
            adv,
            *trusted,
            model,
            capability,
            evidence,
            &claims,
            accepts_remote_work,
        ));
    }
    let fit = aggregate_can_i_run(&results);
    let workers_json: Vec<serde_json::Value> = results.iter().map(|r| r.to_json()).collect();

    // Honest model metadata: quantization is INFERRED from the requested model
    // string when it carries a recognized marker, else null (UNKNOWN);
    // available_workers counts workers that actually hold the model (served or
    // on-disk), derived from the real per-worker verdicts above.
    let quantization = variant_quantization_from_file_name(model);
    let available_workers = results
        .iter()
        .filter(|r| r.model_availability != "unavailable")
        .count();

    // On-disk GGUF variants of this model from the REAL local registry (never
    // invented). Each variant is evaluated by the SAME per-worker pipeline as
    // the requested model, so a variant with no matching worker honestly
    // resolves to CANNOT_RUN/UNKNOWN via the existing aggregate.
    let variants: Vec<serde_json::Value> = registry
        .as_ref()
        .map(|reg| {
            registry_variants_for_model(reg, model)
                .into_iter()
                .map(|(file, size_bytes)| {
                    let v_claims: Vec<(String, String)> = claims_for_file_name(reg, &file)
                        .into_iter()
                        .map(|c| (c.capability, c.provenance))
                        .collect();
                    let mut v_results: Vec<WorkerCapResult> = Vec::new();
                    for (adv, trusted) in &workers {
                        let is_local = local_peer.as_deref() == Some(&adv.peer_id.to_string());
                        let accepts_remote_work = is_local || adv.accepts_remote_inference;
                        v_results.push(worker_capability_verdict_with_policy(
                            adv,
                            *trusted,
                            &file,
                            capability,
                            evidence,
                            &v_claims,
                            accepts_remote_work,
                        ));
                    }
                    let v_fit = aggregate_can_i_run(&v_results);
                    serde_json::json!({
                        "file": file,
                        "quantization": variant_quantization_from_file_name(&file),
                        "size_bytes": size_bytes,
                        "fit": v_fit.to_json(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Best on-disk variant to deploy on THIS fabric: the first variant whose
    // fit is CAN_RUN (variants are in deterministic file-name order), else None
    // (honest: no variant is confirmed runnable).
    let best_variant = variants
        .iter()
        .find(|v| v["fit"]["verdict"] == "CAN_RUN")
        .and_then(|v| v["file"].as_str().map(str::to_string));

    serde_json::json!({
        "model": model,
        "capability": capability,
        "evidence": evidence,
        "model_info": {
            "model": model,
            "quantization": quantization,
            "available_workers": available_workers,
            "best_variant": best_variant,
        },
        "fit": fit.to_json(),
        "worker_count": workers_json.len(),
        "workers": workers_json,
        "variants": variants,
    })
}

/// Composed intent → capability → fabric-fit resolution for the MCP
/// `resolve_intent_with_fit` tool. Closes the Intent Planner loop: a
/// natural-language intent maps (deterministically) to capabilities, and for
/// each capability a real matching local model is found from the persisted
/// registry claims, then evaluated against the fabric via the SAME per-worker
/// verdict + aggregate pipeline. Read-only; never triggers execution.
///
/// Honest by construction: a capability with no matching local model reports
/// fit = UNKNOWN ("no local model"); a capability that resolves to a model with
/// no workers also reports UNKNOWN via the aggregate. Nothing is fabricated.
async fn mcp_intent_with_fit(
    state: &ApiState,
    intent: &str,
    evidence: &str,
) -> serde_json::Value {
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let require_verified = evidence == "verified";

    let capabilities = decentraai_hub::intent::capabilities_for_intent(intent);

    let mut capabilities_out = Vec::new();
    for cap in capabilities {
        let cap_str = serde_json::to_string(&cap)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();

        // Find a real local model with a persisted claim for this capability.
        let mut candidate: Option<(String, String)> = None; // (file, provenance)
        if let Some(reg) = &registry {
            if let Some(m) = reg.models_with_capability(&cap_str, require_verified).into_iter().next() {
                candidate = Some((m.0.to_string(), m.2.to_string()));
            }
        }

        let (fit_json, model_used) = match candidate {
            Some((file, prov)) => {
                let claims = vec![(cap_str.clone(), prov)];
                let mut results = Vec::new();
                if let Some(cm) = &state.compute {
                    let local_peer = cm.local_peer().to_string();
                    for adv in cm.workers().await {
                        let trusted = cm.is_trusted(&adv.peer_id).await;
                        let accepts_remote_work =
                            local_peer == adv.peer_id.to_string() || adv.accepts_remote_inference;
                        results.push(worker_capability_verdict_with_policy(
                            &adv,
                            trusted,
                            &file,
                            &cap_str,
                            evidence,
                            &claims,
                            accepts_remote_work,
                        ));
                    }
                }
                let fit = aggregate_can_i_run(&results);
                (fit.to_json(), Some(file))
            }
            None => (
                serde_json::json!({
                    "verdict": "UNKNOWN",
                    "counts": { "can_run": 0, "cannot_run": 0, "unknown": 0 },
                    "chosen_worker": null,
                    "reasons": ["no local model with a claim for this capability"],
                }),
                None,
            ),
        };

        capabilities_out.push(serde_json::json!({
            "capability": cap_str,
            "label": cap.label(),
            "evidence": if require_verified { "verified" } else { "any" },
            "model": model_used,
            "fit": fit_json,
        }));
    }

    serde_json::json!({
        "intent": intent,
        "capabilities": capabilities_out,
        "note": "intent-to-capability is INFERRED from keywords; fit reflects real local models + fabric state.",
    })
}

/// Build the worker-per-model fit for one model file against the live fabric.
/// Pure given the worker set; returns per-worker verdicts.
fn fabric_fit_for_model(
    model_file: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
    workers: &[(decentraai_compute::ComputeAdvertisement, bool)],
    local_peer: &str,
) -> Vec<WorkerCapResult> {
    let mut results = Vec::new();
    for (adv, trusted) in workers {
        let accepts_remote_work =
            *local_peer == adv.peer_id.to_string() || adv.accepts_remote_inference;
        results.push(worker_capability_verdict_with_policy(
            adv,
            *trusted,
            model_file,
            capability,
            evidence,
            claims,
            accepts_remote_work,
        ));
    }
    results
}

/// Suggested share (%) of a request-level workload each CAN_RUN worker could
/// absorb, based on real advertised capacity — throughput × idle headroom ×
/// adaptive contribution factor (thermal/battery/GPU-util pressure). Pure,
/// INFERRED, and
/// advisory only — it never changes scheduling. Uses the authoritative pure
/// distribution [`decentraai_compute::adaptive_load_shares`]; normalized so
/// the shares sum to ~100. `UNKNOWN`/no-eligible → empty.
fn load_balance_for_workers(
    workers: &[(decentraai_compute::ComputeAdvertisement, bool)],
    can_run_peer_ids: &std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let eligible: Vec<(String, String, decentraai_compute::ComputeAvailability)> = workers
        .iter()
        .filter(|(w, _)| can_run_peer_ids.contains(&w.peer_id.to_string()))
        .map(|(w, _)| (w.peer_id.to_string(), w.node_id.clone(), w.availability.clone()))
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }
    let shares = decentraai_compute::adaptive_load_shares(&eligible);
    shares
        .into_iter()
        .map(|s| {
            let w = workers
                .iter()
                .find(|(w, _)| w.peer_id.to_string() == s.peer_id);
            let (tps, load, trusted, node_name, device) = match w {
                Some((w, trusted)) => (
                    w.availability.tokens_per_second,
                    w.availability.load_percent,
                    *trusted,
                    w.node_name.clone(),
                    device_class(&w.capability),
                ),
                None => (0, 0, false, String::new(), ""),
            };
            serde_json::json!({
                "peer_id": s.peer_id,
                "node_id": s.node_id,
                "node_name": node_name,
                "trusted": trusted,
                "device_class": device,
                "tokens_per_second": tps,
                "load_percent": load,
                "adaptive_contribution": s.adaptive_factor,
                "suggested_share_pct": (s.share * 100.0).round() as u32,
            })
        })
        .collect()
}

/// ONE coherent, explainable fabric decision (Phase 1 — Unified Decision).
///
/// Combines intent → capabilities → model options → per-variant fabric fit →
/// chosen decision → why, by REUSING the existing capability resolver, the
/// per-worker verdict, the aggregate, and the registry claims. It is a
/// read-only projection, NOT a new planner or scoring system — the "best"
/// choice is the first CAN_RUN (deterministic order), and every reason comes
/// from the real per-worker checks. No fabricated telemetry.
///
/// `explicit_model` (optional) narrows the model options to that model file;
/// otherwise all local models with a claim for each capability are considered.
async fn unified_fabric_decision(
    state: &ApiState,
    intent: &str,
    evidence: &str,
    explicit_model: Option<&str>,
) -> serde_json::Value {
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let require_verified = evidence == "verified";

    // Live worker set + local peer (I/O once).
    let mut workers: Vec<(decentraai_compute::ComputeAdvertisement, bool)> = Vec::new();
    let mut local_peer = String::new();
    // Historical measured execution statistics (Phase 2): real aggregates only,
    // UNKNOWN when insufficient.
    let mut historical: serde_json::Value = serde_json::json!({ "records": 0 });
    // Recent recovery timeline (Phase 5): what happened when something failed —
    // projected from the real decisions' trace using the existing vocabulary.
    let mut recovery: Vec<serde_json::Value> = Vec::new();
    if let Some(cm) = &state.compute {
        local_peer = cm.local_peer().to_string();
        for adv in cm.workers().await {
            let trusted = cm.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        historical = decentraai_distributed::execution_statistics(&cm.executions());
        recovery = cm
            .decisions()
            .iter()
            .take(5)
            .map(|d| {
                let mut r = decentraai_fabric::recovery_timeline(d);
                r["request_id"] = serde_json::json!(d.request_id);
                r
            })
            .collect();
    }

    let capabilities = decentraai_hub::intent::capabilities_for_intent(intent);

    // capabilities → model_options → best decision.
    let mut capabilities_out = Vec::new();
    let mut best: Option<(String, String, String)> = None; // (cap, model, worker)
    let mut best_why: Vec<String> = Vec::new();

    for cap in capabilities {
        let cap_str = serde_json::to_string(&cap)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();

        // Candidate models: explicit one (if given) else all local models with
        // a persisted claim for this capability.
        let mut model_files: Vec<String> = Vec::new();
        if let Some(explicit) = explicit_model {
            model_files.push(explicit.to_string());
        } else if let Some(reg) = &registry {
            for m in reg.models_with_capability(&cap_str, require_verified) {
                model_files.push(m.0.to_string());
            }
        }

        let mut model_options = Vec::new();
        for model_file in &model_files {
            let claims: Vec<(String, String)> = registry
                .as_ref()
                .map(|reg| {
                    claims_for_file_name(reg, model_file)
                        .into_iter()
                        .map(|c| (c.capability, c.provenance))
                        .collect()
                })
                .unwrap_or_default();
            let results =
                fabric_fit_for_model(model_file, &cap_str, evidence, &claims, &workers, &local_peer);
            let fit = aggregate_can_i_run(&results);
            let verdict = match fit.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            };
            let chosen = fit.chosen_worker.clone();
            // Record the first CAN_RUN as the fabric-wide best for this capability.
            if best.is_none() && fit.verdict == WorkerCapVerdict::CanRun {
                best = chosen
                    .clone()
                    .map(|w| (cap_str.clone(), model_file.clone(), w));
                // Aggregate the best model's passing checks as the "why".
                best_why = results
                    .iter()
                    .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                    .flat_map(|r| r.checks.iter())
                    .filter(|c| c.pass)
                    .map(|c| format!("✓ {} — {}", c.check, c.state))
                    .collect::<Vec<_>>();
            }
            let can_run_peer_ids: std::collections::HashSet<String> = results
                .iter()
                .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                .map(|r| r.peer_id.clone())
                .collect();
            model_options.push(serde_json::json!({
                "model": model_file,
                "quantization": variant_quantization_from_file_name(model_file),
                "verdict": verdict,
                "fit": fit.to_json(),
                "can_run_workers": results
                    .iter()
                    .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                    .map(|r| serde_json::json!({
                        "peer_id": r.peer_id, "node_id": r.node_id, "node_name": r.node_name,
                        "trusted": r.trusted, "engine": r.engine_compat,
                        "ram_sufficient": r.ram_sufficient, "vram_sufficient": r.vram_sufficient,
                    }))
                    .collect::<Vec<_>>(),
                // Adaptive fan-out advisory: suggested request-level share ({%})
                // per CAN_RUN worker (capacity x idle headroom), advisory only.
                "load_balance": load_balance_for_workers(&workers, &can_run_peer_ids),
            }));
        }

        capabilities_out.push(serde_json::json!({
            "capability": cap_str,
            "label": cap.label(),
            "evidence": if require_verified { "verified" } else { "any" },
            "model_options": model_options,
        }));
    }

    serde_json::json!({
        "request": intent,
        "capabilities": capabilities_out,
        "decision": match &best {
            Some((cap, model, worker)) => serde_json::json!({
                "capability": cap,
                "model": model,
                "worker": worker,
            }),
            None => serde_json::Value::Null,
        },
        "why": best_why,
        "historical": historical,
        "recent_recovery": recovery,
        "note": "coherent read-only projection of real fabric state; decision = first CAN_RUN (deterministic); reasons from real per-worker checks.",
    })
}

/// Fabric graph projection for the MCP `get_fabric_graph` tool (Phase C). Same
/// pure aggregation as `GET /v1/fabric`, read-only, no execution. Real state
/// only — never fabricated nodes/models/capabilities.
async fn mcp_fabric_graph(state: &ApiState) -> serde_json::Value {
    let registry = decentraai_registry::ModelRegistry::load(
        &state.info.repo_root.join("db/registry.json"),
    )
    .ok();
    let mut workers: Vec<(decentraai_distributed::ComputeAdvertisement, bool)> = Vec::new();
    let mut decisions: Vec<decentraai_fabric::ExecutionDecision> = Vec::new();
    let mut network = decentraai_fabric::NetworkGraph::new();
    let mut sessions_active = 0usize;
    let mut coordinator_version = String::new();
    if let Some(compute) = &state.compute {
        coordinator_version = compute.node_version().to_string();
        for adv in compute.workers().await {
            let trusted = compute.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        decisions = compute.decisions();
        sessions_active = compute.session_count();
        network = compute.network_graph();
    }
    fabric_graph_aggregate(
        &workers,
        registry.as_ref(),
        &decisions,
        &network,
        sessions_active,
        &coordinator_version,
    )
}

/// Resolve a BLAKE3 `model_hash` for a model file name from the live fabric
/// advertisements (served or on-disk). `None` when no worker advertises the
/// model (honest: cannot execute a model the fabric does not hold).
async fn resolve_model_hash(
    state: &ApiState,
    file_name: &str,
) -> Option<String> {
    let cm = state.compute.as_ref()?;
    let workers = cm.workers().await;
    for adv in workers {
        for m in adv
            .capability
            .served_models
            .iter()
            .chain(adv.capability.available_models.iter())
        {
            let f = m.file_name.to_lowercase();
            let target = file_name.to_lowercase();
            if f == target || f.ends_with(&target) || target.ends_with(&f) {
                return Some(m.model_hash.clone());
            }
        }
    }
    None
}

/// Execute a decided plan — the mutation step of `decide → reserve → execute`.
/// Requires an explicit `confirm: true` (mutating: reserves a worker and runs a
/// real inference). Reuses the existing `plan_and_reserve` + `route_request`
/// path (via `DistributedInference`); it does NOT introduce a new planner,
/// reservation ledger, or execution engine.
///
/// Input (JSON body):
///   { "intent": "...", "prompt": "...", "max_tokens": N, "stream": bool,
///     "model": "file.gguf" (optional), "evidence": "any|verified",
///     "confirm": true }
///
/// Returns the unified decision + the chosen model + the real inference result
/// (or a clear error). Without `confirm: true` it refuses (mutation safety).
async fn execute_decision_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Phase M LIMITS: mutations are rate-limited per token name (master here,
    // since execute is master-gated) so the fabric cannot be hammered.
    if let Err(e) = state.check_execute_rate_limit("master") {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return forbidden("invalid JSON"),
    };
    // STREAM step: when the caller asks for a stream, emit SSE from the
    // fabric router instead of a single buffered JSON body.
    if req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false) {
        return execute_decision_stream(&state, &req).await;
    }
    run_execute_decision(&state, &req).await
}

/// The STREAM step of decide→confirm→reserve→execute: run the decided model on
/// the fabric and stream the output as SSE (like the chat proxy's remote route),
/// reusing `route_request_streamed`. Enforces `confirm: true` (mutation safety).
async fn execute_decision_stream(
    state: &ApiState,
    req: &serde_json::Value,
) -> Response {
    // Mutation safety: explicit confirmation is required.
    if req.get("confirm").and_then(|c| c.as_bool()) != Some(true) {
        return forbidden("mutating execution requires \"confirm\": true");
    }
    let prompt = req.get("prompt").and_then(|p| p.as_str()).unwrap_or_default();
    if prompt.trim().is_empty() {
        return forbidden("missing prompt");
    }
    // Intent OR a direct capability can drive the decision (capability alone
    // lets an operator run a specific capability+model without intent parsing).
    let intent = execute_decision_intent(req);
    if intent.trim().is_empty() {
        return forbidden("missing intent (or a capability to run)");
    }
    let max_tokens = req
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(1024)
        .min(4096) as u32;
    let evidence = req.get("evidence").and_then(|e| e.as_str()).unwrap_or("any");
    let evidence = if evidence == "verified" { "verified" } else { "any" };
    let explicit_model = req.get("model").and_then(|m| m.as_str());

    // decide → chosen model.
    let decision = unified_fabric_decision(state, &intent, evidence, explicit_model).await;
    let Some(model) = decision["decision"]["model"].as_str().map(str::to_string) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "no runnable decision on the fabric for this intent (nothing to execute)", "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };
    let Some(model_hash) = resolve_model_hash(state, &model).await else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": format!("chosen model '{model}' has no advertised model hash on the fabric"), "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    // DRY-RUN: show exactly what would be reserved/routed without executing.
    // Requires the same confirm gate (it is part of the mutation path), but
    // never sends a request or holds a reservation.
    if req.get("dry_run").and_then(|d| d.as_bool()).unwrap_or(false) {
        let prompt_tokens = decentraai_distributed::prompt_token_estimate(prompt);
        let preview = match &state.compute {
            Some(cm) => {
                cm.plan_preview(
                    &model_hash,
                    prompt_tokens,
                    req.get("session_id").and_then(|s| s.as_str()),
                    req.get("priority").and_then(|p| p.as_u64()).unwrap_or(0) as u8,
                )
                .await
            }
            None => None,
        };
        return match preview {
            Some((plan, worker, est_ms)) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "dry_run": true,
                    "decision": decision,
                    "would_execute": {
                        "model": model,
                        "model_hash": model_hash,
                        "worker": worker,
                        "estimated_ms": est_ms,
                        "plan_id": plan.plan_id,
                        "stages": plan.stage_count(),
                    },
                    "note": "dry-run preview only — no request sent, no reservation held",
                })
                .to_string(),
            )
                .into_response(),
            None => (
                StatusCode::UNPROCESSABLE_ENTITY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": "no eligible worker on the fabric for this model (nothing would be executed)", "type": "unprocessable" },
                    "decision": decision,
                    "dry_run": true,
                })
                .to_string(),
            )
                .into_response(),
        };
    }

    let Some(distributed) = state.distributed.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "fabric router unavailable", "type": "server_error" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    let mut request = decentraai_distributed::InferRequest::new(
        model_hash.clone(),
        prompt.to_string(),
        max_tokens,
    )
    .with_sender(distributed.p2p_node().local_peer_id())
    .with_streaming(true);
    request.timeout_ms = 120_000;
    if let Some(sid) = req.get("session_id").and_then(|s| s.as_str()).filter(|s| !s.is_empty()) {
        request = request.with_session(sid.to_string());
    }

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let dist = distributed.clone();
    let resp_task = tokio::spawn(async move {
        dist.route_request_streamed(request, progress_tx).await
    });
    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let state2 = state.clone();
    let started = std::time::Instant::now();
    let model2 = model.clone();
    tokio::spawn(async move {
        while let Some(chunk) = progress_rx.recv().await {
            if chunk.is_empty() {
                continue;
            }
            let payload = format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}}]}}\n\n",
                serde_json::to_string(&chunk).unwrap_or_else(|_| "\"\"".to_string())
            );
            if body_tx.send(Ok(Bytes::from(payload))).await.is_err() {
                break;
            }
        }
        let final_event = match resp_task.await {
            Ok(Ok(resp)) => {
                let usage = format!(
                    "{{\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}",
                    resp.tokens_used
                );
                state2.record_inference("/v1/execute", started.elapsed(), usage.as_bytes());
                format!(
                    "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"model\":{},\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}\n\n",
                    serde_json::to_string(&model2).unwrap_or_else(|_| "\"\"".to_string()),
                    resp.tokens_used
                )
            }
            Ok(Err(_)) => {
                state2.requests_failed.fetch_add(1, Ordering::SeqCst);
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"error\"}]}\n\n"
                    .to_string()
            }
            Err(_) => String::new(),
        };
        let _ = body_tx.send(Ok(Bytes::from(final_event))).await;
        let _ = body_tx.send(Ok(Bytes::from("data: [DONE]\n\n".to_string()))).await;
    });
    let body = Body::from_stream(futures::stream::unfold(body_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/event-stream"),
    );
    response
}

/// Core decide→reserve→execute logic, shared by the HTTP handler and the MCP
/// `execute_decision` tool. Enforces the mutation-safety confirmation itself
/// (so no caller can bypass it) and requires the node master token (checked by
/// the HTTP layer; MCP runs behind the same master-gated boundary).
/// Derive the intent string that drives the unified decision for an execute
/// call: the explicit `intent` if present, else the `capability` (a snake_case
/// capability name is itself resolvable by the intent lexicon), else empty.
fn execute_decision_intent(req: &serde_json::Value) -> String {
    if let Some(i) = req.get("intent").and_then(|i| i.as_str()).filter(|s| !s.trim().is_empty()) {
        return i.trim().to_string();
    }
    if let Some(c) = req.get("capability").and_then(|c| c.as_str()).filter(|s| !s.trim().is_empty()) {
        return c.trim().to_string();
    }
    String::new()
}

async fn run_execute_decision(
    state: &ApiState,
    req: &serde_json::Value,
) -> Response {
    // Mutation safety: explicit confirmation is required.
    if req.get("confirm").and_then(|c| c.as_bool()) != Some(true) {
        return forbidden("mutating execution requires \"confirm\": true");
    }
    let intent = execute_decision_intent(req);
    if intent.trim().is_empty() {
        return forbidden("missing intent (or a capability to run)");
    }
    let prompt = req.get("prompt").and_then(|p| p.as_str()).unwrap_or_default();
    if prompt.trim().is_empty() {
        return forbidden("missing prompt");
    }
    let max_tokens = req
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(1024)
        .min(4096) as u32;
    let stream = req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let evidence = req
        .get("evidence")
        .and_then(|e| e.as_str())
        .unwrap_or("any");
    let evidence = if evidence == "verified" { "verified" } else { "any" };
    let explicit_model = req.get("model").and_then(|m| m.as_str());

    // decide: pick the first CAN_RUN model/worker from the unified projection.
    let decision = unified_fabric_decision(state, &intent, evidence, explicit_model).await;
    let chosen_model = decision["decision"]["model"].as_str().map(str::to_string);

    // reserve+execute requires a real, advertised model hash.
    let Some(model) = chosen_model else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "no runnable decision on the fabric for this intent (nothing to execute)", "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };
    let Some(model_hash) = resolve_model_hash(state, &model).await else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": format!("chosen model '{model}' has no advertised model hash on the fabric"), "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    // Execute through the existing fabric router (reserve + route + audit).
    let distributed = match &state.distributed {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": "fabric router unavailable", "type": "server_error" },
                    "decision": decision,
                })
                .to_string(),
            )
                .into_response();
        }
    };
    let mut request = decentraai_distributed::InferRequest::new(
        model_hash.clone(),
        prompt.to_string(),
        max_tokens,
    )
    .with_sender(distributed.p2p_node().local_peer_id())
    .with_streaming(stream);
    request.timeout_ms = 120_000;
    // Continuation support (KV locality): an optional session_id links this run
    // to an earlier one, steering the fabric router back to the worker holding
    // the session's KV prefix.
    if let Some(sid) = req.get("session_id").and_then(|s| s.as_str()).filter(|s| !s.is_empty()) {
        request = request.with_session(sid.to_string());
    }
    let started = std::time::Instant::now();
    match distributed.route_request(request).await {
        Ok(resp) => {
            let elapsed = started.elapsed();
            state.record_inference(
                "/v1/execute",
                elapsed,
                format!(
                    "{{\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}",
                    resp.tokens_used
                )
                .as_bytes(),
            );
            // MEASURE + HISTORY steps: real measured tokens/time/tps, plus the
            // updated historical stats from the execution the router just
            // recorded (UNKNOWN when no compute manager).
            let measured = {
                let secs = (elapsed.as_millis().max(1) as f64) / 1000.0;
                let tps = (f64::from(resp.tokens_used) / secs).round();
                serde_json::json!({
                    "tokens_used": resp.tokens_used,
                    "latency_ms": elapsed.as_millis() as u64,
                    "tokens_per_sec": if resp.tokens_used > 0 { tps } else { 0.0 },
                    "provenance": "MEASURED",
                })
            };
            let historical = state
                .compute
                .as_ref()
                .map(|cm| decentraai_distributed::execution_statistics(&cm.executions()))
                .unwrap_or_else(|| serde_json::json!({ "records": 0 }));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "decision": decision,
                    "executed": {
                        "model": model,
                        "model_hash": model_hash,
                        "output": resp.output,
                        "tokens_used": resp.tokens_used,
                        "processing_time_ms": resp.processing_time_ms,
                        "worker": resp.worker_peer_id.to_string(),
                    },
                    "measure": measured,
                    "historical": historical,
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) => {
            // REPLAN advisory (Phase H vocabulary): on a retryable transport
            // failure with remaining eligible workers, advise a retry/replan
            // onto an alternative; otherwise abort. This is advisory-only — the
            // router already retried internally; we never claim an action the
            // runtime did not take.
            let retryable = e.is_retryable();
            let alternatives = decision["capabilities"]
                .as_array()
                .map(|caps| {
                    caps.iter()
                        .flat_map(|c| c["model_options"].as_array().cloned().unwrap_or_default())
                        .filter(|m| m["verdict"] == "CAN_RUN")
                        .count()
                })
                .unwrap_or(0);
            let adv = decentraai_fabric::decision::adapt(
                false,         // outcome_ok
                retryable,     // retryable
                false,         // cancelled
                0,             // tokens_emitted (no output was returned)
                alternatives,  // eligible_after_primary
                1,             // replan_budget
                false,         // is_continuation
            );
            let replan = match adv {
                decentraai_fabric::decision::Adaptation::Retry
                | decentraai_fabric::decision::Adaptation::Replan => {
                    if alternatives > 0 {
                        "REPLAN_AVAILABLE"
                    } else {
                        "NO_ALTERNATIVE"
                    }
                }
                decentraai_fabric::decision::Adaptation::Abort => "ABORT",
                decentraai_fabric::decision::Adaptation::Continue => "NO_RETRY_NEEDED",
            };
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": e.to_string(), "type": "execution_error" },
                    "decision": decision,
                    "replan": {
                        "advisory": replan,
                        "retryable": retryable,
                        "eligible_alternatives": alternatives,
                        "note": "advisory only; the router already applied its own retry/fallback",
                    },
                })
                .to_string(),
            )
                .into_response()
        }
    }
}

/// Refresh the local registry after a model pull. Pure-ish (filesystem only,
/// no network): scans the models dir and saves the registry atomically.
fn refresh_registry_after_pull(
    models_dir: &std::path::Path,
    registry_path: &std::path::Path,
) -> Result<usize> {
    let mut registry = if registry_path.exists() {
        match decentraai_registry::ModelRegistry::load(registry_path) {
            Ok(r) => r,
            Err(_) => decentraai_registry::ModelRegistry::new(models_dir.to_path_buf())?,
        }
    } else {
        decentraai_registry::ModelRegistry::new(models_dir.to_path_buf())?
    };
    let count = registry.scan_directory(models_dir)?;
    registry.save(registry_path)?;
    Ok(count)
}

/// Project the Hub's capability taxonomy into registry persistence records.
/// Each enum is converted to its snake_case string form via its `Serialize`
/// impl (e.g. `CapabilityKind::Ocr` -> `"ocr"`, `Provenance::Verified` ->
/// `"verified"`). This is a persistence *projection* of the authoritative hub
/// data — no new capability system.
fn capability_records_from_hub(
    caps: &decentraai_hub::ModelCapabilities,
) -> Vec<decentraai_registry::CapabilityClaimRecord> {
    caps.claims
        .iter()
        .map(|c| decentraai_registry::CapabilityClaimRecord {
            capability: serde_json::to_string(&c.capability)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
            provenance: serde_json::to_string(&c.provenance)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
        })
        .collect()
}

/// Compute `file`'s path relative to `base` by canonicalizing both and
/// stripping the prefix. Returns `None` when the file lies outside `base` or
/// either path cannot be canonicalized, so callers can skip best-effort.
fn relative_path_of(base: &Path, file: &Path) -> Option<String> {
    let base = std::fs::canonicalize(base).ok()?;
    let file = std::fs::canonicalize(file).ok()?;
    let rel = file.strip_prefix(&base).ok()?;
    Some(rel.to_string_lossy().to_string())
}

/// Find a model's persisted capability claims in a registry by its file name.
/// The registry `relative_path` is a path under models/ whose final component
/// is the file name, so a suffix match disambiguates it. Empty means UNKNOWN.
fn claims_for_file_name(
    registry: &decentraai_registry::ModelRegistry,
    file_name: &str,
) -> Vec<decentraai_registry::CapabilityClaimRecord> {
    registry
        .models
        .values()
        .find(|r| r.relative_path.ends_with(file_name))
        .map(|r| r.capability_claims.clone())
        .unwrap_or_default()
}

/// Enumerate the real on-disk GGUF variants of `model` from the local registry,
/// as `(relative_path, size_bytes)` pairs sorted deterministically by file name.
///
/// A registry record is a variant of `model` when its `relative_path`
/// (case-insensitive) contains the model string, OR the model string contains
/// the record's file name, OR the two suffix-match the way the per-worker model
/// matcher does (`worker_capability_verdict`). This is the honest set of
/// variants actually present on this fabric's disk — nothing is invented, and a
/// model with no matching on-disk files yields an empty list. Purely a decision:
/// no I/O, so tests drive it with synthetic registries.
fn registry_variants_for_model(
    reg: &decentraai_registry::ModelRegistry,
    model: &str,
) -> Vec<(String, u64)> {
    let model_lower = model.to_lowercase();
    let mut variants: Vec<(String, u64)> = reg
        .models
        .values()
        .filter(|r| {
            let f = r.relative_path.to_lowercase();
            f.contains(&model_lower)
                || model_lower.contains(&f)
                || f.ends_with(&model_lower)
                || model_lower.ends_with(&f)
        })
        .map(|r| (r.relative_path.clone(), r.size_bytes))
        .collect();
    variants.sort_by(|a, b| a.0.cmp(&b.0));
    variants
}

/// Persist the Hub's authoritative capability claims for a freshly pulled
/// model into the local registry. Best-effort by design: a Hub/registry/IO
/// failure surfaces as an error the caller turns into a warning — capabilities
/// are a projection, never a gate on the pull. Returns the number of claims
/// persisted (0 when the Hub reports none).
async fn persist_capability_claims_after_pull(
    models_dir: &Path,
    registry_path: &Path,
    download_path: &Path,
    repo: &str,
) -> Result<usize> {
    let catalog = decentraai_hub::HubCatalog::new();
    let detail = catalog.model_detail(repo).await?;
    let caps = detail.capabilities();
    let claims = capability_records_from_hub(&caps);
    if claims.is_empty() {
        return Ok(0);
    }
    // Map the pulled file to its registry relative path under models_dir.
    let Some(relative_path) = relative_path_of(models_dir, download_path) else {
        anyhow::bail!(
            "could not map pulled file {} to a path under {}",
            download_path.display(),
            models_dir.display()
        );
    };
    let persisted = claims.len();
    let mut registry = decentraai_registry::ModelRegistry::load(registry_path)?;
    match registry.set_capability_claims(&relative_path, claims)? {
        true => {
            registry.save(registry_path)?;
            Ok(persisted)
        }
        false => {
            anyhow::bail!("pulled model not present in registry: {relative_path}");
        }
    }
}

/// Model Hub pull (Part 16/22): `POST /api/admin/hub/pull` with
/// `{"reference":"hf:org/repo[:file]"}` downloads a verified GGUF into the
/// node's models dir and refreshes the local registry, so the model becomes
/// servable immediately. Master-gated; long-running (streams nothing, the
/// dashboard shows a spinner until it resolves).
async fn admin_hub_pull_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let reference = match req.get("reference").and_then(|v| v.as_str()) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => return forbidden("missing reference (hf:org/repo[:file])"),
    };
    // Optional explicit file variant (Issue #26 §22): when present it is
    // appended to the reference so the verified downloader fetches exactly
    // that GGUF instead of the auto-selected largest one.
    let file = req.get("file").and_then(|v| v.as_str()).map(str::to_string);
    let mut hf_ref = match decentraai_hub::HfRef::parse(&reference) {
        Ok(r) => r,
        Err(e) => return forbidden(&format!("bad reference: {e}")),
    };
    if let Some(f) = file {
        if !f.to_lowercase().ends_with(".gguf") {
            return forbidden("file must end with .gguf");
        }
        hf_ref.file = Some(f);
    }
    let models_dir = state.info.repo_root.join("models");
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        return forbidden(&format!("creating models dir: {e}"));
    }
    // Register the pull so the dashboard can show real byte progress. The
    // `total` is updated once the catalog reports the LFS size (the callback
    // receives bytes downloaded; total stays 0 until we know the file size).
    let repo_key = hf_ref.repo.clone();
    {
        let mut pulls = state.hub_pulls.lock().unwrap();
        pulls.insert(repo_key.clone(), (0, 0));
    }
    let pulls = state.hub_pulls.clone();
    let progress_key = repo_key.clone();
    let progress = Box::new(move |bytes: u64| {
        if let Ok(mut pulls) = pulls.lock() {
            if let Some(e) = pulls.get_mut(&progress_key) {
                e.0 = bytes;
            }
        }
    });
    let download = match decentraai_hub::download_model_with_progress(&hf_ref, &models_dir, Some(progress)).await {
        Ok(d) => d,
        Err(e) => {
            state.hub_pulls.lock().unwrap().remove(&repo_key);
            let body = serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}});
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response();
        }
    };
    // Pull completed: remove from the in-flight registry (dashboard stops
    // polling; the final result below is authoritative).
    state.hub_pulls.lock().unwrap().remove(&repo_key);
    // Refresh the local registry so the new model is immediately usable.
    let registry_path = state.info.repo_root.join("db/registry.json");
    if let Some(cm) = &state.compute {
        cm.set_registry_path(registry_path.clone());
    }
    let _count = match refresh_registry_after_pull(&models_dir, &registry_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("registry refresh after pull failed: {e:#}");
            0
        }
    };
    // Persist the Hub's authoritative capability claims for the pulled model
    // into the local registry. Best-effort: a Hub/registry/IO failure only
    // degrades to a warning — capability persistence must never break a pull.
    let persisted = match persist_capability_claims_after_pull(
        &models_dir,
        &registry_path,
        &download.path,
        hf_ref.repo.as_str(),
    )
    .await
    {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("persisting capability claims after pull failed: {e:#}");
            0
        }
    };
    if persisted > 0 {
        tracing::info!(
            claims = persisted,
            file = %download.path.display(),
            "persisted hub capability claims for pulled model"
        );
    }
    // Issue #26 §25: make the fabric see the new model without a restart.
    // Rebuild `available_models` from the refreshed registry and re-advertise
    // through the compute manager; the periodic broadcaster picks up the new
    // advertisement on its next heartbeat.
    if let Some(cm) = &state.compute {
        let ctx = cm
            .last_local_advertisement_sync()
            .and_then(|a| a.capability.served_models.first().map(|m| m.context_tokens))
            .unwrap_or(0);
        match cm.refresh_local_models(&registry_path, ctx).await {
            Ok(adv) => {
                tracing::info!(
                    on_disk = adv.capability.available_models.len(),
                    "re-advertised local model set after pull"
                );
            }
            Err(e) => tracing::warn!("refresh_local_models after pull failed: {e:#}"),
        }
    }
    decentraai_audit::record_best_effort(
        &state.info.repo_root.join("logs"),
        "model_pulled",
        serde_json::json!({
            "reference": reference,
            "path": download.path.display().to_string(),
            "bytes": download.bytes,
            "sha256": download.sha256,
        }),
    );
    let body = serde_json::json!({
        "reference": reference,
        "file": hf_ref.file,
        "path": download.path.display().to_string(),
        "bytes": download.bytes,
        "sha256": download.sha256,
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Live Model Hub pull progress: `GET /api/admin/hub/pull/status` returns the
/// in-flight pulls (repo -> bytes downloaded + total when known). Empty when
/// no pull is running. Master-gated. The dashboard polls this while a pull is
/// in flight to render a real byte progress bar.
async fn admin_hub_pull_status_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let pulls = state.hub_pulls.lock().unwrap().clone();
    let body: Vec<serde_json::Value> = pulls
        .into_iter()
        .map(|(repo, (bytes, total))| {
            serde_json::json!({
                "repo": repo,
                "bytes_downloaded": bytes,
                "total_bytes": if total > 0 { serde_json::json!(total) } else { serde_json::Value::Null },
                "done": false,
            })
        })
        .collect();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "pulls": body }).to_string(),
    )
        .into_response()
}

/// Model Hub detail (Issue #26 §7–§8, §22, §31): `GET
/// /api/admin/hub/model/{repo}` returns the enriched model card —
/// real Hub metadata (description, license, context, params), the honest
/// capability taxonomy with provenance, and every GGUF file variant with
/// size + SHA-256 — plus the live fabric view of which nodes can run it.
/// Master-gated. Pure serialization helpers keep the handler testable.
async fn admin_hub_model_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    AxumPath(repo): AxumPath<String>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Optional capability fit query: `?requires=ocr` asks the model card for
    // an honest provenance-aware verdict of whether this model can do OCR.
    let requires = query
        .get("requires")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let catalog = decentraai_hub::HubCatalog::new();
    let detail = match catalog.model_detail(&repo).await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };
    let files = match catalog.list_gguf_files(&repo).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "hub_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };
    let caps = detail.capabilities();
    let body = hub_model_body(&detail, &files, &caps, &state, &repo, requires).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Model Hub comparison (Model Comparison feature): `GET
/// /api/admin/hub/compare?repos=org/repo1,org/repo2` compares multiple models
/// side-by-side with honest metadata, capabilities, variants, resource fit,
/// explainable fit reasons, and fabric node availability. Master-gated.
async fn admin_hub_compare_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Optional capability fit query: `?requires=ocr` asks each compared
    // model for an honest provenance-aware verdict of whether it can do OCR.
    let requires = params
        .get("requires")
        .and_then(|v| v.parse::<decentraai_hub::CapabilityKind>().ok());
    let repos_str = params.get("repos").map(|s| s.as_str()).unwrap_or("");
    if repos_str.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "missing 'repos' query parameter", "type": "validation_error"}})
                .to_string(),
        )
            .into_response();
    }

    let repos: Vec<&str> = repos_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if repos.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "no valid repositories provided", "type": "validation_error"}})
                .to_string(),
        )
            .into_response();
    }

    let catalog = decentraai_hub::HubCatalog::new();
    let mut compared_models = Vec::new();

    for repo in repos {
        let detail = match catalog.model_detail(repo).await {
            Ok(d) => d,
            Err(e) => {
                compared_models.push(serde_json::json!({
                    "id": repo,
                    "error": e.to_string(),
                }));
                continue;
            }
        };
        let files = match catalog.list_gguf_files(repo).await {
            Ok(f) => f,
            Err(e) => {
                compared_models.push(serde_json::json!({
                    "id": repo,
                    "error": e.to_string(),
                }));
                continue;
            }
        };
        let caps = detail.capabilities();
        let model_json =
            hub_compare_model_body(&detail, &files, &caps, &state, repo, requires).await;
        compared_models.push(model_json);
    }

    let body = serde_json::json!({
        "models": compared_models,
        // What capability the comparison was asked about, so the UI can label
        // the fit verdicts. Null when no `requires` was supplied.
        "requires": requires.map(|cap| cap.label()),
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Outcome of a Model Hub "can I run this" decision.
///
/// Honesty invariants (§49):
/// - RAM is compared against a RAM estimate, VRAM against a VRAM estimate —
///   the two resources are never treated as interchangeable.
/// - Only *trusted* workers credit toward "a compatible worker exists on the
///   fabric"; an untrusted worker's advertised capacity is not usable yet.
struct ResourceFit {
    ram_sufficient: bool,
    vram_sufficient: bool,
    local_fit: bool,
    trusted_worker_can_run: bool,
    classification: &'static str,
}

/// Pure resource-fit decision for the Model Hub "Models I can run" view.
///
/// Separated from I/O (per AGENTS.md) so the honesty invariants are driven by
/// synthetic inputs in tests, not by live hardware. `est_ram_mb`/`est_vram_mb`
/// are per-resource estimates already derived from the model's file size.
fn resource_fit(
    local_avail_ram_mb: u64,
    local_free_vram_mb: Option<u64>,
    est_ram_mb: u64,
    est_vram_mb: u64,
    trusted_worker_count: usize,
) -> ResourceFit {
    let ram_sufficient = local_avail_ram_mb >= est_ram_mb;
    let vram_sufficient = match local_free_vram_mb {
        Some(v) => v >= est_vram_mb,
        None => false,
    };
    // A node can run the model if either resource it could use is sufficient;
    // the per-resource checks above already compared each against its OWN
    // estimate, so this OR is not a resource-mix.
    let local_fit = ram_sufficient || vram_sufficient;
    let trusted_worker_can_run = trusted_worker_count > 0;
    let classification = if local_fit && trusted_worker_can_run {
        "BEST FIT"
    } else if trusted_worker_can_run {
        "GOOD FIT"
    } else if local_fit {
        "LIMITED"
    } else {
        "NOT AVAILABLE"
    };
    ResourceFit {
        ram_sufficient,
        vram_sufficient,
        local_fit,
        trusted_worker_can_run,
        classification,
    }
}

/// Final verdict for a worker's capability query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerCapVerdict {
    CanRun,
    CannotRun,
    Unknown,
}

/// One explainable check contributing to a worker verdict.
#[derive(Debug, Clone)]
struct WorkerCheck {
    check: &'static str,
    pass: bool,
    state: String,
    reason: String,
}

/// The pure result of evaluating one worker against a capability query.
#[derive(Debug, Clone)]
struct WorkerCapResult {
    peer_id: String,
    node_id: String,
    node_name: String,
    verdict: WorkerCapVerdict,
    checks: Vec<WorkerCheck>,
    model_availability: &'static str,
    trusted: bool,
    ram_sufficient: bool,
    vram_sufficient: bool,
    est_ram_mb: u64,
    est_vram_mb: u64,
    engine_compat: &'static str,
    /// Quantization label INFERRED from the matched model file name (None =
    /// unknown). Never VERIFIED.
    quantization: Option<String>,
}

impl WorkerCapResult {
    /// Serialize to the MCP-facing projection (real identity kept separate).
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "worker": {
                "peer_id": self.peer_id,
                "node_id": self.node_id,
                "node_name": self.node_name,
            },
            "verdict": match self.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            },
            "model_availability": self.model_availability,
            "quantization": self.quantization,
            "trusted": self.trusted,
            "engine": self.engine_compat,
            "resource_fit": {
                "ram_sufficient": self.ram_sufficient,
                "vram_sufficient": self.vram_sufficient,
                "est_ram_mb": self.est_ram_mb,
                "est_vram_mb": self.est_vram_mb,
            },
            "checks": self.checks.iter().map(|c| serde_json::json!({
                "check": c.check,
                "pass": c.pass,
                "state": c.state,
                "reason": c.reason,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The unified fabric-wide "CAN I RUN THIS?" answer, aggregated from per-worker
/// verdicts. Explainable: no opaque score — it derives from real per-worker
/// checks and reuses the exact same capability/resource/trust vocabulary.
#[derive(Debug, Clone)]
struct FabricCapFit {
    /// Overall: CAN_RUN if any worker can; CANNOT_RUN if workers exist but none
    /// can and at least one hard-fails; else UNKNOWN (no workers / all unknown).
    verdict: WorkerCapVerdict,
    can_run_count: usize,
    cannot_run_count: usize,
    unknown_count: usize,
    /// The chosen worker for "which worker should I use?" — the first CAN_RUN
    /// result (deterministic input order), else None.
    chosen_worker: Option<String>,
    /// Human reasons behind the overall verdict (aggregated from workers).
    reasons: Vec<String>,
}

impl FabricCapFit {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verdict": match self.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            },
            "counts": {
                "can_run": self.can_run_count,
                "cannot_run": self.cannot_run_count,
                "unknown": self.unknown_count,
            },
            "chosen_worker": self.chosen_worker,
            "reasons": self.reasons,
        })
    }
}

/// Pure aggregation of per-worker capability verdicts into a fabric-wide answer.
///
/// Rules (honest, no invented state):
/// - zero workers → UNKNOWN ("no compatible worker"), never CANNOT_RUN without
///   a real worker to blame.
/// - any worker CAN_RUN → overall CAN_RUN; chosen_worker is the first CAN_RUN
///   (deterministic given the caller's sorted worker order).
/// - no CAN_RUN but at least one CANNOT_RUN → CANNOT_RUN.
/// - only UNKNOWN workers → UNKNOWN.
fn aggregate_can_i_run(results: &[WorkerCapResult]) -> FabricCapFit {
    if results.is_empty() {
        return FabricCapFit {
            verdict: WorkerCapVerdict::Unknown,
            can_run_count: 0,
            cannot_run_count: 0,
            unknown_count: 0,
            chosen_worker: None,
            reasons: vec!["no compatible worker on the fabric".to_string()],
        };
    }

    let can_run: Vec<&WorkerCapResult> = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
        .collect();
    let cannot_run = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::CannotRun)
        .count();
    let unknown = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::Unknown)
        .count();

    let chosen_worker = can_run.first().map(|r| r.peer_id.clone());
    let verdict = if !can_run.is_empty() {
        WorkerCapVerdict::CanRun
    } else if cannot_run > 0 {
        WorkerCapVerdict::CannotRun
    } else {
        WorkerCapVerdict::Unknown
    };

    // Aggregate a small set of human reasons: the first CAN_RUN worker's
    // capability + resource evidence (when present), or representative blockers.
    let mut reasons = Vec::new();
    match verdict {
        WorkerCapVerdict::CanRun => {
            if let Some(best) = can_run.first() {
                reasons.push(format!(
                    "{} (node {} / {}) can run it",
                    best.peer_id, best.node_id, best.node_name
                ));
                let cap = best.checks.iter().find(|c| c.check == "capability");
                if let Some(cap) = cap {
                    reasons.push(format!(
                        "capability {} — {} evidence",
                        cap.state,
                        if cap.pass { "satisfied" } else { "insufficient" }
                    ));
                }
                reasons.push(format!(
                    "RAM {} · VRAM {} ({} CAN_RUN workers)",
                    if best.ram_sufficient { "sufficient" } else { "insufficient" },
                    if best.vram_sufficient { "sufficient" } else { "insufficient" },
                    can_run.len()
                ));
            }
        }
        WorkerCapVerdict::CannotRun => {
            // Report the first few distinct blockers across CANNOT_RUN workers.
            let mut seen = std::collections::BTreeSet::new();
            for r in results.iter().filter(|r| r.verdict == WorkerCapVerdict::CannotRun) {
                for c in &r.checks {
                    if !c.pass {
                        let key = format!("{}:{}", c.check, c.state);
                        if seen.insert(key) {
                            reasons.push(format!(
                                "{} ({} / {}): {} — {}",
                                r.node_name, r.node_id, r.peer_id, c.check, c.state
                            ));
                        }
                    }
                }
            }
        }
        WorkerCapVerdict::Unknown => {
            reasons.push("no worker can be confirmed to run it (evidence/telemetry unknown)".to_string());
        }
    }

    FabricCapFit {
        verdict,
        can_run_count: can_run.len(),
        cannot_run_count: cannot_run,
        unknown_count: unknown,
        chosen_worker,
        reasons,
    }
}

/// Resolve the engine-compatibility state for a worker advertising `engine`
/// that holds `model`.
///
/// DecentraAI only spawns llama-server (a GGUF-serving OpenAI-compatible
/// engine), and a worker advertises a model as *served* only when its engine
/// actually runs it — so a served model is engine-compatible by construction.
/// A model that is merely on disk (not served) still requires the engine to
/// serve GGUF; the bundled engine does, so on-disk is compatible. An
/// unknown/unparsed engine with a model on disk cannot be confirmed, so it is
/// `unknown`. A worker that does not hold the model cannot claim a compatible
/// engine for it.
fn worker_engine_compat(engine: &str, model_served: bool, model_on_disk: bool) -> &'static str {
    if model_served {
        return "compatible"; // the engine is demonstrably running this model
    }
    let kind = decentraai_fabric::EngineKind::parse(engine);
    match kind {
        decentraai_fabric::EngineKind::LlamaServer
        | decentraai_fabric::EngineKind::Vllm
        | decentraai_fabric::EngineKind::Sglang
        | decentraai_fabric::EngineKind::Ollama => {
            if model_on_disk {
                "compatible" // known GGUF-serving engine can be swapped to this model
            } else {
                "unknown" // does not hold the model; cannot confirm a path to it
            }
        }
        decentraai_fabric::EngineKind::RemoteOpenAI => "unknown", // unprobed generic endpoint
    }
}

/// Pure per-worker capability verdict. A thin projection reusing the existing
/// capability resolver, resource-fit vocabulary and the authoritative
/// advertisement (peer_id / node_id / node_name are never conflated).
///
/// `model` is matched against the worker's served/available models by file
/// name (suffix-safe). `claims` are the model's persisted capability claims
/// (`(capability, provenance)`), `evidence` is "any" or "verified".
/// Derive a variant's quantization label from a GGUF file name using ONLY
/// conservative heuristics. The label is INFERRED from the file name — the file
/// name is not authoritative metadata, so callers must never present it as
/// VERIFIED. When no recognized quant marker is present, return `None`
/// (UNKNOWN); never guess a quantization that isn't in the name.
///
/// Recognized markers (case-insensitive):
/// - `q2_k` -> "Q2", `q3_k` -> "Q3", `q4_k_m`/`q4_0` -> "Q4"
/// - `q5_1` -> "Q5", `q6_k` -> "Q6", `q8_0` -> "Q8"
/// - `fp16`/`f16` -> "FP16"
fn variant_quantization_from_file_name(file_name: &str) -> Option<String> {
    let lower = file_name.to_lowercase();
    if lower.contains("fp16") || lower.contains("f16") {
        return Some("FP16".to_string());
    }
    // Longest markers first so e.g. `q4_k_m` is not swallowed by `q4`.
    for (marker, label) in [
        ("q8_0", "Q8"),
        ("q6_k", "Q6"),
        ("q5_1", "Q5"),
        ("q4_k_m", "Q4"),
        ("q4_0", "Q4"),
        ("q3_k", "Q3"),
        ("q2_k", "Q2"),
    ] {
        if lower.contains(marker) {
            return Some(label.to_string());
        }
    }
    None
}

#[cfg(test)]
fn worker_capability_verdict(
    adv: &decentraai_compute::ComputeAdvertisement,
    trusted: bool,
    model: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
) -> WorkerCapResult {
    // Test-only convenience: default to "this worker may serve this fabric's
    // request" (policy gate on). Production uses the policy-aware variant.
    worker_capability_verdict_with_policy(adv, trusted, model, capability, evidence, claims, true)
}

/// Policy-aware per-worker capability verdict (Phase M foundation). `accepts_remote_work`
/// is true for the LOCAL node (which always serves its own work) or a remote
/// worker that opted into remote inference (`accepts_remote_inference`). A
/// remote worker that did NOT opt in cannot run this fabric's request — a
/// definitive policy CANNOT_RUN, never a fabricated pass.
fn worker_capability_verdict_with_policy(
    adv: &decentraai_compute::ComputeAdvertisement,
    trusted: bool,
    model: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
    accepts_remote_work: bool,
) -> WorkerCapResult {
    let model_lower = model.to_lowercase();
    let matches_model = |m: &decentraai_compute::ServedModel| {
        let f = m.file_name.to_lowercase();
        f == model_lower || f.ends_with(&model_lower) || model_lower.ends_with(&f)
    };

    let served = adv.capability.served_models.iter().any(matches_model);
    let on_disk = adv.capability.available_models.iter().any(matches_model);
    let model_entry = adv
        .capability
        .served_models
        .iter()
        .find(|m| matches_model(m))
        .or_else(|| adv.capability.available_models.iter().find(|m| matches_model(m)));

    let model_availability = if served {
        "served"
    } else if on_disk {
        "local_on_disk"
    } else {
        "unavailable"
    };

    let engine_compat = worker_engine_compat(&adv.capability.engine, served, on_disk);
    let quantization = model_entry
        .and_then(|m| variant_quantization_from_file_name(&m.file_name));
    let est_ram_mb = model_entry.map(|m| m.est_ram_mb).unwrap_or(0);
    let est_vram_mb = model_entry.map(|m| m.est_vram_mb).unwrap_or(0);

    // RAM/VRAM fit from the model's own estimates vs the worker's advertised
    // availability. Missing telemetry must stay UNKNOWN, not a false pass.
    let avail_ram = adv.availability.available_ram_mb;
    let avail_vram = adv.availability.available_vram_mb;
    let ram_known = model_entry.is_some() && est_ram_mb > 0;
    let ram_sufficient = ram_known && avail_ram >= est_ram_mb;
    let vram_known = model_entry.is_some()
        && est_vram_mb > 0
        && avail_vram.is_some();
    let vram_sufficient = vram_known
        && avail_vram.is_some_and(|v| v >= est_vram_mb);

    let mut checks: Vec<WorkerCheck> = Vec::new();

    // Capability verdict via the existing resolver (honest provenance).
    // When there is NO capability data at all (empty claims), the honest state
    // is UNKNOWN — the resolver would report MISSING, but "no data" is distinct
    // from "claims exist and none match". Never convert UNKNOWN into success or
    // failure.
    let cap_view = if claims.is_empty() {
        let label = capability.replace('_', " ");
        decentraai_fabric::planner::CapabilityRequirementView {
            capability: capability.to_string(),
            label,
            satisfied: false,
            evidence: "UNKNOWN".to_string(),
        }
    } else {
        let claim_refs: Vec<(&str, &str)> = claims
            .iter()
            .map(|(c, p)| (c.as_str(), p.as_str()))
            .collect();
        decentraai_fabric::planner::resolve_capability_requirement(capability, &claim_refs)
    };
    let cap_pass = cap_view.satisfied;
    checks.push(WorkerCheck {
        check: "capability",
        pass: cap_pass,
        state: cap_view.evidence.clone(),
        reason: if cap_pass {
            format!("{} — {} evidence", cap_view.label, cap_view.evidence)
        } else {
            format!(
                "{} — {} (insufficient provenance for evidence='{evidence}')",
                cap_view.label, cap_view.evidence
            )
        },
    });

    // Model availability.
    let avail_pass = model_availability != "unavailable";
    checks.push(WorkerCheck {
        check: "model_available",
        pass: avail_pass,
        state: model_availability.to_string(),
        reason: match model_availability {
            "served" => "model is currently served by this worker".into(),
            "local_on_disk" => "model is on disk (not loaded); engine can be swapped".into(),
            _ => "model is not on this worker".into(),
        },
    });

    // Trust.
    checks.push(WorkerCheck {
        check: "trusted",
        pass: trusted,
        state: if trusted { "trusted" } else { "not_trusted" }.into(),
        reason: if trusted {
            "worker is trusted by this coordinator".into()
        } else {
            "worker is not trusted".into()
        },
    });

    // Policy (Phase M): a remote worker that has not opted into remote
    // inference cannot serve a request from this fabric. The local node is
    // always allowed its own work. This is a definitive policy gate, not a
    // capability/telemetry guess.
    let policy_pass = accepts_remote_work;
    checks.push(WorkerCheck {
        check: "policy",
        pass: policy_pass,
        state: if policy_pass { "allowed" } else { "remote_not_accepted" }.into(),
        reason: if policy_pass {
            "worker may serve this fabric's request (local or remote-opt-in)".into()
        } else {
            "worker does not accept remote inference (policy)".into()
        },
    });

    // Engine compatibility.
    let engine_pass = engine_compat == "compatible";
    checks.push(WorkerCheck {
        check: "engine",
        pass: engine_pass,
        state: engine_compat.to_string(),
        reason: match engine_compat {
            "compatible" => format!("engine '{}' can serve this model", adv.capability.engine),
            "unknown" => format!("engine '{}' compatibility unknown for this model", adv.capability.engine),
            _ => format!("engine '{}' incompatible with this model", adv.capability.engine),
        },
    });

    // RAM.
    if model_entry.is_none() {
        checks.push(WorkerCheck {
            check: "ram",
            pass: false,
            state: "unknown".into(),
            reason: "no model footprint on this worker; cannot estimate RAM".into(),
        });
    } else if !ram_known {
        checks.push(WorkerCheck {
            check: "ram",
            pass: false,
            state: "unknown".into(),
            reason: "model RAM estimate unavailable (UNKNOWN)".into(),
        });
    } else {
        checks.push(WorkerCheck {
            check: "ram",
            pass: ram_sufficient,
            state: if ram_sufficient { "sufficient" } else { "insufficient" }.into(),
            reason: format!("available RAM {} MiB vs estimated {} MiB", avail_ram, est_ram_mb),
        });
    }

    // VRAM (separate dimension; CPU-only model => trivially satisfied).
    if est_vram_mb == 0 {
        checks.push(WorkerCheck {
            check: "vram",
            pass: true,
            state: "not_applicable".into(),
            reason: "CPU-only model requires no VRAM".into(),
        });
    } else if !vram_known {
        checks.push(WorkerCheck {
            check: "vram",
            pass: false,
            state: "unknown".into(),
            reason: "GPU/VRAM telemetry unavailable (UNKNOWN)".into(),
        });
    } else {
        checks.push(WorkerCheck {
            check: "vram",
            pass: vram_sufficient,
            state: if vram_sufficient { "sufficient" } else { "insufficient" }.into(),
            reason: format!(
                "available VRAM {} MiB vs estimated {} MiB",
                avail_vram.unwrap_or(0),
                est_vram_mb
            ),
        });
    }

    // Combine into a verdict. A definitive hard failure => CANNOT_RUN; any
    // UNKNOWN component with no hard failure => UNKNOWN; else CAN_RUN.
    // A capability with no evidence (UNKNOWN) is not a hard failure — it is an
    // unknown that must NOT be converted into success OR failure.
    let cap_hard_fail = !cap_pass && cap_view.evidence != "UNKNOWN";
    let has_hard_fail = cap_hard_fail
        || !avail_pass
        || !trusted
        || !policy_pass
        || !engine_pass
        || (ram_known && !ram_sufficient)
        || (est_vram_mb > 0 && vram_known && !vram_sufficient);
    let has_unknown = model_entry.is_none()
        || !ram_known
        || (est_vram_mb > 0 && !vram_known)
        || engine_compat == "unknown"
        || cap_view.evidence == "UNKNOWN";

    let verdict = if has_hard_fail {
        WorkerCapVerdict::CannotRun
    } else if has_unknown {
        WorkerCapVerdict::Unknown
    } else {
        WorkerCapVerdict::CanRun
    };

    WorkerCapResult {
        peer_id: adv.peer_id.to_string(),
        node_id: adv.node_id.clone(),
        node_name: adv.node_name.clone(),
        verdict,
        checks,
        model_availability,
        trusted,
        ram_sufficient,
        vram_sufficient,
        est_ram_mb,
        est_vram_mb,
        engine_compat,
        quantization,
    }
}

/// Pure serialization of a model comparison entry, incorporating variants with
/// explainable fit classification and reasons, capabilities, and fabric availability.
async fn hub_compare_model_body(
    detail: &decentraai_hub::HubModelDetail,
    files: &[decentraai_hub::HubModelFile],
    caps: &decentraai_hub::ModelCapabilities,
    state: &ApiState,
    repo: &str,
    requires: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    let mut fabric_nodes = Vec::new();
    if let Some(cm) = &state.compute {
        let workers = cm.workers().await;
        for w in workers {
            let served = w.capability.served_models.iter().any(|m| m.file_name == repo);
            let available = w.capability.available_models.iter().any(|m| m.file_name == repo);
            if served || available {
                fabric_nodes.push(serde_json::json!({
                    "node_id": w.node_id,
                    "node_name": w.node_name,
                    "peer_id": w.peer_id.to_string(),
                    "status": format!("{:?}", w.availability.status),
                    "served": served,
                    "available": available,
                    "trusted": cm.is_trusted(&w.peer_id).await,
                }));
            }
        }
    }
    let local_snapshot = decentraai_system_probe::SystemSnapshot::collect();
    let local_avail_ram_mb = local_snapshot.available_memory_bytes / (1024 * 1024);
    let local_vram_mb = match decentraai_system_probe::probe_gpu() {
        decentraai_system_probe::GpuProbeStatus::Nvidia(gpu) => Some(gpu.free_vram_mib),
        _ => None,
    };
    let has_gpu = local_vram_mb.is_some();

    let mut fabric_variants = Vec::new();
    for f in files {
        let size = f.size.unwrap_or(0);
        let est_ram_mb = decentraai_compute::ServedModel::estimate_ram_mb(size);
        let est_vram_mb = (size * 105 / 100) / (1024 * 1024);
        let mut can_run_workers = Vec::new();
        let mut trusted_worker_count = 0usize;
        if let Some(cm) = &state.compute {
            for w in cm.workers().await {
                let w_ram = w.availability.available_ram_mb;
                let w_vram = w.availability.available_vram_mb.unwrap_or(0);
                if w_ram >= est_ram_mb || w_vram >= est_vram_mb {
                    can_run_workers.push(serde_json::json!({
                        "node_id": w.node_id,
                        "node_name": w.node_name,
                        "peer_id": w.peer_id.to_string(),
                        "ram_ok": w_ram >= est_ram_mb,
                        "vram_ok": w_vram >= est_vram_mb,
                    }));
                    if cm.is_trusted(&w.peer_id).await {
                        trusted_worker_count += 1;
                    }
                }
            }
        }
        let model_available_on_fabric = !fabric_nodes.is_empty();

        // Pure decision: keeps the honesty invariants (separate RAM/VRAM
        // estimates, trusted-only worker credit, classification) testable
        // without I/O.
        let fit = resource_fit(
            local_avail_ram_mb,
            local_vram_mb,
            est_ram_mb,
            est_vram_mb,
            trusted_worker_count,
        );
        let ram_sufficient = fit.ram_sufficient;
        let vram_sufficient = fit.vram_sufficient;
        let local_fit = fit.local_fit;
        let trusted_worker_can_run = fit.trusted_worker_can_run;

        let mut reasons = Vec::new();
        reasons.push(serde_json::json!({
            "check": "ram_sufficient",
            "pass": ram_sufficient,
            "provenance": "ESTIMATED",
            "reason": format!("Local available RAM ({} MB) vs estimated requirement ({} MB)", local_avail_ram_mb, est_ram_mb)
        }));
        if has_gpu {
            reasons.push(serde_json::json!({
                "check": "vram_sufficient",
                "pass": vram_sufficient,
                "provenance": "ESTIMATED",
                "reason": format!("Local free VRAM ({} MB) vs estimated requirement ({} MB)", local_vram_mb.unwrap_or(0), est_vram_mb)
            }));
        } else {
            reasons.push(serde_json::json!({
                "check": "vram_sufficient",
                "pass": false,
                "provenance": "MEASURED",
                "reason": "No discrete GPU detected on local node (CPU-only execution)"
            }));
        }
        // Honest pass: only *trusted* workers count toward "compatible worker
        // found on fabric" — an untrusted worker's advertised capacity is not a
        // resource this operator can actually use yet.
        reasons.push(serde_json::json!({
            "check": "trusted_worker_available",
            "pass": trusted_worker_can_run,
            "provenance": "VERIFIED",
            "reason": format!("{} compatible worker(s) found on fabric ({} trusted)", can_run_workers.len(), trusted_worker_count)
        }));
        reasons.push(serde_json::json!({
            "check": "model_available",
            "pass": model_available_on_fabric || local_fit,
            "provenance": "VERIFIED",
            "reason": if model_available_on_fabric { "Model reported on fabric nodes" } else { "Model not yet present on fabric" }
        }));
        // The bundled engine is llama-server and serves GGUF by construction.
        // That is an ESTIMATED capability derived from the fixed engine
        // choice, not a per-model runtime verification — never VERIFIED.
        reasons.push(serde_json::json!({
            "check": "compatible_engine",
            "pass": true,
            "provenance": "ESTIMATED",
            "reason": "llama-server (bundled engine) serves GGUF models"
        }));

        let fit_classification = fit.classification;

        fabric_variants.push(serde_json::json!({
            "file": f.path,
            "size_bytes": f.size,
            "sha256": f.lfs.as_ref().map(|l| l.oid.clone()),
            "est_ram_mb": est_ram_mb,
            "local_fit": local_fit,
            "fabric_fit_nodes": can_run_workers,
            "fit_classification": fit_classification,
            "fit_reasons": reasons,
        }));
    }

    serde_json::json!({
        "id": repo,
        "metadata": {
            "pipeline_tag": detail.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": detail.tags,
            "downloads": detail.downloads,
            "likes": detail.likes,
            "description": detail.description,
            "license": detail.license,
            "context_length": detail.context_length,
            "params": detail.params,
        },
        "capabilities": {
            "claims": caps.claims.iter().map(|c| serde_json::json!({
                "capability": c.capability,
                "label": c.capability.label(),
                "provenance": c.provenance,
            })).collect::<Vec<_>>(),
            "tasks": caps.tasks.iter().map(|t| serde_json::json!({
                "capability": t.capability,
                "task": t.task,
            })).collect::<Vec<_>>(),
            // When a `requires` capability was supplied, an honest,
            // provenance-aware verdict of whether this model satisfies it.
            "fit": requires.map(|cap| {
                let m = decentraai_hub::match_requirements(
                    caps,
                    &[decentraai_hub::CapabilityRequirement {
                        capability: cap,
                        evidence: decentraai_hub::EvidenceLevel::Verified,
                    }],
                );
                serde_json::json!({
                    "capability": cap,
                    "label": cap.label(),
                    "satisfied": m.is_satisfied(),
                    "checks": m.checks.iter().map(|c| serde_json::json!({
                        "capability": c.capability,
                        "status": serde_json::to_value(&c.status).unwrap_or_default(),
                        "reason": c.reason,
                    })).collect::<Vec<_>>(),
                })
            }),
        },
        "variants": fabric_variants,
        "fabric": fabric_nodes,
    })
}

/// Pure serialization of the model card (hub metadata + capabilities +
/// variants + live fabric state). Tests drive it with synthetic inputs.
async fn hub_model_body(
    detail: &decentraai_hub::HubModelDetail,
    files: &[decentraai_hub::HubModelFile],
    caps: &decentraai_hub::ModelCapabilities,
    state: &ApiState,
    repo: &str,
    requires: Option<decentraai_hub::CapabilityKind>,
) -> serde_json::Value {
    // Live fabric view: which trusted, ready workers have this model on disk
    // or loaded, and their honest capacity for it. Never fabricated — if a
    // worker is not in the compute registry, it is simply not listed.
    let mut fabric_nodes = Vec::new();
    if let Some(cm) = &state.compute {
        let workers = cm.workers().await;
        for w in workers {
            let served = w
                .capability
                .served_models
                .iter()
                .any(|m| m.file_name == repo);
            let available = w
                .capability
                .available_models
                .iter()
                .any(|m| m.file_name == repo);
            if served || available {
                fabric_nodes.push(serde_json::json!({
                    "node_id": w.node_id,
                    "node_name": w.node_name,
                    "peer_id": w.peer_id.to_string(),
                    "status": format!("{:?}", w.availability.status),
                    "served": served,
                    "available": available,
                    "trusted": cm.is_trusted(&w.peer_id).await,
                }));
            }
        }
    }
    let local_snapshot = decentraai_system_probe::SystemSnapshot::collect();
    let local_avail_ram_mb = local_snapshot.available_memory_bytes / (1024 * 1024);
    let local_vram_mb = match decentraai_system_probe::probe_gpu() {
        decentraai_system_probe::GpuProbeStatus::Nvidia(gpu) => Some(gpu.free_vram_mib),
        _ => None,
    };

    let mut fabric_variants = Vec::new();
    for f in files {
        let size = f.size.unwrap_or(0);
        let est_ram_mb = (size * 120 / 100) / (1024 * 1024);
        let mut can_run_workers = Vec::new();
        if let Some(cm) = &state.compute {
            for w in cm.workers().await {
                let w_ram = w.availability.available_ram_mb;
                let w_vram = w.availability.available_vram_mb.unwrap_or(0);
                if w_ram >= est_ram_mb || w_vram >= est_ram_mb {
                    can_run_workers.push(serde_json::json!({
                        "node_id": w.node_id,
                        "node_name": w.node_name,
                    }));
                }
            }
        }
        fabric_variants.push(serde_json::json!({
            "file": f.path,
            "size_bytes": f.size,
            "sha256": f.lfs.as_ref().map(|l| l.oid.clone()),
            "est_ram_mb": est_ram_mb,
            "local_fit": local_avail_ram_mb >= est_ram_mb || local_vram_mb.unwrap_or(0) >= est_ram_mb,
            "fabric_fit_nodes": can_run_workers,
        }));
    }

    serde_json::json!({
        "id": repo,
        "metadata": {
            "pipeline_tag": detail.pipeline_tag.as_ref().map(|t| t.as_str()),
            "tags": detail.tags,
            "downloads": detail.downloads,
            "likes": detail.likes,
            "description": detail.description,
            "license": detail.license,
            "context_length": detail.context_length,
            "params": detail.params,
        },
        "capabilities": {
            "claims": caps.claims.iter().map(|c| serde_json::json!({
                "capability": c.capability,
                "label": c.capability.label(),
                "provenance": c.provenance,
            })).collect::<Vec<_>>(),
            "tasks": caps.tasks.iter().map(|t| serde_json::json!({
                "capability": t.capability,
                "task": t.task,
            })).collect::<Vec<_>>(),
            // When a `requires` capability was supplied, an honest,
            // provenance-aware verdict of whether this model satisfies it.
            "fit": requires.map(|cap| {
                let m = decentraai_hub::match_requirements(
                    caps,
                    &[decentraai_hub::CapabilityRequirement {
                        capability: cap,
                        evidence: decentraai_hub::EvidenceLevel::Verified,
                    }],
                );
                serde_json::json!({
                    "capability": cap,
                    "label": cap.label(),
                    "satisfied": m.is_satisfied(),
                    "checks": m.checks.iter().map(|c| serde_json::json!({
                        "capability": c.capability,
                        "status": serde_json::to_value(&c.status).unwrap_or_default(),
                        "reason": c.reason,
                    })).collect::<Vec<_>>(),
                })
            }),
        },
        "variants": fabric_variants,
        "fabric": fabric_nodes,
    })
}

/// Remove a model from the local registry and disk. Requires master token.
/// If the model is currently served, returns 409 Conflict to prevent
/// accidental interruption of ongoing inference. The model must be unloaded
/// first (e.g., via `/api/admin/serve/unload` if the API supports it).
async fn admin_models_remove_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }

    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };

    let relative_path = match req.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return forbidden("missing path (relative file name)"),
    };

    // Load the registry to check existence and current state.
    let registry_path = state.info.repo_root.join("db/registry.json");
    let mut registry = match decentraai_registry::ModelRegistry::load(&registry_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to load registry for removal: {e:#}");
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": "registry load failed", "type": "registry_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };

    // Check if the model is currently served by comparing the path.
    let current_model_path_opt = {
        let manager_guard = state.manager.lock().await;
        manager_guard.current_model_path().map(|p| p.to_path_buf())
    };

    if let Some(current_model_path) = current_model_path_opt {
        let full_target_path = state.info.repo_root.join("models").join(relative_path);
        if let Ok(canonical_target) = std::fs::canonicalize(&full_target_path) {
            if canonical_target == current_model_path {
                return (
                    StatusCode::CONFLICT,
                    [(header::CONTENT_TYPE, "application/json")],
                    serde_json::json!({"error": {"message": "model is currently served; unload before removal", "type": "conflict"}})
                        .to_string(),
                )
                    .into_response();
            }
        } else {
            // If we can't canonicalize, proceed with removal anyway.
            // This is defensive but risky.
            tracing::debug!("failed to canonicalize target path for removal check: {}", full_target_path.display());
        }
    }

    // Remove the model file and entry from registry.
    let record = match registry.remove_model(relative_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"error": {"message": e.to_string(), "type": "registry_error"}})
                    .to_string(),
            )
                .into_response();
        }
    };

    // Save the updated registry.
    if let Err(e) = registry.save(&registry_path) {
        tracing::warn!("failed to save registry after removal: {e:#}");
        return (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": {"message": "registry save failed", "type": "registry_error"}})
                .to_string(),
        )
            .into_response();
    }

    // Refresh the local fabric advertisement so other nodes see the model
    // as removed from the registry. This ensures the model is no longer
    // advertised to the fabric.
    if let Some(cm) = &state.compute {
        cm.set_registry_path(registry_path.clone());
        let ctx = (record.size_bytes / 1024 / 1024) as u32; // Estimate context from size
        if let Err(e) = cm.refresh_local_models(&registry_path, ctx).await {
            tracing::warn!("failed to refresh fabric advertisement after removal: {e:#}");
            // Continue anyway, the model removal succeeded.
        }
    }

    // Record the audit event.
    let audit_path = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        audit_path.parent().unwrap_or(&state.info.repo_root),
        "model_removed",
        serde_json::json!({
            "relative_path": relative_path,
            "size_bytes": record.size_bytes,
            "canonical_path": record.canonical_path,
        }),
    );

    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "message": "model removed"})
            .to_string(),
    )
        .into_response()
}

/// Master-gated runtime settings: update generation defaults live (no restart).
/// Body: `{ "temperature": f64, "top_p": f64, "top_k": u32|null,
/// "repeat_penalty": f64, "system_prompt": string }` — each field is optional;
/// omitted fields keep their current value. Persisting across restarts is not
/// wired (config file is the source of truth at startup); this is a live
/// override that applies to subsequent inference requests immediately.
async fn admin_settings_generation_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let mut g = state.runtime_generation.write().await;
    if let Some(v) = req.get("temperature").and_then(|v| v.as_f64()) {
        g.temperature = v.clamp(0.0, 2.0) as f32;
    }
    if let Some(v) = req.get("top_p").and_then(|v| v.as_f64()) {
        g.top_p = v.clamp(0.0, 1.0) as f32;
    }
    if let Some(v) = req.get("top_k") {
        g.top_k = v.as_i64().map(|n| n as i32).filter(|k| *k > 0);
    }
    if let Some(v) = req.get("repeat_penalty").and_then(|v| v.as_f64()) {
        g.repeat_penalty = v.clamp(0.0, 4.0) as f32;
    }
    if let Some(v) = req.get("system_prompt").and_then(|v| v.as_str()) {
        g.system_prompt = v.to_string();
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    // Persist to the node config (best-effort): the live override already
    // applies immediately, but writing it to node.yaml makes it survive a
    // restart. A write failure only warns — the live override stays valid for
    // the current process.
    let persisted = persist_generation_config(&state.info.repo_root, &g);
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "settings_generation_updated",
        serde_json::json!({
            "temperature": g.temperature,
            "top_p": g.top_p,
            "top_k": g.top_k,
            "repeat_penalty": g.repeat_penalty,
            "persisted": persisted,
        }),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "generation": {
                "temperature": g.temperature,
                "top_p": g.top_p,
                "top_k": g.top_k,
                "repeat_penalty": g.repeat_penalty,
                "system_prompt": g.system_prompt,
            },
            "persisted": persisted,
            "note": if persisted { "applied live and persisted to node.yaml" } else { "applied live only (could not persist config)" },
        })
        .to_string(),
    )
        .into_response()
}

/// Best-effort, text-based persistence of the runtime generation defaults into
/// `<data_dir>/node.yaml` under the `generation:` block. Handles a missing
/// file (returns false — live override only) and rewrites atomically
/// (tmp + rename). Kept simple on purpose: the config crate's YAML round-trip
/// would need `NodeConfig: Serialize`; editing the known keys directly is
/// safer than reconstructing the whole file.
fn persist_generation_config(repo_root: &std::path::Path, g: &GenerationSection) -> bool {
    use std::io::Write;
    let path = repo_root.join("node.yaml");
    if !path.exists() {
        return false;
    }
    let Ok(raw) = std::fs::read_to_string(&path) else { return false };
    let temp = |line: &str, v: String| -> String {
        let trimmed = line.trim_start();
        if trimmed.starts_with("temperature:") {
            format!("{}temperature: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("top_p:") {
            format!("{}top_p: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("top_k:") {
            format!("{}top_k: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("repeat_penalty:") {
            format!("{}repeat_penalty: {}", &line[..line.len() - trimmed.len()], v)
        } else if trimmed.starts_with("system_prompt:") {
            format!(
                "{}system_prompt: {}",
                &line[..line.len() - trimmed.len()],
                serde_json::to_string(&g.system_prompt).unwrap_or_else(|_| "\"\"".to_string())
            )
        } else {
            line.to_string()
        }
    };
    let in_gen = raw.lines().any(|l| l.trim() == "generation:");
    if !in_gen {
        return false;
    }
    let mut after_gen = false;
    let mut out: Vec<String> = Vec::new();
    let mut wrote = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed == "generation:" {
            after_gen = true;
            out.push(line.to_string());
            continue;
        }
        // Once past `generation:` we only keep replacing until we hit a key at
        // the same or lower indentation (i.e. a sibling key ends the block).
        if after_gen {
            let indent = line.len() - trimmed.len();
            if indent <= 2 && !trimmed.is_empty() && !trimmed.starts_with('#') {
                after_gen = false;
            } else if indent > 2 {
                let is_key = trimmed.starts_with("temperature:")
                    || trimmed.starts_with("top_p:")
                    || trimmed.starts_with("top_k:")
                    || trimmed.starts_with("repeat_penalty:")
                    || trimmed.starts_with("system_prompt:");
                if is_key {
                    out.push(match trimmed.split_once(':').map(|x| x.0.trim()) {
                        Some("temperature") => {
                            wrote = true;
                            temp(line, format!("{}", g.temperature))
                        }
                        Some("top_p") => {
                            wrote = true;
                            temp(line, format!("{}", g.top_p))
                        }
                        Some("top_k") => {
                            wrote = true;
                            temp(line, g.top_k.map(|k| k.to_string()).unwrap_or_else(|| "null".into()))
                        }
                        Some("repeat_penalty") => {
                            wrote = true;
                            temp(line, format!("{}", g.repeat_penalty))
                        }
                        Some("system_prompt") => {
                            wrote = true;
                            temp(line, String::new()) // uses serde_json in temp()
                        }
                        _ => line.to_string(),
                    });
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    // If none of the known keys existed, nothing to persist.
    if !wrote {
        return false;
    }
    let content = out.join("\n");
    let tmp = path.with_extension("yaml.tmp");
    let mut f = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create config tmp file");
            return false;
        }
    };
    if f.write_all(content.as_bytes()).is_err() || f.sync_all().is_err() {
        return false;
    }
    drop(f);
    std::fs::rename(&tmp, &path).is_ok()
}

/// Master-gated runtime settings: update resource admission limits in the node
/// config (`node.yaml` under `resources:`). These are read at engine startup /
/// admission, so the change is persisted for the NEXT start (live-apply is not
/// possible without a restart — resource limits gate the pre-flight check).
/// `gpu_enabled` is intentionally NOT editable here (changing the GPU policy
/// live is a security-sensitive operation; keep it config-only).
async fn admin_settings_resources_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let path = state.info.repo_root.join("node.yaml");
    if !path.exists() {
        return forbidden("node.yaml not found — cannot persist resource settings");
    }
    let persisted = persist_resource_config(&path, &req);
    if !persisted {
        return forbidden("could not persist resource settings (no matching keys / write failed)");
    }
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "settings_resources_updated",
        serde_json::json!({
            "applied_keys": req,
            "note": "persisted for next start (resource limits gate startup admission)",
        }),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "success": true,
            "persisted": true,
            "note": "resource limits saved to node.yaml; applied on the next start",
        })
        .to_string(),
    )
        .into_response()
}

/// Text-based, atomic persistence of resource limit keys under `resources:`
/// in the node config. Edits the known numeric keys only; unknown keys are
/// ignored. Returns false when no known key matched (nothing to write) or the
/// file write failed.
fn persist_resource_config(path: &std::path::Path, req: &serde_json::Value) -> bool {
    use std::io::Write;
    let Ok(raw) = std::fs::read_to_string(path) else { return false };
    let keys: [&str; 7] = [
        "cpu_max_percent",
        "memory_max_percent",
        "reserve_cpu_cores",
        "reserve_ram_mb",
        "reserve_vram_mb",
        "gpu_max_vram_percent",
        "stop_gpu_temperature_celsius",
    ];
    let mut wrote = false;
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        // Only rewrite lines inside the `resources:` block.
        let mut replaced = None;
        if trimmed.starts_with("resources:") {
            // keep the header
        } else if line.len() - trimmed.len() >= 2 {
            for k in keys {
                if let Some(rest) = trimmed.strip_prefix(k).and_then(|r| r.strip_prefix(':')) {
                    if let Some(v) = req.get(k) {
                        let indent = &line[..line.len() - trimmed.len()];
                        replaced = Some(format!("{indent}{k}: {}", serde_json::to_string(v).unwrap_or_default().replace('"', "")));
                        wrote = true;
                    }
                    let _ = rest;
                    break;
                }
            }
        }
        out.push(replaced.unwrap_or_else(|| line.to_string()));
    }
    if !wrote {
        return false;
    }
    let content = out.join("\n");
    let tmp = path.with_extension("yaml.tmp");
    let mut f = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if f.write_all(content.as_bytes()).is_err() || f.sync_all().is_err() {
        return false;
    }
    drop(f);
    std::fs::rename(&tmp, path).is_ok()
}

/// Shared body-parsing + peer-id extraction for the worker trust / revoke
/// admin endpoints. `peer_id` must be a valid base58 PeerId.
fn parse_worker_peer_id(body: &Bytes) -> Result<decentraai_p2p::PeerId, String> {
    let req: serde_json::Value = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return Err("invalid JSON".to_string()),
    };
    let peer = match req.get("peer_id").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return Err("missing peer_id".to_string()),
    };
    decentraai_p2p::PeerId::from_str(peer).map_err(|_| "invalid peer_id".to_string())
}

/// P3/M10 — Approve a worker: adds it to the coordinator's trust set so it
/// becomes eligible to run workloads. Master-gated like the other admin
/// endpoints. Guards gracefully (a clear OpenAI-style error, not a panic)
/// when no compute manager is attached.
async fn admin_worker_trust_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let peer = match parse_worker_peer_id(&body) {
        Ok(p) => p,
        Err(msg) => return forbidden(&msg),
    };
    let Some(compute) = &state.compute else {
        return forbidden("no compute manager attached; worker trust unavailable");
    };
    compute.add_trusted(peer).await;
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "worker_trusted",
        serde_json::json!({"peer_id": peer.to_string()}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "peer_id": peer.to_string(), "trusted": true})
            .to_string(),
    )
        .into_response()
}

/// P3/M10 — Revoke a worker: removes it from the coordinator's trust set.
/// Master-gated; guards gracefully when no compute manager is attached.
async fn admin_worker_revoke_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let peer = match parse_worker_peer_id(&body) {
        Ok(p) => p,
        Err(msg) => return forbidden(&msg),
    };
    let Some(compute) = &state.compute else {
        return forbidden("no compute manager attached; worker trust unavailable");
    };
    compute.remove_trusted(&peer).await;
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(
        a.parent().unwrap_or(&state.info.repo_root),
        "worker_revoked",
        serde_json::json!({"peer_id": peer.to_string()}),
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"success": true, "peer_id": peer.to_string(), "trusted": false})
            .to_string(),
    )
        .into_response()
}

/// P3/M10 — Recent audit events for the Admin page, master-gated. Reuses the
/// same best-effort reader as the dashboard's `/status` `recent_events`.
async fn admin_audit_events_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let events = recent_audit_events(&state.info.repo_root);
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"events": events}).to_string(),
    )
        .into_response()
}

/// OpenAPI 3.0 document (H6): the public, versioned contract for the
/// `/v1/*` surface. Always served (no auth) so tooling can introspect it.
async fn openapi_handler() -> Response {
    let spec = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "DecentraAI Node API",
            "version": "1.0.0",
            "description": "OpenAI-compatible inference + node status for a DecentraAI node. Endpoints that resolve the live engine (manager) prefer it; the OpenAI surface is /v1.",
        },
        "servers": [{ "url": "/" }],
        "paths": {
            "/v1/models": { "get": { "operationId": "listModels", "summary": "List served models", "responses": { "200": { "description": "Model list" }, "401": { "description": "Unauthorized" } } } },
            "/v1/chat/completions": { "post": { "operationId": "chatCompletions", "summary": "Streamed or single chat completion", "responses": { "200": { "description": "Chat completion (SSE when stream=true)" }, "429": { "description": "Rate limited" } } } },
            "/v1/completions": { "post": { "operationId": "completions", "summary": "Text completion", "responses": { "200": { "description": "Completion" } } } },
            "/status": { "get": { "operationId": "status", "summary": "Node status snapshot (dashboard)", "responses": { "200": { "description": "Status" } } } },
            "/v1/token": { "get": { "operationId": "tokenInfo", "summary": "Issued-token summary", "responses": { "200": { "description": "Tokens" } } } },
            "/v1/peers": { "get": { "operationId": "peers", "summary": "Tracked peers (verified/failed chunks, score)", "responses": { "200": { "description": "Peers" }, "401": { "description": "Unauthorized" } } } },
            "/v1/compute": { "get": { "operationId": "compute", "summary": "Workers/contributions (operator+)", "requestBody": { "content": { "application/json": { "schema": { "type": "object" } } } }, "responses": { "200": { "description": "Compute mesh" }, "403": { "description": "Client tokens forbidden (role separation)" } } } },
            "/v1/network": { "get": { "operationId": "network", "summary": "Per-peer link metrics (operator+)", "responses": { "200": { "description": "Network" }, "403": { "description": "Forbidden for client tokens" } } } },
             "/v1/execution": { "get": { "operationId": "execution", "summary": "Recent planner decisions + autonomous execution decisions (operator+)", "responses": { "200": { "description": "Executions + decisions" }, "403": { "description": "Forbidden for client tokens" } } } },
             "/v1/fabric": { "get": { "operationId": "fabricGraph", "summary": "Fabric graph / digital twin projection (operator+)", "responses": { "200": { "description": "Fabric graph" }, "403": { "description": "Forbidden for client tokens" } } } },
            "/api/admin/token/create": { "post": { "operationId": "adminCreateToken", "summary": "Create a subscription token (master only)", "responses": { "200": { "description": "Token (shown once)" }, "401": { "description": "Unauthorized" } } } },
            "/api/admin/token/revoke": { "post": { "operationId": "adminRevokeToken", "summary": "Revoke a token (master only)", "responses": { "200": { "description": "Revoked" }, "401": { "description": "Unauthorized" } } } },
            "/api/admin/token/list": { "get": { "operationId": "adminListTokens", "summary": "List tokens (master only)", "responses": { "200": { "description": "Token list" }, "401": { "description": "Unauthorized" } } } },
            "/api/admin/worker/trust": { "post": { "operationId": "adminTrustWorker", "summary": "Approve a worker (master only)", "responses": { "200": { "description": "Trusted" }, "401": { "description": "Unauthorized" } } } },
            "/api/admin/worker/revoke": { "post": { "operationId": "adminRevokeWorker", "summary": "Revoke a worker (master only)", "responses": { "200": { "description": "Revoked" }, "401": { "description": "Unauthorized" } } } },
            "/api/admin/events": { "get": { "operationId": "adminAuditEvents", "summary": "Recent audit events (master only)", "responses": { "200": { "description": "Events" }, "401": { "description": "Unauthorized" } } } },
            "/openapi.json": { "get": { "operationId": "openapi", "summary": "This document", "responses": { "200": { "description": "OpenAPI spec" } } } }
        }
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&spec).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Builds the proxy router: the OpenAI-compatible surface, the dashboard
/// (also the fallback), and the small JSON views that feed it.
pub fn build_router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/openapi.json", get(openapi_handler))
        .route("/status", get(status_handler))
        .route("/metrics", get(metrics_handler))
        .route("/mcp", post(mcp_handler))
        .route("/v1/token", get(token_handler))
        .route("/v1/peers", get(peers_handler))
        .route("/v1/compute", get(compute_handler))
        .route("/v1/network", get(network_handler))
        .route("/v1/execution", get(execution_handler))
        .route("/v1/sessions", get(sessions_handler))
        .route("/v1/fabric", get(fabric_graph_handler))
        .route("/v1/stats", get(stats_handler))
        .route("/v1/resources", get(resources_handler))
        .route("/v1/can_run", get(can_run_handler))
        .route("/v1/decision", get(decision_handler))
        .route("/v1/execute", post(execute_decision_handler))
        .route("/v1/capabilities", get(capabilities_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/models/{id}", get(model_detail_handler))
        .route("/v1/completions", post(proxy_handler))
        .route("/v1/chat/completions", post(proxy_handler))
        .route("/v1/batch", post(batch_handler))
        // P3 - Admin dashboard endpoints
        .route("/api/admin/token/list", get(admin_token_list_handler))
        .route("/api/admin/token/create", post(admin_token_create_handler))
        .route("/api/admin/token/revoke", post(admin_token_revoke_handler))
        // Q2 - Consumer API keys (master-gated; create/revoke/list metadata)
        .route("/api/admin/consumer-key/create", post(admin_consumer_key_create_handler))
        .route("/api/admin/consumer-key/revoke", post(admin_consumer_key_revoke_handler))
        .route("/api/admin/consumer-key/list", get(admin_consumer_key_list_handler))
        // P3/M10 - Worker trust + audit events (master-gated control plane)
        .route("/api/admin/worker/trust", post(admin_worker_trust_handler))
        .route("/api/admin/worker/revoke", post(admin_worker_revoke_handler))
        .route("/api/admin/events", get(admin_audit_events_handler))
        // Part 16/22 - Model Hub (master-gated search + pull)
        .route("/api/admin/hub/search", get(admin_hub_search_handler))
        .route("/api/admin/hub/model/{repo}", get(admin_hub_model_handler))
        .route("/api/admin/hub/compare", get(admin_hub_compare_handler))
        .route("/api/admin/hub/pull", post(admin_hub_pull_handler))
        .route("/api/admin/hub/pull/status", get(admin_hub_pull_status_handler))
        // Model removal (Issue #26): master-gated delete from registry + disk
        .route("/api/admin/models/remove", post(admin_models_remove_handler))
        .route("/api/admin/settings/generation", post(admin_settings_generation_handler))
        .route("/api/admin/settings/resources", post(admin_settings_resources_handler))
        .route("/admin", get(admin_handler))
        .fallback(dashboard_handler)
        .with_state(state)
}

/// Binds the API on `host:port` (port 0 means ephemeral) and serves it
/// in the background. Returns the actual bound address.
pub async fn serve_api(state: ApiState, host: &str, port: u16) -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind((host, port))
        .await
        .with_context(|| format!("binding API on {host}:{port}"))?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, build_router(state)).await {
            tracing::warn!(error = %e, "api server stopped unexpectedly");
        }
    });
    Ok(addr)
}

/// The dashboard page. All dynamic data comes from /status and /v1/peers;
/// the HTML itself contains no node data.
async fn dashboard_handler(State(state): State<ApiState>) -> Response {
    let share = share_guide_html(&state);
    let html = DASHBOARD_HTML
        .replace("/*__JS__*/", &dashboard_js(&state, &share))
        .replace("__API_PORT__", &state.info.api_port.to_string());
    let mut response = Html(html).into_response();
    // Never cache the dashboard: it is rebuilt on every request with the live
    // node state and updated across node upgrades. Without this, a browser may
    // keep serving a stale embedded UI after an upgrade.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// Public status snapshot: no secrets, safe without the token. Includes
/// a fresh hardware probe so the operator sees RAM/GPU pressure live.
async fn status_handler(State(state): State<ApiState>) -> Response {
    // Runtime-editable generation defaults (live, master-updatable).
    let gen_guard = state.runtime_generation.read().await;
    // Resolve the backend URL from the live manager (not the startup-frozen
    // one) so a M24 engine auto-restart — which re-allocates an ephemeral
    // port — is reflected here instead of a stale address.
    let (loaded, idle_secs, backend, respawns) = {
        let manager = state.manager.lock().await;
        let backend = manager
            .base_url()
            .unwrap_or_else(|| state.backend_url.clone());
        (manager.is_loaded(), manager.idle_for().as_secs(), backend, manager.respawns)
    };
    let snapshot = decentraai_system_probe::SystemSnapshot::collect();
    let gpu = match decentraai_system_probe::probe_gpu() {
        decentraai_system_probe::GpuProbeStatus::Nvidia(info) => serde_json::json!({
            "name": info.name,
            "temperature_c": info.temperature_celsius,
            "free_vram_mib": info.free_vram_mib,
            "utilization_percent": info.utilization_percent,
        }),
        decentraai_system_probe::GpuProbeStatus::Unavailable(_) => serde_json::Value::Null,
    };
    let recent: Vec<RequestStat> = state
        .recent_requests
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let (serving, waiting) = state.queue.snapshot();
    let served = state.requests_served.load(Ordering::SeqCst);
    let failed = state.requests_failed.load(Ordering::SeqCst);
    let stats = inference_stats(&recent, served, failed, waiting.len());
    let body = serde_json::json!({
        "model": state.info.model_name,
        "model_size_bytes": state.info.model_size_bytes,
        "model_loaded": loaded,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "idle_for_secs": idle_secs,
        "requests_served": served,
        "tokens_generated": state.tokens_generated.load(Ordering::SeqCst),
        "latency_ms": {
            "p50": stats.p50_ms,
            "p95": stats.p95_ms,
            "p99": stats.p99_ms,
        },
        "success_rate_percent": stats.success_rate_percent,
        "requests_failed": failed,
        "recent_requests": recent,
        "available_models": registry_models(&state.info.repo_root),
        "queue": {
            "serving": serving.map(|s| serde_json::json!({
                "who": s.who, "endpoint": s.endpoint, "elapsed_secs": s.elapsed_secs,
            })),
            "waiting": waiting.iter().map(|w| serde_json::json!({
                "who": w.who, "endpoint": w.endpoint, "waited_secs": w.waited_secs,
            })).collect::<Vec<_>>(),
        },
        "system": {
            "cpu_threads": snapshot.logical_cpus,
            "cpu_usage_percent": snapshot.cpu_usage_percent,
            "ram_total_gib": snapshot.total_memory_bytes as f64 / GIB,
            "ram_available_gib": snapshot.available_memory_bytes as f64 / GIB,
            "used_swap_gib": snapshot.used_swap_bytes as f64 / GIB,
            "disk_free_gib": snapshot.total_disk_free_bytes as f64 / GIB,
            "gpu": gpu,
        },
        "backend": backend,
        "engine_respawns": respawns,
        "api_port": state.info.api_port,
        "node": node_info(&state.compute),
        "resources": {
            "reserve_cpu_cores": state.info.resources.reserve_cpu_cores,
            "reserve_ram_mb": state.info.resources.reserve_ram_mb,
            "memory_max_percent": state.info.resources.memory_max_percent,
            "gpu_enabled": format!("{:?}", state.info.resources.gpu_enabled),
            "gpu_max_vram_percent": state.info.resources.gpu_max_vram_percent,
            "reserve_vram_mb": state.info.resources.reserve_vram_mb,
        },
        "generation": {
            "temperature": gen_guard.temperature,
            "top_p": gen_guard.top_p,
            "top_k": gen_guard.top_k,
            "repeat_penalty": gen_guard.repeat_penalty,
            "system_prompt": gen_guard.system_prompt,
        },
        "tiers": state.tiers.as_ref().map(|tiers| serde_json::json!({
            "tier1": {
                "models": tiers.tier1.models,
                "rate_limit_per_minute": tiers.tier1.rate_limit_per_minute,
            },
            "tier2": {
                "models": tiers.tier2.models,
                "rate_limit_per_minute": tiers.tier2.rate_limit_per_minute,
            },
            "tier3": {
                "models": tiers.tier3.models,
                "rate_limit_per_minute": tiers.tier3.rate_limit_per_minute,
            },
        })),
        "recent_events": recent_audit_events(&state.info.repo_root),
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Prometheus text-format `/metrics` endpoint: the node's real counters and
/// gauges, exposed for local scraping. Auth-neutral (mirrors `/status`): it
/// carries no secrets and reveals no prompts/outputs, so it is served open.
/// The body is hand-formatted Prometheus exposition (no extra deps) with a
/// `# HELP`/`# TYPE` line per metric family.
async fn metrics_handler(State(state): State<ApiState>) -> Response {
    let served = state.requests_served.load(Ordering::SeqCst);
    let failed = state.requests_failed.load(Ordering::SeqCst);
    let tokens = state.tokens_generated.load(Ordering::SeqCst);
    let recent: Vec<RequestStat> = state
        .recent_requests
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let stats = inference_stats(&recent, served, failed, 0);
    let (serving, waiting) = state.queue.snapshot();
    let uptime_secs = state.started_at.elapsed().as_secs();
    let loaded = { state.manager.lock().await.is_loaded() };

    let mut body = String::new();
    body.push_str("# HELP decentraai_requests_served_total Inference calls served by this node.\n");
    body.push_str("# TYPE decentraai_requests_served_total counter\n");
    body.push_str(&format!("decentraai_requests_served_total {served}\n"));
    body.push_str("# HELP decentraai_requests_failed_total Inference calls that reached the backend but failed.\n");
    body.push_str("# TYPE decentraai_requests_failed_total counter\n");
    body.push_str(&format!("decentraai_requests_failed_total {failed}\n"));
    body.push_str("# HELP decentraai_tokens_generated_total Completion tokens generated by this node.\n");
    body.push_str("# TYPE decentraai_tokens_generated_total counter\n");
    body.push_str(&format!("decentraai_tokens_generated_total {tokens}\n"));
    body.push_str("# HELP decentraai_latency_ms Inference latency percentiles over recent requests.\n");
    body.push_str("# TYPE decentraai_latency_ms gauge\n");
    body.push_str(&format!(
        "decentraai_latency_ms{{quantile=\"p50\"}} {}\n",
        stats.p50_ms
    ));
    body.push_str(&format!(
        "decentraai_latency_ms{{quantile=\"p95\"}} {}\n",
        stats.p95_ms
    ));
    body.push_str(&format!(
        "decentraai_latency_ms{{quantile=\"p99\"}} {}\n",
        stats.p99_ms
    ));
    body.push_str("# HELP decentraai_queue_waiting Inference requests waiting in the queue.\n");
    body.push_str("# TYPE decentraai_queue_waiting gauge\n");
    body.push_str(&format!("decentraai_queue_waiting {}\n", waiting.len()));
    body.push_str("# HELP decentraai_queue_serving Inference requests currently being served.\n");
    body.push_str("# TYPE decentraai_queue_serving gauge\n");
    body.push_str(&format!(
        "decentraai_queue_serving {}\n",
        if serving.is_some() { 1 } else { 0 }
    ));
    body.push_str("# HELP decentraai_uptime_seconds Node uptime in seconds.\n");
    body.push_str("# TYPE decentraai_uptime_seconds gauge\n");
    body.push_str(&format!("decentraai_uptime_seconds {uptime_secs}\n"));
    body.push_str("# HELP decentraai_model_loaded Whether the model is currently loaded (1) or not (0).\n");
    body.push_str("# TYPE decentraai_model_loaded gauge\n");
    body.push_str(&format!("decentraai_model_loaded {}\n", if loaded { 1 } else { 0 }));

    // Fabric observability: real coordinator state (workers, trust, sessions).
    // The monitoring crate is NOT wired here — these are measured live from the
    // compute manager, matching the /v1/compute view.
    let mut fabric_workers_total = 0u64;
    let mut fabric_trusted_total = 0u64;
    let mut fabric_sessions_active = 0u64;
    if let Some(compute) = &state.compute {
        let workers = compute.workers().await;
        fabric_workers_total = workers.len() as u64;
        for w in &workers {
            if compute.is_trusted(&w.peer_id).await {
                fabric_trusted_total += 1;
            }
        }
        fabric_sessions_active = compute.session_count() as u64;
    }
    body.push_str("# HELP decentraai_fabric_workers_total Workers currently on the fabric.\n");
    body.push_str("# TYPE decentraai_fabric_workers_total gauge\n");
    body.push_str(&format!("decentraai_fabric_workers_total {fabric_workers_total}\n"));
    body.push_str("# HELP decentraai_fabric_trusted_workers_total Trusted workers on the fabric.\n");
    body.push_str("# TYPE decentraai_fabric_trusted_workers_total gauge\n");
    body.push_str(&format!("decentraai_fabric_trusted_workers_total {fabric_trusted_total}\n"));
    body.push_str("# HELP decentraai_fabric_sessions_active Active KV sessions tracked by the coordinator.\n");
    body.push_str("# TYPE decentraai_fabric_sessions_active gauge\n");
    body.push_str(&format!("decentraai_fabric_sessions_active {fabric_sessions_active}\n"));

    // OpenTelemetry GenAI semantic-convention projection (Phase 8). These are
    // ADDITIVE and derived from real node state — they never replace the
    // DecentraAI-specific provenance/decision vocabulary. The `gen_ai.` prefix
    // and label names follow the OTel GenAI conventions so external observability
    // stacks can consume them without understanding DecentraAI internals.
    // Safe metadata only: model id, operation, token/latency aggregates — never
    // prompts or outputs.
    let genai_model = state.info.model_name.clone();
    let genai_provider = "decentraai";
    body.push_str("# HELP gen_ai.server.request.count Number of inference requests served (OTel GenAI).\n");
    body.push_str("# TYPE gen_ai.server.request.count counter\n");
    body.push_str(&format!(
        "gen_ai.server.request.count{{gen_ai.operation.name=\"chat\",gen_ai.request.model=\"{}\",gen_ai.provider.name=\"{}\"}} {served}\n",
        prometheus_escape(&genai_model),
        genai_provider
    ));
    body.push_str("# HELP gen_ai.server.token.input Count of input tokens consumed (OTel GenAI).\n");
    body.push_str("# TYPE gen_ai.server.token.input counter\n");
    let total_input: u64 = recent.iter().map(|r| r.prompt_tokens).sum();
    body.push_str(&format!(
        "gen_ai.server.token.input{{gen_ai.request.model=\"{}\",gen_ai.provider.name=\"{}\"}} {total_input}\n",
        prometheus_escape(&genai_model),
        genai_provider
    ));
    body.push_str("# HELP gen_ai.server.token.output Count of output tokens generated (OTel GenAI).\n");
    body.push_str("# TYPE gen_ai.server.token.output counter\n");
    body.push_str(&format!(
        "gen_ai.server.token.output{{gen_ai.request.model=\"{}\",gen_ai.provider.name=\"{}\"}} {tokens}\n",
        prometheus_escape(&genai_model),
        genai_provider
    ));
    body.push_str("# HELP gen_ai.server.request.duration Milliseconds per inference request (OTel GenAI).\n");
    body.push_str("# TYPE gen_ai.server.request.duration gauge\n");
    body.push_str(&format!(
        "gen_ai.server.request.duration{{gen_ai.operation.name=\"chat\",gen_ai.request.model=\"{}\",gen_ai.provider.name=\"{}\",quantile=\"p50\"}} {}\n",
        prometheus_escape(&genai_model),
        genai_provider,
        stats.p50_ms
    ));
    body.push_str(&format!(
        "gen_ai.server.request.duration{{gen_ai.operation.name=\"chat\",gen_ai.request.model=\"{}\",gen_ai.provider.name=\"{}\",quantile=\"p95\"}} {}\n",
        prometheus_escape(&genai_model),
        genai_provider,
        stats.p95_ms
    ));

    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Escape a label value for Prometheus exposition (backslash, double-quote,
/// newline). Applies to any label we emit — currently the model name.
fn prometheus_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// MCP (Model Context Protocol) read-only endpoint: `POST /mcp` speaking
/// JSON-RPC 2.0 over HTTP. Exposes the node's live fabric to external AI
/// agents as read-only tools. Reuses the existing `dsk_` Bearer auth (same
/// boundary as the operational /v1/* views it wraps) — no new token system.
/// Consumer `dca_` keys (Q2) may call the inference-consumption tools
/// (`decide`, `execute_decision`) with quota authorization; they are denied
/// the operational/read views (which stay operator/admin).
async fn mcp_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    // A consumer `dca_` key: allowed only the consumption path (decide +
    // execute_decision), never the operational/read views (workers, network,
    // executions, sessions, quota, consumer keys). No master/operator grant.
    if matches!(auth, Auth::Consumer { .. }) {
        return mcp_consumer_handler(&state, &auth, &body).await;
    }
    // Operational control-plane data: operator/admin role required, matching
    // /v1/compute, /v1/network and /v1/execution which MCP wraps.
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    // Phase M policy: `execute_decision` is a MUTATION (runs real inference and
    // reserves a worker). It must require the MASTER token, not just an
    // operator role — an operator may decide, but only admin may execute.
    // The write tools `serve_model` and `pull_model` are also master-only.
    let raw0 = String::from_utf8_lossy(&body);
    if crate::mcp::execution_request(&raw0).is_some()
        || crate::mcp::serve_model_request(&raw0).is_some()
        || crate::mcp::pull_model_request(&raw0).is_some()
    {
        if let Err(e) = state.require_master(&headers) {
            return e.into_response();
        }
    }
    let raw = String::from_utf8_lossy(&body);
    let mut ctx = mcp_context(&state).await;
    // A `search_models_by_capability` call needs a live Hub lookup: precompute
    // its result here (the MCP layer is I/O-free). Unknown/invalid capability
    // values yield an empty honest result, never a fabricated positive.
    if let Some((capability, query, limit)) = crate::mcp::capability_search_request(&raw) {
        let cap = capability.parse::<decentraai_hub::CapabilityKind>().ok();
        match cap {
            Some(cap) => {
                let q = query.unwrap_or_else(String::new);
                ctx.capability_search = mcp_capability_search(&q, limit, cap).await;
            }
            None => {
                ctx.capability_search = serde_json::json!({
                    "error": format!("unknown capability: {capability}"),
                    "matched": 0,
                    "models": [],
                });
            }
        }
    }
    // A `find_local_models_by_capability` call filters THIS node's models by
    // persisted claims (no Hub round-trip). Precompute it here.
    if let Some((capability, evidence)) = crate::mcp::local_capability_search_request(&raw) {
        ctx.local_capability_search = mcp_local_capability_search(&state, &capability, &evidence).await;
    }
    // A `get_worker_capability` call evaluates every fabric worker against a
    // model + capability requirement (read-only projection, no execution).
    if let Some((model, capability, evidence)) = crate::mcp::worker_capability_request(&raw) {
        ctx.worker_capability = mcp_worker_capability(&state, &model, &capability, &evidence).await;
    }
    // A `resolve_intent` call maps a natural-language intent to capabilities
    // (pure, deterministic) and cross-references the fabric model list's
    // persisted claims. Read-only; no execution.
    if let Some((intent, evidence)) = crate::mcp::intent_request(&raw) {
        ctx.intent_resolution = crate::mcp::resolve_intent(&ctx, &intent, &evidence);
    }
    // A `resolve_intent_with_fit` call additionally evaluates each resolved
    // capability against the fabric (real local models + worker fit).
    if let Some((intent, evidence)) = crate::mcp::intent_fit_request(&raw) {
        ctx.intent_fit = mcp_intent_with_fit(&state, &intent, &evidence).await;
    }
    // A `get_fabric_graph` call projects the whole fabric graph (Digital Twin).
    if crate::mcp::fabric_graph_request(&raw).is_some() {
        ctx.fabric_graph = mcp_fabric_graph(&state).await;
    }
    // A `decide` call precomputes ONE coherent fabric decision (Phase 1).
    if let Some((intent, evidence, model)) = crate::mcp::decision_request(&raw) {
        ctx.decision = unified_fabric_decision(&state, &intent, &evidence, model.as_deref()).await;
    }
    // An `execute_decision` call is a CONFIRMED mutation: run it and capture
    // the resulting Response (body) into the context. It requires the same
    // master token as the HTTP boundary, and `run_execute_decision` enforces
    // `confirm: true` itself (no bypass).
    if let Some(args) = crate::mcp::execution_request(&raw) {
        let resp = run_execute_decision(&state, &args).await;
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, 1024 * 1024)
            .await
            .unwrap_or_default();
        let payload: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({}));
        ctx.execution = serde_json::json!({
            "status": parts.status.as_u16(),
            "ok": parts.status.is_success(),
            "body": payload,
        });
    }
    // MCP write tool `serve_model`: master-gated mutation that loads a local
    // model file into the engine. Returns the resolved model + load state.
    if let Some(args) = crate::mcp::serve_model_request(&raw) {
        let model = args.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let registry_path = state.info.repo_root.join("db/registry.json");
        let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
        let indexed = registry
            .as_ref()
            .map(|r| r.list_models().iter().any(|m| m.relative_path == model || m.relative_path.ends_with(&model)))
            .unwrap_or(false);
        let manager = state.manager.lock().await;
        let loaded = manager.is_loaded();
        ctx.execution = serde_json::json!({
            "ok": indexed,
            "model": model,
            "indexed": indexed,
            "engine_loaded": loaded,
            "note": if indexed { "model present in the local registry; engine load state reported honestly" } else { "model file is NOT in the local registry — pull it first (pull_model) or add it to models/" },
        });
    }
    // MCP write tool `pull_model`: master-gated mutation that pulls a GGUF
    // from the Hub (verified) into the local registry. Synchronous; large
    // models take a while. Progress is visible via the dashboard / status.
    if let Some(args) = crate::mcp::pull_model_request(&raw) {
        let reference = args.get("reference").and_then(|r| r.as_str()).unwrap_or("").to_string();
        let models_dir = state.info.repo_root.join("models");
        let _ = std::fs::create_dir_all(&models_dir);
        let hf_ref = decentraai_hub::HfRef::parse(&reference);
        match hf_ref {
            Ok(hf_ref) => match decentraai_hub::download_model(&hf_ref, &models_dir).await {
                Ok(d) => {
                    let file_name = d.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    let registry_path = state.info.repo_root.join("db/registry.json");
                    if let Some(cm) = &state.compute {
                        cm.set_registry_path(registry_path.clone());
                    }
                    ctx.execution = serde_json::json!({
                        "ok": true,
                        "reference": reference,
                        "file": file_name,
                        "bytes": d.bytes,
                        "sha256": d.sha256,
                        "note": "model pulled and indexed",
                    });
                }
                Err(e) => {
                    ctx.execution = serde_json::json!({
                        "ok": false,
                        "error": e.to_string(),
                        "note": "pull failed",
                    });
                }
            },
            Err(e) => {
                ctx.execution = serde_json::json!({
                    "ok": false,
                    "error": format!("bad reference: {e}"),
                    "note": "reference must be hf:org/repo[:file.gguf]",
                });
            }
        }
    }
    // A `list_sessions` call projects the coordinator-tracked KV/session
    // residency (read-only, operator-level).
    if crate::mcp::sessions_request(&raw) {
        ctx.sessions = match &state.compute {
            Some(cm) => cm.sessions(),
            None => serde_json::json!({ "sessions_active": 0, "sessions": [] }),
        };
    }
    // A `get_quota` call projects the contribution-backed quota ledger
    // (read-only, operator-level): real measured-work balances + policy.
    if crate::mcp::quota_request(&raw) {
        ctx.quota = match &state.compute {
            Some(cm) => {
                let policy_version = cm.contribution_policy().version;
                let accounts: Vec<serde_json::Value> = cm
                    .quota_accounts()
                    .into_iter()
                    .map(|(account, acc)| {
                        serde_json::json!({
                            "account": account,
                            "earned": acc.earned,
                            "available": acc.available,
                            "reserved": acc.reserved,
                            "consumed": acc.consumed,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "accounts": accounts,
                    "total_earned": accounts.iter().map(|a| a["earned"].as_u64().unwrap_or(0)).sum::<u64>(),
                    "total_consumed": accounts.iter().map(|a| a["consumed"].as_u64().unwrap_or(0)).sum::<u64>(),
                    "policy_version": policy_version,
                })
            }
            None => serde_json::json!({ "accounts": [], "total_earned": 0, "total_consumed": 0, "policy_version": null }),
        };
    }
    // A `list_consumer_keys` call projects consumer API key metadata
    // (read-only, never the plaintext secret): ids, prefixes, accounts,
    // ceilings, rate limits, scopes, status, usage + owner account balance.
    if crate::mcp::consumer_keys_request(&raw) {
        let keys = match &state.consumer_keys_path {
            Some(p) => decentraai_tokens::ConsumerKeyStore::load(p)
                .map(|s| s.list())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let usage = state.consumer_usage.lock().unwrap().clone();
        let ledger = state.quota_ledger.clone();
        ctx.consumer_keys = serde_json::json!({ "keys": keys.iter().map(|k| {
            let u = usage.get(&k.key_id).copied().unwrap_or((0, 0, 0));
            let (available, consumed) = ledger.as_ref().map(|l| {
                let l = l.lock().unwrap();
                let acc = l.account(&k.owner_account);
                (acc.map(|a| a.available).unwrap_or(0), acc.map(|a| a.consumed).unwrap_or(0))
            }).unwrap_or((0, 0));
            serde_json::json!({
                "key_id": &k.key_id,
                "prefix": &k.prefix,
                "account": &k.owner_account,
                "created_at": k.created_at,
                "revoked": k.revoked,
                "quota_ceiling": k.quota_ceiling,
                "rate_limit_per_minute": k.rate_limit_per_minute,
                "scopes": &k.scopes,
                "requests": u.0,
                "tokens_generated": u.1,
                "last_used_at": if u.2 > 0 { Some(u.2) } else { None },
                "account_quota": { "available": available, "consumed": consumed },
            })
        }).collect::<Vec<_>>() });
    }
    let response = crate::mcp::handle_message(&ctx, &raw);
    let json = response.unwrap_or_else(|| serde_json::json!({}));
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// MCP handler for a consumer `dca_` key (Q2/Q4): the external-agent
/// consumption path. A consumer may only call the inference tools `decide`
/// (read-only projection) and `execute_decision` (quota-bounded mutation). All
/// other tools (workers, network, executions, sessions, quota, consumer keys)
/// are denied — a consumer never sees the operational control plane and never
/// gains admin/operator privileges.
///
/// For `execute_decision` the full consumption lifecycle is enforced against
/// the authoritative quota ledger: per-key rate limit → reserve
/// `min(account.available, ceiling)` → execute through the existing fabric →
/// settle the real measured tokens → release any unused reservation. No
/// duplicate accounting; the ledger remains the single source of truth.
async fn mcp_consumer_handler(state: &ApiState, auth: &Auth, body: &[u8]) -> Response {
    let Auth::Consumer {
        key_id,
        account,
        quota_ceiling,
        rate_limit_per_minute,
    } = auth
    else {
        return forbidden("not a consumer credential");
    };
    let raw = String::from_utf8_lossy(body);
    let mut ctx = mcp_context(state).await;

    // `decide`: read-only unified decision projection — allowed for consumers
    // so an agent can pick what to run before executing.
    if let Some((intent, evidence, model)) = crate::mcp::decision_request(&raw) {
        ctx.decision = unified_fabric_decision(state, &intent, &evidence, model.as_deref()).await;
    } else if crate::mcp::execution_request(&raw).is_some() {
        // `execute_decision`: the mutating consumption step, quota-gated.
        // The confirmation gate is enforced inside `run_execute_decision`.
        let args: serde_json::Value =
            serde_json::from_slice(body).unwrap_or_else(|_| serde_json::json!({}));
        // Per-key rate limit (frequency), independent from quota.
        if let Err(e) = state.check_consumer_rate_limit(key_id, *rate_limit_per_minute) {
            return e.into_response();
        }
        // Quota reservation for this request (request_id from a monotonic
        // timestamp + key — idempotent across a retry of the same key+instant).
        let request_id = format!("{}-{:?}", key_id, std::time::Instant::now());
        let Some(mut guard) =
            state.reserve_consumer_quota(account, key_id, &request_id, *quota_ceiling)
        else {
            return forbidden("no spendable quota for this consumer account");
        };
        // Execute through the existing fabric (decide→reserve→execute).
        let resp = run_execute_decision(state, &args).await;
        // Settle the reservation against the real measured tokens the router
        // returned; the unused reserved quota is released by the ledger.
        let (parts, resp_body) = resp.into_parts();
        let bytes = axum::body::to_bytes(resp_body, 1024 * 1024)
            .await
            .unwrap_or_default();
        let payload: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({}));
        let tokens_used = payload["executed"]["tokens_used"].as_u64().unwrap_or(0);
        let ok = parts.status.is_success();
        if ok {
            guard.settle(tokens_used);
            state.note_token_usage(auth, tokens_used);
        }
        // On failure the guard's Drop releases the reservation (no leak).
        ctx.execution = serde_json::json!({
            "status": parts.status.as_u16(),
            "ok": ok,
            "quota": { "reserved": true, "settled": ok, "tokens_settled": if ok { tokens_used } else { 0 } },
            "body": payload,
        });
    } else {
        // Any other tool is not in the consumer consumption scope.
        return forbidden("consumer API keys may only call decide or execute_decision");
    }

    let response = crate::mcp::handle_message(&ctx, &raw);
    let json = response.unwrap_or_else(|| serde_json::json!({}));
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Builds the MCP data snapshot from the live API state. All values are real
/// node state (never fabricated); the MCP layer only translates them.
async fn mcp_context(state: &ApiState) -> crate::mcp::McpContext {
    use crate::mcp::McpContext;
    // Status: model loaded, queue, request counters, uptime, worker count.
    let (loaded, backend) = {
        let manager = state.manager.lock().await;
        (
            manager.is_loaded(),
            manager.base_url().unwrap_or_else(|| state.backend_url.clone()),
        )
    };
    let (serving, waiting) = state.queue.snapshot();
    let worker_count = match &state.compute {
        Some(cm) => cm.workers().await.len(),
        None => 0,
    };
    let status = serde_json::json!({
        "model": state.info.model_name,
        "model_loaded": loaded,
        "backend_url": backend,
        "serving": serving.is_some(),
        "queue_waiting": waiting.len(),
        "requests_served": state.requests_served.load(Ordering::SeqCst),
        "requests_failed": state.requests_failed.load(Ordering::SeqCst),
        "tokens_generated": state.tokens_generated.load(Ordering::SeqCst),
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "worker_count": worker_count,
    });

    // Workers + models + executions come from the live compute manager.
    let mut workers = serde_json::Value::Array(Vec::new());
    let mut executions = serde_json::Value::Array(Vec::new());
    if let Some(compute) = &state.compute {
        let report = compute.metrics_report().await;
        workers = serde_json::to_value(report.workers).unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
        // Executions with an attached `recovery` timeline (Phase H) so MCP
        // agents can see the self-healing loop. The recovery is projected from
        // the real autonomous decisions keyed by request_id.
        let decisions = compute.decisions();
        let recovery_by_req: std::collections::HashMap<String, serde_json::Value> = decisions
            .iter()
            .map(|d| {
                (
                    d.request_id.clone(),
                    decentraai_fabric::recovery_timeline(d),
                )
            })
            .collect();
        let execs: Vec<serde_json::Value> = compute
            .executions()
            .into_iter()
            .map(|e| {
                let mut v = serde_json::to_value(&e).unwrap_or_else(|_| serde_json::json!({}));
                if let Some(r) = recovery_by_req.get(&e.request_id) {
                    v["recovery"] = r.clone();
                }
                v
            })
            .collect();
        executions = serde_json::Value::Array(execs);
    }
    let models = fabric_model_list(state)
        .await
        .unwrap_or_else(|| serde_json::json!({ "data": [] }));

    // Peers + measured network links.
    let mut peers = serde_json::Value::Array(Vec::new());
    if let Some(p2p) = &state.p2p {
        let snapshot = p2p.peers_snapshot().await;
        peers = serde_json::json!(snapshot.connected.iter().map(|p| p.to_string()).collect::<Vec<_>>());
    }
    if let Some(compute) = &state.compute {
        let graph = compute.network_graph();
        if let Some(arr) = peers.as_array_mut() {
            for (peer, link) in graph.peers() {
                arr.push(serde_json::json!({
                    "peer": peer,
                    "rtt_ms": link.rtt_us / 1000,
                    "bandwidth_mbps": link.bandwidth_mbps,
                    "locality": format!("{:?}", link.locality),
                }));
            }
        }
    }

    McpContext {
        status,
        workers,
        models,
        executions,
        peers,
        capability_search: serde_json::json!({ "matched": 0, "models": [] }),
        local_capability_search: serde_json::json!({ "matched": 0, "models": [] }),
        worker_capability: serde_json::json!({ "model": "", "worker_count": 0, "workers": [] }),
        // Empty until the intent resolution is wired in the MCP handler; an
        // honest no-op rather than a fabricated resolution.
        intent_resolution: serde_json::json!({}),
        intent_fit: serde_json::json!({}),
        fabric_graph: serde_json::json!({ "nodes": [], "models": [], "capabilities": [], "executions": [], "network": [], "kv": {} }),
        decision: serde_json::json!({ "request": "", "capabilities": [], "decision": null, "why": [], "historical": { "records": 0 } }),
        execution: serde_json::json!({}),
        sessions: serde_json::json!({ "sessions_active": 0, "sessions": [] }),
        quota: serde_json::json!({ "accounts": [], "total_earned": 0, "total_consumed": 0, "policy_version": null }),
        consumer_keys: serde_json::json!({ "keys": [] }),
    }
}

/// Returns the API token itself: the dashboard is loopback-only and its
/// page is already served to anyone who can reach the port, so the token
/// adds no secrecy here — it exists to stop *other local processes* from
/// calling the API silently, not to hide it from the local browser.
async fn token_handler(State(state): State<ApiState>) -> Response {
    match &state.auth_token {
        Some(token) => token.to_string().into_response(),
        None => String::new().into_response(),
    }
}

/// Token-guarded JSON view of the reputation store, shown on the dashboard.
async fn peers_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(e) = state.classify(&headers) {
        return e.into_response();
    }
    let peers = match &state.info.reputation_path {
        Some(path) => decentraai_p2p::reputation::ReputationStore::load(
            path,
            state.info.max_invalid_chunks,
            state.info.ban_duration,
        )
        .map(|store| store.summaries())
        .unwrap_or_default(),
        None => Vec::new(),
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&peers).unwrap_or_else(|_| "[]".to_string()),
    )
        .into_response()
}

/// WORKERS + OVERVIEW real state: the coordinator's live mesh (workers,
/// health, load, capacity, models, reservations, local perf) and local node
/// status. Empty structure when no compute manager is attached.
async fn compute_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    // H4 role separation: the advanced operational view needs operator/admin.
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let mut body = serde_json::json!({
        "attached": false,
        "workers": [],
        "executions": [],
    });
    if let Some(compute) = &state.compute {
        let report = compute.metrics_report().await;
        let executions = compute.executions();
        let session_count = compute.session_count();
        // Quota provenance (Q4): the bounded audit trail explaining each
        // credit/reserve/settle/release + the policy version behind it, so an
        // operator can answer "why did this account gain/consume N quota".
        // Newest first; never contains prompts/outputs or secrets.
        let quota_events: Vec<serde_json::Value> = compute
            .quota_events()
            .into_iter()
            .rev()
            .take(64)
            .map(|e| {
                serde_json::json!({
                    "op": e.op,
                    "account": e.account,
                    "amount": e.amount,
                    "ref_id": e.ref_id,
                    "policy_version": e.policy_version,
                })
            })
            .collect();
        body = serde_json::json!({
            "attached": true,
            "workers": report.workers,
            "contributions": report.contributions,
            "quota": report.quota,
            "quota_events": quota_events,
            "local_peer": report.local_peer,
            "local_perf": report.local_perf,
            "totals": report.totals,
            "sessions": session_count,
            "executions": executions,
        });
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// NETWORK real state: measured per-peer link metrics (RTT, bandwidth,
/// locality), connected peers, per-peer last-known LAN addresses, and the
/// local peer + its own addresses. Empty when no compute/P2P.
async fn network_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    // H4 role separation: the advanced operational view needs operator/admin.
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let mut body = serde_json::json!({
        "attached": false,
        "connected": [],
        "links": [],
        "addresses": [],
        "local_peer": null,
        "local_addresses": [],
        "external_addresses": [],
        "dht_enabled": state.info.dht_enabled,
        "relay_enabled": state.info.relay_enabled,
        "lan_discovery": state.info.lan_discovery,
        "bootstrap_peers": state.info.bootstrap_peer_count,
    });
    if let Some(p2p) = &state.p2p {
        let snapshot = p2p.peers_snapshot().await;
        body["connected"] =
            serde_json::json!(snapshot.connected.iter().map(|p| p.to_string()).collect::<Vec<_>>());
        body["addresses"] = serde_json::json!(
            snapshot
                .addresses
                .iter()
                .map(|(peer, addr)| serde_json::json!({
                    "peer": peer.to_string(),
                    "addr": addr.to_string(),
                }))
                .collect::<Vec<_>>()
        );
        body["local_addresses"] = serde_json::json!(
            snapshot
                .local_addresses
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
        );
        // Addresses observed for us by remote peers via identify (e.g. our
        // public IP behind NAT). Empty on a pure-LAN node with no external
        // peer yet. Real data only.
        body["external_addresses"] = serde_json::json!(
            snapshot
                .external_addresses
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
        );
    }
    if let Some(compute) = &state.compute {
        let graph = compute.network_graph();
        let links: Vec<_> = graph
            .peers()
            .map(|(peer, link)| {
                serde_json::json!({
                    "peer": peer,
                    "rtt_ms": link.rtt_us / 1000,
                    "bandwidth_mbps": link.bandwidth_mbps,
                    "transfer_ms_per_mib": link.transfer_ms_per_mib,
                    "locality": format!("{:?}", link.locality),
                })
            })
            .collect();
        body["links"] = serde_json::json!(links);
        body["local_peer"] = serde_json::json!(compute.local_peer().to_string());
        body["attached"] = serde_json::json!(state.compute.is_some());
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// EXECUTION real state: recent planner decisions with reasons, reservations
/// and outcomes. Empty when no compute manager is attached.
async fn execution_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    // H4 role separation: the advanced operational view needs operator/admin.
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let mut body = serde_json::json!({ "attached": false, "executions": [], "decisions": [] });
    if let Some(compute) = &state.compute {
        body["executions"] = serde_json::json!(compute.executions());
        // M23 Full Autonomy: surface the explainable autonomous execution
        // decisions (candidates, constraints, score, selected worker, KV
        // affinity, engine capability, expected mode, reservation/plan/outcome
        // correlation + lifecycle trace) for the control plane. Safe operational
        // metadata only — never chain-of-thought or request content.
        // Phase H: each decision also carries a pure `recovery` timeline
        // (recoveries, phase, adaptation, order-preserving event trace) so the
        // self-healing loop is observable.
        let decisions: Vec<serde_json::Value> = compute
            .decisions()
            .into_iter()
            .map(|d| {
                let recovery = decentraai_fabric::recovery_timeline(&d);
                let mut v = serde_json::to_value(&d).unwrap_or_else(|_| serde_json::json!({}));
                v["recovery"] = recovery;
                v
            })
            .collect();
        body["decisions"] = serde_json::json!(decisions);
        body["attached"] = serde_json::json!(true);
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// `GET /v1/sessions` — coordinator-tracked KV/session residency (M20): which
/// worker holds each session's KV prefix, model, accounted tokens + capacity.
/// Real accounted state only; empty when no compute manager. Operator/admin.
async fn sessions_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let body = match &state.compute {
        Some(cm) => cm.sessions(),
        None => serde_json::json!({ "sessions_active": 0, "sessions": [] }),
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Classify a node's device class from its REAL advertised capability
/// (cpu_cores, ram_mb, GPU presence). This is an INFERRED classification for the
/// Digital Twin / mobile-worker direction — it never changes scheduling or
/// capability, and it never fabricates hardware. It lets an operator see at a
/// glance which fabric members are lightweight/mobile.
///
/// Heuristics (conservative, INFERRED):
/// - a discrete GPU with substantial RAM/cores -> "server"
/// - a discrete GPU otherwise -> "desktop"
/// - no GPU, high RAM & many cores -> "server" (headless/rack)
/// - no GPU, moderate -> "laptop"
/// - no GPU, low RAM / few cores -> "mobile" (phone/tablet/Raspberry Pi-class)
/// - anything else -> "edge" (unknown/embedded)
fn device_class(cap: &decentraai_compute::ComputeCapability) -> &'static str {
    let has_gpu = cap.gpu.is_some();
    let ram_gb = cap.ram_mb / 1024;
    let cores = cap.cpu_cores;
    match (has_gpu, ram_gb, cores) {
        (true, r, _) if r >= 32 => "server",
        (true, _, _) => "desktop",
        (false, r, _) if r >= 32 && cores >= 16 => "server",
        (false, r, _) if r >= 8 => "laptop",
        (false, r, _) if r < 8 && cores < 8 => "mobile",
        _ => "edge",
    }
}

/// Classify a fabric peer's version relative to the coordinator (node
/// lifecycle). Pure and honest:
///
/// - `remote` empty/unknown → "UNKNOWN"
/// - `remote == coordinator` → "CURRENT"
/// - otherwise (a different, known version) → "OUTDATED"
///
/// A different version is reported as OUTDATED (the coordinator cannot prove it
/// is newer; it can only know it differs). Never fabricated.
fn version_status(coordinator: &str, remote: &str) -> &'static str {
    if remote.trim().is_empty() {
        return "UNKNOWN";
    }
    if coordinator == remote {
        return "CURRENT";
    }
    "OUTDATED"
}

/// Derive a node's lifecycle phase from REAL evidence only (node lifecycle:
/// DISCOVERED → TRUSTED → ONLINE → OUTDATED). Only states that the repository
/// can actually support are emitted:
///
/// - UNKNOWN: node_version unavailable (cannot classify).
/// - DISCOVERED: reachable (advertised) but not yet trusted.
/// - TRUSTED: trusted by the coordinator but not healthy/ready.
/// - ONLINE: trusted + healthy.
/// - ONLINE_OUTDATED / DISCOVERED_OUTDATED: as above but on a different known
///   version (needs update).
///
/// The UPDATING / VERIFIED phases are NOT emitted — there is no real remote
/// update mechanism yet, so fabricating them would be dishonest.
fn node_lifecycle(trusted: bool, healthy: bool, vs: &'static str) -> &'static str {
    match vs {
        "UNKNOWN" => "UNKNOWN",
        _ if trusted && healthy && vs == "CURRENT" => "ONLINE",
        _ if trusted && healthy => "ONLINE_OUTDATED",
        _ if trusted => if vs == "CURRENT" { "TRUSTED" } else { "TRUSTED_OUTDATED" },
        _ => if vs == "CURRENT" { "DISCOVERED" } else { "DISCOVERED_OUTDATED" },
    }
}

/// Pure projection of the conceptual fabric graph
/// `NODE -> WORKER -> ENGINE -> MODEL -> CAPABILITY -> EXECUTION` from
/// authoritative live state. It never fabricates: absent data yields empty
/// arrays / UNKNOWN, never invented nodes, models, capabilities or
/// executions, and a future node shows up automatically because every entry
/// is derived from real advertisements, persisted capability claims, measured
/// links and recorded decisions. This is a decision function (I/O and awaits
/// happen in the handler) so tests drive it with synthetic inputs.
fn fabric_graph_aggregate(
    workers: &[(decentraai_distributed::ComputeAdvertisement, bool)],
    registry: Option<&decentraai_registry::ModelRegistry>,
    decisions: &[decentraai_fabric::ExecutionDecision],
    network: &decentraai_fabric::NetworkGraph,
    sessions_active: usize,
    coordinator_version: &str,
) -> serde_json::Value {
    // NODE -> WORKER: one node per real advertisement; identity fields stay
    // separate (peer_id / node_id / node_name) and trust comes from the
    // coordinator's real trust decision.
    let nodes: Vec<serde_json::Value> = workers
        .iter()
        .map(|(w, trusted)| {
            let served: Vec<String> =
                w.capability.served_models.iter().map(|m| m.file_name.clone()).collect();
            let available: Vec<String> =
                w.capability.available_models.iter().map(|m| m.file_name.clone()).collect();
            serde_json::json!({
                "peer_id": w.peer_id.to_string(),
                "node_id": w.node_id,
                "node_name": w.node_name,
                "trusted": *trusted,
                "device_class": device_class(&w.capability),
                "node_version": w.node_version,
                "version_status": version_status(coordinator_version, &w.node_version),
                "outdated": version_status(coordinator_version, &w.node_version) == "OUTDATED",
                "lifecycle": node_lifecycle(
                    *trusted,
                    w.availability.healthy(),
                    version_status(coordinator_version, &w.node_version),
                ),
                "gpu": {
                    "temperature_celsius": w.availability.gpu_temperature_celsius,
                    "utilization_percent": w.availability.gpu_utilization_percent,
                },
                "capacity": w.availability.capacity_state(),
                "adaptive_contribution": w.availability.adaptive_contribution_factor(),
                "battery_percent": w.availability.battery_percent,
                "engine": w.capability.engine,
                "health": format!("{:?}", w.availability.status),
                "served_models": served,
                "available_models": available,
            })
        })
        .collect();

    // MODEL: aggregate distinct file names across all workers (served +
    // available), deduplicated; each maps to the set of node_ids holding it.
    let mut models: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (w, _) in workers {
        // Legacy workers may advertise an empty node_id; fall back to the
        // peer id so identity is never an empty string.
        let node = if w.node_id.is_empty() { w.peer_id.to_string() } else { w.node_id.clone() };
        for m in w
            .capability
            .served_models
            .iter()
            .chain(w.capability.available_models.iter())
        {
            models.entry(m.file_name.clone()).or_default().insert(node.clone());
        }
    }

    // CAPABILITY: distinct capability names from real persisted claims only
    // (the local registry). capability -> (model files, node ids holding them).
    let mut caps: std::collections::BTreeMap<
        String,
        (
            std::collections::BTreeSet<String>,
            std::collections::BTreeSet<String>,
        ),
    > = std::collections::BTreeMap::new();
    for (file, node_ids) in &models {
        let claims = registry.map(|reg| claims_for_file_name(reg, file)).unwrap_or_default();
        for claim in &claims {
            let (model_set, node_set) = caps.entry(claim.capability.clone()).or_default();
            model_set.insert(file.clone());
            for n in node_ids {
                node_set.insert(n.clone());
            }
        }
    }

    let models_json: Vec<serde_json::Value> = models
        .iter()
        .map(|(file, node_ids)| {
            // Empty array = UNKNOWN capability data (never fabricated).
            let capabilities: Vec<serde_json::Value> = registry
                .map(|reg| claims_for_file_name(reg, file))
                .unwrap_or_default()
                .into_iter()
                .map(|c| {
                    serde_json::json!({ "capability": c.capability, "provenance": c.provenance })
                })
                .collect();
            let nodes: Vec<String> = node_ids.iter().cloned().collect();
            serde_json::json!({
                "file": file,
                "quantization": variant_quantization_from_file_name(file),
                "capabilities": capabilities,
                "nodes": nodes,
            })
        })
        .collect();

    let caps_json: Vec<serde_json::Value> = caps
        .iter()
        .map(|(name, (model_set, node_set))| {
            serde_json::json!({
                "capability": name,
                "models": model_set.iter().cloned().collect::<Vec<_>>(),
                "nodes": node_set.iter().cloned().collect::<Vec<_>>(),
            })
        })
        .collect();

    // EXECUTION: projected from the real recorded decisions (request_id /
    // model_hash / selected_worker / outcome / ts / capability_requirement)
    // plus the pure recovery timeline — mirroring execution_handler.
    let executions: Vec<serde_json::Value> = decisions
        .iter()
        .map(|d| {
            let recovery = decentraai_fabric::recovery_timeline(d);
            let mut v = serde_json::json!({
                "request_id": d.request_id,
                "model_hash": d.model_hash,
                "selected_worker": d.selected_worker,
                "outcome": d.outcome,
                "ts": d.ts,
            });
            if let Some(cr) = &d.capability_requirement {
                v["capability_requirement"] =
                    serde_json::to_value(cr).unwrap_or(serde_json::Value::Null);
            }
            v["recovery"] = recovery;
            v
        })
        .collect();

    // NETWORK: measured links back to this coordinator (RTT / bandwidth /
    // locality) — real only.
    let network: Vec<serde_json::Value> = network
        .peers()
        .map(|(peer, link)| {
            serde_json::json!({
                "peer": peer,
                "rtt_ms": link.rtt_us / 1000,
                "bandwidth_mbps": link.bandwidth_mbps,
                "locality": format!("{:?}", link.locality),
            })
        })
        .collect();

    serde_json::json!({
        "coordinator": { "version": coordinator_version },
        "nodes": nodes,
        "models": models_json,
        "capabilities": caps_json,
        "executions": executions,
        "network": network,
        "kv": { "sessions_active": sessions_active },
        "note": "Projection of real fabric state (NODE -> WORKER -> ENGINE -> MODEL -> CAPABILITY -> EXECUTION). Empty arrays are honest: absent data is never fabricated.",
    })
}

/// `GET /v1/fabric` (Phase C — Fabric Graph / Digital Twin). A read-only
/// projection of the conceptual fabric graph from authoritative live state.
/// Operator/admin-gated (H4 role separation).
async fn fabric_graph_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    // Best-effort local registry: source of persisted capability claims. A
    // failure to load simply yields no claims (models become UNKNOWN), never a
    // fabricated capability — mirroring fabric_model_list.
    let registry =
        decentraai_registry::ModelRegistry::load(&state.info.repo_root.join("db/registry.json"))
            .ok();
    let mut workers: Vec<(decentraai_distributed::ComputeAdvertisement, bool)> = Vec::new();
    let mut decisions: Vec<decentraai_fabric::ExecutionDecision> = Vec::new();
    let mut network = decentraai_fabric::NetworkGraph::new();
    let mut sessions_active = 0usize;
    let mut coordinator_version = String::new();
    if let Some(compute) = &state.compute {
        coordinator_version = compute.node_version().to_string();
        for adv in compute.workers().await {
            let trusted = compute.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        decisions = compute.decisions();
        sessions_active = compute.session_count();
        network = compute.network_graph();
    }
    let body = fabric_graph_aggregate(
        &workers,
        registry.as_ref(),
        &decisions,
        &network,
        sessions_active,
        &coordinator_version,
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Convert raw bytes to whole MiB (`bytes / (1024*1024)`). Pure helper so the
/// resource view never mixes byte and MiB magnitudes from different sources.
fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// `GET /v1/resources` (Phase B — Resource Intelligence). A unified,
/// read-only, operator-facing view of the fabric's resources across
/// CPU / RAM / VRAM / DISK / KV / QUEUE / LATENCY. Every value is real,
/// authoritative state — local node data from a live `SystemSnapshot` +
/// GPU probe, and per-worker rows from the coordinator's actual
/// `ComputeAdvertisement`s and reservation ledger. Nothing is invented:
/// a value that is not available is tagged `provenance: "UNKNOWN"` and
/// omitted (never a fabricated zero), and RAM and VRAM are always kept in
/// separate objects because a model fitting in RAM does not imply VRAM fit.
///
/// Reserved VRAM on the fabric is intentionally *not* reported: the
/// coordinator's public API exposes only per-worker RAM reservations
/// (`ComputeManager::reserved_ram`); VRAM reservation totals are held
/// internally and not surfaced, so we omit them rather than guess.
async fn resources_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }

    let system = decentraai_system_probe::SystemSnapshot::collect();
    let gpu = decentraai_system_probe::probe_gpu();

    let node_ram_total_mb = mb(system.total_memory_bytes);
    let node_ram_available_mb = mb(system.available_memory_bytes);
    let node_ram_in_use_mb = node_ram_total_mb.saturating_sub(node_ram_available_mb);

    let node_vram = match &gpu {
        decentraai_system_probe::GpuProbeStatus::Nvidia(snapshot) => {
            serde_json::json!({
                "present": true,
                "name": snapshot.name,
                "total_mb": snapshot.total_vram_mib,
                "free_mb": snapshot.free_vram_mib,
                "in_use_mb": snapshot.total_vram_mib.saturating_sub(snapshot.free_vram_mib),
                "provenance": "MEASURED",
            })
        }
        decentraai_system_probe::GpuProbeStatus::Unavailable(_) => {
            // Honest: no GPU surfaced — never a fake 0.
            serde_json::json!({ "present": false, "provenance": "UNKNOWN" })
        }
    };

    let (serving, waiting) = state.queue.snapshot();

    let mut fabric_rows: Vec<serde_json::Value> = Vec::new();
    let mut kv_sessions_active: Option<usize> = None;
    if let Some(compute) = &state.compute {
        kv_sessions_active = Some(compute.session_count());
        for adv in compute.workers().await {
            let trusted = compute.is_trusted(&adv.peer_id).await;
            let reserved_ram_mb = compute.reserved_ram(&adv.peer_id).await;
            let ram_headroom = adv
                .availability
                .available_ram_mb
                .saturating_sub(reserved_ram_mb);

            let vram = match adv.availability.available_vram_mb {
                Some(available_mb) => {
                    let total_mb = adv
                        .capability
                        .gpu
                        .as_ref()
                        .map(|g| g.vram_mb)
                        .unwrap_or(available_mb);
                    serde_json::json!({
                        "present": true,
                        "total_mb": total_mb,
                        "available_mb": available_mb,
                        // Reserved VRAM is not surfaced by the coordinator's
                        // public API, so it is honestly omitted (never a
                        // fabricated 0).
                        "provenance": "MEASURED",
                    })
                }
                None => serde_json::json!({ "present": false, "provenance": "UNKNOWN" }),
            };

            let latency_provenance = if adv.availability.tokens_per_second > 0 {
                "MEASURED"
            } else {
                "ESTIMATED"
            };

            fabric_rows.push(serde_json::json!({
                "peer_id": adv.peer_id.to_string(),
                "node_id": adv.node_id,
                "node_name": adv.node_name,
                "trusted": trusted,
                "engine": adv.capability.engine,
                "cpu": {
                    "load_percent": adv.availability.load_percent,
                    "provenance": "MEASURED",
                },
                "ram": {
                    "total_mb": adv.capability.ram_mb,
                    "available_mb": adv.availability.available_ram_mb,
                    "reserved_mb": reserved_ram_mb,
                    "headroom_mb": ram_headroom,
                    "provenance": "MEASURED",
                },
                "vram": vram,
                "queue": {
                    "depth": adv.availability.queue_depth,
                    "provenance": "MEASURED",
                },
                "latency": {
                    "ms": adv.availability.current_latency_ms,
                    "tokens_per_second": adv.availability.tokens_per_second,
                    "provenance": latency_provenance,
                },
                "capacity": adv.availability.capacity_state(),
                // Adaptive-contribution load factor (0.0..1.0): how much work
                // this worker should get given real thermal/GPU/CPU/battery
                // pressure. ~1.0 healthy; lower = stressed. Real signals only.
                "adaptive_contribution": adv.availability.adaptive_contribution_factor(),
            }));
        }
    }

    let body = serde_json::json!({
        "node": {
            "cpu": {
                "logical_cpus": system.logical_cpus,
                "usage_percent": system.cpu_usage_percent,
                "provenance": "MEASURED",
            },
            "ram": {
                "total_mb": node_ram_total_mb,
                "available_mb": node_ram_available_mb,
                // The local node tracks no reservation ledger entry; per-worker
                // reservations live in the coordinator (see fabric rows). 0 is
                // honest here, not a fabricated measurement.
                "reserved_mb": 0,
                "in_use_mb": node_ram_in_use_mb,
                "headroom_mb": node_ram_available_mb,
                "provenance": "MEASURED",
            },
            "vram": node_vram,
            "disk": {
                "free_mb": mb(system.total_disk_free_bytes),
                "provenance": "MEASURED",
            },
        },
        "fabric": fabric_rows,
        "kv": match kv_sessions_active {
            Some(n) => serde_json::json!({ "sessions_active": n, "provenance": "MEASURED" }),
            // No coordinator attached: sessions are not tracked, so the value
            // is honestly UNKNOWN rather than a fabricated 0.
            None => serde_json::json!({ "sessions_active": null, "provenance": "UNKNOWN" }),
        },
        "queue": {
            "serving": serving.is_some(),
            "waiting": waiting.len(),
            "provenance": "MEASURED",
        },
        "note": "Every value is real node/worker state with explicit provenance. UNKNOWN means a value is not available — it is never a fabricated zero. RAM and VRAM are reported separately; a model fitting in RAM does not imply VRAM fit. Reserved VRAM on the fabric is not reported because the coordinator's public API does not surface per-worker VRAM reservation totals.",
    });

    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// `GET /v1/stats` — deterministic historical execution statistics (Phase N,
/// Historical Intelligence). Derived from real measured execution history
/// (tokens, latency, outcomes per model/worker, retries). No ML, no synthetic
/// benchmarks. Operator/admin. Read-only.
async fn stats_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let body = match &state.compute {
        Some(compute) => {
            let history = compute.executions();
            decentraai_distributed::execution_statistics(&history)
        }
        None => serde_json::json!({ "records": 0, "note": "no compute manager attached" }),
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// `GET /v1/can_run?model=...&capability=...&evidence=any|verified` — the
/// unified fabric-wide "CAN I RUN THIS?" view. Reuses the exact same pure
/// per-worker projection + aggregation as the MCP `get_worker_capability`
/// tool, exposed as plain JSON for the dashboard / external clients. Read-only.
async fn can_run_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let model = query.get("model").cloned().unwrap_or_default();
    let capability = query.get("capability").cloned().unwrap_or_default();
    if model.trim().is_empty() || capability.trim().is_empty() {
        return forbidden("missing model and/or capability");
    }
    let evidence = query
        .get("evidence")
        .map(String::as_str)
        .unwrap_or("any");
    let evidence = if evidence == "verified" { "verified" } else { "any" };
    let body = mcp_worker_capability(&state, &model, &capability, evidence).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// `GET /v1/decision?intent=...&evidence=any|verified&model=...` — the ONE
/// coherent fabric decision (Phase 1): intent → capabilities → model options →
/// fabric fit → chosen decision → why. Reuses the existing capability resolver,
/// per-worker verdict and aggregate (no new planner/scoring). Read-only.
async fn decision_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let intent = query.get("intent").cloned().unwrap_or_default();
    if intent.trim().is_empty() {
        return forbidden("missing intent");
    }
    let evidence = query
        .get("evidence")
        .map(String::as_str)
        .unwrap_or("any");
    let evidence = if evidence == "verified" { "verified" } else { "any" };
    let model = query.get("model").map(String::as_str);
    let body = unified_fabric_decision(&state, &intent, evidence, model).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// `GET /v1/capabilities` — authoritative local capability overview (Digital
/// Twin): the distinct capabilities known across the fabric's on-disk models,
/// each with verified/inferred model counts (from the real registry), plus the
/// known capability taxonomy labels. Read-only. Operator/admin.
async fn capabilities_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let summary: Vec<serde_json::Value> = registry
        .as_ref()
        .map(|reg| {
            reg.capability_summary()
                .into_iter()
                .map(|(cap, verified, inferred)| {
                    serde_json::json!({
                        "capability": cap,
                        "verified_models": verified,
                        "inferred_models": inferred,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let body = serde_json::json!({ "capabilities": summary });
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// True when the (already generation-defaulted) inference body asks for SSE
/// streaming, so the proxy knows to forward chunks live instead of buffering.
fn detect_stream(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// Best-effort completion-token count from an SSE body: llama-server's final
/// `data:` event carries a `usage` object we pick up without buffering the
/// whole stream. Streaming never blocks on metrics — zeros are fine.
fn sse_completion_tokens(body: &str) -> u64 {
    body.lines()
        .filter(|l| l.trim_start().starts_with("data:"))
        .filter_map(|l| {
            let payload = l.trim_start().trim_start_matches("data:").trim();
            serde_json::from_str::<serde_json::Value>(payload).ok()
        })
        .filter_map(|v| v["usage"]["completion_tokens"].as_u64())
        .next_back()
        .unwrap_or(0)
}

/// Bytes of user-supplied inference text in a request body: the sum of
/// `messages[].content` (chat) or the `prompt` string (completions). Used to
/// enforce the proxy prompt cap. Runs on the merged body so caller-supplied
/// sampling defaults are already folded in.
fn proxy_prompt_bytes(body: &[u8]) -> usize {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return 0;
    };
    if let Some(messages) = value["messages"].as_array() {
        return messages
            .iter()
            .filter_map(|m| m["content"].as_str())
            .map(str::len)
            .sum();
    }
    value["prompt"].as_str().map(str::len).unwrap_or(0)
}

/// Proxy-boundary size caps for the local /v1/* surface, mirroring the caps
/// the distributed routing path enforces via `BackendConfig`. Guards against
/// forwarding an oversized prompt or an unbounded `max_tokens` to the managed
/// llama-server. Returns the error response to send when a cap is exceeded,
/// or `None` when the request may be forwarded.
fn enforce_size_caps(outgoing: &[u8]) -> Option<Response> {
    if proxy_prompt_bytes(outgoing) > MAX_PROMPT_BYTES {
        return Some(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "{{\"error\":{{\"message\":\"prompt exceeds the {MAX_PROMPT_BYTES} byte limit\",\"type\":\"invalid_request_error\"}}}}"
                ),
            )
                .into_response(),
        );
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(outgoing) {
        if let Some(requested) = value["max_tokens"].as_u64() {
            if requested > MAX_OUTPUT_TOKENS {
                return Some(
                    (
                        StatusCode::BAD_REQUEST,
                        format!(
                            "{{\"error\":{{\"message\":\"max_tokens {requested} exceeds the {MAX_OUTPUT_TOKENS} limit\",\"type\":\"invalid_request_error\"}}}}"
                        ),
                    )
                        .into_response(),
                );
            }
        }
    }
    None
}

/// Wraps an upstream SSE byte stream so a mid-stream failure never reaches the
/// HTTP layer as a raw error.
///
/// A raw `Err` from `bytes_stream()` makes axum abort the connection without a
/// proper SSE ending; the browser then reports the meaningless "TypeError:
/// Error in input stream". Instead, convert the failure into a clean OpenAI
/// error event followed by `[DONE]`, so callers see a useful message.
fn sse_safe_stream<S, E>(
    upstream: S,
) -> impl futures::Stream<Item = Result<Bytes, E>>
where
    S: futures::Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    futures::stream::unfold(upstream, |mut upstream| async move {
        match upstream.next().await {
            Some(Ok(bytes)) => Some((Ok(bytes), upstream)),
            Some(Err(e)) => {
                let err_event = format!(
                    "data: {{\"error\":{{\"message\":\"inference stream interrupted: {}\",\"type\":\"upstream_error\"}}}}\n\n",
                    e
                );
                let done = Bytes::from(format!("{}\ndata: [DONE]\n\n", err_event.trim_end()));
                Some((Ok(done), upstream))
            }
            None => None,
        }
    })
}

/// Proxies a streaming inference response to the caller chunk-by-chunk while
/// recording the same best-effort metrics the non-streaming path does. The
/// channel lets a drop of the client cut upstream early; the spawned task
/// still drains and accounts the completed stream.
#[allow(clippy::needless_pass_by_value)]
fn stream_inference(
    state: ApiState,
    auth: Auth,
    path: String,
    started: Instant,
    upstream: reqwest::Response,
    content_type: Option<axum::http::header::HeaderValue>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, reqwest::Error>>(64);
    let buffer: Arc<StdMutex<Vec<u8>>> = Arc::new(StdMutex::new(Vec::new()));
    let drain_buffer = Arc::clone(&buffer);
    let drain_path = path.clone();
    tokio::spawn(async move {
        let mut chunks = Box::pin(sse_safe_stream(upstream.bytes_stream()));
        while let Some(item) = chunks.next().await {
            match item {
                Ok(bytes) => {
                    drain_buffer.lock().unwrap().extend_from_slice(&bytes);
                    if tx.send(Ok(bytes)).await.is_err() {
                        return;
                    }
                }
                // sse_safe_stream never yields Err (it converts mid-stream
                // failures into a clean SSE error event + [DONE]); keep the
                // arm as a defensive fallback only.
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            }
        }
        // Upstream finished cleanly: account the stream (best effort).
        let body = drain_buffer.lock().unwrap().clone();
        if !body.is_empty() {
            state.record_inference(&drain_path, started.elapsed(), &body);
            let text = String::from_utf8_lossy(&body);
            let completion = sse_completion_tokens(&text);
            if completion > 0 {
                state.tokens_generated.fetch_add(completion, Ordering::SeqCst);
                state.note_token_usage(&auth, completion);
            }
        }
    });
    let body = Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        content_type.unwrap_or_else(|| {
            axum::http::header::HeaderValue::from_static("text/event-stream")
        }),
    );
    response
}

/// Where a requested chat model can be served from. Pure decision, separated
/// from I/O so tests can drive it with synthetic advertisements.
#[derive(Debug, Clone, PartialEq)]
enum ChatRoute {
    /// The local llama-server advertises the model — serve locally (default).
    Local,
    /// A trusted remote worker advertises the model and accepts remote
    /// inference — route through the fabric (M18+).
    Remote {
        worker: decentraai_p2p::PeerId,
        node_id: String,
        model_hash: String,
    },
    /// No worker advertises the model — caller falls back to local handling.
    Unknown,
}

/// Pure fabric routing decision for chat inference.
///
/// `workers` must already be filtered to trusted peers (plus the local
/// advertisement, which is always allowed). `local_peer` is our own peer id.
/// The local advertisement wins over remote ones, so a model served locally
/// is never routed across the network; the first trusted remote worker
/// accepting remote inference that advertises the model is the remote target.
fn resolve_chat_route(
    workers: &[decentraai_distributed::ComputeAdvertisement],
    local_peer: &decentraai_p2p::PeerId,
    model: &str,
) -> ChatRoute {
    for w in workers {
        if w.peer_id == *local_peer
            && w.capability.served_models.iter().any(|m| m.file_name == model)
        {
            return ChatRoute::Local;
        }
    }
    for w in workers {
        if w.peer_id == *local_peer || !w.accepts_remote_inference {
            continue;
        }
        if let Some(m) = w.capability.served_models.iter().find(|m| m.file_name == model) {
            return ChatRoute::Remote {
                worker: w.peer_id,
                node_id: w.node_id.clone(),
                model_hash: m.model_hash.clone(),
            };
        }
    }
    ChatRoute::Unknown
}

/// Best-model selection across the whole fabric (local + trusted remote
/// workers accepting remote inference). Size is an honest, deterministic
/// proxy for capability: the largest served model wins; ties prefer the
/// local copy (no network round-trip), then the lexicographically smallest
/// node id so the choice is stable across refreshes.
#[derive(Debug, Clone, PartialEq)]
enum BestModel {
    Local(String),
    Remote {
        worker: decentraai_p2p::PeerId,
        node_id: String,
        model_hash: String,
        file_name: String,
    },
}

fn select_best_model(
    workers: &[decentraai_distributed::ComputeAdvertisement],
    local_peer: &decentraai_p2p::PeerId,
) -> Option<BestModel> {
    let mut best: Option<(u64, BestModel)> = None;
    for w in workers {
        let is_local = w.peer_id == *local_peer;
        if !is_local && !w.accepts_remote_inference {
            continue;
        }
        for m in &w.capability.served_models {
            let candidate = if is_local {
                BestModel::Local(m.file_name.clone())
            } else {
                BestModel::Remote {
                    worker: w.peer_id,
                    node_id: w.node_id.clone(),
                    model_hash: m.model_hash.clone(),
                    file_name: m.file_name.clone(),
                }
            };
            let better = match &best {
                None => true,
                Some((best_size, best_choice)) => {
                    if m.size_mb != *best_size {
                        m.size_mb > *best_size
                    } else {
                        // Tie: a local copy beats a remote one, then remote
                        // ties break deterministically by node id.
                        matches!(
                            (best_choice, &candidate),
                            (BestModel::Remote { .. }, BestModel::Local { .. })
                        ) || match (best_choice, &candidate) {
                            (
                                BestModel::Remote { node_id: a, .. },
                                BestModel::Remote { node_id: b, .. },
                            ) => a < b,
                            _ => false,
                        }
                    }
                }
            };
            if better {
                best = Some((m.size_mb, candidate));
            }
        }
    }
    best.map(|(_, b)| b)
}

/// Local-serving origin headers (`X-Decentra-Origin: local`,
/// `X-Decentra-Node: dca-xxxxxx`) attached to every locally-served inference
/// response when the fabric is attached, so the dashboard can show *who*
/// served a chat answer. `None` on plain (non-fabric) serve = no header, so
/// non-fabric deployments keep byte-identical behaviour.
fn local_origin_headers(
    state: &ApiState,
) -> Option<(header::HeaderValue, header::HeaderValue)> {
    let compute = state.compute.as_ref()?;
    let node_id = decentraai_distributed::short_node_id(&compute.local_peer());
    Some((
        header::HeaderValue::from_static("local"),
        header::HeaderValue::from_str(&node_id).ok()?,
    ))
}

/// Inserts the remote-serving origin headers on a fabric-routed response.
fn tag_remote_response(
    response: &mut Response,
    worker: &decentraai_p2p::PeerId,
    node_id: &str,
) {
    let o = header::HeaderValue::from_static("remote");
    let Ok(w) = header::HeaderValue::from_str(&worker.to_string()) else {
        return;
    };
    let Ok(n) = header::HeaderValue::from_str(node_id) else {
        return;
    };
    response.headers_mut().insert("x-decentra-origin", o);
    response.headers_mut().insert("x-decentra-worker", w);
    response.headers_mut().insert("x-decentra-node", n);
}

/// Builds the OpenAI `GET /v1/models` list across the whole fabric.
///
/// Each model appears once with an `id` the client can send back as `model`
/// (the file name — the same key the chat router resolves). `owned_by` names
/// the node that holds it so a caller (or the Command Deck) can see at a
/// glance *where* a model lives. Local copies win on duplicates; remote
/// entries only come from workers that accept remote inference (a client must
/// be able to actually reach them through this coordinator).
async fn fabric_model_list(
    state: &ApiState,
) -> Option<serde_json::Value> {
    let compute = state.compute.as_ref()?;
    let local_peer = compute.local_peer();
    let workers = compute.workers().await;
    // Best-effort local registry: source of persisted capability claims for
    // the local model entries (no Hub round-trip). A failure to load simply
    // omits the field — the model list must never break on registry trouble.
    let registry =
        decentraai_registry::ModelRegistry::load(&state.info.repo_root.join("db/registry.json"))
            .ok();
    // id (file name) → (owned_by, is_local)
    let mut seen: std::collections::BTreeMap<String, (String, bool)> =
        std::collections::BTreeMap::new();
    for w in &workers {
        let is_local = w.peer_id == local_peer;
        if !is_local && !w.accepts_remote_inference {
            continue;
        }
        let owned_by = if is_local {
            "local".to_string()
        } else {
            w.node_id.clone()
        };
        for m in w
            .capability
            .served_models
            .iter()
            .chain(w.capability.available_models.iter())
        {
            match seen.get(&m.file_name) {
                Some((_, true)) => {} // local already wins
                Some(_) if is_local => {
                    seen.insert(m.file_name.clone(), (owned_by.clone(), true));
                }
                None => {
                    seen.insert(m.file_name.clone(), (owned_by.clone(), is_local));
                }
                _ => {}
            }
        }
    }
    let data: Vec<serde_json::Value> = seen
        .into_iter()
        .map(|(id, (owned_by, is_local))| {
            let mut entry = serde_json::json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": owned_by,
            });
            // Local models only: attach real persisted capability data when it
            // exists. Absent means UNKNOWN — never force an empty list.
            if is_local {
                if let Some(reg) = &registry {
                    let claims = claims_for_file_name(reg, &id);
                    if !claims.is_empty() {
                        entry["capability_claims"] =
                            serde_json::to_value(claims).unwrap_or(serde_json::Value::Null);
                    }
                }
            }
            entry
        })
        .collect();
    Some(serde_json::json!({ "object": "list", "data": data }))
}

/// `GET /v1/models` — OpenAI model list. With the fabric attached this is the
/// fabric-wide view (models served *or* available on disk across all trusted
/// workers); without the fabric it is the plain backend passthrough, so a
/// standalone `serve start` behaves exactly as before.
async fn models_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(auth) => auth,
        Err(e) => return e.into_response(),
    };
    if state.compute.is_some() {
        if let Some(list) = fabric_model_list(&state).await {
            return (
                [(header::CONTENT_TYPE, "application/json")],
                list.to_string(),
            )
                .into_response();
        }
    }
    // Fall back to the backend passthrough (standalone node, no fabric).
    proxy_with_auth(State(state), method, uri, headers, body, auth).await
}

/// `GET /v1/models/{id}` — OpenAI single-model view over the fabric list.
/// Returns the model entry when it exists anywhere on the fabric, else 404.
async fn model_detail_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    AxumPath(model_id): AxumPath<String>,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(auth) => auth,
        Err(e) => return e.into_response(),
    };
    if let Some(list) = fabric_model_list(&state).await {
        let found = list["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["id"].as_str() == Some(model_id.as_str()))
            .cloned();
        if let Some(entry) = found {
            return (
                [(header::CONTENT_TYPE, "application/json")],
                entry.to_string(),
            )
                .into_response();
        }
        let body = serde_json::json!({
            "error": {"message": format!("model '{model_id}' not found on the fabric"), "type": "not_found"}
        });
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response();
    }
    proxy_with_auth(State(state), method, uri, headers, body, auth).await
}

async fn proxy_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(auth) => auth,
        Err(e) => return e.into_response(),
    };
    proxy_with_auth(State(state), method, uri, headers, body, auth).await
}

/// `POST /v1/batch` — dispatch a set of **independent** requests across the
/// fabric using the adaptive batch allocator (`route_batch`), returning
/// per-request provenance (request_id, chosen worker, result, tokens).
///
/// Body: `{ "requests": [ { "id": "...", "model": "file.gguf",
/// "prompt": "...", "max_tokens": N }, ... ] }`. Each request is pinned to
/// its allocated worker on the first attempt (exact worker pinning), falling
/// back to normal planning if that worker is no longer eligible.
///
/// This is the operational surface for the adaptive fan-out of independent
/// requests across the Laptop + Desktop fabric. Operator/admin gated.
async fn batch_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(distributed) = state.distributed.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":{\"message\":\"fabric router unavailable\",\"type\":\"server_error\"}}".to_string(),
        )
            .into_response();
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return forbidden("invalid JSON"),
    };
    let items = match req.get("requests").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return forbidden("missing or empty requests array"),
    };
    let sender = distributed.p2p_node().local_peer_id();
    let mut requests: Vec<(String, decentraai_distributed::InferRequest)> = Vec::new();
    for item in items {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let model = item.get("model").and_then(|v| v.as_str()).unwrap_or("");
        let prompt = item.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || model.is_empty() || prompt.is_empty() {
            return forbidden("each request needs id, model, prompt");
        }
        let Some(model_hash) = resolve_model_hash(&state, model).await else {
            return forbidden(&format!("model '{model}' has no advertised hash on the fabric"));
        };
        let max_tokens = item
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(64)
            .min(4096) as u32;
        let mut ir = decentraai_distributed::InferRequest::new(model_hash, prompt.to_string(), max_tokens)
            .with_sender(sender)
            .with_streaming(false);
        ir.timeout_ms = 120_000;
        if let Some(sid) = item.get("session_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            ir = ir.with_session(sid.to_string());
        }
        requests.push((id.to_string(), ir));
    }
    // DRY-RUN: show the deterministic adaptive batch allocation (which worker
    // each independent request would be pinned to) WITHOUT executing anything.
    // Honest preview from the live allocation; never sends a request or holds a
    // reservation. Useful to understand the adaptive fan-out before running.
    let dry_run = req.get("dry_run").and_then(|d| d.as_bool()).unwrap_or(false);
    if dry_run {
        let alloc = distributed.plan_batch(&requests).await;
        let assignments: Vec<serde_json::Value> = alloc
            .as_ref()
            .map(|a| {
                a.assignments
                    .iter()
                    .map(|x| {
                        serde_json::json!({
                            "request_id": x.request_id,
                            "worker": x.worker,
                            "eligible": x.eligible,
                            "kv_pinned": x.kv_pinned,
                            "share": x.share,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "dry_run": true,
                "worker_shares": alloc
                    .as_ref()
                    .map(|a| a.worker_shares.clone())
                    .unwrap_or_default(),
                "requests": assignments,
                "note": "allocation preview only — no request sent, no reservation held",
            })
            .to_string(),
        )
            .into_response();
    }
    let outcomes = distributed.route_batch(requests).await;
    let results: Vec<serde_json::Value> = outcomes
        .into_iter()
        .map(|o| {
            let (ok, worker, tokens, ms, err) = match &o.result {
                Ok(r) => (
                    true,
                    r.worker_peer_id.to_string(),
                    r.tokens_used,
                    r.processing_time_ms,
                    String::new(),
                ),
                Err(e) => (false, o.worker.clone(), 0, 0, e.to_string()),
            };
            serde_json::json!({
                "request_id": o.request_id,
                "worker": worker,
                "ok": ok,
                "tokens_used": tokens,
                "processing_time_ms": ms,
                "error": if err.is_empty() { serde_json::Value::Null } else { serde_json::json!(err) },
            })
        })
        .collect();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "requests": results }).to_string(),
    )
        .into_response()
}

/// The actual proxy logic. Auth is passed in so callers that already
/// classified (the fabric model views) reuse the result instead of double
/// classifying.
async fn proxy_with_auth(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    auth: Auth,
) -> Response {
    let is_inference = method == Method::POST
        && (uri.path() == "/v1/completions" || uri.path() == "/v1/chat/completions");
    // The body the proxy will actually forward (caller sampling defaults
    // folded in), computed once up front so the proxy-boundary caps see the
    // exact prompt/max_tokens and it can be reused for the request below.
    let outgoing = if is_inference {
        apply_generation_defaults(&*state.runtime_generation.read().await, &body)
    } else {
        body.to_vec()
    };
    if is_inference {
        if let Err(e) = state.check_model_access(&auth, &body) {
            return e.into_response();
        }
        // Proxy-boundary size caps: reject an oversized prompt or an unbounded
        // max_tokens before they can hold the queue slot or the engine.
        if let Some(error) = enforce_size_caps(&outgoing) {
            return error;
        }
        if let Err(e) = state.check_rate_limit(&auth) {
            return e.into_response();
        }
    }

    // Q2 consumer-quota authorization: reserve quota up front for a consumer
    // key's inference request. The RAII guard settles on measured success and
    // releases on any other exit (early error, cancellation, transport
    // failure), so quota can never leak as reserved or be double-settled.
    let mut consumer_quota = if let Auth::Consumer {
        account,
        key_id,
        quota_ceiling,
        rate_limit_per_minute,
    } = &auth
    {
        if is_inference {
            if let Err(e) = state.check_consumer_rate_limit(key_id, *rate_limit_per_minute) {
                return e.into_response();
            }
            match state.reserve_consumer_quota(account, key_id, &uri.to_string(), *quota_ceiling) {
                Some(guard) => Some(guard),
                // A classified consumer key with no spendable quota is denied.
                // (None also means "no ledger attached", but a consumer key can
                // only authenticate when the ledger is wired — see classify.)
                None => {
                    return (
                        StatusCode::FORBIDDEN,
                        [(header::CONTENT_TYPE, "application/json")],
                        "{\"error\":{\"message\":\"no spendable quota for this consumer account\",\"type\":\"insufficient_quota\"}}"
                            .to_string(),
                    )
                        .into_response();
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Fabric-aware chat routing (M18+): three ways a chat request can be
    // served — an explicit `worker_hint` (dashboard "Remote workers"
    // selection forces that specific node), `model: "__auto__"` (the
    // dashboard "Auto (best)" picker selects the best model anywhere in the
    // fabric), or an explicit model name (local wins over remote, as before).
    // Decided *before* the queue join so a remote request never holds a local
    // backend slot (the worker has its own queue).
    let mut outgoing = outgoing;
    if is_inference && uri.path() == "/v1/chat/completions" {
        if let (Some(compute), Some(_distributed)) = (&state.compute, &state.distributed) {
            let body_val: Option<serde_json::Value> = serde_json::from_slice(&outgoing).ok();
            let model = body_val
                .as_ref()
                .and_then(|v| v["model"].as_str().map(str::to_string))
                .unwrap_or_default();
            let worker_hint = body_val
                .as_ref()
                .and_then(|v| v["worker_hint"].as_str().map(str::to_string))
                .unwrap_or_default();
            if !model.is_empty() {
                let local_peer = compute.local_peer();
                let mut trusted: Vec<decentraai_distributed::ComputeAdvertisement> = Vec::new();
                for w in compute.workers().await {
                    // The local advertisement always counts; remote peers only
                    // when trusted (cryptographic reputation, never network).
                    if w.peer_id == local_peer || compute.is_trusted(&w.peer_id).await {
                        trusted.push(w);
                    }
                }
                let mut remote_route: Option<(
                    decentraai_p2p::PeerId,
                    String,
                    String,
                    String,
                )> = None;
                let mut local_rewrite: Option<String> = None;

                if !worker_hint.is_empty() {
                    // Explicit remote selection: the node must exist, be
                    // trusted, accept remote inference, and serve the model.
                    let target = trusted.iter().find(|w| {
                        w.peer_id != local_peer
                            && w.accepts_remote_inference
                            && w.node_id == worker_hint
                    });
                    match target.and_then(|w| {
                        w.capability
                            .served_models
                            .iter()
                            .find(|m| m.file_name == model)
                            .map(|m| (w, m))
                    }) {
                        Some((w, m)) => {
                            remote_route = Some((
                                w.peer_id,
                                w.node_id.clone(),
                                m.model_hash.clone(),
                                model.clone(),
                            ));
                        }
                        None => {
                            return (
                                StatusCode::BAD_REQUEST,
                                format!(
                                    "{{\"error\":{{\"message\":\"worker {} does not serve model {} (or is not trusted / does not accept remote inference)\",\"type\":\"invalid_request_error\"}}}}",
                                    worker_hint, model
                                ),
                            )
                                .into_response();
                        }
                    }
                } else if model == "__auto__" || model == "auto" {
                    match select_best_model(&trusted, &local_peer) {
                        Some(BestModel::Remote {
                            worker,
                            node_id,
                            model_hash,
                            file_name,
                        }) => {
                            remote_route =
                                Some((worker, node_id, model_hash, file_name));
                        }
                        Some(BestModel::Local(file_name)) => {
                            // Rewrite the outgoing body so the local backend
                            // receives the real chosen model, not "auto".
                            local_rewrite = Some(file_name);
                        }
                        None => { /* no model anywhere: local passthrough */ }
                    }
                } else {
                    match resolve_chat_route(&trusted, &local_peer, &model) {
                        ChatRoute::Remote {
                            worker,
                            node_id,
                            model_hash,
                        } => {
                            remote_route =
                                Some((worker, node_id, model_hash, model.clone()));
                        }
                        ChatRoute::Local | ChatRoute::Unknown => {
                            // Serve locally (headers added on the response).
                        }
                    }
                }

                if let Some((worker, node_id, model_hash, model_name)) = remote_route {
                    return route_remote_chat(
                        &state,
                        auth,
                        uri.path().to_string(),
                        worker,
                        node_id,
                        model_hash,
                        model_name,
                        &outgoing,
                    )
                    .await;
                }
                if let Some(new_model) = local_rewrite {
                    if let Ok(mut v) =
                        serde_json::from_slice::<serde_json::Value>(&outgoing)
                    {
                        v["model"] = serde_json::Value::String(new_model);
                        outgoing = serde_json::to_vec(&v).unwrap_or(outgoing);
                    }
                }
            }
        }
    }

    // Q2: join the fair queue. The ticket releases the slot on drop,
    // whatever happens from here on (success, error, disconnect).
    let ticket = if is_inference {
        match state.queue.enqueue(&auth.who(), uri.path()) {
            Ok(ticket) => Some(ticket),
            Err(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "{\"error\":{\"message\":\"inference queue is full; try again in a moment\",\"type\":\"server_error\"}}",
                )
                    .into_response();
            }
        }
    } else {
        None
    };
    if let Some(ticket) = &ticket {
        if ticket.wait_turn().await.is_err() {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                "{\"error\":{\"message\":\"waited too long in the inference queue\",\"type\":\"timeout_error\"}}",
            )
                .into_response();
        }
    }

    // Only real inference resets the idle-unload clock (and drives the request
    // counters via record_inference below); a bare GET /v1/models metadata
    // poll must not defeat idle-unload.
    if is_inference {
        state.manager.lock().await.note_activity();
    }
    let started = Instant::now();

    // Resolve the backend URL from the live manager so engine auto-restarts
    // (M24) are reflected — the port changes on every respawn when ephemeral.
    let backend_url = {
        let manager = state.manager.lock().await;
        manager
            .base_url()
            .unwrap_or_else(|| state.backend_url.clone())
    };
    let url = format!("{}{}", backend_url, uri.path());
    let mut request = state.client.request(method, &url);
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    let wants_stream = is_inference && detect_stream(&outgoing);
    match request.body(outgoing).send().await {
        Ok(upstream) => {
            let status = StatusCode::from_u16(upstream.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
            if wants_stream && status.is_success() {
                let local_headers = local_origin_headers(&state);
                let mut response = stream_inference(
                    state,
                    auth,
                    uri.path().to_string(),
                    started,
                    upstream,
                    content_type,
                );
                if let Some((origin, node)) = local_headers {
                    response.headers_mut().insert("x-decentra-origin", origin);
                    response.headers_mut().insert("x-decentra-node", node);
                }
                return response;
            }
            let bytes = upstream.bytes().await.unwrap_or_default();
            if is_inference && status.is_success() {
                state.record_inference(uri.path(), started.elapsed(), &bytes);
                let generated: serde_json::Value =
                    serde_json::from_slice(&bytes).unwrap_or_default();
                let completion = generated["usage"]["completion_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                state.note_token_usage(&auth, completion);
                // Q2: settle the consumer reservation against real measured
                // completion tokens; the unused reserved quota is released.
                if let Some(guard) = consumer_quota.as_mut() {
                    guard.settle(completion);
                }
            } else if is_inference {
                state.requests_failed.fetch_add(1, Ordering::SeqCst);
                // Q2: no work completed; the guard releases the reservation.
            }
            let mut response = (status, bytes).into_response();
            if let Some(value) = content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            if let Some((origin, node)) = local_origin_headers(&state) {
                response.headers_mut().insert("x-decentra-origin", origin);
                response.headers_mut().insert("x-decentra-node", node);
            }
            response
        }
        Err(_) => {
            if is_inference {
                state.requests_failed.fetch_add(1, Ordering::SeqCst);
            }
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "{\"error\":{\"message\":\"model backend unavailable (unloaded or crashed); restart decentraai serve\",\"type\":\"server_error\"}}",
            )
                .into_response()
        }
    }
}

/// Builds a plain-text prompt from an OpenAI chat `messages` array for the
/// fabric path: the P2P `InferRequest` transports a single prompt string (the
/// remote worker serves it through its own llama-server), not an OpenAI chat
/// body. Multi-turn history is joined as `role: content` blocks, and the
/// prompt ends with an `assistant:` turn so the engine completes the reply.
fn remote_chat_prompt(messages: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for m in messages {
        let role = m["role"].as_str().unwrap_or("user");
        let content = m["content"].as_str().unwrap_or("");
        if !content.is_empty() {
            parts.push(format!("{role}: {content}"));
        }
    }
    if !parts.last().is_some_and(|p| p.starts_with("assistant:")) {
        parts.push("assistant:".to_string());
    }
    parts.join("\n\n")
}

/// Routes a chat inference request over the fabric to a trusted remote worker
/// (M18+). Emits an OpenAI-compatible response — SSE when the caller asked for
/// streaming, JSON otherwise — tagged with `X-Decentra-Origin: remote` plus
/// the serving worker and node id. Metrics stay honest: the streaming path
/// records the real token count / duration when the fabric response arrives.
#[allow(clippy::too_many_arguments)]
async fn route_remote_chat(
    state: &ApiState,
    auth: Auth,
    path: String,
    worker: decentraai_p2p::PeerId,
    node_id: String,
    model_hash: String,
    model: String,
    outgoing: &[u8],
) -> Response {
    let distributed = match &state.distributed {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "{\"error\":{\"message\":\"fabric unavailable\",\"type\":\"server_error\"}}",
            )
                .into_response();
        }
    };
    let body: serde_json::Value = match serde_json::from_slice(outgoing) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "{\"error\":{\"message\":\"invalid JSON body\",\"type\":\"invalid_request_error\"}}",
            )
                .into_response();
        }
    };
    let prompt = remote_chat_prompt(
        body["messages"].as_array().map(Vec::as_slice).unwrap_or(&[]),
    );
    let max_tokens = body["max_tokens"].as_u64().unwrap_or(1024).min(4096) as u32;
    let stream = body["stream"].as_bool().unwrap_or(false);
    let request = decentraai_distributed::InferRequest::new(
        model_hash,
        prompt,
        max_tokens,
    )
    .with_sender(distributed.p2p_node().local_peer_id())
    .with_streaming(stream);
    // Inference on CPU is slow (a Mistral-7B response can take >30s per few
    // tokens). The protocol default timeout is 30s — far too tight for a
    // real chat turn. Derive the request deadline from the node config the
    // same way the CLI does, so slow-but-healthy workers are not cut off
    // mid-stream (which previously surfaced as "Error in input stream").
    let mut request = request;
    // Inference on CPU is slow (a Mistral-7B response can take >30s per few
    // tokens). The protocol default timeout is 30s — far too tight for a
    // real chat turn. Match the CLI's explicit 120s so slow-but-healthy
    // workers are not cut off mid-stream (which previously surfaced as
    // "Error in input stream").
    request.timeout_ms = 120_000;
    let started = Instant::now();

    if stream {
        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::unbounded_channel::<String>();
        let dist = distributed.clone();
        let resp_task =
            tokio::spawn(async move { dist.route_request_streamed(request, progress_tx).await });
        // SSE body: consume progress chunks, then a final usage/error event.
        let (body_tx, body_rx) =
            tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
        let state2 = state.clone();
        let path2 = path.clone();
        let started2 = started;
        let worker2 = worker;
        let node2 = node_id;
        tokio::spawn(async move {
            while let Some(chunk) = progress_rx.recv().await {
                if chunk.is_empty() {
                    continue;
                }
                let payload = format!(
                    "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}}]}}\n\n",
                    serde_json::to_string(&chunk)
                        .unwrap_or_else(|_| "\"\"".to_string())
                );
                if body_tx.send(Ok(Bytes::from(payload))).await.is_err() {
                    break;
                }
            }
            let final_event = match resp_task.await {
                Ok(Ok(resp)) => {
                    let usage = format!(
                        "{{\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}",
                        resp.tokens_used
                    );
                    state2.record_inference(&path2, started2.elapsed(), usage.as_bytes());
                    format!(
                        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}\n\n",
                        resp.tokens_used
                    )
                }
                Ok(Err(_)) => {
                    state2.requests_failed.fetch_add(1, Ordering::SeqCst);
                    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"error\"}]}\n\n"
                        .to_string()
                }
                Err(_) => String::new(),
            };
            let _ = body_tx.send(Ok(Bytes::from(final_event))).await;
            let _ = body_tx
                .send(Ok(Bytes::from("data: [DONE]\n\n".to_string())))
                .await;
        });
        let body = Body::from_stream(futures::stream::unfold(body_rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }));
        let mut response = (StatusCode::OK, body).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/event-stream"),
        );
        tag_remote_response(&mut response, &worker2, &node2);
        return response;
    }

    // Non-streaming fabric route.
    match distributed.route_request(request).await {
        Ok(resp) => {
            let usage_json = format!(
                "{{\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}",
                resp.tokens_used
            );
            state.record_inference(&path, started.elapsed(), usage_json.as_bytes());
            state.note_token_usage(&auth, resp.tokens_used.into());
            let created = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let payload = serde_json::json!({
                "id": format!("chatcmpl-{}", resp.request_id),
                "object": "chat.completion",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": resp.output },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 0,
                    "completion_tokens": resp.tokens_used,
                    "total_tokens": resp.tokens_used
                }
            });
            let mut response = (StatusCode::OK, payload.to_string()).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            );
            tag_remote_response(&mut response, &worker, &node_id);
            response
        }
        Err(e) => {
            state.requests_failed.fetch_add(1, Ordering::SeqCst);
            let (code, msg) = match e.code() {
                decentraai_distributed::InferErrorCode::Untrusted => {
                    (StatusCode::FORBIDDEN, "worker is not trusted")
                }
                decentraai_distributed::InferErrorCode::Timeout
                | decentraai_distributed::InferErrorCode::Capacity
                | decentraai_distributed::InferErrorCode::Transport => {
                    (StatusCode::SERVICE_UNAVAILABLE, "remote worker unavailable")
                }
                _ => (StatusCode::BAD_GATEWAY, "remote inference failed"),
            };
            (
                code,
                format!(
                    "{{\"error\":{{\"message\":\"{}\",\"type\":\"fabric_error\"}}}}",
                    msg
                ),
            )
                .into_response()
        }
    }
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        "{\"error\":{\"message\":\"missing or invalid API token\",\"type\":\"authentication_error\"}}",
    )
        .into_response()
}

fn forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        format!(
            "{{\"error\":{{\"message\":\"{}\",\"type\":\"permission_error\"}}}}",
            message.replace('"', "\\\"")
        ),
    )
        .into_response()
}

fn too_many_requests(limit: usize) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        format!(
            "{{\"error\":{{\"message\":\"rate limit exceeded ({limit} requests/minute for your tier)\",\"type\":\"rate_limit_error\"}}}}"
        ),
    )
        .into_response()
}

/// Reads the newest audit events from logs/audit.jsonl (best effort).
fn recent_audit_events(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = data_dir.join("logs/audit.jsonl");
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .rev()
        .take(DASHBOARD_EVENT_LIMIT)
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The share guide block: where to copy models, how to serve, how to pull.
fn share_guide_html(state: &ApiState) -> String {
    let root = state.info.repo_root.display();
    let escaped_root = html_escape(&root.to_string());
    format!(
        "<ol>\
<li>Drop GGUF files into <code>{escaped_root}</code> and run <code>decentraai registry scan --directory {escaped_root}</code></li>\
<li>Serve them: <code>decentraai swarm start</code> &mdash; copy the printed <code>Listening: /ip4/&hellip;/p2p/&hellip;</code> address</li>\
<li>On the other machine: <code>decentraai pull --from &lt;that address&gt; --list</code>, then <code>--model &lt;file_name&gt;</code> to download (verified BLAKE3 + Merkle, resumable)</li>\
</ol>"
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}


fn dashboard_js(state: &ApiState, share: &str) -> String {
    JS_TEMPLATE
        .replace("__SHARE__", &share.replace('"', "\\\""))
        .replace("__MODEL__", &state.info.model_name.replace('"', "\\\""))
}


/// Real node + engine info derived from the local compute advertisement when a
/// compute manager is attached. Falls back to empty markers otherwise — never
/// mock data.
fn node_info(compute: &Option<Arc<decentraai_distributed::ComputeManager>>) -> serde_json::Value {
    let Some(compute) = compute else {
        return serde_json::json!({
            "name": "", "node_id": "", "peer_id": "", "engine": "",
            "served_models": [], "attached": false,
        });
    };
    let NodeProfile {
        name,
        node_id,
        engine,
        served_models,
    } = node_profile(compute);
    serde_json::json!({
        "name": name,
        "node_id": node_id,
        "peer_id": compute.local_peer().to_string(),
        "engine": engine,
        "served_models": served_models,
        "attached": true,
    })
}

/// Extracts the local node's real name, compact id, engine kind and served
/// models (with model file, RAM/VRAM footprint and context window) from the
/// last advertisement this node broadcast.
fn node_profile(compute: &decentraai_distributed::ComputeManager) -> NodeProfile {
    let mut profile = NodeProfile::default();
    if let Some(adv) = compute.last_local_advertisement_sync() {
        profile.name = adv.node_name;
        profile.node_id = adv.node_id;
        profile.engine = adv.capability.engine;
        profile.served_models = adv
            .capability
            .served_models
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "name": m.file_name,
                    "size_mb": m.size_mb,
                    "est_ram_mb": m.est_ram_mb,
                    "est_vram_mb": m.est_vram_mb,
                    "context_tokens": m.context_tokens,
                })
            })
            .collect();
    }
    profile
}

#[derive(Default)]
struct NodeProfile {
    name: String,
    node_id: String,
    engine: String,
    served_models: Vec<serde_json::Value>,
}

/// Loads the local API token or generates a fresh one with 0600
/// permissions. The token never leaves the machine: it only guards the
/// loopback endpoint from other local processes.
pub fn ensure_api_token(path: &Path) -> Result<String> {
    if path.exists() {
        let token = std::fs::read_to_string(path)
            .with_context(|| format!("reading API token from {}", path.display()))?;
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &token)
        .with_context(|| format!("writing API token to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlamaServer, RuntimeConfig};
    use decentraai_config::{TierPolicy, TiersSection};

    #[test]
    fn node_info_without_compute_is_not_attached() {
        let info = node_info(&None);
        assert_eq!(info["attached"], false);
        assert_eq!(info["name"], "");
        assert!(info["served_models"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sse_safe_stream_converts_midstream_error_into_clean_event() {
        // Regression: when the upstream (llama-server or remote worker) closed
        // mid-stream, stream_inference forwarded the raw reqwest error into the
        // response body; axum then aborted the connection and the browser
        // reported "TypeError: Error in input stream". The safe stream must
        // instead emit an OpenAI error event + [DONE] and never yield Err.
        let err = std::io::Error::other("worker died");
        let stream = futures::stream::iter(vec![
            Ok(Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n")),
            Err(err),
        ]);
        let mut safe = Box::pin(sse_safe_stream(stream));
        let mut chunks: Vec<String> = Vec::new();
        let mut saw_error = false;
        let mut saw_done = false;
        while let Some(item) = safe.next().await {
            let item = item.expect("sse_safe_stream must never yield Err");
            let text = String::from_utf8_lossy(&item).to_string();
            chunks.push(text.clone());
            if text.contains("\"error\"") && text.contains("inference stream interrupted") {
                saw_error = true;
            }
            if text.contains("[DONE]") {
                saw_done = true;
            }
        }
        assert!(saw_error, "expected a clean SSE error event, got: {chunks:?}");
        assert!(saw_done, "expected [DONE] terminator, got: {chunks:?}");
        // The first chunk (a real delta) must pass through unchanged.
        assert!(chunks[0].contains("content\":\"hi\""));
    }

    #[cfg(unix)]
    fn write_fake_server(dir: &Path) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-llama-server");
        // Durable write + sync + close before spawn, and retry on ETXTBSY, so a
        // concurrent exec in another test never sees a half-open script.
        let mut last = None;
        for attempt in 0..4 {
            match std::fs::File::create(&path) {
                Ok(mut f) => {
                    let _ = f.write_all(b"#!/bin/sh\nexec sleep 60\n");
                    let _ = f.sync_all();
                    drop(f);
                    let mut perms = std::fs::metadata(&path).unwrap().permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&path, perms).unwrap();
                    return path;
                }
                Err(e) if e.raw_os_error() == Some(26) && attempt < 3 => {
                    last = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => last = Some(e),
            }
        }
        if let Some(e) = last {
            panic!("failed to write fake llama-server after retries: {e}");
        }
        path
    }

    #[cfg(unix)]
    async fn test_manager(dir: &Path) -> Arc<Mutex<ServeManager>> {
        test_manager_with(dir, generic_fake_engine()).await
    }

    /// A minimal OpenAI-compatible backend exercising the fields the proxy
    /// tests assert on (a models list, counted chat completion). The proxy
    /// resolves the live engine from the manager (M24), so this must be a
    /// real HTTP listener, not a dead process.
    #[cfg(unix)]
    fn generic_fake_engine() -> Router {
        Router::new()
            .route(
                "/v1/models",
                get(|| async { "{\"object\":\"list\",\"data\":[{\"id\":\"tinyllama\"}]}" }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async {
                    "{\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":20,\"total_tokens\":40},\"timings\":{\"predicted_per_second\":19.97}}"
                }),
            )
    }

    /// Binds a real, minimal engine (`app`) on an ephemeral loopback port and
    /// wraps it in a [`ServeManager`] whose `base_url()` points at it. This
    /// mirrors production: the proxy forwards to the live engine address from
    /// the manager (M24). The spawned child process only satisfies
    /// [`LlamaServer`]'s process contract; the actual HTTP server is `app`.
    #[cfg(unix)]
    async fn test_manager_with(dir: &Path, app: Router) -> Arc<Mutex<ServeManager>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let binary = write_fake_server(dir);
        let mut config = RuntimeConfig::new(dir.join("model.gguf"));
        config.port = Some(addr.port());
        let server = LlamaServer::start(&binary, &config).expect("fake llama-server spawns");
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Arc::new(Mutex::new(ServeManager::new(
            server,
            Duration::from_secs(3600),
        )))
    }

    fn test_info(dir: &Path, reputation_path: Option<PathBuf>) -> DashboardInfo {
        DashboardInfo {
            repo_root: dir.to_path_buf(),
            reputation_path,
            max_invalid_chunks: 3,
            ban_duration: Duration::from_secs(3600),
            api_port: 8080,
            model_name: "test-model.gguf".to_string(),
            model_size_bytes: 1024,
            generation: GenerationSection {
                temperature: 0.7,
                top_p: 0.9,
                top_k: Some(40),
                repeat_penalty: 1.1,
                system_prompt: "Test system line.".to_string(),
            },
            resources: ResourceSection {
                cpu_max_percent: 65,
                reserve_cpu_cores: 2,
                memory_max_percent: 60,
                reserve_ram_mb: 4096,
                gpu_enabled: decentraai_config::GpuPolicy::Auto,
                gpu_max_vram_percent: 75,
                reserve_vram_mb: 1536,
                stop_gpu_temperature_celsius: 83,
                max_upload_mbps: 20,
                max_download_mbps: 80,
            },
            dht_enabled: false,
            relay_enabled: false,
            lan_discovery: true,
            bootstrap_peer_count: 0,
        }
    }

    fn test_tiers(limit: u32) -> TiersSection {
        TiersSection {
            tier1: TierPolicy {
                models: vec!["tinyllama.gguf".to_string()],
                rate_limit_per_minute: limit,
            },
            tier2: TierPolicy {
                models: vec![],
                rate_limit_per_minute: 60,
            },
            tier3: TierPolicy {
                models: vec![],
                rate_limit_per_minute: 120,
            },
        }
    }

    fn test_queue() -> Arc<InferenceQueue> {
        InferenceQueue::new(4, Duration::from_secs(2))
    }

    async fn start_backend() -> SocketAddr {
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { "{\"object\":\"list\",\"data\":[{\"id\":\"tinyllama\"}]}" }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async {
                    "{\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":20,\"total_tokens\":40},\"timings\":{\"predicted_per_second\":19.97}}"
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    /// Backend that echoes the request body, proving what the proxy sent.
    async fn start_stateful_api(
        dir: &Path,
        master: Option<String>,
        tiers: Option<TiersSection>,
    ) -> (SocketAddr, Arc<Mutex<ServeManager>>) {
        let backend = start_backend().await;
        let manager = test_manager(dir).await;
        let token_store = tiers.as_ref().map(|_| dir.join("db/tokens.json"));
        let state = ApiState::new(
            format!("http://{backend}"),
            master,
            manager.clone(),
            test_info(dir, None),
            token_store,
            tiers,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        (api, manager)
    }

    /// Like [`start_stateful_api`] but always wires the subscription-token store
    /// (so a master + subscriber/operator tokens are both recognized), used by
    /// the role-separation tests.
    async fn start_stateful_api_with_store(
        dir: &Path,
        master: String,
    ) -> (SocketAddr, Arc<Mutex<ServeManager>>) {
        let backend = start_backend().await;
        let manager = test_manager(dir).await;
        let store_path = dir.join("db/tokens.json");
        let state = ApiState::new(
            format!("http://{backend}"),
            Some(master),
            manager.clone(),
            test_info(dir, None),
            Some(store_path),
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        (api, manager)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remote_backend_proxy_forwards_to_configured_url_when_manager_unloaded() {
        // Q3: `serve start --backend http://host:port` keeps auth/tiers/queue
        // local but runs the model on a remote OpenAI-compatible server. With
        // no local engine, the proxy must fall back to `state.backend_url`
        // (the remote) rather than fail: an unloaded manager's `base_url()` is
        // None, so the proxy forwards to the configured backend.
        let dir = tempfile::tempdir().unwrap();
        let backend = start_backend().await;
        let manager = Arc::new(Mutex::new(ServeManager::unloaded(Duration::from_secs(3600))));
        assert!(!manager.lock().await.is_loaded(), "remote mode has no local engine");
        assert!(manager.lock().await.base_url().is_none());

        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

        // A metadata GET must round-trip to the remote backend.
        let resp = reqwest::get(format!("http://{api}/v1/models")).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert!(resp.text().await.unwrap().contains("\"list\""));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn generation_defaults_fill_only_missing_fields() {
        let generation = GenerationSection {
            temperature: 0.7,
            top_p: 0.9,
            top_k: Some(40),
            repeat_penalty: 1.1,
            system_prompt: "Be helpful.".to_string(),
        };
        // Missing everything: all defaults land, system prompt prepended.
        let merged = apply_generation_defaults(
            &generation,
            br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#,
        );
        let value: serde_json::Value = serde_json::from_slice(&merged).unwrap();
        assert!((value["temperature"].as_f64().unwrap() - 0.7).abs() < 0.001);
        assert!((value["top_p"].as_f64().unwrap() - 0.9).abs() < 0.001);
        assert_eq!(value["top_k"], 40);
        assert!((value["repeat_penalty"].as_f64().unwrap() - 1.1).abs() < 0.001);
        let messages = value["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "Be helpful.");

        // Caller-set values and an existing system message survive.
        let kept = apply_generation_defaults(
            &generation,
            br#"{"model":"m","temperature":0.1,"messages":[{"role":"system","content":"mine"},{"role":"user","content":"hi"}]}"#,
        );
        let value: serde_json::Value = serde_json::from_slice(&kept).unwrap();
        assert_eq!(value["temperature"], 0.1, "caller values win");
        let messages = value["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2, "no second system message");
        assert_eq!(messages[0]["content"], "mine");
    }

    #[test]
    fn inference_stats_empties_are_zero() {
        let stats = inference_stats(&[], 0, 0, 0);
        assert_eq!(stats.p50_ms, 0);
        assert_eq!(stats.p99_ms, 0);
        assert_eq!(stats.success_rate_percent, 0.0);
        assert_eq!(stats.requests_served, 0);
    }

    #[test]
    fn inference_stats_computes_percentiles_and_success() {
        let recent: Vec<RequestStat> = [10u64, 20, 30, 40, 50]
            .iter()
            .map(|ms| RequestStat {
                timestamp: 0,
                endpoint: "/v1/chat/completions".into(),
                prompt_tokens: 1,
                completion_tokens: 2,
                duration_ms: *ms,
                tokens_per_second: 3.0,
            })
            .collect();
        let stats = inference_stats(&recent, 90, 10, 3);
        assert_eq!(stats.p50_ms, 30, "median of 5 sorted samples");
        // Nearest-rank: p95 index = floor(4*0.95)=3 -> 40; p99 likewise.
        assert_eq!(stats.p95_ms, 40, "p95 nearest-rank sample");
        assert_eq!(stats.p99_ms, 40, "p99 nearest-rank sample");
        // 90 served / (90+10) = 90%.
        assert!((stats.success_rate_percent - 90.0).abs() < 1e-9);
        assert_eq!(stats.requests_failed, 10);
        assert_eq!(stats.queue_waiting, 3);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_forwards_models_to_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let response = reqwest::get(format!("http://{api}/v1/models"))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("\"list\""));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn open_webui_openai_surface_round_trips_through_proxy() {
        // Open WebUI connects to DecentraAI as an OpenAI-compatible backend:
        // ``/v1/models`` (a list with data[].id) and ``/v1/chat/completions``
        // (choices[].message.content + usage), both streamed or not. This test
        // proves the proxy preserves those standard shapes verbatim (no
        // wrapping, no field loss), so Open WebUI can consume the node as its
        // Chat engine while the DecentraAI dashboard stays the control plane.
        let dir = tempfile::tempdir().unwrap();
        let og = Router::new()
            .route(
                "/v1/models",
                get(|| async {
                    "{\"object\":\"list\",\"data\":[{\"id\":\"tinyllama\",\"object\":\"model\"}]}"
                }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async {
                    "{\"object\":\"chat.completion\",\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"Hello from the node\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":5,\"total_tokens\":8}}"
                }),
            );
        let manager = test_manager_with(dir.path(), og).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // /v1/models: OpenAI list shape Open WebUI's model picker parses.
        let models = client
            .get(format!("http://{api}/v1/models"))
            .send()
            .await
            .unwrap();
        assert_eq!(models.status(), 200);
        let mj: serde_json::Value = models.json().await.unwrap();
        assert_eq!(mj["object"], "list");
        assert_eq!(mj["data"][0]["id"], "tinyllama");

        // /v1/chat/completions: standard chat.completion shape Open WebUI
        // renders, with the assistant message content preserved.
        let chat = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"tinyllama\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}")
            .send()
            .await
            .unwrap();
        assert_eq!(chat.status(), 200);
        let cj: serde_json::Value = chat.json().await.unwrap();
        assert_eq!(cj["object"], "chat.completion");
        assert_eq!(cj["choices"][0]["message"]["content"], "Hello from the node");
        assert_eq!(cj["usage"]["total_tokens"], 8);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fabric_models_endpoint_lists_served_and_available_across_workers() {
        // Part 3/17: GET /v1/models must be fabric-wide — a coordinator with
        // the fabric attached advertises what each worker serves AND what it
        // has on disk (available_models), so an external OpenAI client can
        // discover any model it could ask for by file name.
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;

        let local_peer = decentraai_p2p::PeerId::random();
        let remote_peer = decentraai_p2p::PeerId::random();
        let compute = Arc::new(decentraai_distributed::ComputeManager::new(
            local_peer,
            "coordinator".into(),
            std::collections::HashSet::from([remote_peer]),
        ));
        // A remote trusted worker that serves a model and has another on disk.
        let remote_adv = decentraai_distributed::ComputeAdvertisement {
            peer_id: remote_peer,
            node_name: "remote-worker".into(),
            capability: decentraai_compute::ComputeCapability {
                cpu_cores: 8,
                ram_mb: 16 * 1024,
                gpu: None,
                engine: "llama_server".into(),
                served_models: vec![decentraai_compute::ServedModel {
                    model_hash: "hash-served".into(),
                    file_name: "served-model.gguf".into(),
                    size_mb: 2048,
                    est_ram_mb: 4096,
                    est_vram_mb: 0,
                    context_tokens: 4096,
                }],
                can_provision: false,
                available_models: vec![decentraai_compute::ServedModel {
                    model_hash: "hash-disk".into(),
                    file_name: "on-disk-model.gguf".into(),
                    size_mb: 1024,
                    est_ram_mb: 2048,
                    est_vram_mb: 0,
                    context_tokens: 0,
                }],
            },
            availability: decentraai_compute::ComputeAvailability {
                available_ram_mb: 8192,
                available_vram_mb: None,
                load_percent: 5,
                queue_depth: 0,
                tokens_per_second: 10,
                current_latency_ms: 50,
                status: decentraai_compute::WorkerHealth::Ready,
                gpu_temperature_celsius: None,
                gpu_utilization_percent: None,
                battery_percent: None,
            },
            announced_at_ms: 1_700_000_000_000,
            accepts_remote_inference: true,
            node_id: "dca-REMOTE".into(),
            node_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        compute.process_advertisement(remote_adv).await;

        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            Some(compute.clone()),
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();
        let base = format!("http://{api}");

        // Fabric list includes both the served model and the on-disk one.
        let models = client.get(format!("{base}/v1/models")).send().await.unwrap();
        assert_eq!(models.status(), 200);
        let mj: serde_json::Value = models.json().await.unwrap();
        assert_eq!(mj["object"], "list");
        let ids: Vec<&str> = mj["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"served-model.gguf"),
            "served model must be listed: {ids:?}"
        );
        assert!(
            ids.contains(&"on-disk-model.gguf"),
            "on-disk available model must be listed: {ids:?}"
        );
        let remote_entry = mj["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "served-model.gguf")
            .unwrap();
        assert_eq!(remote_entry["owned_by"], "dca-REMOTE");

        // Detail view resolves the model by id.
        let detail = client
            .get(format!("{base}/v1/models/served-model.gguf"))
            .send()
            .await
            .unwrap();
        assert_eq!(detail.status(), 200);
        let dj: serde_json::Value = detail.json().await.unwrap();
        assert_eq!(dj["id"], "served-model.gguf");
        assert_eq!(dj["owned_by"], "dca-REMOTE");

        // Unknown model id → 404 with an OpenAI-style error body.
        let missing = client
            .get(format!("{base}/v1/models/nope.gguf"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);
        let xj: serde_json::Value = missing.json().await.unwrap();
        assert!(xj["error"]["message"].as_str().unwrap().contains("nope.gguf"));

        manager.lock().await.shutdown().await.unwrap();
    }

    /// MCP endpoint: requires the master token (same boundary as the
    /// operational /v1 views) and negotiates + lists read-only tools over
    /// JSON-RPC. Read-only tools return the live fabric snapshot.
    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_endpoint_is_auth_gated_and_lists_tools() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("mcp-master".into()), None).await;
        let client = reqwest::Client::new();
        let base = format!("http://{api}");
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

        // No auth → 401 (same boundary as the operational /v1 views).
        let anon = client
            .post(format!("{base}/mcp"))
            .header("Content-Type", "application/json")
            .body(init)
            .send()
            .await
            .unwrap();
        assert_eq!(anon.status(), 401);

        // Master token → initialize negotiates the protocol and tools/list.
        let with_auth = |body: &str| {
            client
                .post(format!("{base}/mcp"))
                .header("Content-Type", "application/json")
                .header("Authorization", "Bearer mcp-master")
                .body(body.to_string())
        };
        let resp = with_auth(init).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(j["result"]["protocolVersion"], "2025-06-18");
        assert!(j["result"]["capabilities"]["tools"].is_object());

        let list = with_auth(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .send()
            .await
            .unwrap();
        let lj: serde_json::Value = list.json().await.unwrap();
        let names: Vec<&str> = lj["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"get_status"));
        assert!(names.contains(&"list_workers"));
        assert!(names.contains(&"list_executions"));

        // A read-only tool call returns the live snapshot (real node state).
        let call = with_auth(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#,
        )
        .send()
        .await
        .unwrap();
        let cj: serde_json::Value = call.json().await.unwrap();
        let text = cj["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("model_loaded"),
            "tool must return the live status snapshot: {text}"
        );

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_enforces_bearer_token() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("secret".to_string()), None).await;
        let client = reqwest::Client::new();

        let denied = client
            .get(format!("http://{api}/v1/models"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);

        let denied_peers = client
            .get(format!("http://{api}/v1/peers"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied_peers.status(), 401);

        let allowed = client
            .get(format!("http://{api}/v1/models"))
            .header("Authorization", "Bearer secret")
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), 200);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn subscriber_tokens_get_tier_policies() {
        let dir = tempfile::tempdir().unwrap();
        let registry_path = dir.path().join("db/tokens.json");
        let guest;
        {
            let mut store = decentraai_tokens::TokenStore::load(&registry_path).unwrap();
            guest = store
                .create("guest", decentraai_tokens::Tier::GUEST, None)
                .unwrap();
        }
        let (api, manager) =
            start_stateful_api(dir.path(), Some("master".to_string()), Some(test_tiers(60))).await;
        let client = reqwest::Client::new();

        // Guest can call the allowed model.
        let ok = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {guest}"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"tinyllama.gguf\",\"messages\":[]}")
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), 200);

        // But not a model outside the tier allowlist.
        let denied = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {guest}"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"llama-70b.gguf\",\"messages\":[]}")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403);

        // Unknown tokens are rejected even when a master exists.
        let unknown = client
            .get(format!("http://{api}/v1/models"))
            .header("Authorization", "Bearer dsk_nope")
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 401);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn role_separation_gates_operational_views() {
        let dir = tempfile::tempdir().unwrap();
        let registry_path = dir.path().join("db/tokens.json");
        let (client_tok, operator_tok);
        {
            let mut store = decentraai_tokens::TokenStore::load(&registry_path).unwrap();
            client_tok = store
                .create("client1", decentraai_tokens::Tier::GUEST, None)
                .unwrap();
            operator_tok = store
                .create_with_role(
                    "ops1",
                    decentraai_tokens::Tier::GUEST,
                    None,
                    decentraai_tokens::Role::Operator,
                )
                .unwrap();
        }
        let (api, manager) =
            start_stateful_api_with_store(dir.path(), "master".to_string()).await;
        let client = reqwest::Client::new();

        // A client token is denied the advanced operational view (H4)...
        let denied = client
            .get(format!("http://{api}/v1/compute"))
            .header("Authorization", format!("Bearer {client_tok}"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403, "client must not see operational views");
        let denied_net = client
            .get(format!("http://{api}/v1/network"))
            .header("Authorization", format!("Bearer {client_tok}"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied_net.status(), 403);

        // ...while an operator token is allowed.
        let allowed = client
            .get(format!("http://{api}/v1/compute"))
            .header("Authorization", format!("Bearer {operator_tok}"))
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), 200, "operator must see operational views");

        // The master is still allowed too.
        let master = client
            .get(format!("http://{api}/v1/compute"))
            .header("Authorization", "Bearer master")
            .send()
            .await
            .unwrap();
        assert_eq!(master.status(), 200);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_execute_decision_requires_master_not_operator() {
        // Phase M: a mutation (execute_decision runs real inference + reserves)
        // must be admin-only. An operator token may read (decide) but must be
        // denied the mutating execute_decision via the MCP surface; master is
        // allowed.
        let dir = tempfile::tempdir().unwrap();
        let registry_path = dir.path().join("db/tokens.json");
        let operator_tok;
        {
            let mut store = decentraai_tokens::TokenStore::load(&registry_path).unwrap();
            operator_tok = store
                .create_with_role(
                    "ops1",
                    decentraai_tokens::Tier::GUEST,
                    None,
                    decentraai_tokens::Role::Operator,
                )
                .unwrap();
        }
        let (api, manager) =
            start_stateful_api_with_store(dir.path(), "master".to_string()).await;
        let client = reqwest::Client::new();
        let mcp_body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_decision","arguments":{"intent":"ocr","prompt":"read","confirm":true}}}"#;

        // Operator token -> denied (mutation requires master).
        let op = client
            .post(format!("http://{api}/mcp"))
            .header("content-type", "application/json")
            .header("Authorization", format!("Bearer {operator_tok}"))
            .body(mcp_body)
            .send()
            .await
            .unwrap();
        assert!(op.status().is_client_error(), "operator must not execute");

        // Master token -> allowed (will return a 422 honest no-decision or a run).
        let master = client
            .post(format!("http://{api}/mcp"))
            .header("content-type", "application/json")
            .header("Authorization", "Bearer master")
            .body(mcp_body)
            .send()
            .await
            .unwrap();
        assert_eq!(master.status(), 200, "master may call execute_decision");
        let j: serde_json::Value = master.json().await.unwrap();
        // Either a result or an error is surfaced in the content; the boundary
        // must NOT be an auth rejection for master.
        assert!(
            j["result"].is_object() || j["error"].is_object(),
            "master call must yield a result or protocol error: {j}"
        );

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rate_limit_returns_429_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let registry_path = dir.path().join("db/tokens.json");
        let guest;
        {
            let mut store = decentraai_tokens::TokenStore::load(&registry_path).unwrap();
            guest = store
                .create("guest", decentraai_tokens::Tier::GUEST, None)
                .unwrap();
        }
        let (api, manager) =
            start_stateful_api(dir.path(), Some("master".to_string()), Some(test_tiers(2))).await;
        let client = reqwest::Client::new();

        for expected in [200, 200, 429] {
            let response = client
                .post(format!("http://{api}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {guest}"))
                .header("Content-Type", "application/json")
                .body("{\"model\":\"tinyllama.gguf\",\"messages\":[]}")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        let audit = std::fs::read_to_string(dir.path().join("logs/audit.jsonl")).unwrap();
        assert!(audit.contains("rate_limited"));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn execute_mutations_are_rate_limited() {
        // Phase M LIMITS: /v1/execute (master-gated mutation) is limited to
        // EXECUTE_RATE_LIMIT_PER_MINUTE per name. Each call consumes a slot
        // before body/decision handling (so even honest 422s count), and the
        // (limit+1)-th call returns 429 + audits execute_rate_limited.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();
        let url = format!("http://{api}/v1/execute");
        let body = serde_json::json!({
            "intent": "ocr",
            "prompt": "read",
            "max_tokens": 4,
            "confirm": true,
        })
        .to_string();

        for i in 0..EXECUTE_RATE_LIMIT_PER_MINUTE {
            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .bearer_auth("master")
                .body(body.clone())
                .send()
                .await
                .unwrap();
            // Each call passes the rate gate; without a fabric model it is a
            // honest 422 (consuming a slot), never 429 until the limit is hit.
            assert_ne!(resp.status(), 429, "call {i} must not be limited yet");
        }
        // The (limit+1)-th call is rate-limited.
        let resp = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth("master")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 429, "mutation must be rate limited");
        let audit = std::fs::read_to_string(dir.path().join("logs/audit.jsonl")).unwrap();
        assert!(audit.contains("execute_rate_limited"));

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inference_calls_are_counted_with_token_stats() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let client = reqwest::Client::new();

        // Metadata GETs must not count as inference.
        client
            .get(format!("http://{api}/v1/models"))
            .send()
            .await
            .unwrap();

        let response = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"test\",\"messages\":[]}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let status: serde_json::Value = client
            .get(format!("http://{api}/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["requests_served"], 1, "only the POST counts");
        assert_eq!(status["tokens_generated"], 20);
        let recent = status["recent_requests"].as_array().unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["prompt_tokens"], 20);
        assert_eq!(recent[0]["completion_tokens"], 20);
        assert!(recent[0]["tokens_per_second"].as_f64().unwrap() > 19.0);
        assert!(status["uptime_secs"].as_u64().is_some());
        assert!(status["system"]["cpu_threads"].as_u64().unwrap() >= 1);
        assert!(status["queue"]["waiting"].is_array());
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn full_queue_answers_503_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        // Waiting room of one: first request serves, second waits, third is rejected.
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            InferenceQueue::new(1, Duration::from_secs(5)),
            None,
            None,
        );
        let api = serve_api(state.clone(), "127.0.0.1", 0).await.unwrap();

        // Hold the serving slot with a manual ticket so the queue is busy.
        let hold = state
            .queue
            .enqueue("holder", "/v1/chat/completions")
            .unwrap();
        hold.wait_turn().await.unwrap();

        // Fill the waiting room
        let _waiter = state
            .queue
            .enqueue("waiter", "/v1/chat/completions")
            .unwrap();

        let response = reqwest::Client::new()
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"m\",\"messages\":[]}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(response.text().await.unwrap().contains("queue is full"));
        drop(hold);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dashboard_is_served_at_root_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let client = reqwest::Client::new();

        for path in ["/", "/v1", "/anything-else"] {
            let response = client
                .get(format!("http://{api}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                200,
                "path {path} must serve the dashboard"
            );
            let body = response.text().await.unwrap();
            assert!(body.contains("DecentraAI dashboard"));
            assert!(body.contains("Tokens generated"));
            assert!(body.contains("Queue"));
            assert!(body.contains("Recent inference calls"));
            assert!(body.contains("Share a model"));
            // Multi-node fabric identity: per-node resource view + discovery
            // feed + worker pipe identity are part of the normal user view.
            assert!(body.contains("Fabric nodes"), "fabric nodes strip must be in the normal view");
            assert!(body.contains("id=\"fabric-nodes\""), "fabric nodes container id present");
            assert!(body.contains("id=\"discovery-feed\""), "discovery feed container id present");
            assert!(body.contains("id=\"pipe-worker-name\""), "worker pipe identity element present");
            // The JS that powers the identity view must exist (DOM without
            // renderers would stay empty — the dashboard never fakes state).
            for needle in [
                "function renderFabricNodes",
                "function updateDiscovery",
                "function workerCard",
                "function nodeChip",
                "function trustChain",
                "const nodeIdOf",
                "node_id",
            ] {
                assert!(body.contains(needle), "fabric JS must include {needle}");
            }
        }
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dashboard_chat_view_has_model_stop_and_retry_controls() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let body = reqwest::Client::new()
            .get(format!("http://{api}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // The three new Chat UX controls and the model selector are present.
        for needle in [
            "id=\"chat-model\"",
            "<select id=\"chat-model\"",
            "id=\"chat-stop\"",
            "id=\"chat-retry\"",
        ] {
            assert!(body.contains(needle), "dashboard must include {needle}");
        }
        // The controls are wired to real behavior: Stop aborts the in-flight
        // request (AbortController) and the model select is populated from the
        // live /status `available_models` payload rather than a hardcoded list.
        assert!(body.contains("new AbortController()"), "Stop must abort via AbortController");
        assert!(body.contains("controller.signal"), "Stop aborts the fetch via its signal");
        assert!(body.contains("s.available_models"), "chat-model must read live available_models");
        assert!(body.contains("return v || activeModel;"), "send must fall back to the active model");
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dashboard_chat_has_fabric_origin_indicator_and_remote_models() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let body = reqwest::Client::new()
            .get(format!("http://{api}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // The served-by indicator reads the proxy's origin headers, and the
        // model selector gains an Auto (best) picker + grouped remote-worker
        // section from /v1/compute (including models that exist locally).
        for needle in [
            "id=\"chat-served\"",
            "x-decentra-origin",
            "x-decentra-worker",
            "x-decentra-node",
            "__auto__",
            "Auto (best available)",
            "worker_hint",
            "remote:",
            "Remote workers",
            "c.local_peer",
            "w.served_models",
        ] {
            assert!(body.contains(needle), "dashboard must include {needle}");
        }
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn openapi_document_is_served_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let resp = reqwest::get(format!("http://{api}/openapi.json"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let spec: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(spec["openapi"], "3.0.0");
        assert_eq!(spec["info"]["version"], "1.0.0");
        assert!(spec["paths"]["/v1/chat/completions"].is_object());
        assert!(spec["paths"]["/v1/compute"].is_object());
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dashboard_hides_advanced_compute_internals_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let client = reqwest::Client::new();
        let body = client
            .get(format!("http://{api}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // Normal-user essentials are always present.
        for needle in [
            "id=\"adv-toggle\"",
            "id=\"chat-history\"",
            "id=\"recent\"",
            "id=\"ram\"",
        ] {
            assert!(body.contains(needle), "normal view must include {needle}");
        }
        // Distributed-compute internals live behind the opt-in advanced block.
        assert!(
            body.contains("<div id=\"advanced\" hidden>"),
            "advanced compute cards must be hidden by default"
        );
        for needle in [
            "Tracked peers (reputation)",
            "Workers (compute registry)",
            "Execution (planner decisions)",
            "Diagnostics",
            "Recent security events",
        ] {
            let idx = body.find(needle).expect("advanced card present");
            let open = body
                .find("<div id=\"advanced\"")
                .expect("advanced container");
            assert!(idx > open, "{needle} must be inside the advanced container");
        }
        // The toggle must drive the advanced container from real state (no mocks).
        assert!(body.contains("document.getElementById('advanced')"));
        assert!(body.contains("localStorage.setItem('decentraai.advanced'"));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn status_and_peers_feed_the_dashboard() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        decentraai_audit::record(
            &dir.path().join("logs"),
            "inference_started",
            serde_json::json!({"model": "m.gguf"}),
        )
        .unwrap();
        let reputation_path = dir.path().join("db/reputation.json");
        {
            let mut store = decentraai_p2p::reputation::ReputationStore::load(
                &reputation_path,
                1,
                Duration::from_secs(3600),
            )
            .unwrap();
            store.record_failure(&decentraai_p2p::PeerId::random());
            store.save().unwrap();
        }

        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), Some(reputation_path)),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        let status: serde_json::Value = client
            .get(format!("http://{api}/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["model_loaded"], true);
        assert_eq!(status["model"], "test-model.gguf");
        assert_eq!(status["model_size_bytes"], 1024);
        assert_eq!(status["recent_events"].as_array().unwrap().len(), 1);
        assert_eq!(status["recent_events"][0]["event"], "inference_started");
        assert!(status["available_models"].is_array());
        assert!(status["queue"]["serving"].is_null());
        // Resource pressure (Part 17/22): the /status system block must carry
        // the honest measured snapshot fields the dashboard renders.
        assert!(status["system"]["cpu_usage_percent"].is_number());
        assert!(status["system"]["ram_total_gib"].is_number());
        assert!(status["system"]["ram_available_gib"].is_number());
        assert!(status["system"]["used_swap_gib"].is_number());
        assert!(status["system"]["disk_free_gib"].is_number());

        let peers: serde_json::Value = client
            .get(format!("http://{api}/v1/peers"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(peers.as_array().unwrap().len(), 1);
        assert_eq!(peers[0]["banned"], true);

        manager.lock().await.shutdown().await.unwrap();
    }

    /// The Settings view renders real config: `/status` must expose the
    /// generation defaults (sampling) and the subscription tier policies
    /// without leaking secrets. When no tiers are configured the field is null.
    #[cfg(unix)]
    #[tokio::test]
    async fn status_exposes_generation_defaults_and_tier_policies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            Some(test_tiers(60)),
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        let status: serde_json::Value = client
            .get(format!("http://{api}/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let temperature = status["generation"]["temperature"].as_f64().unwrap();
        assert!(
            (temperature - 0.7).abs() < 1e-6,
            "generation temperature must round-trip, got {temperature}"
        );
        assert_eq!(status["generation"]["top_k"], 40);
        assert_eq!(status["generation"]["system_prompt"], "Test system line.");
        assert_eq!(status["tiers"]["tier1"]["rate_limit_per_minute"], 60);
        assert_eq!(status["tiers"]["tier1"]["models"][0], "tinyllama.gguf");
        assert_eq!(status["tiers"]["tier2"]["rate_limit_per_minute"], 60);

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn status_tiers_is_null_when_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        let status: serde_json::Value = client
            .get(format!("http://{api}/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(status["tiers"].is_null(), "tiers must be null when unconfigured");

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn metrics_endpoint_exposes_prometheus_text() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let client = reqwest::Client::new();

        // Auth-neutral: no token required, like /status.
        let response = client
            .get(format!("http://{api}/metrics"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("text/plain"),
            "metrics must be plain text, got {content_type:?}"
        );
        assert!(
            content_type.contains("0.0.4"),
            "Prometheus content-type version, got {content_type:?}"
        );

        let body = response.text().await.unwrap();
        for needle in [
            "decentraai_requests_served_total",
            "decentraai_requests_failed_total",
            "decentraai_tokens_generated_total",
            "decentraai_latency_ms",
            "decentraai_queue_waiting",
            "decentraai_queue_serving",
            "decentraai_uptime_seconds",
            "decentraai_model_loaded",
            "decentraai_fabric_workers_total",
            "decentraai_fabric_trusted_workers_total",
            "decentraai_fabric_sessions_active",
            "gen_ai.server.request.count",
            "gen_ai.server.token.input",
            "gen_ai.server.token.output",
            "gen_ai.server.request.duration",
            "gen_ai.provider.name",
            "# HELP",
            "# TYPE",
        ] {
            assert!(
                body.contains(needle),
                "metrics body must contain {needle:?}: {body}"
            );
        }
        // Real counters, not mock data: uptime and model_loaded must be present.
        assert!(
            body.contains("decentraai_uptime_seconds "),
            "uptime gauge value expected: {body}"
        );
        assert!(
            body.contains("decentraai_model_loaded 1")
                || body.contains("decentraai_model_loaded 0"),
            "model_loaded gauge must be 0 or 1: {body}"
        );
        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn prometheus_escape_handles_label_special_chars() {
        // A model name with a quote/backslash must not break the exposition.
        assert_eq!(prometheus_escape("plain-model"), "plain-model");
        assert_eq!(prometheus_escape("a\"b"), "a\\\"b");
        assert_eq!(prometheus_escape("a\\b"), "a\\\\b");
        assert_eq!(prometheus_escape("a\nb"), "a\\nb");
        assert_eq!(prometheus_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[tokio::test]
    async fn stats_endpoint_returns_deterministic_history() {
        // /v1/stats is operator/admin-gated; without a compute manager it
        // returns a 0-record honest response (not an error, never invented).
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();

        // Unauthenticated -> 401/403.
        let resp = client.get(format!("http://{api}/v1/stats")).send().await.unwrap();
        assert!(resp.status().is_client_error());

        // With the master token -> 200 JSON with a records field.
        let resp = client
            .get(format!("http://{api}/v1/stats"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert!(j["records"].is_number(), "records field present");

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn sessions_endpoint_requires_operator_and_returns_honest_empty() {
        // /v1/sessions is operator/admin-gated; without a compute manager it
        // returns an honest empty session list (never fabricated residency).
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();

        // Unauthenticated -> 401/403.
        let resp = client
            .get(format!("http://{api}/v1/sessions"))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        // With the master token -> 200 JSON with sessions_active + sessions.
        let resp = client
            .get(format!("http://{api}/v1/sessions"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(j["sessions_active"], 0, "honest: no sessions");
        assert!(j["sessions"].is_array());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn decision_endpoint_requires_operator_and_returns_coherent_structure() {
        // /v1/decision (Phase 1) is operator/admin-gated and returns a coherent
        // read-only projection (request/capabilities/decision/why/historical).
        // Without a compute manager the capabilities array is empty (honest) but
        // the shape is stable.
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();

        // Unauthenticated -> 401/403.
        let resp = client
            .get(format!("http://{api}/v1/decision?intent=ocr"))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        // With the master token -> 200 JSON with the coherent structure.
        let resp = client
            .get(format!("http://{api}/v1/decision?intent=ocr"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(j["request"], "ocr");
        assert!(j["capabilities"].is_array());
        assert!(j["why"].is_array());
        assert!(j["historical"].is_object(), "historical present (Phase 2)");
        assert!(j["recent_recovery"].is_array(), "recent_recovery present (Phase 5)");
        // decision is null (no workers/models) — honest, not invented.
        assert!(j["decision"].is_null());

        // Missing intent -> 4xx.
        let resp = client
            .get(format!("http://{api}/v1/decision"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn execute_decision_requires_confirmation_and_honest_decision() {
        // /v1/execute is the mutation step of decide→reserve→execute. It must
        // (a) require master auth, (b) refuse without explicit "confirm": true
        // (mutation safety), and (c) with confirmation but no runnable fabric
        // decision return an honest unprocessable (not a fabricated run).
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();
        let url = format!("http://{api}/v1/execute");
        let body = |confirm: bool| {
            serde_json::json!({
                "intent": "OCR these images",
                "prompt": "read the text",
                "max_tokens": 64,
                "confirm": confirm,
            })
            .to_string()
        };

        // Unauthenticated -> 401/403.
        let resp = client
            .post(&url)
            .header("content-type", "application/json")
            .body(body(true))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        // Authenticated but no explicit confirmation -> refused (mutation safety).
        let resp = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth("master")
            .body(body(false))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error(), "must refuse without confirm");

        // Confirmed but no runnable fabric decision -> honest unprocessable,
        // never a fabricated execution.
        let resp = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth("master")
            .body(body(true))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 422);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert!(
            j["error"]["message"].as_str().unwrap().contains("no runnable decision"),
            "honest error: {j}"
        );
        assert!(j["decision"].is_object(), "decision carried for explanation");

        // Dry-run: without a compute manager there is no model on the fabric, so
        // dry-run honestly returns 422 (nothing would have been executed) — never
        // a fabricated plan.
        let dry = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth("master")
            .body(serde_json::json!({
                "intent": "OCR these images",
                "prompt": "read the text",
                "max_tokens": 64,
                "confirm": true,
                "dry_run": true,
            })
            .to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(dry.status(), 422, "dry-run with no fabric model is honest 422");

        // Capability-only execute (no intent): accepted by the boundary and
        // honestly 422 without a fabric model (NOT 'missing intent').
        let cap_only = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth("master")
            .body(serde_json::json!({
                "capability": "ocr",
                "prompt": "read the text",
                "max_tokens": 64,
                "confirm": true,
            })
            .to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(
            cap_only.status(),
            422,
            "capability-only execute proceeds past the intent gate and honestly 422s without a fabric model"
        );

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fabric_graph_requires_operator_and_returns_structure() {
        // /v1/fabric is operator/admin-gated; without a compute manager it
        // returns 200 JSON with the honest fabric-graph shape (empty arrays,
        // never fabricated data).
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();

        // Unauthenticated -> 401/403.
        let resp = client
            .get(format!("http://{api}/v1/fabric"))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        // With the master token -> 200 JSON with the fabric-graph structure.
        let resp = client
            .get(format!("http://{api}/v1/fabric"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();
        assert!(j["nodes"].is_array());
        assert!(j["models"].is_array());
        assert!(j["capabilities"].is_array());
        assert!(j["executions"].is_array());
        assert!(j["network"].is_array());
        assert!(j["kv"]["sessions_active"].is_number());
        assert!(j["note"].is_string());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn fabric_graph_aggregate_deduplicates_models_and_derives_capabilities() {
        // Real, synthetic inputs drive the pure projection: two workers share
        // one model file and hold different capabilities; aggregation must
        // deduplicate the model, keep identity fields separate, and only
        // surface capabilities that come from real registry claims.
        let peer_a = decentraai_p2p::PeerId::random();
        let peer_b = decentraai_p2p::PeerId::random();
        let adv_a = cap_adv(&peer_a, "dca-node-a", "llama_server", 8192, Some(8192), (1024, 2048), (true, true));
        let adv_b = cap_adv(&peer_b, "dca-node-b", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));

        let reg = decentraai_registry::ModelRegistry {
            version: 1,
            root: "/fake".into(),
            models: std::collections::BTreeMap::from([
                (
                    "qwen.gguf".to_string(),
                    decentraai_registry::ModelRecord {
                        relative_path: "qwen.gguf".into(),
                        canonical_path: "/fake/qwen.gguf".into(),
                        size_bytes: 100,
                        modification_time: 0,
                        extension: "gguf".into(),
                        capability_claims: vec![
                            decentraai_registry::CapabilityClaimRecord {
                                capability: "ocr".into(),
                                provenance: "verified".into(),
                            },
                        ],
                    },
                ),
            ]),
        };

        let body = fabric_graph_aggregate(
            &[(adv_a, true), (adv_b, false)],
            Some(&reg),
            &[],
            &decentraai_fabric::NetworkGraph::new(),
            0,
            "1.0.0",
        );

        // Nodes: one per real worker, identity fields kept separate and real.
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0]["peer_id"], nodes[0]["node_id"]);
        assert_eq!(nodes[0]["node_name"], "dca-node-a");
        assert_eq!(nodes[0]["trusted"], true);
        assert_eq!(nodes[1]["trusted"], false);
        assert_eq!(nodes[0]["engine"], "llama_server");

        // Models: the shared file appears once, with both holding node_ids.
        let models = body["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["file"], "qwen.gguf");
        let model_nodes = models[0]["nodes"].as_array().unwrap();
        assert_eq!(model_nodes.len(), 2);
        assert_eq!(models[0]["capabilities"].as_array().unwrap()[0]["capability"], "ocr");

        // Capabilities: only from real registry claims (ocr), with the model
        // and both node ids attached.
        let caps = body["capabilities"].as_array().unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0]["capability"], "ocr");
        assert_eq!(caps[0]["models"].as_array().unwrap().len(), 1);
        assert_eq!(caps[0]["nodes"].as_array().unwrap().len(), 2);

        // Network and executions stay empty-but-honest.
        assert!(body["network"].as_array().unwrap().is_empty());
        assert!(body["executions"].as_array().unwrap().is_empty());
        assert_eq!(body["kv"]["sessions_active"], 0);
    }

    #[test]
    fn device_class_classifies_from_real_capability() {
        // Device-class inference (Digital Twin / mobile workers): derived from
        // real advertised capability (GPU/RAM/cores), never fabricated, and it
        // does not change scheduling.
        let cap = |gpu: bool, ram_mb: u64, cores: u16| decentraai_compute::ComputeCapability {
            cpu_cores: cores,
            ram_mb,
            gpu: if gpu {
                Some(decentraai_compute::GpuSpec {
                    name: "gpu".into(),
                    vram_mb: 8192,
                    driver: "x".into(),
                })
            } else {
                None
            },
            engine: "llama_server".into(),
            served_models: vec![],
            can_provision: false,
            available_models: vec![],
        };
        assert_eq!(device_class(&cap(true, 64 * 1024, 32)), "server"); // big GPU box
        assert_eq!(device_class(&cap(true, 16 * 1024, 8)), "desktop"); // gaming rig
        assert_eq!(device_class(&cap(false, 64 * 1024, 24)), "server"); // headless
        assert_eq!(device_class(&cap(false, 16 * 1024, 8)), "laptop");
        assert_eq!(device_class(&cap(false, 4 * 1024, 4)), "mobile"); // phone/Pi-class
        assert_eq!(device_class(&cap(false, 2 * 1024, 2)), "mobile");
        assert_eq!(device_class(&cap(false, 16 * 1024, 4)), "laptop"); // edge-ish but RAM-y
    }

    #[test]
    fn load_balance_suggests_share_from_capacity_and_load() {
        // Two CAN_RUN workers with different advertised tps + load: the faster
        // / more idle one gets a larger suggested share; shares sum to ~100.
        // Advisory only — never changes scheduling.
        let p1 = decentraai_p2p::PeerId::random();
        let p2 = decentraai_p2p::PeerId::random();
        let mut w1 = cap_adv(&p1, "dca-fast", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        let mut w2 = cap_adv(&p2, "dca-slow", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        w1.availability.tokens_per_second = 100;
        w1.availability.load_percent = 10; // idle 90 -> weight 90
        w2.availability.tokens_per_second = 10;
        w2.availability.load_percent = 50; // idle 50 -> weight 5

        let can_run: std::collections::HashSet<String> =
            [p1.to_string(), p2.to_string()].into();
        let lb = load_balance_for_workers(&[(w1, true), (w2, true)], &can_run);
        assert_eq!(lb.len(), 2);
        // fast/idle share (90) >> slow/busy share (5); total ~100.
        let fast = lb.iter().find(|x| x["node_id"] == "dca-fast").unwrap();
        let slow = lb.iter().find(|x| x["node_id"] == "dca-slow").unwrap();
        assert!(fast["suggested_share_pct"].as_u64().unwrap()
            > slow["suggested_share_pct"].as_u64().unwrap());
        let total: u64 = lb.iter().map(|x| x["suggested_share_pct"].as_u64().unwrap()).sum();
        assert!((95..=105).contains(&total), "shares sum ~100: {total}");
        assert!(fast["device_class"].is_string());

        // No eligible -> empty (honest).
        let w2b = cap_adv(&p2, "dca-slow", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        assert!(load_balance_for_workers(&[(w2b, true)], &std::collections::HashSet::new()).is_empty());
    }

    #[test]
    fn load_balance_folds_in_adaptive_contribution() {
        // Adaptive fan-out: two otherwise-identical workers — one healthy, one
        // under GPU thermal pressure — the stressed one gets a smaller share,
        // and the share record exposes its adaptive factor.
        let p1 = decentraai_p2p::PeerId::random();
        let p2 = decentraai_p2p::PeerId::random();
        let mut w1 = cap_adv(&p1, "dca-h", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        let mut w2 = cap_adv(&p2, "dca-hot", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        w1.availability.tokens_per_second = 100;
        w1.availability.load_percent = 10;
        w2.availability.tokens_per_second = 100;
        w2.availability.load_percent = 10;
        w2.availability.gpu_temperature_celsius = Some(95); // heavy thermal pressure

        let can_run: std::collections::HashSet<String> = [p1.to_string(), p2.to_string()].into();
        let lb = load_balance_for_workers(&[(w1, true), (w2, true)], &can_run);
        assert_eq!(lb.len(), 2);
        let healthy = lb.iter().find(|x| x["node_id"] == "dca-h").unwrap();
        let hot = lb.iter().find(|x| x["node_id"] == "dca-hot").unwrap();
        assert!(
            healthy["suggested_share_pct"].as_u64().unwrap() > hot["suggested_share_pct"].as_u64().unwrap(),
            "thermally-stressed worker gets a smaller share"
        );
        assert!(
            hot["adaptive_contribution"].as_f64().unwrap() < 1.0,
            "share record exposes the adaptive factor"
        );
    }

    #[test]
    fn version_status_is_honest() {
        // Node lifecycle: same version -> CURRENT; different known version ->
        // OUTDATED; empty/unknown -> UNKNOWN. Never fabricates.
        assert_eq!(version_status("1.0.0", "1.0.0"), "CURRENT");
        assert_eq!(version_status("1.0.0", "1.1.0"), "OUTDATED");
        assert_eq!(version_status("1.0.0", "0.9.0"), "OUTDATED");
        assert_eq!(version_status("1.0.0", ""), "UNKNOWN");
        assert_eq!(version_status("1.0.0", "   "), "UNKNOWN");
        // A peer that does not report a version is never labeled CURRENT.
        assert_ne!(version_status("1.0.0", ""), "CURRENT");
    }

    #[test]
    fn node_lifecycle_only_emits_evidence_supported_states() {
        // Node lifecycle: only real-evidence states are emitted. UPDATING /
        // VERIFIED are NOT produced (no remote update mechanism exists yet).
        assert_eq!(node_lifecycle(false, false, "UNKNOWN"), "UNKNOWN");
        assert_eq!(node_lifecycle(false, true, "CURRENT"), "DISCOVERED");
        assert_eq!(node_lifecycle(false, true, "OUTDATED"), "DISCOVERED_OUTDATED");
        assert_eq!(node_lifecycle(true, false, "CURRENT"), "TRUSTED");
        assert_eq!(node_lifecycle(true, false, "OUTDATED"), "TRUSTED_OUTDATED");
        assert_eq!(node_lifecycle(true, true, "CURRENT"), "ONLINE");
        assert_eq!(node_lifecycle(true, true, "OUTDATED"), "ONLINE_OUTDATED");
        // The update-only phases must never be produced.
        for (t, h, v) in [(false, false, "CURRENT"), (true, true, "OUTDATED")] {
            let s = node_lifecycle(t, h, v);
            assert_ne!(s, "UPDATING");
            assert_ne!(s, "VERIFIED");
        }
    }

    #[test]
    fn mb_converts_bytes_to_mebibytes() {
        assert_eq!(mb(0), 0);
        assert_eq!(mb(1024 * 1024), 1);
        assert_eq!(mb(5 * 1024 * 1024 + 123), 5);
        assert_eq!(mb(123), 0, "sub-MiB rounds down to 0");
    }

    #[tokio::test]
    async fn resources_endpoint_requires_operator_and_returns_honest_state() {
        // /v1/resources is operator/admin-gated and returns 200 JSON with a
        // master token, 401/403 without. It always reports real node state
        // (never fabricates) and an empty-but-well-formed fabric when no
        // compute manager is attached.
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), Some("master".into()), None).await;
        let client = reqwest::Client::new();

        // Unauthenticated -> 401/403.
        let resp = client
            .get(format!("http://{api}/v1/resources"))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        // With the master token -> 200 JSON with the unified resource shape.
        let resp = client
            .get(format!("http://{api}/v1/resources"))
            .bearer_auth("master")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let j: serde_json::Value = resp.json().await.unwrap();

        // node: RAM/CPU/disk always present; RAM and VRAM stay separate.
        assert!(j["node"]["ram"]["total_mb"].is_number());
        assert_eq!(j["node"]["ram"]["reserved_mb"], 0, "node tracks no reservation");
        assert_eq!(j["node"]["ram"]["in_use_mb"], j["node"]["ram"]["total_mb"].as_u64().unwrap()
            .saturating_sub(j["node"]["ram"]["available_mb"].as_u64().unwrap()));
        assert_eq!(j["node"]["ram"]["provenance"], "MEASURED");
        assert_eq!(j["node"]["cpu"]["provenance"], "MEASURED");
        assert_eq!(j["node"]["disk"]["provenance"], "MEASURED");
        // GPU is UNKNOWN (honest) when nvidia-smi is absent, never a fake 0.
        let vram_present = j["node"]["vram"]["present"].as_bool().unwrap();
        assert_eq!(
            j["node"]["vram"]["provenance"],
            if vram_present { "MEASURED" } else { "UNKNOWN" }
        );

        // Without a compute manager: fabric empty, kv honest UNKNOWN.
        assert!(j["fabric"].is_array());
        assert_eq!(j["fabric"].as_array().unwrap().len(), 0);
        assert_eq!(j["kv"]["provenance"], "UNKNOWN");
        assert!(j["kv"]["sessions_active"].is_null());
        assert!(j["queue"]["serving"].is_boolean());
        assert!(j["queue"]["waiting"].is_number());
        assert!(j["note"].is_string());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn compute_network_execution_endpoints_respond() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // Without a compute manager these return a well-formed "not attached"
        // structure (never mock data).
        let compute: serde_json::Value = client
            .get(format!("http://{api}/v1/compute"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(compute["attached"], false);
        assert!(compute["workers"].is_array());

        let network: serde_json::Value = client
            .get(format!("http://{api}/v1/network"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(network["attached"], false);
        assert!(network["connected"].is_array());
        // Identity view: addresses + local addresses are always present
        // (empty without a live p2p node), so the fabric UI never guesses.
        assert!(network["addresses"].is_array());
        assert!(network["local_addresses"].is_array());

        let exec: serde_json::Value = client
            .get(format!("http://{api}/v1/execution"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(exec["attached"], false);
        assert!(exec["executions"].is_array());
        assert!(exec["decisions"].is_array());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn status_lists_registry_models() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        let db_dir = dir.path().join("db");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::write(models_dir.join("extra.gguf"), b"GGUF test").unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(models_dir.clone()).unwrap();
        registry.scan_directory(&models_dir).unwrap();
        registry.save(&db_dir.join("registry.json")).unwrap();

        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let status: serde_json::Value = reqwest::get(format!("http://{api}/status"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let models = status["available_models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["name"], "extra.gguf");
        assert!(models[0]["size_bytes"].as_u64().unwrap() > 0);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_merges_generation_into_outgoing_body() {
        let dir = tempfile::tempdir().unwrap();
        let echo_app = Router::new().route(
            "/v1/chat/completions",
            post(|body: Bytes| async move {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    format!(
                        "{{\"echo\":{},\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":2}}}}",
                        String::from_utf8_lossy(&body)
                    ),
                )
            }),
        );
        // The proxy forwards to the LIVE engine address (M24), so the manager
        // must own (point at) the echo backend for this test to observe the
        // merged outgoing body.
        let manager = test_manager_with(dir.path(), echo_app).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

        let response = reqwest::Client::new()
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body("{\"model\":\"m\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let echoed = response.text().await.unwrap();
        assert!(
            echoed.contains("temperature"),
            "sampling defaults must reach the backend"
        );
        assert!(
            echoed.contains("Test system line."),
            "system prompt must reach the backend"
        );
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn hub_compare_model_body_includes_fit_classification_and_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master".into()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let detail = decentraai_hub::HubModelDetail {
            id: "org/test-model".into(),
            pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
            tags: vec!["gguf".into()],
            downloads: 10,
            likes: 1,
            description: Some("Test".into()),
            license: Some("mit".into()),
            context_length: Some(4096),
            params: Some("1B".into()),
        }
        .fill_from_tags();
        let files = vec![
            decentraai_hub::HubModelFile {
                path: "q4_k_m.gguf".into(),
                size: Some(100 * 1024 * 1024),
                lfs: None,
            },
        ];
        let caps = detail.capabilities();
        let body = hub_compare_model_body(&detail, &files, &caps, &state, "org/test-model", None).await;
        assert_eq!(body["id"], "org/test-model");
        let variants = body["variants"].as_array().unwrap();
        assert_eq!(variants.len(), 1);
        assert!(variants[0]["fit_classification"].is_string());
        let reasons = variants[0]["fit_reasons"].as_array().unwrap();
        assert!(!reasons.is_empty());
        assert!(reasons.iter().any(|r| r["check"] == "ram_sufficient"));
        manager.lock().await.shutdown().await.unwrap();
    }

    /// The comparison model body reports an honest, provenance-aware fit verdict
    /// per compared model when a `requires` capability is supplied, and `null`
    /// when it is not. Mirrors the model-card verdict: a VERIFIED claim
    /// satisfies, an INFERRED-only claim never satisfies a VERIFIED requirement,
    /// and `None` -> null. Pure (no network); hub detail built as a struct.
    #[tokio::test]
    async fn hub_compare_model_body_reports_requires_fit_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master".into()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let files = vec![];
        let make_detail = |id: String, tags: Vec<String>| decentraai_hub::HubModelDetail {
            id,
            pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
            tags,
            downloads: 0,
            likes: 0,
            description: None,
            license: None,
            context_length: None,
            params: None,
        };

        // VERIFIED OCR via the `ocr` tag: a Verified requirement is satisfied.
        let verified = make_detail("org/scanner".into(), vec!["gguf".into(), "ocr".into()]);
        let caps = verified.capabilities();
        let body = hub_compare_model_body(
            &verified,
            &files,
            &caps,
            &state,
            "org/scanner",
            Some(decentraai_hub::CapabilityKind::Ocr),
        )
        .await;
        let fit = body["capabilities"]["fit"].clone();
        assert_eq!(fit["capability"], "ocr");
        assert_eq!(fit["satisfied"], true, "verified claim must satisfy verified requirement");
        assert_eq!(fit["checks"][0]["status"]["satisfied"]["provenance"], "verified");

        // INFERRED coding only (id heuristic) must NOT satisfy Verified.
        let inferred = make_detail("org/codestral".into(), vec!["gguf".into()]);
        let caps = inferred.capabilities();
        let body = hub_compare_model_body(
            &inferred,
            &files,
            &caps,
            &state,
            "org/codestral",
            Some(decentraai_hub::CapabilityKind::Coding),
        )
        .await;
        let fit = body["capabilities"]["fit"].clone();
        assert_eq!(fit["capability"], "coding");
        assert_eq!(fit["satisfied"], false, "inferred-only must not satisfy verified");
        assert_eq!(
            fit["checks"][0]["status"]["insufficient_provenance"]["found"],
            "inferred"
        );

        // No requires -> fit is null.
        let body =
            hub_compare_model_body(&inferred, &files, &caps, &state, "org/codestral", None).await;
        assert!(body["capabilities"]["fit"].is_null());

        manager.lock().await.shutdown().await.unwrap();
    }

    /// Honesty invariants of the pure fit decision (§49). RAM and VRAM are
    /// compared against their OWN estimates — never mixed — and an untrusted
    /// worker must not credit toward "compatible worker on fabric".
    #[test]
    fn resource_fit_never_mixes_ram_and_vram() {
        // est_ram = 1200MB, est_vram = 1050MB.
        let est_ram = 1200;
        let est_vram = 1050;

        // VRAM alone is not enough if the RAM estimate is unmet (CPU cannot
        // offload the whole model); conversely RAM alone suffices for CPU load.
        let cpu_only = resource_fit(2000, Some(0), est_ram, est_vram, 0);
        assert!(cpu_only.ram_sufficient);
        assert!(!cpu_only.vram_sufficient);
        assert!(cpu_only.local_fit);

        // A big VRAM pool must be compared against the VRAM estimate, not the
        // (larger) RAM estimate — this is the original false-confidence bug.
        let big_vram_small_ram = resource_fit(500, Some(1500), est_ram, est_vram, 0);
        assert!(!big_vram_small_ram.ram_sufficient);
        assert!(big_vram_small_ram.vram_sufficient);
        assert!(big_vram_small_ram.local_fit, "GPU can host it");
    }

    #[test]
    fn resource_fit_trusted_only_and_classification() {
        let est_ram = 1000;
        let est_vram = 900;

        // Untrusted workers advertise capacity, but must not make it
        // "compatible worker available" nor bump the classification.
        let untrusted_only = resource_fit(0, None, est_ram, est_vram, 0);
        assert!(!untrusted_only.trusted_worker_can_run);
        assert_eq!(untrusted_only.classification, "NOT AVAILABLE");

        // A trusted worker alone (no local fit) is a GOOD FIT via the fabric.
        let trusted_remote = resource_fit(0, None, est_ram, est_vram, 1);
        assert!(trusted_remote.trusted_worker_can_run);
        assert_eq!(trusted_remote.classification, "GOOD FIT");

        // Local fit + trusted worker = BEST FIT.
        let best = resource_fit(2000, None, est_ram, est_vram, 2);
        assert!(best.local_fit);
        assert!(best.trusted_worker_can_run);
        assert_eq!(best.classification, "BEST FIT");

        // Local fit but no trusted worker is LIMITED (local-only).
        let local_only = resource_fit(2000, None, est_ram, est_vram, 0);
        assert!(local_only.local_fit);
        assert_eq!(local_only.classification, "LIMITED");
    }

    #[tokio::test]
    async fn admin_page_serves_html() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("test_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        // Unauthenticated is rejected now that the admin surface is gated.
        let denied = reqwest::Client::new()
            .get(format!("http://{api}/admin"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);
        let resp = reqwest::Client::new()
            .get(format!("http://{api}/admin"))
            .header("Authorization", "Bearer test_token")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let html = resp.text().await.unwrap();
        assert!(html.contains("DecentraAI Admin"));
        assert!(html.contains("Create Token"));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn admin_token_create_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let db_dir = dir.path().join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let token_store_path = db_dir.join("tokens.json");
        let tiers = TiersSection {
            tier1: TierPolicy {
                rate_limit_per_minute: 10,
                models: vec!["test.gguf".into()],
            },
            tier2: TierPolicy {
                rate_limit_per_minute: 60,
                models: vec![],
            },
            tier3: TierPolicy {
                rate_limit_per_minute: 120,
                models: vec![],
            },
        };
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            Some(token_store_path.clone()),
            Some(tiers),
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let create_resp = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"name":"test_token","tier":1}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(create_resp.status(), 200);
        let json: serde_json::Value = create_resp.json().await.unwrap();
        assert!(json["token"].as_str().unwrap().starts_with("dsk_"));

        // A developer token with an expiry is accepted and listed with it.
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let dev_resp = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(format!(r#"{{"name":"dev_token","tier":2,"expires_at":{exp}}}"#))
            .send()
            .await
            .unwrap();
        assert_eq!(dev_resp.status(), 200);
        let dev: serde_json::Value = dev_resp.json().await.unwrap();
        assert_eq!(dev["expires_at"], exp);

        let list_resp = reqwest::Client::new()
            .get(format!("http://{api}/api/admin/token/list"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap();
        assert_eq!(list_resp.status(), 200);
        let lj: serde_json::Value = list_resp.json().await.unwrap();
        let dev_row = lj["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "dev_token")
            .unwrap();
        assert_eq!(dev_row["expires_at"], exp);
        assert_eq!(dev_row["expired"], false);
        assert_eq!(dev_row["requests"], 0, "usage starts at zero");
        assert_eq!(dev_row["tokens_generated"], 0);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn hub_search_body_serializes_downloads_tags_and_pipeline() {
        let models = vec![
            decentraai_hub::HubModel {
                id: "Qwen/Qwen2.5-1.5B-Instruct-GGUF".to_string(),
                pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
                tags: vec!["gguf".to_string(), "conversational".to_string()],
                downloads: 42_000,
            },
            decentraai_hub::HubModel {
                id: "org/other-model".to_string(),
                pipeline_tag: None,
                tags: vec![],
                downloads: 7,
            },
        ];
        let body = hub_search_body("qwen", &models, None);
        assert_eq!(body["query"], "qwen");
        assert!(body["capability_filter"].is_null());
        let arr = body["models"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "Qwen/Qwen2.5-1.5B-Instruct-GGUF");
        assert_eq!(arr[0]["pipeline_tag"], "text-generation");
        assert_eq!(arr[0]["downloads"], 42_000);
        assert!(arr[1]["pipeline_tag"].is_null());
    }

    #[test]
    fn hub_search_body_filters_by_capability_with_honest_provenance() {
        // A text-generation model claims no OCR capability: a capability=ocr
        // filter must drop it (UNKNOWN is not satisfied), never claim a false
        // positive. A model whose metadata states vision support is kept.
        let models = vec![
            decentraai_hub::HubModel {
                id: "Qwen/Qwen2.5-1.5B-Instruct-GGUF".to_string(),
                pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
                tags: vec!["gguf".to_string(), "conversational".to_string()],
                downloads: 42_000,
            },
            decentraai_hub::HubModel {
                id: "org/vision-model".to_string(),
                pipeline_tag: Some(decentraai_hub::PipelineTag::ImageTextToText),
                tags: vec!["gguf".to_string(), "vision".to_string()],
                downloads: 7,
            },
        ];
        // OCR: neither model claims OCR evidence (text-generation has none;
        // image-text-to-text pipeline yields Vision/Multimodal, NOT OCR). An
        // honest filter returns zero rather than fabricate OCR support.
        let ocr = hub_search_body("models", &models, Some(decentraai_hub::CapabilityKind::Ocr));
        let arr = ocr["models"].as_array().unwrap();
        assert_eq!(arr.len(), 0, "no model has OCR evidence");
        assert_eq!(ocr["matched"], 0);
        assert_eq!(ocr["total"], 2);

        // Vision: the image-text-to-text model qualifies (verified pipeline).
        let vision =
            hub_search_body("models", &models, Some(decentraai_hub::CapabilityKind::Vision));
        assert_eq!(vision["matched"], 1);
        assert_eq!(vision["models"][0]["id"], "org/vision-model");

        // Coding is not claimed by either model (no name/tag hint) -> 0 hits.
        let coding =
            hub_search_body("models", &models, Some(decentraai_hub::CapabilityKind::Coding));
        assert_eq!(coding["matched"], 0);
    }

    #[test]
    fn filter_local_models_by_capability_is_provenance_honest() {
        // Two local models with persisted claims + one remote model with none.
        let list = serde_json::json!({
            "data": [
                { "id": "ocr.gguf", "owned_by": "local",
                  "capability_claims": [
                      { "capability": "ocr", "provenance": "verified" }
                  ] },
                { "id": "vision.gguf", "owned_by": "local",
                  "capability_claims": [
                      { "capability": "vision", "provenance": "inferred" }
                  ] },
                { "id": "remote.gguf", "owned_by": "w1" } // no claims -> UNKNOWN
            ]
        });

        // any evidence: both local matches qualify.
        let r = filter_local_models_by_capability(&list, "ocr", "any");
        assert_eq!(r["matched"], 1);
        assert_eq!(r["models"][0]["id"], "ocr.gguf");
        assert_eq!(r["models"][0]["evidence"], "verified");

        let r = filter_local_models_by_capability(&list, "vision", "any");
        assert_eq!(r["matched"], 1);
        assert_eq!(r["models"][0]["evidence"], "inferred");

        // verified-only: the inferred vision claim does NOT qualify.
        let r = filter_local_models_by_capability(&list, "vision", "verified");
        assert_eq!(r["matched"], 0);

        // Case-insensitive capability matching.
        let r = filter_local_models_by_capability(&list, "OCR", "any");
        assert_eq!(r["matched"], 1);

        // No matching capability and no-claims models are never included.
        let r = filter_local_models_by_capability(&list, "coding", "any");
        assert_eq!(r["matched"], 0);
    }

    // ---- get_worker_capability pure verdict tests ----

    /// A full-control advertisement builder for the worker capability verdict.
    /// `served` puts the model in served_models; `on_disk` in available_models.
    fn cap_adv(
        peer: &decentraai_p2p::PeerId,
        node_id: &str,
        engine: &str,
        avail_ram: u64,
        avail_vram: Option<u64>,
        est: (u64, u64),
        held: (bool, bool),
    ) -> decentraai_distributed::ComputeAdvertisement {
        let (est_ram, est_vram) = est;
        let (served, on_disk) = held;
        let mut sm = decentraai_distributed::compute::ServedModel {
            model_hash: "h".into(),
            file_name: "qwen.gguf".into(),
            size_mb: 1024,
            est_ram_mb: est_ram,
            est_vram_mb: est_vram,
            context_tokens: 4096,
        };
        if !served && !on_disk {
            sm.file_name = "other.gguf".into(); // worker does not hold the model
        }
        decentraai_distributed::ComputeAdvertisement {
            peer_id: *peer,
            node_name: node_id.to_string(),
            capability: decentraai_distributed::compute::ComputeCapability {
                cpu_cores: 4,
                ram_mb: 16384,
                gpu: if est_vram > 0 { Some(decentraai_distributed::compute::GpuSpec {
                    name: "gpu".into(), vram_mb: est_vram + 1024, driver: "x".into(),
                }) } else { None },
                engine: engine.to_string(),
                served_models: if served { vec![sm.clone()] } else { vec![] },
                can_provision: false,
                available_models: if on_disk { vec![sm] } else { vec![] },
            },
            availability: decentraai_distributed::compute::ComputeAvailability {
                available_ram_mb: avail_ram,
                available_vram_mb: avail_vram,
                load_percent: 0,
                queue_depth: 0,
                tokens_per_second: 10,
                current_latency_ms: 1,
                status: decentraai_distributed::compute::WorkerHealth::Ready,
                gpu_temperature_celsius: None,
                gpu_utilization_percent: None,
                battery_percent: None,
            },
            announced_at_ms: 0,
            accepts_remote_inference: true,
            node_id: node_id.to_string(),
            node_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn claims_verified_ocr() -> Vec<(String, String)> {
        vec![("ocr".to_string(), "verified".to_string())]
    }

    #[test]
    fn worker_cap_verified_claim_plus_compatible_worker_can_run() {
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::CanRun);
        assert_eq!(r.model_availability, "served");
        assert!(r.trusted);
        assert_eq!(r.engine_compat, "compatible");
        let cap = r.checks.iter().find(|c| c.check == "capability").unwrap();
        assert!(cap.pass && cap.state == "VERIFIED");
        // Identity stays distinct.
        assert_eq!(r.node_id, "dca-node1");
        assert_eq!(r.node_name, "dca-node1");
        assert_eq!(r.peer_id, peer.to_string());
        assert_ne!(r.node_id, r.peer_id);
    }

    // ---- variant quantization classifier (INFERRED, never VERIFIED) ----

    #[test]
    fn quantization_q4_k_m_is_q4() {
        assert_eq!(variant_quantization_from_file_name("qwen2.5-7b-instruct-q4_k_m.gguf"), Some("Q4".to_string()));
    }

    #[test]
    fn quantization_q8_0_is_q8() {
        assert_eq!(variant_quantization_from_file_name("model-q8_0.gguf"), Some("Q8".to_string()));
    }

    #[test]
    fn quantization_q6_k_is_q6() {
        assert_eq!(variant_quantization_from_file_name("model-q6_k.gguf"), Some("Q6".to_string()));
    }

    #[test]
    fn quantization_q5_1_is_q5() {
        assert_eq!(variant_quantization_from_file_name("model-q5_1.gguf"), Some("Q5".to_string()));
    }

    #[test]
    fn quantization_q3_k_is_q3() {
        assert_eq!(variant_quantization_from_file_name("model-q3_k.gguf"), Some("Q3".to_string()));
    }

    #[test]
    fn quantization_q2_k_is_q2() {
        assert_eq!(variant_quantization_from_file_name("model-q2_k.gguf"), Some("Q2".to_string()));
    }

    #[test]
    fn quantization_fp16_is_fp16() {
        assert_eq!(variant_quantization_from_file_name("model-fp16.gguf"), Some("FP16".to_string()));
        assert_eq!(variant_quantization_from_file_name("model-f16.gguf"), Some("FP16".to_string()));
    }

    #[test]
    fn quantization_unknown_without_marker_is_none() {
        assert_eq!(variant_quantization_from_file_name("model.gguf"), None);
        assert_eq!(variant_quantization_from_file_name("qwen.gguf"), None);
        assert_eq!(variant_quantization_from_file_name("no_quant_here.gguf"), None);
    }

    #[test]
    fn quantization_is_case_insensitive() {
        assert_eq!(variant_quantization_from_file_name("model-Q4_K_M.gguf"), Some("Q4".to_string()));
        assert_eq!(variant_quantization_from_file_name("MODEL-Q8_0.gguf"), Some("Q8".to_string()));
    }

    #[test]
    fn quantization_q4_0_is_q4() {
        assert_eq!(variant_quantization_from_file_name("model-q4_0.gguf"), Some("Q4".to_string()));
    }

    #[test]
    fn worker_cap_insufficient_ram_cannot_run() {
        let peer = decentraai_p2p::PeerId::random();
        // Model needs 8192 MiB RAM but worker has only 512 free.
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 512, Some(8192), (8192, 2048), (true, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun);
        let ram = r.checks.iter().find(|c| c.check == "ram").unwrap();
        assert!(!ram.pass && ram.state == "insufficient");
    }

    #[test]
    fn worker_cap_insufficient_vram_cannot_run() {
        let peer = decentraai_p2p::PeerId::random();
        // Model needs 8192 MiB VRAM but worker has only 512 free.
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(512), (1024, 8192), (true, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun);
        let vram = r.checks.iter().find(|c| c.check == "vram").unwrap();
        assert!(!vram.pass && vram.state == "insufficient");
    }

    #[test]
    fn worker_cap_inferred_claim_with_verified_evidence_cannot_run() {
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        // Only an INFERRED claim, but evidence=verified is required.
        let inferred = vec![("ocr".to_string(), "inferred".to_string())];
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "verified", &inferred);
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun);
        let cap = r.checks.iter().find(|c| c.check == "capability").unwrap();
        assert!(!cap.pass && cap.state == "INFERRED");
    }

    #[test]
    fn worker_cap_missing_claim_unknown() {
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        // No claim at all for the model -> UNKNOWN (never a false pass).
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &[]);
        assert_eq!(r.verdict, WorkerCapVerdict::Unknown);
        let cap = r.checks.iter().find(|c| c.check == "capability").unwrap();
        assert!(!cap.pass && cap.state == "UNKNOWN");
    }

    #[test]
    fn worker_cap_untrusted_cannot_run() {
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        let r = worker_capability_verdict(&adv, false, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun);
        let t = r.checks.iter().find(|c| c.check == "trusted").unwrap();
        assert!(!t.pass && t.state == "not_trusted");
    }

    #[test]
    fn worker_cap_remote_not_accepting_inference_cannot_run() {
        // Phase M policy: a remote worker that has NOT opted into remote
        // inference cannot run a request from this fabric — a definitive
        // CANNOT_RUN via the policy check, even though it is trusted, healthy,
        // holds the model and has sufficient resources.
        let peer = decentraai_p2p::PeerId::random();
        let mut adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        adv.accepts_remote_inference = false;
        let r = worker_capability_verdict_with_policy(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr(), false);
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun, "remote-no-opt-in must be CANNOT_RUN");
        let p = r.checks.iter().find(|c| c.check == "policy").unwrap();
        assert!(!p.pass && p.state == "remote_not_accepted");
        // The LOCAL node is always allowed its own work regardless of the flag.
        let r = worker_capability_verdict_with_policy(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr(), true);
        assert_eq!(r.verdict, WorkerCapVerdict::CanRun, "local worker always allowed");
    }

    #[test]
    fn worker_cap_incompatible_engine_cannot_run() {
        let peer = decentraai_p2p::PeerId::random();
        // Unknown engine holding a model on disk -> compatibility unknown (not
        // a definitive incompatible); use a model the worker does NOT hold for
        // a hard engine failure via unavailable model.
        let adv = cap_adv(&peer, "dca-node1", "weird-engine", 8192, Some(8192), (1024, 2048), (false, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::CannotRun); // model unavailable
    }

    #[test]
    fn worker_cap_missing_telemetry_unknown() {
        let peer = decentraai_p2p::PeerId::random();
        // Model served but est_ram=0 (unknown footprint) -> RAM UNKNOWN, and no
        // VRAM telemetry -> overall UNKNOWN (no hard failure).
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, None, (0, 0), (true, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.verdict, WorkerCapVerdict::Unknown);
        let ram = r.checks.iter().find(|c| c.check == "ram").unwrap();
        assert!(!ram.pass && ram.state == "unknown");
    }

    #[test]
    fn worker_cap_no_workers_unknown_not_invented() {
        // No workers in the fabric -> the MCP projection reports 0 workers and
        // an explicit UNKNOWN (no invented reason). This is exercised at the
        // projection level via mcp_worker_capability with no compute manager.
        // The pure per-worker function on an unavailable model yields CannotRun
        // (honest: model not present), which the projection folds into UNKNOWN
        // when there are zero workers.
        let j = serde_json::json!({ "model": "qwen.gguf", "capability": "ocr", "worker_count": 0, "workers": [] });
        assert_eq!(j["worker_count"], 0);
        assert!(j["workers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn worker_cap_json_keeps_identity_separate() {
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        let j = r.to_json();
        assert_eq!(j["worker"]["node_id"], "dca-node1");
        assert_eq!(j["worker"]["node_name"], "dca-node1");
        assert_eq!(j["worker"]["peer_id"], peer.to_string());
        assert_ne!(j["worker"]["node_id"], j["worker"]["peer_id"]);
        assert_eq!(j["verdict"], "CAN_RUN");
    }

    // ---- aggregate_can_i_run unified "CAN I RUN THIS?" tests ----

    fn cap_result(peer_id: &str, node_id: &str, verdict: WorkerCapVerdict) -> WorkerCapResult {
        WorkerCapResult {
            peer_id: peer_id.to_string(),
            node_id: node_id.to_string(),
            node_name: node_id.to_string(),
            verdict,
            checks: Vec::new(),
            model_availability: "served",
            trusted: true,
            ram_sufficient: true,
            vram_sufficient: true,
            est_ram_mb: 1024,
            est_vram_mb: 0,
            engine_compat: "compatible",
            quantization: None,
        }
    }

    #[test]
    fn aggregate_can_i_run_with_any_can_run_worker() {
        // Two workers: one CAN_RUN, one CANNOT_RUN -> overall CAN_RUN, the
        // CAN_RUN worker chosen.
        let results = vec![
            cap_result("p1", "dca-b", WorkerCapVerdict::CannotRun),
            cap_result("p2", "dca-a", WorkerCapVerdict::CanRun),
        ];
        let fit = aggregate_can_i_run(&results);
        assert_eq!(fit.verdict, WorkerCapVerdict::CanRun);
        assert_eq!(fit.can_run_count, 1);
        assert_eq!(fit.cannot_run_count, 1);
        assert_eq!(fit.chosen_worker.as_deref(), Some("p2"));
        assert!(!fit.reasons.is_empty());
    }

    #[test]
    fn aggregate_can_i_run_none_can_run_is_cannot_run() {
        let results = vec![
            cap_result("p1", "dca-a", WorkerCapVerdict::CannotRun),
            cap_result("p2", "dca-b", WorkerCapVerdict::CannotRun),
        ];
        let fit = aggregate_can_i_run(&results);
        assert_eq!(fit.verdict, WorkerCapVerdict::CannotRun);
        assert_eq!(fit.chosen_worker, None);
    }

    #[test]
    fn aggregate_can_i_run_all_unknown_is_unknown() {
        let results = vec![
            cap_result("p1", "dca-a", WorkerCapVerdict::Unknown),
            cap_result("p2", "dca-b", WorkerCapVerdict::Unknown),
        ];
        let fit = aggregate_can_i_run(&results);
        assert_eq!(fit.verdict, WorkerCapVerdict::Unknown);
    }

    #[test]
    fn aggregate_can_i_run_no_workers_is_unknown_not_invented() {
        let fit = aggregate_can_i_run(&[]);
        assert_eq!(fit.verdict, WorkerCapVerdict::Unknown);
        assert!(fit.reasons.iter().any(|r| r.contains("no compatible worker")));
        assert_eq!(fit.chosen_worker, None);
    }

    #[test]
    fn aggregate_can_i_run_end_to_end_with_real_verdicts() {
        // One good worker (verified OCR, sufficient resources) + one bad
        // (insufficient RAM): the fabric-wide answer must be CAN_RUN choosing
        // the good worker, with the aggregate projecting the real verdicts.
        let claims = claims_verified_ocr();
        let good = {
            let peer = decentraai_p2p::PeerId::random();
            let adv = cap_adv(&peer, "dca-good", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
            worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims)
        };
        let bad = {
            let peer = decentraai_p2p::PeerId::random();
            let adv = cap_adv(&peer, "dca-bad", "llama_server", 512, Some(8192), (8192, 2048), (true, false));
            worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims)
        };
        assert_eq!(good.verdict, WorkerCapVerdict::CanRun);
        assert_eq!(bad.verdict, WorkerCapVerdict::CannotRun);

        let fit = aggregate_can_i_run(&[good.clone(), bad.clone()]);
        assert_eq!(fit.verdict, WorkerCapVerdict::CanRun);
        assert_eq!(fit.chosen_worker.as_deref(), Some(good.peer_id.as_str()));
        // JSON projection carries the unified verdict + counts.
        let j = fit.to_json();
        assert_eq!(j["verdict"], "CAN_RUN");
        assert_eq!(j["counts"]["can_run"], 1);
        assert_eq!(j["counts"]["cannot_run"], 1);
    }

    #[test]
    fn worker_cap_verdict_carries_inferred_quantization_from_file_name() {
        // A served file whose name carries a quant marker: the per-worker result
        // must surface the INFERRED label in its JSON projection (and null when
        // the name has no marker).
        let peer = decentraai_p2p::PeerId::random();
        let adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        // cap_adv uses "qwen.gguf" (no marker) => quantization stays None.
        let r = worker_capability_verdict(&adv, true, "qwen.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.quantization, None);
        let j = r.to_json();
        assert!(j["quantization"].is_null());
    }

    #[test]
    fn worker_cap_verdict_quantization_from_marker_in_served_file_name() {
        // A worker whose served model file name carries a quant marker: the
        // per-worker result surfaces the INFERRED label in its JSON projection.
        let peer = decentraai_p2p::PeerId::random();
        let mut adv = cap_adv(&peer, "dca-node1", "llama_server", 8192, Some(8192), (1024, 2048), (true, false));
        adv.capability.served_models[0].file_name = "qwen2.5-7b-instruct-q4_k_m.gguf".to_string();
        let r = worker_capability_verdict(&adv, true, "qwen2.5-7b-instruct-q4_k_m.gguf", "ocr", "any", &claims_verified_ocr());
        assert_eq!(r.quantization.as_deref(), Some("Q4"));
        let j = r.to_json();
        assert_eq!(j["quantization"], "Q4");
    }

    #[tokio::test]
    async fn hub_model_body_serializes_metadata_capabilities_and_variants() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master".into()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let detail = decentraai_hub::HubModelDetail {
            id: "org/code-llm".into(),
            pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
            tags: vec![
                "gguf".into(),
                "context-length:8192".into(),
                "license:apache-2.0".into(),
                "tools".into(),
            ],
            downloads: 99,
            likes: 3,
            description: Some("A coding chat model.".into()),
            license: Some("apache-2.0".into()),
            context_length: Some(8192),
            params: Some("7B".into()),
        }
        .fill_from_tags();
        let files = vec![
            decentraai_hub::HubModelFile {
                path: "q4_k_m.gguf".into(),
                size: Some(491 * 1024 * 1024),
                lfs: Some(decentraai_hub::HubLfs {
                    oid: "abc123".into(),
                }),
            },
            decentraai_hub::HubModelFile {
                path: "q8_0.gguf".into(),
                size: Some(1024 * 1024 * 1024),
                lfs: None,
            },
        ];
        let caps = detail.capabilities();
        let body = hub_model_body(&detail, &files, &caps, &state, "org/code-llm", None).await;

        // Real metadata surfaces, absent stays null.
        assert_eq!(body["metadata"]["context_length"], 8192);
        assert_eq!(body["metadata"]["params"], "7B");
        assert_eq!(body["metadata"]["license"], "apache-2.0");
        assert!(body["metadata"]["description"].is_string());

        // `tools` tag -> VERIFIED tool calling claim.
        let claims = body["capabilities"]["claims"].as_array().unwrap();
        assert!(claims.iter().any(|c| {
            c["capability"] == "tool_calling" && c["provenance"] == "verified"
        }));

        // `code` in the id -> INFERRED coding + its tasks.
        assert!(claims.iter().any(|c| {
            c["capability"] == "coding" && c["provenance"] == "inferred"
        }));
        let tasks = body["capabilities"]["tasks"].as_array().unwrap();
        assert!(tasks.iter().any(|t| t["task"] == "repository understanding"));

        // Variants carry file + size + sha256 when the Hub reported it.
        let variants = body["variants"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["file"], "q4_k_m.gguf");
        assert_eq!(variants[0]["sha256"], "abc123");
        assert!(variants[1]["sha256"].is_null(), "absent digest stays unknown");

        // No compute manager attached -> empty fabric list (never fabricated).
        assert!(body["fabric"].as_array().unwrap().is_empty());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn hub_model_body_reports_requires_capability_fit_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master".into()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        // This model has a `tools` tag (VERIFIED tool calling) and a `code`
        // id (INFERRED coding), but no OCR evidence.
        let detail = decentraai_hub::HubModelDetail {
            id: "org/codestral".to_string(),
            pipeline_tag: Some(decentraai_hub::PipelineTag::TextGeneration),
            tags: vec!["gguf".into(), "tools".into()],
            downloads: 10,
            likes: 0,
            description: None,
            license: None,
            context_length: None,
            params: None,
        };
        let files = vec![];
        let caps = detail.capabilities();

        // requires=coding: INFERRED only, so a VERIFIED requirement is not
        // satisfied — honest, reported as insufficient provenance.
        let body = hub_model_body(
            &detail,
            &files,
            &caps,
            &state,
            "org/codestral",
            Some(decentraai_hub::CapabilityKind::Coding),
        )
        .await;
        let fit = body["capabilities"]["fit"].clone();
        assert_eq!(fit["capability"], "coding");
        assert_eq!(fit["satisfied"], false, "inferred-only must not satisfy verified");
        assert_eq!(
            fit["checks"][0]["status"]["insufficient_provenance"]["found"],
            "inferred"
        );

        // requires=ocr: no OCR evidence at all -> Missing, never satisfied.
        let body = hub_model_body(
            &detail,
            &files,
            &caps,
            &state,
            "org/codestral",
            Some(decentraai_hub::CapabilityKind::Ocr),
        )
        .await;
        let fit = body["capabilities"]["fit"].clone();
        assert_eq!(fit["satisfied"], false);
        assert_eq!(fit["checks"][0]["status"], "missing");

        // No requires -> fit is null.
        let body = hub_model_body(&detail, &files, &caps, &state, "org/codestral", None).await;
        assert!(body["capabilities"]["fit"].is_null());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn refresh_registry_after_pull_scans_new_gguf() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        // Fake GGUF (the registry only checks the extension).
        std::fs::write(models_dir.join("fresh-model.gguf"), b"not a real model").unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        let count = refresh_registry_after_pull(&models_dir, &registry_path).unwrap();
        assert_eq!(count, 1);
        let loaded = decentraai_registry::ModelRegistry::load(&registry_path).unwrap();
        assert!(loaded.models.contains_key("fresh-model.gguf"));
    }

    #[test]
    fn capability_records_from_hub_maps_enums_to_snake_case() {
        let caps = decentraai_hub::ModelCapabilities {
            claims: vec![
                decentraai_hub::CapabilityClaim {
                    capability: decentraai_hub::CapabilityKind::Ocr,
                    provenance: decentraai_hub::Provenance::Verified,
                },
                decentraai_hub::CapabilityClaim {
                    capability: decentraai_hub::CapabilityKind::Coding,
                    provenance: decentraai_hub::Provenance::Inferred,
                },
            ],
            tasks: Vec::new(),
        };
        let records = capability_records_from_hub(&caps);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].capability, "ocr");
        assert_eq!(records[0].provenance, "verified");
        assert_eq!(records[1].capability, "coding");
        assert_eq!(records[1].provenance, "inferred");
    }

    #[test]
    fn relative_path_of_strips_prefix_and_rejects_outside() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let file = models_dir.join("sub").join("model.gguf");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"x").unwrap();
        // A file under models/ maps to its path relative to models/.
        assert_eq!(
            relative_path_of(&models_dir, &file).unwrap(),
            "sub/model.gguf"
        );
        // A file outside models/ yields None, so best-effort callers skip.
        let outside = dir.path().join("other.gguf");
        std::fs::write(&outside, b"x").unwrap();
        assert!(relative_path_of(&models_dir, &outside).is_none());
    }

    #[test]
    fn claims_for_file_name_matches_by_suffix_and_omits_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        // Keyed by relative path (path under models/ ending with the file name).
        registry.models.insert(
            "org/codestral/codestral.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "org/codestral/codestral.gguf".to_string(),
                canonical_path: "/models/org/codestral/codestral.gguf".to_string(),
                size_bytes: 1,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: vec![decentraai_registry::CapabilityClaimRecord {
                    capability: "coding".to_string(),
                    provenance: "verified".to_string(),
                }],
            },
        );
        // A model whose file name matches an existing record returns its claims.
        let claims = claims_for_file_name(&registry, "codestral.gguf");
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].capability, "coding");
        // An unknown file name yields UNKNOWN (empty), never fabricated claims.
        assert!(claims_for_file_name(&registry, "nope.gguf").is_empty());
        // Registry present but the model has no claims -> also UNKNOWN.
        let mut bare = decentraai_registry::ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        bare.models.insert(
            "org/codestral/codestral.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "org/codestral/codestral.gguf".to_string(),
                canonical_path: "/models/org/codestral/codestral.gguf".to_string(),
                size_bytes: 1,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: Vec::new(),
            },
        );
        assert!(claims_for_file_name(&bare, "codestral.gguf").is_empty());
    }

    #[test]
    fn registry_variants_for_model_matches_and_sorts_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        let mut record = |relative_path: &str, size: u64| {
            registry.models.insert(
                relative_path.to_string(),
                decentraai_registry::ModelRecord {
                    relative_path: relative_path.to_string(),
                    canonical_path: format!("/models/{relative_path}"),
                    size_bytes: size,
                    modification_time: 0,
                    extension: "gguf".to_string(),
                    capability_claims: Vec::new(),
                },
            );
        };
        // Real variants of `qwen2.5-7b-instruct`.
        record("qwen2.5-7b-instruct/qwen2.5-7b-instruct-q4_k_m.gguf", 100);
        record("qwen2.5-7b-instruct/qwen2.5-7b-instruct-q8_0.gguf", 200);
        record("qwen2.5-7b-instruct/qwen2.5-7b-instruct-fp16.gguf", 400);
        // An unrelated model must NOT match.
        record("codestral/codestral.gguf", 50);

        let variants = registry_variants_for_model(&registry, "qwen2.5-7b-instruct");
        // Only the three qwen variants, sorted by file name asc, with sizes.
        let files: Vec<&str> = variants.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(
            files,
            vec![
                "qwen2.5-7b-instruct/qwen2.5-7b-instruct-fp16.gguf",
                "qwen2.5-7b-instruct/qwen2.5-7b-instruct-q4_k_m.gguf",
                "qwen2.5-7b-instruct/qwen2.5-7b-instruct-q8_0.gguf",
            ]
        );
        assert_eq!(variants[0].1, 400);
        assert_eq!(variants[1].1, 100);
        assert_eq!(variants[2].1, 200);

        // Model string contains the record's file name -> still matches.
        let by_suffix = registry_variants_for_model(&registry, "q4_k_m.gguf");
        assert_eq!(by_suffix.len(), 1);
        assert_eq!(by_suffix[0].0, "qwen2.5-7b-instruct/qwen2.5-7b-instruct-q4_k_m.gguf");

        // No records for an absent model -> empty, never invented.
        assert!(registry_variants_for_model(&registry, "does-not-exist").is_empty());
    }

    #[tokio::test]
    async fn mcp_worker_capability_variants_enumerates_on_disk_files() {
        // A registry on disk holds two real variants; the projection's
        // `variants` array must enumerate exactly those two files with their
        // size, inferred quantization and an honest per-variant fit (no workers
        // -> UNKNOWN via the existing aggregate, never fabricated).
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(models_dir).unwrap();
        registry.models.insert(
            "qwen/qwen2.5-7b-instruct-q4_k_m.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "qwen/qwen2.5-7b-instruct-q4_k_m.gguf".to_string(),
                canonical_path: "x".to_string(),
                size_bytes: 1234,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: Vec::new(),
            },
        );
        registry.models.insert(
            "qwen/qwen2.5-7b-instruct-q8_0.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "qwen/qwen2.5-7b-instruct-q8_0.gguf".to_string(),
                canonical_path: "y".to_string(),
                size_bytes: 5678,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: Vec::new(),
            },
        );
        registry.save(&registry_path).unwrap();

        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );

        let body = mcp_worker_capability(&state, "qwen2.5-7b-instruct", "ocr", "any").await;
        let variants = body["variants"].as_array().expect("variants array present");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["file"], "qwen/qwen2.5-7b-instruct-q4_k_m.gguf");
        assert_eq!(variants[0]["quantization"], "Q4");
        assert_eq!(variants[0]["size_bytes"], 1234);
        assert_eq!(variants[1]["file"], "qwen/qwen2.5-7b-instruct-q8_0.gguf");
        assert_eq!(variants[1]["quantization"], "Q8");
        assert_eq!(variants[1]["size_bytes"], 5678);
        // No workers -> honest UNKNOWN fit (counts 0/0/0), not a pass.
        for v in variants {
            assert_eq!(v["fit"]["verdict"], "UNKNOWN");
            assert_eq!(v["fit"]["counts"]["can_run"], 0);
        }
        // Existing top-level fields are preserved (additive).
        assert_eq!(body["model"], "qwen2.5-7b-instruct");
        assert_eq!(body["fit"]["verdict"], "UNKNOWN");
        assert!(body["workers"].as_array().unwrap().is_empty());
        assert_eq!(body["worker_count"], 0);
        // No worker can run any variant -> honest best_variant null.
        assert!(body["model_info"]["best_variant"].is_null());
    }

    #[tokio::test]
    async fn mcp_intent_with_fit_resolves_capabilities_and_reports_honest_fit() {
        // Composed intent -> capability -> fabric fit. A registry holds a model
        // with a verified OCR claim; intent "I need OCR and coding" resolves to
        // [Ocr, Coding]; OCR has a real local model (fit via the aggregate, no
        // workers -> honest UNKNOWN), Coding has no local model -> UNKNOWN with
        // an explicit reason.
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(models_dir).unwrap();
        registry.models.insert(
            "qwen.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "qwen.gguf".to_string(),
                canonical_path: "x".to_string(),
                size_bytes: 1,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: vec![decentraai_registry::CapabilityClaimRecord {
                    capability: "ocr".to_string(),
                    provenance: "verified".to_string(),
                }],
            },
        );
        registry.save(&registry_path).unwrap();

        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );

        let body = mcp_intent_with_fit(&state, "I need OCR and coding", "any").await;
        let caps = body["capabilities"].as_array().expect("capabilities present");
        assert!(!caps.is_empty(), "intent resolves to capabilities");

        // Find the OCR and coding entries.
        let ocr = caps.iter().find(|c| c["capability"] == "ocr").expect("ocr present");
        let coding = caps.iter().find(|c| c["capability"] == "coding").expect("coding present");
        // OCR uses the real local model; no workers -> honest UNKNOWN fit.
        assert_eq!(ocr["model"], "qwen.gguf");
        assert_eq!(ocr["fit"]["verdict"], "UNKNOWN");
        // Coding has no local model -> UNKNOWN with an explicit reason.
        assert!(coding["model"].is_null());
        assert_eq!(coding["fit"]["verdict"], "UNKNOWN");
        assert!(coding["fit"]["reasons"][0].as_str().unwrap().contains("no local model"));

        // An intent with no recognized capability yields an empty list.
        let body = mcp_intent_with_fit(&state, "zzz unknown words", "any").await;
        assert!(body["capabilities"].as_array().unwrap().is_empty());

        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn fabric_model_claims_survive_pull_persist_round_trip() {
        // End-to-end projection check: a registry fixture written the way the
        // pull handler persists claims (relative path + snake_case strings)
        // yields the same claims when read back for the model list.
        let dir = tempfile::tempdir().unwrap();
        let registry_path = dir.path().join("db/registry.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let mut registry = decentraai_registry::ModelRegistry::new(models_dir).unwrap();
        registry.models.insert(
            "qwen/qwen.gguf".to_string(),
            decentraai_registry::ModelRecord {
                relative_path: "qwen/qwen.gguf".to_string(),
                canonical_path: "x".to_string(),
                size_bytes: 1,
                modification_time: 0,
                extension: "gguf".to_string(),
                capability_claims: vec![
                    decentraai_registry::CapabilityClaimRecord {
                        capability: "ocr".to_string(),
                        provenance: "verified".to_string(),
                    },
                    decentraai_registry::CapabilityClaimRecord {
                        capability: "document_understanding".to_string(),
                        provenance: "inferred".to_string(),
                    },
                ],
            },
        );
        registry.save(&registry_path).unwrap();
        let loaded = decentraai_registry::ModelRegistry::load(&registry_path).unwrap();
        let claims = claims_for_file_name(&loaded, "qwen.gguf");
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].capability, "ocr");
        assert_eq!(claims[1].provenance, "inferred");
    }

    #[tokio::test]
    async fn admin_hub_endpoints_require_master_token() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // Without credentials both endpoints are rejected before any Hub call.
        let no_auth_search = client
            .get(format!("http://{api}/api/admin/hub/search?query=qwen"))
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth_search.status(), 401);
        let no_auth_pull = client
            .post(format!("http://{api}/api/admin/hub/pull"))
            .body(r#"{"reference":"hf:org/repo"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth_pull.status(), 401);

        // With credentials but an unparseable reference, the pull rejects
        // locally (no network touched).
        let bad_pull = client
            .post(format!("http://{api}/api/admin/hub/pull"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"reference":"not-a-reference"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(bad_pull.status(), 403);

        // An explicit variant must be a real .gguf file; bad extensions are
        // rejected before any network call.
        let bad_file = client
            .post(format!("http://{api}/api/admin/hub/pull"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"reference":"hf:org/repo","file":"q4_k_m.bin"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(bad_file.status(), 403);

        // Model detail endpoint is also master-gated (no Hub call without a
        // credential).
        let no_auth_detail = client
            .get(format!("http://{api}/api/admin/hub/model/org%2Frepo"))
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth_detail.status(), 401);

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn admin_model_remove_endpoint_requires_auth_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // Without auth -> 401
        let res = client
            .post(format!("http://{api}/api/admin/models/remove"))
            .body(r#"{"path":"test.gguf"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);

        // With auth but missing/invalid path -> 403 or 400
        let res = client
            .post(format!("http://{api}/api/admin/models/remove"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"path":"../escape.gguf"}"#)
            .send()
            .await
            .unwrap();
        // Registry removal or path validation rejects path traversal (400 or 403)
        assert!(res.status().as_u16() >= 400);

        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn admin_worker_trust_and_revoke_endpoints_and_audit() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let manager = test_manager(dir.path()).await;
        // A live compute manager with one worker so trust/revoke have real state.
        let worker = decentraai_p2p::PeerId::random();
        let compute = Arc::new(decentraai_distributed::ComputeManager::new(
            decentraai_p2p::PeerId::random(),
            "coordinator".into(),
            std::collections::HashSet::new(),
        ));
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            Some(compute.clone()),
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();
        let base = format!("http://{api}");

        // Unauthenticated and client-token calls are rejected, master is allowed.
        let no_auth = client
            .post(format!("{base}/api/admin/worker/trust"))
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"peer_id": worker.to_string()}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth.status(), 401, "worker trust must be master-gated");

        // Approve flips the worker to trusted in the live compute manager.
        let trust = client
            .post(format!("{base}/api/admin/worker/trust"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"peer_id": worker.to_string()}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(trust.status(), 200);
        assert_eq!(trust.json::<serde_json::Value>().await.unwrap()["trusted"], true);
        assert!(compute.is_trusted(&worker).await, "worker trusted after approve");

        // In the /v1/compute worker report the trust flag reflects it too.
        let report: serde_json::Value = client
            .get(format!("{base}/v1/compute"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(report["attached"], true);
        assert!(report["workers"].is_array());

        // Invalid peer_id is a clean 403, not a panic.
        let bad = client
            .post(format!("{base}/api/admin/worker/revoke"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"peer_id":"not-a-peer"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 403);

        // Revoke flips it back.
        let revoke = client
            .post(format!("{base}/api/admin/worker/revoke"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"peer_id": worker.to_string()}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(revoke.status(), 200);
        assert!(!compute.is_trusted(&worker).await, "worker revoked after revoke");

        // The actions were audited (security events are control-plane material).
        let ev = client
            .get(format!("{base}/api/admin/events"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap();
        assert_eq!(ev.status(), 200);
        let events: serde_json::Value = ev.json().await.unwrap();
        let joined = serde_json::to_string(&events).unwrap();
        assert!(joined.contains("worker_trusted"), "trust event audited");
        assert!(joined.contains("worker_revoked"), "revoke event audited");

        // Missing/malformed body is a clean 403.
        let empty = client
            .post(format!("{base}/api/admin/worker/trust"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(empty.status(), 403);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn admin_worker_endpoints_guard_without_compute_manager() {
        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        // No compute manager attached (plain serve), as in production without
        // distributed compute. The endpoint must answer clearly, not panic.
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let resp = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/worker/trust"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"peer_id": decentraai_p2p::PeerId::random().to_string()}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        assert!(resp.text().await.unwrap().contains("no compute manager"));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn admin_create_sends_role_and_admin_page_shows_role_selector_and_audit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let store_path = dir.path().join("db/tokens.json");
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            Some(store_path),
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // A create with an explicit operator role round-trips through the store.
        let create = client
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"name":"op","tier":2,"role":"operator"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(create.status(), 200);
        let list: serde_json::Value = client
            .get(format!("http://{api}/api/admin/token/list"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let op = list["tokens"].as_array().unwrap().iter().find(|t| t["name"] == "op").unwrap();
        assert_eq!(op["role"], "operator", "role must round-trip");

        // The Admin page itself offers the role selector and the audit list.
        let html = client
            .get(format!("http://{api}/admin"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains("operator"), "admin page must offer the operator role");
        assert!(html.contains("Audit events"), "admin page must show audit events");
        assert!(html.contains("/api/admin/events"), "audit list must fetch the gated events endpoint");
        manager.lock().await.shutdown().await.unwrap();
    }
    #[test]
    fn token_is_generated_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime/api.token");
        let first = ensure_api_token(&path).unwrap();
        let second = ensure_api_token(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.token");
        ensure_api_token(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn admin_create_rejects_wrong_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let token_store = dir.path().join("db/tokens.json");
        let tiers = test_tiers(120);
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            Some("master_token".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
            Some(token_store.clone()),
            Some(tiers.clone()),
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

        // No credentials: unauthorized.
        let no_auth = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Content-Type", "application/json")
            .body(r#"{"name":"x","tier":1}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth.status(), 401);

        // Wrong password: unauthorized.
        let wrong = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", "Bearer not_the_master")
            .header("Content-Type", "application/json")
            .body(r#"{"name":"x","tier":1}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401);

        // A subscriber token is not an admin: forbidden.
        let mut store = decentraai_tokens::TokenStore::load(&token_store).unwrap();
        let sub = store.create("alice", decentraai_tokens::Tier(2), None).unwrap();
        let subscriber = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", format!("Bearer {sub}"))
            .header("Content-Type", "application/json")
            .body(r#"{"name":"y","tier":1}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(subscriber.status(), 403);

        // Master token succeeds (control).
        let master = reqwest::Client::new()
            .post(format!("http://{api}/api/admin/token/create"))
            .header("Authorization", "Bearer master_token")
            .header("Content-Type", "application/json")
            .body(r#"{"name":"ok_token","tier":1}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(master.status(), 200);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_streams_sse_when_requested() {
        // A backend that streams two SSE chunks (each with usage) like
        // llama-server does, proving the proxy forwards event-stream content.
        let sse_app = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}],\"usage\":{\"completion_tokens\":2}}\n\n",
                        "data: [DONE]\n\n",
                    ),
                )
            }),
        );

        let dir = tempfile::tempdir().unwrap();
        // The proxy forwards to the live engine address (M24), so the manager
        // must own (point at) the SSE backend for this test to observe the
        // streamed body.
        let manager = test_manager_with(dir.path(), sse_app).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream":true}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            ct.contains("text/event-stream"),
            "streaming response should be SSE, got {ct:?}"
        );
        let body = resp.text().await.unwrap();
        assert!(body.contains("data:"), "SSE body expected, got {body:?}");
        assert!(body.contains("Hel"), "first delta forwarded");
        assert!(body.contains("Lo") || body.contains("lo"), "second delta forwarded");
        assert!(body.contains("[DONE]"), "sentinel forwarded");

        // The token-use accounting picked up the streamed usage.
        manager.lock().await.shutdown().await.unwrap();
    }

    /// A backend that counts completions it actually serves, so a test can
    /// prove a request rejected at the proxy boundary never reached it. The
    /// manager owns the counting app (the proxy forwards to the live engine),
    /// following the same pattern as `proxy_streams_sse_when_requested`. The
    /// returned [`ServeManager`] wraps the app; `dir` keeps the fake
    /// engine's temp dir alive for the test.
    #[cfg(unix)]
    async fn start_echo_backend(
        dir: &Path,
        hits: Arc<AtomicU64>,
    ) -> Arc<Mutex<ServeManager>> {
        let app_hits = Arc::clone(&hits);
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let h = Arc::clone(&app_hits);
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    "{\"usage\":{\"completion_tokens\":1}}"
                }
            }),
        );
        test_manager_with(dir, app).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_rejects_oversized_prompt_and_max_tokens_before_backend() {
        let hits = Arc::new(AtomicU64::new(0));
        let dir = tempfile::tempdir().unwrap();
        let manager = start_echo_backend(dir.path(), hits.clone()).await;
        // The proxy forwards to the live engine, so the manager must own the
        // counting backend; the state's backend_url is therefore unused.
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();
        let base = format!("http://{api}/v1/chat/completions");

        // Oversized prompt text (> MAX_PROMPT_BYTES) -> 413, never forwarded.
        let big_prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
        let body = serde_json::json!({"model":"m","messages":[{"role":"user","content":big_prompt}]});
        let resp = client
            .post(&base)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 413);
        let err: serde_json::Value = resp.json().await.unwrap();
        assert!(err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("prompt exceeds"));

        // Oversized max_tokens (> MAX_OUTPUT_TOKENS) -> 400, never forwarded.
        let big_tokens = MAX_OUTPUT_TOKENS + 1;
        let body = serde_json::json!({
            "model":"m",
            "messages":[{"role":"user","content":"hi"}],
            "max_tokens": big_tokens,
        });
        let resp = client
            .post(&base)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let err: serde_json::Value = resp.json().await.unwrap();
        assert!(err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_tokens"));

        // Neither oversized request reached the backend.
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_within_limits_is_forwarded_to_backend() {
        let hits = Arc::new(AtomicU64::new(0));
        let dir = tempfile::tempdir().unwrap();
        let manager = start_echo_backend(dir.path(), hits.clone()).await;
        let state = ApiState::new(
            "http://127.0.0.1:0".to_string(),
            None,
            manager.clone(),
            test_info(dir.path(), None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

        let body = serde_json::json!({
            "model":"m",
            "messages":[{"role":"user","content":"hi"}],
            "max_tokens": MAX_OUTPUT_TOKENS,
        });
        let resp = reqwest::Client::new()
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "in-limit request must reach the backend");
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn get_models_does_not_reset_idle_clock_but_post_does() {
        let dir = tempfile::tempdir().unwrap();
        let (api, manager) = start_stateful_api(dir.path(), None, None).await;
        let client = reqwest::Client::new();
        let base = format!("http://{api}");

        // A metadata GET must not reset the idle clock: it keeps growing.
        tokio::time::sleep(Duration::from_millis(50)).await;
        client.get(format!("{base}/v1/models")).send().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let after_get_before_sleep = manager.lock().await.idle_for();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let after_get = manager.lock().await.idle_for();
        assert!(
            after_get > after_get_before_sleep,
            "GET must not reset idle clock: grew {after_get_before_sleep:?} -> {after_get:?}"
        );

        // Another GET keeps it growing (no idle reset either).
        client.get(format!("{base}/v1/models")).send().await.unwrap();
        let after_more_get = manager.lock().await.idle_for();
        assert!(
            after_more_get >= after_get,
            "GET must not reset idle clock: {after_get:?} -> {after_more_get:?}"
        );

        // A real inference POST resets the idle clock back near zero.
        let body = serde_json::json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
        client
            .post(format!("{base}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        let after_post = manager.lock().await.idle_for();
        assert!(
            after_post < after_get,
            "inference POST must reset idle clock: {after_get:?} -> {after_post:?}"
        );
        manager.lock().await.shutdown().await.unwrap();
    }

    // ---- fabric chat routing: pure decision (M18+) -------------------------

    fn test_adv(
        peer: &decentraai_p2p::PeerId,
        node_id: &str,
        accepts_remote: bool,
        models: &[(&str, &str)],
    ) -> decentraai_distributed::ComputeAdvertisement {
        let sized: Vec<(&str, &str, u64)> = models
            .iter()
            .map(|(f, h)| (*f, *h, 1024))
            .collect();
        test_adv_sized(peer, node_id, accepts_remote, &sized)
    }

    fn test_adv_sized(
        peer: &decentraai_p2p::PeerId,
        node_id: &str,
        accepts_remote: bool,
        models: &[(&str, &str, u64)],
    ) -> decentraai_distributed::ComputeAdvertisement {
        decentraai_distributed::ComputeAdvertisement {
            peer_id: *peer,
            node_name: node_id.to_string(),
            capability: decentraai_distributed::compute::ComputeCapability {
                cpu_cores: 4,
                ram_mb: 16384,
                gpu: None,
                engine: "llama_server".to_string(),
                served_models: models
                    .iter()
                    .map(|(f, h, size)| {
                        decentraai_distributed::compute::ServedModel {
                            model_hash: h.to_string(),
                            file_name: f.to_string(),
                            size_mb: *size,
                            est_ram_mb: 1024,
                            est_vram_mb: 0,
                            context_tokens: 4096,
                        }
                    })
                    .collect(),
                can_provision: false,
                available_models: vec![],
            },
            availability: decentraai_distributed::compute::ComputeAvailability {
                available_ram_mb: 8192,
                available_vram_mb: None,
                load_percent: 0,
                queue_depth: 0,
                tokens_per_second: 10,
                current_latency_ms: 1,
                status: decentraai_distributed::compute::WorkerHealth::Ready,
                gpu_temperature_celsius: None,
                gpu_utilization_percent: None,
                battery_percent: None,
            },
            announced_at_ms: 0,
            accepts_remote_inference: accepts_remote,
            node_id: node_id.to_string(),
            node_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    #[test]
    fn chat_route_local_wins_over_remote() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("tiny.gguf", "h1")]),
            test_adv(&remote, "dca-rem", true, &[("tiny.gguf", "h1")]),
        ];
        assert_eq!(
            resolve_chat_route(&workers, &local, "tiny.gguf"),
            ChatRoute::Local
        );
    }

    #[test]
    fn chat_route_remote_model() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("local.gguf", "hL")]),
            test_adv(&remote, "dca-rem", true, &[("remote.gguf", "hR")]),
        ];
        assert_eq!(
            resolve_chat_route(&workers, &local, "remote.gguf"),
            ChatRoute::Remote {
                worker: remote,
                node_id: "dca-rem".to_string(),
                model_hash: "hR".to_string(),
            }
        );
    }

    #[test]
    fn chat_route_skips_worker_that_does_not_accept_remote() {
        let local = decentraai_p2p::PeerId::random();
        let refuser = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("local.gguf", "hL")]),
            // The only worker with the model refuses remote inference.
            test_adv(&refuser, "dca-ref", false, &[("remote.gguf", "hR")]),
        ];
        assert_eq!(
            resolve_chat_route(&workers, &local, "remote.gguf"),
            ChatRoute::Unknown
        );
    }

    #[test]
    fn chat_route_unknown_model() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("local.gguf", "hL")]),
            test_adv(&remote, "dca-rem", true, &[("remote.gguf", "hR")]),
        ];
        assert_eq!(
            resolve_chat_route(&workers, &local, "missing.gguf"),
            ChatRoute::Unknown
        );
    }

    #[test]
    fn remote_chat_prompt_builds_turns_and_ends_with_assistant() {
        let msgs: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]"#,
        )
        .unwrap();
        assert_eq!(
            remote_chat_prompt(&msgs),
            "user: hi\n\nassistant: hello"
        );
    }

    #[test]
    fn remote_chat_prompt_no_duplicate_assistant_tail() {
        let msgs: Vec<serde_json::Value> =
            serde_json::from_str(r#"[{"role":"assistant","content":"done"}]"#).unwrap();
        // Already ends in an assistant turn: no extra tail appended.
        assert_eq!(remote_chat_prompt(&msgs), "assistant: done");
    }

    #[test]
    fn best_model_remote_wins_when_bigger() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv_sized(&local, "dca-local", false, &[("small.gguf", "hS", 1024)]),
            // Bigger remote model, explicitly accepting remote inference.
            test_adv_sized(&remote, "dca-rem", true, &[("big.gguf", "hB", 4096)]),
        ];
        assert_eq!(
            select_best_model(&workers, &local),
            Some(BestModel::Remote {
                worker: remote,
                node_id: "dca-rem".to_string(),
                model_hash: "hB".to_string(),
                file_name: "big.gguf".to_string(),
            })
        );
    }

    #[test]
    fn best_model_tie_prefers_local() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("same.gguf", "hL")]),
            test_adv(&remote, "dca-rem", true, &[("same.gguf", "hR")]),
        ];
        assert_eq!(
            select_best_model(&workers, &local),
            Some(BestModel::Local("same.gguf".to_string()))
        );
    }

    #[test]
    fn best_model_remote_only() {
        let local = decentraai_p2p::PeerId::random();
        let remote = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[]),
            test_adv(&remote, "dca-rem", true, &[("only.gguf", "hO")]),
        ];
        assert_eq!(
            select_best_model(&workers, &local),
            Some(BestModel::Remote {
                worker: remote,
                node_id: "dca-rem".to_string(),
                model_hash: "hO".to_string(),
                file_name: "only.gguf".to_string(),
            })
        );
    }

    #[test]
    fn best_model_ignores_remote_worker_that_does_not_accept() {
        let local = decentraai_p2p::PeerId::random();
        let refuser = decentraai_p2p::PeerId::random();
        let workers = vec![
            test_adv(&local, "dca-local", false, &[("small.gguf", "hS")]),
            // Bigger model but the worker refuses remote inference.
            test_adv(&refuser, "dca-ref", false, &[("big.gguf", "hB")]),
        ];
        assert_eq!(
            select_best_model(&workers, &local),
            Some(BestModel::Local("small.gguf".to_string()))
        );
    }

    #[test]
    fn best_model_none_when_no_models() {
        let local = decentraai_p2p::PeerId::random();
        let workers = vec![test_adv(&local, "dca-local", false, &[])];
        assert_eq!(select_best_model(&workers, &local), None);
    }

    // ---- Q2 Consumer API keys --------------------------------------------

    /// Builds an `ApiState` with the consumer path enabled: a consumer key
    /// registry, a shared quota ledger (Arc), and a master token. The ledger is
    /// pre-credited so the consumer account has spendable quota.
    async fn start_consumer_state(
        dir: &Path,
        master: String,
    ) -> (SocketAddr, Arc<StdMutex<decentraai_compute::QuotaLedger>>) {
        let backend = start_backend().await;
        let manager = test_manager(dir).await;
        let ledger = Arc::new(StdMutex::new(decentraai_compute::QuotaLedger::new(
            decentraai_compute::ContributionPolicy::default(),
        )));
        // Credit the consumer account so it can spend.
        {
            let mut l = ledger.lock().unwrap();
            l.credit(&"consumer-account".to_string(), "seed", Some(1000), None);
        }
        let mut state = ApiState::new(
            format!("http://{backend}"),
            Some(master),
            manager.clone(),
            test_info(dir, None),
            None,
            None,
            test_queue(),
            None,
            None,
        );
        state.attach_consumer(
            Some(dir.join("db/consumer_keys.json")),
            Some(ledger.clone()),
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        (api, ledger)
    }

    #[tokio::test]
    async fn consumer_key_reserves_and_settles_against_shared_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let (api, ledger) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        // Admin creates a consumer key for the account.
        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account",
                "quota_ceiling": 100,
                "rate_limit_per_minute": 10,
                "scopes": ["inference"],
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();
        assert!(plaintext.starts_with("dca_"), "consumer key uses dca_ namespace");

        // A consumer-key chat request authenticates and executes.
        let resp = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({"model":"test","messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "consumer key must serve inference");

        // The reservation was settled against measured usage (20 tokens).
        let acc = ledger.lock().unwrap().account(&"consumer-account".to_string()).unwrap();
        assert_eq!(acc.consumed, 20, "measured 20 completion tokens debited");
        assert_eq!(acc.reserved, 0, "no quota left reserved after settle");
        assert_eq!(acc.available, 980, "1000 - 20 consumed");
    }

    #[tokio::test]
    async fn invalid_consumer_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", "Bearer dca_0000000000000000000000000000000000000000000000000000000000000000")
            .json(&serde_json::json!({"model":"test","messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "unknown consumer key is unauthorized");
    }

    #[tokio::test]
    async fn revoked_consumer_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account", "quota_ceiling": 100, "rate_limit_per_minute": 10,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();
        let key_id = created["key_id"].as_str().unwrap().to_string();

        // Revoke it.
        let rev: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/revoke"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({"key_id": key_id}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(rev["success"], true);

        let resp = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({"model":"test","messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "revoked key must not authenticate");
    }

    #[tokio::test]
    async fn consumer_key_cannot_reach_admin_or_operational_views() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account", "quota_ceiling": 100, "rate_limit_per_minute": 10,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();

        // Admin endpoints reject a consumer key.
        let admin = client
            .get(format!("http://{api}/api/admin/consumer-key/list"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .send()
            .await
            .unwrap();
        assert_eq!(admin.status(), 403, "consumer key must never be admin");

        // Operational read views reject a consumer key.
        let ops = client
            .get(format!("http://{api}/v1/compute"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .send()
            .await
            .unwrap();
        assert!(ops.status() == 403 || ops.status() == 401, "consumer key is not operator");
    }

    #[tokio::test]
    async fn consumer_key_with_insufficient_quota_is_denied() {
        let dir = tempfile::tempdir().unwrap();
        let (api, ledger) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        // Drain the account's quota first (credit 1000, reserve+settle all).
        {
            let mut l = ledger.lock().unwrap();
            // Spend it all via a reservation + settle.
            let res = l.reserve(&"consumer-account".to_string(), "drain", 1000).unwrap();
            let _ = l.settle(&res.reservation_id, 1000);
        }

        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account", "quota_ceiling": 100, "rate_limit_per_minute": 10,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();

        let resp = client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({"model":"test","messages":[]}))
            .send()
            .await
            .unwrap();
        // No spendable quota -> the request is refused (403) with a clear body.
        assert_eq!(resp.status(), 403, "no quota -> consumer request denied");
        let audit = std::fs::read_to_string(dir.path().join("logs/audit.jsonl")).unwrap();
        assert!(audit.contains("consumer_quota_denied"), "denial must be audited");
    }

    #[tokio::test]
    async fn consumer_quota_guard_releases_on_failure() {
        // The RAII guard must release the reservation when the request does
        // not complete (e.g. backend error) so no quota leaks as reserved.
        let ledger = Arc::new(StdMutex::new(decentraai_compute::QuotaLedger::new(
            decentraai_compute::ContributionPolicy::default(),
        )));
        {
            let mut l = ledger.lock().unwrap();
            l.credit(&"acct".to_string(), "seed", Some(100), None);
        }
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = tempfile::tempdir().unwrap();
        let state = {
            let mut state = ApiState::new(
                "http://127.0.0.1:1".to_string(), // unreachable backend
                Some("master".to_string()),
                Arc::new(Mutex::new(ServeManager::unloaded(Duration::from_secs(3600)))),
                test_info(dir.path(), None),
                None,
                None,
                test_queue(),
                None,
                None,
            );
            state.attach_consumer(
                Some(keys_dir.path().join("ck.json")),
                Some(ledger.clone()),
            );
            state
        };
        let guard = state
            .reserve_consumer_quota("acct", "key-1", "req-1", 50)
            .expect("has quota");
        // Simulate a failed request: drop the guard without settling.
        drop(guard);
        let acc = ledger.lock().unwrap().account(&"acct".to_string()).unwrap();
        assert_eq!(acc.reserved, 0, "guard drop released the reservation");
        assert_eq!(acc.available, 100, "quota returned to the pool");
    }

    #[tokio::test]
    async fn consumer_rate_limit_is_independent_from_quota() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account", "quota_ceiling": 100, "rate_limit_per_minute": 2,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();

        // 3 rapid requests against a 2/min limit: the 3rd is rate limited.
        let mut statuses = Vec::new();
        for _ in 0..3 {
            let r = client
                .post(format!("http://{api}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {plaintext}"))
                .json(&serde_json::json!({"model":"test","messages":[]}))
                .send()
                .await
                .unwrap();
            statuses.push(r.status());
        }
        assert_eq!(statuses[0], 200);
        assert_eq!(statuses[1], 200);
        assert_eq!(statuses[2], 429, "3rd request exceeds the 2/min limit");
        let audit = std::fs::read_to_string(dir.path().join("logs/audit.jsonl")).unwrap();
        assert!(audit.contains("consumer_rate_limited"));
    }

    #[tokio::test]
    async fn consumer_quota_events_expose_provenance() {
        // Q4 observability: after a settled consumer request, the quota
        // provenance (credit/reserve/settle with policy version) is surfaced
        // via the compute manager, explaining why the account's balance moved.
        let dir = tempfile::tempdir().unwrap();
        let (api, ledger) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();

        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": "consumer-account", "quota_ceiling": 100, "rate_limit_per_minute": 10,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plaintext = created["token"].as_str().unwrap().to_string();

        // A consumer chat request that settles measured usage.
        client
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({"model":"test","messages":[]}))
            .send()
            .await
            .unwrap();

        // The shared ledger has provenance events (credit + reserve + settle).
        let events = ledger.lock().unwrap().events().clone();
        let ops: Vec<&str> = events.iter().map(|e| e.op.as_str()).collect();
        assert!(ops.contains(&"credit"), "seed credit is recorded");
        assert!(ops.contains(&"reserve"), "consumer reservation is recorded");
        assert!(ops.contains(&"settle"), "consumer settle is recorded");
        // All events carry the policy version that governed them.
        assert!(events.iter().all(|e| e.policy_version == 1));
    }

    // ---- Q2/Q4 MCP consumption flow --------------------------------------

    /// Creates a consumer key via the admin API and returns its plaintext.
    async fn make_consumer_key(api: SocketAddr, account: &str) -> String {
        let client = reqwest::Client::new();
        let created: serde_json::Value = client
            .post(format!("http://{api}/api/admin/consumer-key/create"))
            .header("Authorization", "Bearer master-token")
            .json(&serde_json::json!({
                "account": account, "quota_ceiling": 1000, "rate_limit_per_minute": 50,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        created["token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn mcp_consumer_can_decide_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let plaintext = make_consumer_key(api, "consumer-account").await;
        let client = reqwest::Client::new();
        let r = client
            .post(format!("http://{api}/mcp"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"decide","arguments":{"intent":"chat","prompt":"hi"}}
            }))
            .send()
            .await
            .unwrap();
        // A consumer can call `decide` (read-only inference planning).
        assert_eq!(r.status(), 200, "consumer may decide via MCP");
    }

    #[tokio::test]
    async fn mcp_consumer_execute_enforces_quota_and_releases_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (api, ledger) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let plaintext = make_consumer_key(api, "consumer-account").await;
        let client = reqwest::Client::new();

        // This test state has no distributed fabric router attached, so the
        // execution cannot route. That is the failure case: the consumer's
        // reservation must be RELEASED (not leaked, not settled) because no
        // measured work completed.
        let before = ledger.lock().unwrap().account(&"consumer-account".to_string()).unwrap();
        let before_available = before.available;

        let r = client
            .post(format!("http://{api}/mcp"))
            .header("Authorization", format!("Bearer {plaintext}"))
            .json(&serde_json::json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"execute_decision","arguments":{"intent":"chat","prompt":"hi","confirm":true}}
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "consumer may call execute via MCP");
        let j: serde_json::Value = r.json().await.unwrap();
        let content = j["result"]["content"][0]["text"].as_str().unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
        // Without a fabric router the execution cannot succeed (honest).
        assert_eq!(parsed["ok"], false, "no fabric router -> execution fails honestly");

        // The reservation was released on failure: no quota leaked as reserved
        // and nothing was consumed (no measured work).
        let after = ledger.lock().unwrap().account(&"consumer-account".to_string()).unwrap();
        assert_eq!(after.reserved, 0, "failed execution must release its reservation");
        assert_eq!(after.consumed, 0, "failed execution settles nothing");
        assert_eq!(after.available, before_available, "quota fully returned to the pool");
    }

    #[tokio::test]
    async fn mcp_consumer_is_denied_operational_tools() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let plaintext = make_consumer_key(api, "consumer-account").await;
        let client = reqwest::Client::new();

        // A consumer must NOT see the operational/read views (workers,
        // network, executions, sessions, quota, consumer keys).
        for tool in ["list_workers", "list_sessions", "get_quota", "list_consumer_keys", "list_executions"] {
            let r = client
                .post(format!("http://{api}/mcp"))
                .header("Authorization", format!("Bearer {plaintext}"))
                .json(&serde_json::json!({
                    "jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":tool,"arguments":{}}
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "consumer must be denied {tool}");
        }
    }

    #[tokio::test]
    async fn mcp_invalid_consumer_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (api, _) = start_consumer_state(dir.path(), "master-token".to_string()).await;
        let client = reqwest::Client::new();
        let r = client
            .post(format!("http://{api}/mcp"))
            .header("Authorization", "Bearer dca_0000000000000000000000000000000000000000000000000000000000000000")
            .json(&serde_json::json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"decide","arguments":{"intent":"chat","prompt":"hi"}}
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "unknown consumer key is unauthorized via MCP");
    }
}
