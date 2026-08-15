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
use axum::extract::State;
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
}

impl Auth {
    fn who(&self) -> String {
        match self {
            Self::Open => "open".to_string(),
            Self::Master => "master".to_string(),
            Self::Subscriber { name, .. } => name.clone(),
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
    /// Subscription registry (db/tokens.json) when tiers are in use.
    token_store_path: Option<PathBuf>,
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
            info,
            token_store_path,
            tiers,
            queue,
            started_at: Instant::now(),
            requests_served: Arc::new(AtomicU64::new(0)),
            tokens_generated: Arc::new(AtomicU64::new(0)),
            requests_failed: Arc::new(AtomicU64::new(0)),
            recent_requests: Arc::new(StdMutex::new(VecDeque::new())),
            rate_windows: Arc::new(StdMutex::new(HashMap::new())),
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

    fn presented_token(headers: &HeaderMap) -> Option<&str> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    }

    /// Classifies the caller: master token, issued subscription token
    /// (resolved through the registry on every request), or open.
    fn classify(&self, headers: &HeaderMap) -> Result<Auth, GateError> {
        let presented = Self::presented_token(headers);
        match &self.auth_token {
            None => Ok(Auth::Open),
            Some(master) => {
                let presented = presented.ok_or(GateError::Unauthorized)?;
                if presented == master.as_ref() {
                    return Ok(Auth::Master);
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
            Err(_) => Err(GateError::Unauthorized),
        }
    }

    /// Per-tier model allowlist. The request body's `model` field is
    /// advisory (llama-server serves what it loaded), but we enforce it
    /// anyway: it is honest about what the tier may use, and it protects
    /// multi-model routing when that lands.
    fn check_model_access(&self, auth: &Auth, body: &[u8]) -> Result<(), GateError> {
        let Auth::Subscriber { tier, name, .. } = auth else {
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
        let Auth::Subscriber { name, .. } = auth else {
            return;
        };
        let mut usage = self.token_usage.lock().unwrap();
        let entry = usage.entry(name.clone()).or_default();
        entry.0 += 1;
        entry.1 += generated;
        entry.2 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
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
<div class="card"><h2>Audit events</h2><ul id="audit" style="list-style:none;padding-left:0"><li class="off">loading&hellip;</li></ul></div>
<p id="api-url"></p></body><script>
var f=document.getElementById('f'),status=document.getElementById('status'),tbl=document.querySelector('#tbl tbody'),tokenEl=document.getElementById('token'),newDiv=document.getElementById('new');
var esc=function(s){return String(s).replace(/[&<>"]/g,function(c){return{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]});};
f.addEventListener('submit',async e=>{e.preventDefault();var n=f.name.value,t=parseInt(f.t.value),role=f.role.value;status.textContent='Creating...';var r=await fetch('/api/admin/token/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n,tier:t,role:role})});var d=await r.json();if(r.ok){tokenEl.textContent=d.token;newDiv.style.display='block';status.innerHTML='<span style="color:green">Saved! Copy now.</span>';f.reset()}else status.innerHTML='<span style="color:red">'+d.error.message+'</span>'};
async function load(){var r=await fetch('/api/admin/token/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();tbl.innerHTML='';d.tokens.forEach(t=>{var row=document.createElement('tr');row.innerHTML='<td>'+esc(t.name)+'</td><td>'+t.tier+'</td><td>'+esc(t.role)+'</td><td><button data-n="'+t.name+'" onclick="revoke(event)">Revoke</button></td>';tbl.appendChild(row)});loadAudit();}
var auditEl=document.getElementById('audit');
async function loadAudit(){var r=await fetch('/api/admin/events',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();var evs=d.events||[];auditEl.innerHTML=evs.length?'':('<li class="off">no security events yet</li>');evs.forEach(function(e){var li=document.createElement('li');var d2=new Date((e.timestamp||0)*1000).toLocaleString();li.innerHTML='<code>'+esc(e.event||'')+'</code> <span class="off">'+d2+'</span> <span class="small">'+esc(JSON.stringify(e.details||Object()))+'</span>';auditEl.appendChild(li);});}
window.onload=load;
function revoke(e){var n=e.target.dataset.n;fetch('/api/admin/token/revoke',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n})}).then(_=>load());}
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
    let body = serde_json::json!({"tokens": tokens.iter().map(|t| serde_json::json!({"name": &t.name, "tier": t.tier, "role": t.role.name(), "created_at": t.created_at, "revoked": t.revoked})).collect::<Vec<_>>()});
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
    let plaintext = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.create_with_role(&name, decentraai_tokens::Tier(tier), None, role) {
                Ok(t) => {
                    let a = state.info.repo_root.join("logs/audit.jsonl");
                    let _ = decentraai_audit::record(
                        a.parent().unwrap_or(&state.info.repo_root),
                        "token_created",
                        serde_json::json!({"name": &name, "tier": tier, "role": role.name()}),
                    );
                    Some(t)
                }
                Err(_) => return forbidden("name taken"),
            }
        }
        None => return forbidden("no store"),
    };
    let body = serde_json::json!({"token": plaintext, "name": name, "tier": tier, "role": role.name()});
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
        .route("/v1/token", get(token_handler))
        .route("/v1/peers", get(peers_handler))
        .route("/v1/compute", get(compute_handler))
        .route("/v1/network", get(network_handler))
        .route("/v1/execution", get(execution_handler))
        .route("/v1/models", get(proxy_handler))
        .route("/v1/completions", post(proxy_handler))
        .route("/v1/chat/completions", post(proxy_handler))
        // P3 - Admin dashboard endpoints
        .route("/api/admin/token/list", get(admin_token_list_handler))
        .route("/api/admin/token/create", post(admin_token_create_handler))
        .route("/api/admin/token/revoke", post(admin_token_revoke_handler))
        // P3/M10 - Worker trust + audit events (master-gated control plane)
        .route("/api/admin/worker/trust", post(admin_worker_trust_handler))
        .route("/api/admin/worker/revoke", post(admin_worker_revoke_handler))
        .route("/api/admin/events", get(admin_audit_events_handler))
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
    Html(html).into_response()
}

/// Public status snapshot: no secrets, safe without the token. Includes
/// a fresh hardware probe so the operator sees RAM/GPU pressure live.
async fn status_handler(State(state): State<ApiState>) -> Response {
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
            "ram_total_gib": snapshot.total_memory_bytes as f64 / GIB,
            "ram_available_gib": snapshot.available_memory_bytes as f64 / GIB,
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
            "temperature": state.info.generation.temperature,
            "top_p": state.info.generation.top_p,
            "top_k": state.info.generation.top_k,
            "repeat_penalty": state.info.generation.repeat_penalty,
            "system_prompt": state.info.generation.system_prompt,
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

    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
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
        body = serde_json::json!({
            "attached": true,
            "workers": report.workers,
            "contributions": report.contributions,
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
        body["decisions"] = serde_json::json!(compute.decisions());
        body["attached"] = serde_json::json!(true);
    }
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
        let mut chunks = upstream.bytes_stream();
        while let Some(item) = chunks.next().await {
            match item {
                Ok(bytes) => {
                    drain_buffer.lock().unwrap().extend_from_slice(&bytes);
                    if tx.send(Ok(bytes)).await.is_err() {
                        return;
                    }
                }
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
    let is_inference = method == Method::POST
        && (uri.path() == "/v1/completions" || uri.path() == "/v1/chat/completions");
    // The body the proxy will actually forward (caller sampling defaults
    // folded in), computed once up front so the proxy-boundary caps see the
    // exact prompt/max_tokens and it can be reused for the request below.
    let outgoing = if is_inference {
        apply_generation_defaults(&state.info.generation, &body)
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
            } else if is_inference {
                state.requests_failed.fetch_add(1, Ordering::SeqCst);
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
        let list_resp = reqwest::Client::new()
            .get(format!("http://{api}/api/admin/token/list"))
            .header("Authorization", "Bearer master_token")
            .send()
            .await
            .unwrap();
        assert_eq!(list_resp.status(), 200);
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
            },
            announced_at_ms: 0,
            accepts_remote_inference: accepts_remote,
            node_id: node_id.to_string(),
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
}
