//! OpenAI-compatible API endpoint (M4c) plus the web dashboard (M7b/M7c):
//! a thin proxy in front of the managed llama-server. It adds local
//! Bearer auth, tracks request activity for the idle-unload lifecycle,
//! and stays deliberately dumb: all inference logic lives in llama.cpp.
//!
//! The dashboard (GET /) renders live node status from /status and
//! /v1/peers only — it never calls the proxied inference endpoints, so
//! watching the page neither inflates the request counter nor resets
//! the idle-unload clock.

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use rand_core::RngCore;
use serde::Serialize;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::ServeManager;

/// Maximum audit events shown on the dashboard.
const DASHBOARD_EVENT_LIMIT: usize = 10;
/// Maximum inference calls kept in the recent-requests ring buffer.
const RECENT_REQUEST_LIMIT: usize = 12;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

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
}

/// Shared proxy state.
#[derive(Clone)]
pub struct ApiState {
    /// Base URL of the managed llama-server (e.g. http://127.0.0.1:41501).
    backend_url: String,
    /// Optional Bearer token; checked on every request when set.
    auth_token: Option<Arc<str>>,
    /// Lifecycle handle; activity is recorded per request.
    manager: Arc<Mutex<ServeManager>>,
    client: reqwest::Client,
    info: DashboardInfo,
    started_at: Instant,
    /// Completed inference calls (POST /v1/completions, /v1/chat/completions).
    requests_served: Arc<AtomicU64>,
    /// Sum of completion tokens across all inference calls.
    tokens_generated: Arc<AtomicU64>,
    /// Newest-first ring buffer of recent inference calls.
    recent_requests: Arc<StdMutex<VecDeque<RequestStat>>>,
}

impl ApiState {
    pub fn new(
        backend_url: String,
        auth_token: Option<String>,
        manager: Arc<Mutex<ServeManager>>,
        info: DashboardInfo,
    ) -> Self {
        Self {
            backend_url,
            auth_token: auth_token.map(Into::into),
            manager,
            client: reqwest::Client::new(),
            info,
            started_at: Instant::now(),
            requests_served: Arc::new(AtomicU64::new(0)),
            tokens_generated: Arc::new(AtomicU64::new(0)),
            recent_requests: Arc::new(StdMutex::new(VecDeque::new())),
        }
    }

    /// Token check shared by the proxy and the guarded JSON views.
    fn is_authorized(&self, headers: &HeaderMap) -> bool {
        match &self.auth_token {
            None => true,
            Some(token) => {
                let expected = format!("Bearer {token}");
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value == expected)
            }
        }
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
        self.tokens_generated.fetch_add(completion, Ordering::SeqCst);
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
}

/// Builds the proxy router: the OpenAI-compatible surface, the dashboard
/// (also the fallback), and the small JSON views that feed it.
pub fn build_router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/status", get(status_handler))
        .route("/v1/token", get(token_handler))
        .route("/v1/peers", get(peers_handler))
        .route("/v1/models", get(proxy_handler))
        .route("/v1/completions", post(proxy_handler))
        .route("/v1/chat/completions", post(proxy_handler))
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
    let (loaded, idle_secs) = {
        let manager = state.manager.lock().await;
        (manager.is_loaded(), manager.idle_for().as_secs())
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
    let body = serde_json::json!({
        "model": state.info.model_name,
        "model_size_bytes": state.info.model_size_bytes,
        "model_loaded": loaded,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "idle_for_secs": idle_secs,
        "requests_served": state.requests_served.load(Ordering::SeqCst),
        "tokens_generated": state.tokens_generated.load(Ordering::SeqCst),
        "recent_requests": recent,
        "system": {
            "cpu_threads": snapshot.logical_cpus,
            "ram_total_gib": snapshot.total_memory_bytes as f64 / GIB,
            "ram_available_gib": snapshot.available_memory_bytes as f64 / GIB,
            "gpu": gpu,
        },
        "backend": state.backend_url,
        "api_port": state.info.api_port,
        "recent_events": recent_audit_events(&state.info.repo_root),
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
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
    if !state.is_authorized(&headers) {
        return unauthorized();
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

async fn proxy_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.is_authorized(&headers) {
        return unauthorized();
    }

    state.manager.lock().await.note_activity();

    // Only real inference calls count as "requests served"; metadata
    // GETs like /v1/models are free.
    let is_inference = method == Method::POST
        && (uri.path() == "/v1/completions" || uri.path() == "/v1/chat/completions");
    let started = Instant::now();

    let url = format!("{}{}", state.backend_url, uri.path());
    let mut request = state.client.request(method, &url);
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    match request.body(body.to_vec()).send().await {
        Ok(upstream) => {
            let status = StatusCode::from_u16(upstream.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = upstream.bytes().await.unwrap_or_default();
            if is_inference && status.is_success() {
                state.record_inference(uri.path(), started.elapsed(), &bytes);
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
</style>
</head>
<body>
<h1>DecentraAI node</h1>
<div class="card">
  <h2>Model</h2>
  <div class="bignum" id="model-name">&hellip;</div>
  <div class="small"><span id="model-size"></span> &middot; <span id="model-status">loading&hellip;</span></div>
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
<div class="card">
  <h2>Tracked peers (reputation)</h2>
  <table><thead><tr><th>Peer</th><th>Verified chunks</th><th>Failed</th><th>Score</th><th>Status</th></tr></thead>
  <tbody id="peers"><tr><td colspan="5" class="off">loading&hellip;</td></tr></tbody></table>
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
let token = '';
try { token = await (await fetch('/v1/token')).text(); } catch (e) {}
const headers = token ? { 'Authorization': 'Bearer ' + token } : {};
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
    document.getElementById('requests').textContent = s.requests_served;
    document.getElementById('tokens').textContent = s.tokens_generated;
    const last = s.recent_requests[0];
    document.getElementById('toksec').textContent = last ? last.tokens_per_second.toFixed(1) + ' tok/s' : '\u2014';
    document.getElementById('uptime').textContent = fmtUptime(s.uptime_secs);
    document.getElementById('idle').textContent = Math.floor(s.idle_for_secs / 60) + ' min';
    document.getElementById('backend').textContent = s.backend;
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
}
refresh(); setInterval(refresh, 3000);
"#;

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
        Arc::new(Mutex::new(ServeManager::new(server, Duration::from_secs(3600))))
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
        }
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

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_forwards_models_to_backend() {
        let dir = tempfile::tempdir().unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();

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
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            Some("secret".to_string()),
            manager.clone(),
            test_info(dir.path(), None),
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
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
    async fn inference_calls_are_counted_with_token_stats() {
        let dir = tempfile::tempdir().unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        // Metadata GETs must not count as inference.
        client.get(format!("http://{api}/v1/models")).send().await.unwrap();

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
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dashboard_is_served_at_root_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let backend = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            None,
            manager.clone(),
            test_info(dir.path(), None),
        );
        let api = serve_api(state, "127.0.0.1", 0).await.unwrap();
        let client = reqwest::Client::new();

        for path in ["/", "/v1", "/anything-else"] {
            let response = client
                .get(format!("http://{api}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "path {path} must serve the dashboard");
            let body = response.text().await.unwrap();
            assert!(body.contains("DecentraAI dashboard"));
            assert!(body.contains("Tokens generated"));
            assert!(body.contains("Recent inference calls"));
            assert!(body.contains("Share a model"));
        }
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
}
