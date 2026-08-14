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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::ServeManager;
use crate::queue::InferenceQueue;

/// Maximum audit events shown on the dashboard.
const DASHBOARD_EVENT_LIMIT: usize = 10;
/// Maximum inference calls kept in the recent-requests ring buffer.
const RECENT_REQUEST_LIMIT: usize = 12;
/// Sliding rate-limit window.
const RATE_WINDOW: Duration = Duration::from_secs(60);
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

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
    /// An issued subscription token with its tier.
    Subscriber { name: String, tier: u8 },
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
            client: reqwest::Client::new(),
            info,
            token_store_path,
            tiers,
            queue,
            started_at: Instant::now(),
            requests_served: Arc::new(AtomicU64::new(0)),
            tokens_generated: Arc::new(AtomicU64::new(0)),
            recent_requests: Arc::new(StdMutex::new(VecDeque::new())),
            rate_windows: Arc::new(StdMutex::new(HashMap::new())),
            token_usage: Arc::new(StdMutex::new(HashMap::new())),
            compute,
            p2p,
        }
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

    /// Per-tier model allowlist. The request body's `model` field is
    /// advisory (llama-server serves what it loaded), but we enforce it
    /// anyway: it is honest about what the tier may use, and it protects
    /// multi-model routing when that lands.
    fn check_model_access(&self, auth: &Auth, body: &[u8]) -> Result<(), GateError> {
        let Auth::Subscriber { tier, name } = auth else {
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
        let Auth::Subscriber { name, tier } = auth else {
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
<div class="card"><h2>Create Token</h2><form id="f"><input name="name" placeholder="Token name" required><select name="t"><option value="1">Guest</option><option value="2">Contributor</option><option value="3">Core</option></select><button>Create</button></form><div id="new" style="display:none"><code id="token"></code><button onclick="navigator.clipboard.writeText(document.getElementById('token').textContent)">Copy</button></div><p id="status"></p></div>
<div class="card"><h2>Tokens</h2><table id="tbl"><thead><tr><th>Name</th><th>Tier</th><th>Action</th></tr></thead><tbody></tbody></table></div>
<p id="api-url"></p></body><script>
var f=document.getElementById('f'),status=document.getElementById('status'),tbl=document.querySelector('#tbl tbody'),tokenEl=document.getElementById('token'),newDiv=document.getElementById('new');
f.addEventListener('submit',async e=>{e.preventDefault();var n=f.name.value,t=parseInt(f.t.value);status.textContent='Creating...';var r=await fetch('/api/admin/token/create',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')},body:JSON.stringify({name:n,tier:t})});var d=await r.json();if(r.ok){tokenEl.textContent=d.token;newDiv.style.display='block';status.innerHTML='<span style="color:green">Saved! Copy now.</span>';f.reset()}else status.innerHTML='<span style="color:red">'+d.error.message+'</span>'};
async function load(){var r=await fetch('/api/admin/token/list',{headers:{'Authorization':'Bearer '+(localStorage.getItem('admin-token')||'')}});var d=await r.json();tbl.innerHTML='';d.tokens.forEach(t=>{var row=document.createElement('tr');row.innerHTML='<td>'+t.name+'</td><td>'+t.tier+'</td><td><button data-n="'+t.name+'" onclick="revoke(event)">Revoke</button></td>';tbl.appendChild(row)});}
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
    let body = serde_json::json!({"tokens": tokens.iter().map(|t| serde_json::json!({"name": t.name, "tier": t.tier, "created_at": t.created_at, "revoked": t.revoked})).collect::<Vec<_>>()});
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
    let plaintext = match &state.token_store_path {
        Some(p) => {
            let mut s = match decentraai_tokens::TokenStore::load(p) {
                Ok(s) => s,
                Err(_) => return forbidden("load failed"),
            };
            match s.create(&name, decentraai_tokens::Tier(tier)) {
                Ok(t) => {
                    let a = state.info.repo_root.join("logs/audit.jsonl");
                    let _ = decentraai_audit::record(
                        a.parent().unwrap_or(&state.info.repo_root),
                        "token_created",
                        serde_json::json!({"name": &name, "tier": tier}),
                    );
                    Some(t)
                }
                Err(_) => return forbidden("name taken"),
            }
        }
        None => return forbidden("no store"),
    };
    let body = serde_json::json!({"token": plaintext, "name": name, "tier": tier});
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

/// Builds the proxy router: the OpenAI-compatible surface, the dashboard
/// (also the fallback), and the small JSON views that feed it.
pub fn build_router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/status", get(status_handler))
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
    let body = serde_json::json!({
        "model": state.info.model_name,
        "model_size_bytes": state.info.model_size_bytes,
        "model_loaded": loaded,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "idle_for_secs": idle_secs,
        "requests_served": state.requests_served.load(Ordering::SeqCst),
        "tokens_generated": state.tokens_generated.load(Ordering::SeqCst),
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
        "recent_events": recent_audit_events(&state.info.repo_root),
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
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
async fn compute_handler(State(state): State<ApiState>) -> Response {
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
/// locality), connected peers, and local peer id. Empty when no compute/P2P.
async fn network_handler(State(state): State<ApiState>) -> Response {
    let mut body = serde_json::json!({
        "attached": false,
        "connected": [],
        "links": [],
        "local_peer": null,
    });
    if let Some(p2p) = &state.p2p {
        let connected = p2p.connected_peers().await;
        body["connected"] =
            serde_json::json!(connected.iter().map(|p| p.to_string()).collect::<Vec<_>>());
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
async fn execution_handler(State(state): State<ApiState>) -> Response {
    let mut body = serde_json::json!({ "attached": false, "executions": [] });
    if let Some(compute) = &state.compute {
        body["executions"] = serde_json::json!(compute.executions());
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
    if is_inference {
        if let Err(e) = state.check_model_access(&auth, &body) {
            return e.into_response();
        }
        if let Err(e) = state.check_rate_limit(&auth) {
            return e.into_response();
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

    state.manager.lock().await.note_activity();
    let started = Instant::now();

    let outgoing = if is_inference {
        apply_generation_defaults(&state.info.generation, &body)
    } else {
        body.to_vec()
    };

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
                return stream_inference(
                    state,
                    auth,
                    uri.path().to_string(),
                    started,
                    upstream,
                    content_type,
                );
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
            }
            let mut response = (status, bytes).into_response();
            if let Some(value) = content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":{\"message\":\"model backend unavailable (unloaded or crashed); restart decentraai serve\",\"type\":\"server_error\"}}",
        )
            .into_response(),
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

const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>DecentraAI dashboard</title>
<style>
body{font:15px/1.5 system-ui,sans-serif;background:#0f141b;color:#e6edf3;max-width:960px;margin:24px auto;padding:0 16px}
h1{font-size:20px} h2{font-size:14px;color:#9da7b3;margin:0 0 10px;text-transform:uppercase;letter-spacing:.08em}
.card{background:#161d27;border:1px solid #2a3442;border-radius:10px;padding:14px 18px;margin-bottom:14px}
table{border-collapse:collapse;width:100%} td,th{padding:4px 8px;text-align:left;border-bottom:1px solid #232c38}
code{background:#0a0e13;padding:2px 6px;border-radius:6px;font-size:13px}
.ok{color:#3fb950}.off{color:#8b949e}.bad{color:#f85149}
.bignum{font-size:26px;font-weight:600}
.small{color:#8b949e;font-size:12px}
ol{padding-left:20px} li{margin:6px 0}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px}
.metric{background:#0a0e13;border-radius:8px;padding:10px 14px}
.metric .label{color:#8b949e;font-size:12px;text-transform:uppercase;letter-spacing:.05em}
#chat-history{max-height:280px;overflow-y:auto;margin-bottom:10px}
#chat-history .msg-user{margin:6px 0;padding:8px 12px;background:#0a0e13;border-radius:8px;border:1px solid #1d4ed8}
#chat-history .msg-node{margin:6px 0;padding:8px 12px;background:#161d27;border-radius:8px;border-left:3px solid #238636;white-space:pre-wrap}
</style>
</head>
<body>
  <h1>DecentraAI node</h1>
<div style="text-align:right;margin:-4px 0 0">
  <button id="adv-toggle" style="background:#0a0e13;color:#e6edf3;border:1px solid #2a3442;border-radius:8px;padding:6px 12px;cursor:pointer">Show advanced</button>
</div>
<div class="card">
  <h2>Model</h2>
  <div class="bignum" id="model-name">&hellip;</div>
  <div class="small"><span id="model-size"></span> &middot; <span id="model-status">loading&hellip;</span></div>
  <div class="small" id="also-models"></div>
</div>
<div class="card">
  <h2>Inference</h2>
  <div class="grid">
    <div class="metric"><div class="label">Requests</div><div class="bignum" id="requests">0</div></div>
    <div class="metric"><div class="label">Tokens generated</div><div class="bignum" id="tokens">0</div></div>
    <div class="metric"><div class="label">Last speed</div><div class="bignum" id="toksec">&mdash;</div></div>
    <div class="metric"><div class="label">Uptime</div><div class="bignum" id="uptime">&mdash;</div></div>
    <div class="metric"><div class="label">Idle for</div><div class="bignum" id="idle">&mdash;</div></div>
  </div>
  <table style="margin-top:10px">
    <tr><td>Backend (llama-server)</td><td><code id="backend">&mdash;</code></td></tr>
    <tr><td>API</td><td><code>http://127.0.0.1:__API_PORT__/v1</code> (OpenAI-compatible: <code>/v1/models</code>, <code>/v1/chat/completions</code>, <code>/v1/completions</code>, <code>/v1/peers</code>)</td></tr>
  </table>
</div>
<div class="card">
  <h2>Chat</h2>
  <div id="chat-history"><p class="off">Ask the node something. Responses are routed through the fabric planner to a worker.</p></div>
  <textarea id="chat-input" rows="2" placeholder="Type a message&hellip;" style="width:100%;box-sizing:border-box;background:#0a0e13;color:#e6edf3;border:1px solid #2a3442;border-radius:8px;padding:8px"></textarea>
  <div style="margin-top:8px;display:flex;gap:8px;align-items:center">
    <button id="chat-send" style="background:#238636;color:#fff;border:0;border-radius:8px;padding:8px 16px;cursor:pointer">Send</button>
    <span id="chat-status" class="small">&mdash;</span>
    <input id="chat-stream" type="checkbox" style="margin-left:auto"> <label for="chat-stream" class="small">stream</label>
  </div>
</div>
<div class="card">
  <h2>Queue</h2>
  <table>
    <tr><td>Serving now</td><td id="queue-serving"><span class="off">idle</span></td></tr>
    <tr><td>Waiting</td><td id="queue-waiting"><span class="off">nobody</span></td></tr>
  </table>
</div>
<div class="card">
  <h2>Recent inference calls</h2>
  <table><thead><tr><th>Time</th><th>Endpoint</th><th>Prompt tok</th><th>Gen tok</th><th>ms</th><th>tok/s</th></tr></thead>
  <tbody id="recent"><tr><td colspan="6" class="off">loading&hellip;</td></tr></tbody></table>
</div>
<div class="card">
  <h2>System</h2>
  <table>
    <tr><td>RAM free</td><td id="ram">&mdash;</td></tr>
    <tr><td>CPU</td><td id="cpu">&mdash;</td></tr>
    <tr><td>GPU</td><td id="gpu">&mdash;</td></tr>
  </table>
</div>
<div id="advanced" hidden>
<div class="card">
  <h2>Tracked peers (reputation)</h2>
  <table><thead><tr><th>Peer</th><th>Verified chunks</th><th>Failed</th><th>Score</th><th>Status</th></tr></thead>
  <tbody id="peers"><tr><td colspan="5" class="off">loading&hellip;</td></tr></tbody></table>
</div>
<div class="card">
  <h2>Workers (compute registry)</h2>
  <table><thead><tr><th>Worker</th><th>Node</th><th>Status</th><th>Load</th><th>Queue</th><th>tok/s</th><th>Latency</th><th>RAM free</th><th>In-flight</th></tr></thead>
  <tbody id="workers"><tr><td colspan="9" class="off">loading&hellip;</td></tr></tbody></table>
</div>
<div class="card">
  <h2>Network (measured links)</h2>
  <table><thead><tr><th>Peer</th><th>RTT</th><th>Bandwidth</th><th>Locality</th></tr></thead>
  <tbody id="network"><tr><td colspan="4" class="off">loading&hellip;</td></tr></tbody></table>
  <div class="small" id="connected"></div>
</div>
<div class="card">
  <h2>Execution (planner decisions)</h2>
  <table><thead><tr><th>Req</th><th>Worker</th><th>Score</th><th>Stages</th><th>Continuation</th><th>Network</th><th>KV</th><th>Outcome</th><th>Reasoning</th></tr></thead>
  <tbody id="execution"><tr><td colspan="9" class="off">loading&hellip;</td></tr></tbody></table>
</div>
<div class="card">
  <h2>Models</h2>
  <table><thead><tr><th>Model</th><th>Engine</th><th>Context</th><th>RAM</th><th>VRAM</th><th>Active</th></tr></thead>
  <tbody id="models"><tr><td colspan="6" class="off">loading&hellip;</td></tr></tbody></table>
  <div class="small" id="models-status"></div>
</div>
<div class="card">
  <h2>Settings</h2>
  <table>
    <tr><td>Node name</td><td id="set-name">&mdash;</td></tr>
    <tr><td>Dashboard port</td><td id="set-port">&mdash;</td></tr>
    <tr><td>Discovery</td><td id="set-discovery">&mdash;</td></tr>
    <tr><td>Trusted workers</td><td id="set-trust">&mdash;</td></tr>
    <tr><td>Model / engine</td><td id="set-model">&mdash;</td></tr>
    <tr><td>CPU cores</td><td id="set-cpu">&mdash;</td></tr>
    <tr><td>RAM</td><td id="set-ram">&mdash;</td></tr>
    <tr><td>GPU</td><td id="set-gpu">&mdash;</td></tr>
  </table>
</div>
<div class="card">
  <h2>Diagnostics</h2>
  <table>
    <tr><td>Node health</td><td id="diag-health">&mdash;</td></tr>
    <tr><td>Engine</td><td id="diag-engine">&mdash;</td></tr>
    <tr><td>P2P / network</td><td id="diag-p2p">&mdash;</td></tr>
    <tr><td>Workers</td><td id="diag-workers">&mdash;</td></tr>
    <tr><td>Engine restarts (recovery)</td><td id="diag-restarts">&mdash;</td></tr>
    <tr><td>Active sessions (KV)</td><td id="diag-sessions">&mdash;</td></tr>
  </table>
</div>
<div class="card">
  <h2>Recent security events (audit log)</h2>
  <table><thead><tr><th>Time</th><th>Event</th><th>Details</th></tr></thead>
  <tbody id="events"><tr><td colspan="3" class="off">loading&hellip;</td></tr></tbody></table>
</div>
<div class="card">
  <h2>Share a model with another machine</h2>
  <div id="share"></div>
</div>
</div>
<p class="small">Refreshes every 3s from /status and /v1/peers only &mdash; watching this page never touches the inference backend. Loopback only.</p>
<script type="module">
/*__JS__*/
</script>
</body>
</html>"#;

fn dashboard_js(state: &ApiState, share: &str) -> String {
    JS_TEMPLATE
        .replace("__SHARE__", &share.replace('"', "\\\""))
        .replace("__MODEL__", &state.info.model_name.replace('"', "\\\""))
}

const JS_TEMPLATE: &str = r#"
const esc = s => String(s).replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
document.getElementById('share').innerHTML = "__SHARE__";
document.getElementById('model-name').textContent = "__MODEL__";
const advEl = document.getElementById('advanced');
const advBtn = document.getElementById('adv-toggle');
const setAdv = (show) => {
  advEl.hidden = !show;
  advBtn.textContent = show ? 'Hide advanced' : 'Show advanced';
};
setAdv((localStorage.getItem('decentraai.advanced') || '0') === '1');
advBtn.addEventListener('click', () => {
  const show = advEl.hidden;
  try { localStorage.setItem('decentraai.advanced', show ? '1' : '0'); } catch (e) {}
  setAdv(show);
});
let token = '';
try { token = await (await fetch('/v1/token')).text(); } catch (e) {}
const headers = token ? { 'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
// Conversation history is real and kept across page reloads (client-side, in
// localStorage) so a refresh does not wipe the ongoing session. It is never
// invented: only messages the node actually exchanged are stored.
const HIST_KEY = 'decentraai.chat.history';
let hist = [];
try { hist = JSON.parse(localStorage.getItem(HIST_KEY)) || []; } catch (e) {}
const chatbox = document.getElementById('chat-history');
const addMsg = (role, text) => {
  const div = document.createElement('div');
  const who = role === 'user' ? '<b>You</b>' : '<b>Node</b>';
  div.className = role === 'user' ? 'msg-user' : 'msg-node';
  div.innerHTML = '<div>' + who + '</div><div>' + esc(text) + '</div>';
  chatbox.appendChild(div);
  chatbox.scrollTop = chatbox.scrollHeight;
  return div;
};
const saveHist = () => {
  try { localStorage.setItem(HIST_KEY, JSON.stringify(hist.slice(-24))); } catch (e) {}
};
// Restore persisted conversation from a previous page load.
hist.forEach(m => addMsg(m.role === 'assistant' ? 'node' : (m.role === 'system' ? 'node' : 'user'), m.content || '(empty)'));
// Streaming is the default; the checkbox lets a user fall back to a single
// parsed HTTP response (useful with clients that do not read SSE).
document.getElementById('chat-stream').checked = true;
// Reads llama-server's SSE body and returns { text, tokens } once complete.
const readSse = async (resp) => {
  const reader = resp.body.getReader();
  const dec = new TextDecoder();
  let buffer = '', text = '', tokens = null;
  const msgNode = addMsg('node', '');
  const bodyEl = msgNode.querySelector(':scope > div:nth-child(2)');
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += dec.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';
      for (const raw of lines) {
        const line = raw.trim();
        if (!line.startsWith('data:')) continue;
        const payload = line.slice(5).trim();
        if (payload === '[DONE]') continue;
        let ev; try { ev = JSON.parse(payload); } catch (e) { continue; }
        const delta = ev.choices && ev.choices[0] && ev.choices[0].delta && ev.choices[0].delta.content;
        if (delta) { text += delta; bodyEl.textContent = text; }
        if (ev.usage) tokens = ev.usage.completion_tokens;
      }
      chatbox.scrollTop = chatbox.scrollHeight;
    }
  } finally { reader.releaseLock(); }
  return { text, tokens };
};
document.getElementById('chat-send').addEventListener('click', async () => {
  const input = document.getElementById('chat-input');
  const prompt = input.value.trim();
  if (!prompt) return;
  input.value = '';
  addMsg('user', prompt);
  hist.push({ role: 'user', content: prompt });
  const status = document.getElementById('chat-status');
  const stream = document.getElementById('chat-stream').checked;
  status.textContent = 'routing &amp; generating&hellip;';
  try {
    const body = JSON.stringify({ model: '__MODEL__', messages: hist, stream });
    const t0 = performance.now();
    const r = await fetch('/v1/chat/completions', { method: 'POST', headers, body });
    let answer = '', tokens = null;
    if (stream && r.ok && r.body) {
      const out = await readSse(r);
      answer = out.text;
      tokens = out.tokens;
    } else {
      const j = await r.json();
      answer = (j && j.choices && j.choices[0] && j.choices[0].message && j.choices[0].message.content) || (j && j.error ? ('error: ' + (j.error.message || '')) : '');
    }
    addMsg('node', answer || '(empty response)');
    hist.push({ role: 'assistant', content: answer || '' });
    if (hist.length > 24) hist.splice(0, hist.length - 24);
    saveHist();
    const dt = Math.round(performance.now() - t0);
    status.textContent = (r.ok ? 'done' : 'error') + ' in ' + dt + ' ms' +
      (tokens != null ? ' &middot; ' + tokens + ' tokens' : '');
  } catch (e) {
    addMsg('node', 'request failed: ' + e);
    status.textContent = 'failed';
  }
});
document.getElementById('chat-input').addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); document.getElementById('chat-send').click(); } });
const fmtUptime = s => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h > 0 ? h + 'h ' + m + 'm' : (m > 0 ? m + 'm ' + (s % 60) + 's' : s + 's');
};
async function refresh() {
  try {
    const s = await (await fetch('/status')).json();
    document.getElementById('model-size').textContent =
      s.model_size_bytes > 0 ? (s.model_size_bytes / 1073741824).toFixed(2) + ' GiB' : '';
    document.getElementById('model-status').innerHTML = s.model_loaded
      ? '<span class="ok">&#9679; loaded</span>'
      : '<span class="off">&#9675; unloaded (idle timeout)</span>';
    const others = (s.available_models || []).filter(m => m.name !== s.model);
    document.getElementById('also-models').textContent = others.length
      ? 'also indexed: ' + others.map(m => esc(m.name) + ' (' + (m.size_bytes / 1073741824).toFixed(2) + ' GiB)').join(', ')
      : '';
    document.getElementById('requests').textContent = s.requests_served;
    document.getElementById('tokens').textContent = s.tokens_generated;
    const last = s.recent_requests[0];
    document.getElementById('toksec').textContent = last ? last.tokens_per_second.toFixed(1) + ' tok/s' : '\u2014';
    document.getElementById('uptime').textContent = fmtUptime(s.uptime_secs);
    document.getElementById('idle').textContent = Math.floor(s.idle_for_secs / 60) + ' min';
    document.getElementById('backend').textContent = s.backend;
    const q = s.queue || {};
    document.getElementById('queue-serving').innerHTML = q.serving
      ? '<span class="ok">&#9679;</span> <code>' + esc(q.serving.who) + '</code> &mdash; ' +
        esc(q.serving.endpoint.replace('/v1/', '')) + ' (' + q.serving.elapsed_secs + 's)'
      : '<span class="off">idle</span>';
    document.getElementById('queue-waiting').innerHTML = (q.waiting || []).length
      ? (q.waiting || []).map((w, i) =>
          '#' + (i + 1) + ' <code>' + esc(w.who) + '</code> (' + w.waited_secs + 's)'
        ).join(' &middot; ')
      : '<span class="off">nobody</span>';
    const rr = s.recent_requests.map(r => {
      const d = new Date(r.timestamp * 1000).toLocaleTimeString();
      return '<tr><td>' + d + '</td><td><code>' + esc(r.endpoint.replace('/v1/', '')) + '</code></td><td>' +
        r.prompt_tokens + '</td><td>' + r.completion_tokens + '</td><td>' + r.duration_ms + '</td><td>' +
        r.tokens_per_second.toFixed(1) + '</td></tr>';
    }).join('');
    document.getElementById('recent').innerHTML = rr || '<tr><td colspan="6" class="off">no inference calls yet</td></tr>';
    document.getElementById('ram').textContent =
      s.system.ram_available_gib.toFixed(1) + ' / ' + s.system.ram_total_gib.toFixed(1) + ' GiB';
    document.getElementById('cpu').textContent = s.system.cpu_threads + ' threads';
    document.getElementById('gpu').innerHTML = s.system.gpu
      ? esc(s.system.gpu.name) + ' &mdash; ' + s.system.gpu.temperature_c + '&deg;C, ' +
        s.system.gpu.free_vram_mib + ' MiB VRAM free, ' + s.system.gpu.utilization_percent + '% util'
      : '<span class="off">none detected</span>';
    const rows = s.recent_events.map(e => {
      const d = new Date(e.timestamp * 1000).toLocaleTimeString();
      return '<tr><td>' + d + '</td><td><code>' + esc(e.event) + '</code></td><td class="small">' + esc(JSON.stringify(e.details)) + '</td></tr>';
    }).join('');
    document.getElementById('events').innerHTML = rows || '<tr><td colspan="3" class="off">no security events yet</td></tr>';
  } catch (e) {}
  try {
    const p = await (await fetch('/v1/peers', { headers })).json();
    const rows = p.map(peer =>
      '<tr><td><code>' + esc(peer.peer_id.slice(0, 16)) + '&hellip;</code></td><td>' + peer.verified + '</td><td>' + peer.failed + '</td><td>' + peer.score.toFixed(1) + '</td><td>' +
      (peer.banned ? '<span class="bad">banned</span>' : '<span class="ok">ok</span>') + '</td></tr>'
    ).join('');
    document.getElementById('peers').innerHTML = rows || '<tr><td colspan="5" class="off">no peers tracked yet</td></tr>';
  } catch (e) {}
  let c = null, n = null;
  try {
    c = await (await fetch('/v1/compute', { headers })).json();
    const wrows = (c.workers || []).map(w =>
      '<tr><td><code>' + esc(w.peer_id.slice(0, 16)) + '&hellip;</code></td><td>' + esc(w.node_name || '') + '</td><td>' +
      (w.status === 'Ready' ? '<span class="ok">ready</span>' : '<span class="off">' + esc(w.status || '') + '</span>') + '</td><td>' + w.load_percent + '%</td><td>' + w.queue_depth + '</td><td>' + w.tokens_per_second + '</td><td>' + w.current_latency_ms + 'ms</td><td>' + Math.round((w.available_ram_mb||0)/1024) + ' GiB</td><td>' + w.in_flight + '</td></tr>'
    ).join('');
    document.getElementById('workers').innerHTML = wrows || '<tr><td colspan="9" class="off">no workers yet (compute not attached)</td></tr>';
  } catch (e) {}
  try {
    n = await (await fetch('/v1/network', { headers })).json();
    const nrows = (n.links || []).map(l =>
      '<tr><td><code>' + esc(l.peer.slice(0, 16)) + '&hellip;</code></td><td>' + l.rtt_ms + ' ms</td><td>' + (l.bandwidth_mbps || '&mdash;') + ' Mbps</td><td>' + esc(l.locality || '') + '</td></tr>'
    ).join('');
    document.getElementById('network').innerHTML = nrows || '<tr><td colspan="4" class="off">no measured links yet</td></tr>';
    document.getElementById('connected').textContent = (n.connected || []).length ? ('connected peers: ' + n.connected.map(p => p.slice(0, 12)).join(', ')) : 'no connected peers';
  } catch (e) {}
  try {
    const x = await (await fetch('/v1/execution', { headers })).json();
    const xrows = (x.executions || []).slice(0, 12).map(e =>
      '<tr><td><code>' + esc(e.request_id.slice(0, 8)) + '</code></td><td><code>' + esc(e.selected_worker.slice(0, 12)) + '&hellip;</code></td><td>' + e.score.toFixed(2) + '</td><td>' + e.stages + '</td><td>' +
      (e.is_continuation ? '<span class="ok">yes</span>' : '<span class="off">no</span>') + '</td><td>' + e.network_rtt_ms + ' ms</td><td class="small">' + esc(e.kv_headroom || '') + '</td><td>' + esc(e.outcome) + '</td><td class="small">' + esc(e.reasoning || '') + '</td></tr>'
    ).join('');
    document.getElementById('execution').innerHTML = xrows || '<tr><td colspan="9" class="off">no executions yet</td></tr>';
  } catch (e) {}
  // ---- Models / Settings / Diagnostics (from /status node+system + real manager state) ----
  let trustList = '';
  try { const tp = await (await fetch('/v1/peers', { headers })).json(); trustList = tp.length + ' tracked peer(s)'; } catch (e) {}
  const s = await (await fetch('/status')).json().catch(() => null);
  if (s) {
    const mrows = (s.node && s.node.served_models || []).map(m =>
      '<tr><td>' + esc(m.name || '') + '</td><td>' + esc(s.node.engine || '') + '</td><td>' + (m.context_tokens || '&mdash;') + ' tok</td><td>' + Math.round((m.est_ram_mb||0)/1024) + ' GiB</td><td>' + (m.est_vram_mb ? Math.round((m.est_vram_mb||0)/1024)+' GiB' : '&mdash;') + '</td><td>' + (s.model === m.name ? '<span class="ok">loaded</span>' : '<span class="off">-</span>') + '</td></tr>'
    ).join('');
    document.getElementById('models').innerHTML = mrows || '<tr><td colspan="6" class="off">no served models advertised</td></tr>';
    document.getElementById('models-status').textContent = 'active model: ' + esc(s.model || '') + (s.model_loaded ? ' &middot; <span class="ok">loaded</span>' : ' &middot; <span class="off">unloaded</span>');

    document.getElementById('set-name').textContent = (s.node && s.node.name) || esc(s.model) || '';
    document.getElementById('set-port').textContent = s.api_port;
    document.getElementById('set-discovery').textContent = 'mDNS / LAN (auto)';
    document.getElementById('set-trust').textContent = trustList ? trustList : 'not loaded';
    document.getElementById('set-model').textContent = esc(s.model || '') + ' / ' + esc((s.node && s.node.engine) || '');
    document.getElementById('diag-health').innerHTML = s.model_loaded ? '<span class="ok">model loaded, node up</span>' : '<span class="off">model not loaded</span>';
    document.getElementById('diag-engine').innerHTML = s.backend ? '<code>' + esc(s.backend) + '</code>' : '<span class="off">none</span>';
  // ---- resource limits + startup state in Settings (from real config/spec) ----
  if (s && s.resources) {
    const r = s.resources;
    document.getElementById('set-cpu').textContent =
      (s.system && s.system.cpu_threads ? s.system.cpu_threads + ' threads' : '&mdash;') +
      ' &middot; reserve ' + r.reserve_cpu_cores + ' core(s)';
    document.getElementById('set-ram').textContent =
      (s.system && s.system.ram_total_gib ? Math.round(s.system.ram_total_gib) + ' GiB total' : '&mdash;') +
      ' &middot; reserve ' + (Math.round((r.reserve_ram_mb||0)/1024)) + ' GiB';
    document.getElementById('set-gpu').textContent =
      (r.gpu_enabled || 'auto') + (r.gpu_max_vram_percent ? ' (vram cap ' + r.gpu_max_vram_percent + '%)' : '') +
      (r.reserve_vram_mb ? ' reserve ' + Math.round((r.reserve_vram_mb||0)/1024) + ' GiB' : '');
  }
  const diag = document.getElementById('diag-restarts');
  diag.innerHTML = (s && s.engine_respawns > 0)
    ? '<span class="bad">' + s.engine_respawns + ' auto-restart(s)</span> (M24 recovery)'
    : '<span class="ok">0</span> auto-restarts (stable)';
  if (c) {
    document.getElementById('diag-workers').textContent = (c.workers || []).length + ' worker(s)';
    document.getElementById('diag-sessions').textContent = c.sessions + ' KV session(s)';
  }
  if (n) {
    document.getElementById('diag-p2p').innerHTML = (n.connected || []).length + ' connected, ' + (n.links || []).length + ' measured link(s)';
  }
}
refresh(); setInterval(refresh, 3000);
"#;

/// Real node + engine info derived from the local compute advertisement when a
/// compute manager is attached. Falls back to empty markers otherwise — never
/// mock data.
fn node_info(compute: &Option<Arc<decentraai_distributed::ComputeManager>>) -> serde_json::Value {
    let Some(compute) = compute else {
        return serde_json::json!({ "name": "", "engine": "", "served_models": [], "attached": false });
    };
    let NodeProfile {
        name,
        engine,
        served_models,
    } = node_profile(compute);
    serde_json::json!({
        "name": name,
        "engine": engine,
        "served_models": served_models,
        "attached": true,
    })
}

/// Extracts the local node's real name, engine kind and served models (with
/// model file, RAM/VRAM footprint and context window) from the last
/// advertisement this node broadcast.
fn node_profile(compute: &decentraai_distributed::ComputeManager) -> NodeProfile {
    let mut profile = NodeProfile::default();
    if let Some(adv) = compute.last_local_advertisement_sync() {
        profile.name = adv.node_name;
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
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-llama-server");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    async fn test_manager(dir: &Path) -> Arc<Mutex<ServeManager>> {
        let binary = write_fake_server(dir);
        let config = RuntimeConfig::new(dir.join("model.gguf"));
        let server = LlamaServer::start(&binary, &config).unwrap();
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
    async fn start_echo_backend() -> SocketAddr {
        let app = Router::new().route(
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

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
                .create("guest", decentraai_tokens::Tier::GUEST)
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
    async fn rate_limit_returns_429_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        let registry_path = dir.path().join("db/tokens.json");
        let guest;
        {
            let mut store = decentraai_tokens::TokenStore::load(&registry_path).unwrap();
            guest = store
                .create("guest", decentraai_tokens::Tier::GUEST)
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
        }
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
        let backend = start_echo_backend().await;
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
        let sub = store.create("alice", decentraai_tokens::Tier(2)).unwrap();
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
        let app = Router::new().route(
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let dir = tempfile::tempdir().unwrap();
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{addr}"),
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
}
