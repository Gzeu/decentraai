//! Managed llama.cpp `llama-server` subprocess plus the admission gate
//! and serve lifecycle (M4a/M4b), the OpenAI-compatible API proxy
//! (M4c, in [`api`]), and the fair FIFO inference queue (Q2, in
//! [`queue`]).
//!
//! The inference engine runs as an external process, not FFI bindings:
//! upgrades are simple binary swaps and a crash in inference never takes
//! the node down. Before any model loads, the admission gate checks the
//! config mode and the live hardware budgets from the system probe, and
//! rejections are written to the audit log (M6).

pub mod api;
pub mod arena;
pub mod authz;
pub mod dashboard;
pub mod dashboard_v2;
pub mod economic_agent;
pub mod fabric_dashboard;
pub mod fabric_flow;
pub mod fabric_landing;
pub mod governor_execution;
pub mod hub;
pub mod intel_assist;
pub mod job;
pub mod m18;
pub mod mcp;
pub mod providers_api;
pub mod queue;
pub mod settlement_tx;
pub mod tools;
pub mod vesper;
pub mod wallet_auth;
pub mod world;
pub mod world_economics;

use anyhow::{Context, Result, bail};
use decentraai_config::{InferenceMode, NodeConfig, ResourceSection};
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tracing::{info, warn};

/// Environment variable that overrides the llama-server binary location.
pub const LLAMA_SERVER_ENV: &str = "DECENTRAAI_LLAMA_SERVER";

/// Candidate binary names searched on PATH.
const BINARY_NAMES: [&str; 2] = ["llama-server", "llama-server.exe"];

/// Extra non-PATH locations probed for llama-server, relative to $HOME.
/// Builds produced by `git clone https://github.com/ggerganov/llama.cpp &&
/// cmake -B build && cmake --build build` land here; distro packages put
/// the binary in /usr/lib/ollama. We probe these so `distributed --model`
/// works out of the box instead of silently falling back to a mock.
const HOME_BINARY_PATHS: [&str; 2] = [
    "llama.cpp/build/bin/llama-server",
    "llama.cpp/build/bin/Release/llama-server",
];
const ABSOLUTE_BINARY_PATHS: [&str; 1] = ["/usr/lib/ollama/llama-server"];

/// How long a single health probe may take before it is abandoned.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Delay between health probes while the model loads.
const PROBE_INTERVAL: Duration = Duration::from_millis(200);

/// Parameters for one llama-server instance, derived from the `inference`
/// section of the node configuration (or test defaults).
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Filesystem path of the GGUF model to load.
    pub model_path: PathBuf,
    /// Host the server binds to; always loopback in the current threat model.
    pub bind_host: String,
    /// Context window, from `inference.max_context_tokens`.
    pub ctx_size: u32,
    /// Concurrent slots, from `inference.max_concurrent_requests`.
    pub parallel: u16,
    /// CPU threads for token generation. `None` leaves the llama.cpp
    /// default; the CLI sets it to the physical-core budget (logical
    /// CPUs minus the configured reserve), because oversubscribed
    /// threads are the most common cause of slow CPU inference.
    pub threads: Option<usize>,
    /// How long to wait for the server to become ready (model load time).
    pub ready_timeout: Duration,
    /// Extra arguments passed through verbatim (e.g. `--n-gpu-layers 99`).
    pub extra_args: Vec<String>,
    /// A fixed port to bind, or `None` to auto-allocate an ephemeral one.
    /// Productized nodes set this so the dashboard can target the model
    /// backend deterministically before it is ready.
    pub port: Option<u16>,
}

impl RuntimeConfig {
    pub fn new(model_path: PathBuf) -> Self {
        Self {
            model_path,
            bind_host: "127.0.0.1".to_string(),
            ctx_size: 4096,
            parallel: 4,
            threads: None,
            ready_timeout: Duration::from_secs(120),
            extra_args: Vec::new(),
            port: None,
        }
    }
}

/// Builds the llama-server CLI arguments. Pure function for easy testing.
/// `--jinja` enables the model's own chat template (required for
/// instruct models to answer properly); `--flash-attn on` is the fast
/// path for prompt processing and smaller KV cache traffic.
pub fn server_args(config: &RuntimeConfig, port: u16) -> Vec<String> {
    let mut args = vec![
        "--model".to_string(),
        config.model_path.to_string_lossy().into_owned(),
        "--host".to_string(),
        config.bind_host.clone(),
        "--port".to_string(),
        port.to_string(),
        "--ctx-size".to_string(),
        config.ctx_size.to_string(),
        "--parallel".to_string(),
        config.parallel.to_string(),
    ];
    if let Some(threads) = config.threads {
        args.push("--threads".to_string());
        args.push(threads.to_string());
    }
    args.push("--flash-attn".to_string());
    args.push("on".to_string());
    args.push("--jinja".to_string());
    args.extend(config.extra_args.iter().cloned());
    args
}

/// Locates the llama-server binary: explicit path, then the override
/// environment variable, then a PATH search.
pub fn find_llama_server(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        bail!("llama-server binary not found at {}", path.display());
    }
    if let Some(env_path) = std::env::var_os(LLAMA_SERVER_ENV) {
        let path = PathBuf::from(env_path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "{LLAMA_SERVER_ENV} points to {}, which is not a file",
            path.display()
        );
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in BINARY_NAMES {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    if let Some(candidate) = probe_common_locations(std::env::var_os("HOME")) {
        return Ok(candidate);
    }
    bail!("llama-server not found on PATH; install llama.cpp or set {LLAMA_SERVER_ENV}")
}

/// Probes non-PATH install locations for llama-server. Pure so tests can
/// drive it with a synthetic $HOME.
fn probe_common_locations(home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if let Some(home) = home {
        let home = PathBuf::from(home);
        for rel in HOME_BINARY_PATHS {
            let candidate = home.join(rel);
            if candidate.is_file() {
                info!(path = %candidate.display(), "found llama-server in common build location");
                return Some(candidate);
            }
        }
    }
    for path in ABSOLUTE_BINARY_PATHS {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            info!(path = %candidate.display(), "found llama-server in common install location");
            return Some(candidate);
        }
    }
    None
}

/// Allocates an ephemeral port by binding and releasing port 0.
pub fn allocate_port(host: &str) -> Result<u16> {
    let listener = TcpListener::bind((host, 0)).context("allocating ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

/// Resolves a model reference to a filesystem path: the registry is
/// consulted first (by relative path), then a direct path fallback.
pub fn resolve_model(registry: &ModelRegistry, reference: &str) -> Result<PathBuf> {
    if let Some(record) = registry.get_model(reference) {
        return Ok(PathBuf::from(&record.canonical_path));
    }
    let path = PathBuf::from(reference);
    if path.is_file() {
        return Ok(path);
    }
    bail!("model '{reference}' not found in the registry or on disk")
}

/// Hard gate for `inference.enabled`: `never` disables the runtime entirely.
pub fn check_inference_mode(mode: InferenceMode) -> Result<()> {
    if mode == InferenceMode::Never {
        bail!("inference is disabled by configuration (inference.enabled = never)");
    }
    Ok(())
}

/// Evaluates the admission policy against a hardware snapshot. Split from
/// [`ensure_admitted`] so tests can drive it with synthetic snapshots.
pub fn evaluate_admission(
    snapshot: &SystemSnapshot,
    gpu: &GpuProbeStatus,
    resources: &ResourceSection,
    max_cache_gb: u32,
    min_free_disk_gb: u32,
) -> Result<()> {
    let budget = snapshot.derive_budget(resources, max_cache_gb, min_free_disk_gb);
    match snapshot.admit_inference(&budget, gpu, resources.stop_gpu_temperature_celsius) {
        AdmissionDecision::Admit => Ok(()),
        AdmissionDecision::Reject(reason) => bail!("inference admission rejected: {reason}"),
    }
}

/// Full pre-flight check before loading a model: config mode first, then a
/// fresh hardware probe evaluated against the configured budgets.
/// Rejections are written to the audit log (M6).
pub fn ensure_admitted(config: &NodeConfig) -> Result<()> {
    check_inference_mode(config.inference.enabled)?;
    let result = evaluate_admission(
        &SystemSnapshot::collect(),
        &probe_gpu(),
        &config.resources,
        config.storage.max_cache_gb,
        config.storage.min_free_disk_gb,
    );
    if let Err(e) = &result {
        decentraai_audit::record_best_effort(
            &PathBuf::from(expand_home(&config.node.data_dir)).join("logs"),
            "inference_admission_rejected",
            serde_json::json!({"reason": e.to_string()}),
        );
    }
    result
}

/// Expands a leading `~` in config paths (the CLI has the same helper).
fn expand_home(value: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if value == "~" {
        return home;
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    value.to_string()
}

/// Handle to a running llama-server child process.
/// Kills the child on drop as a backstop; prefer [`LlamaServer::stop`].
pub struct LlamaServer {
    child: Child,
    host: String,
    port: u16,
}

impl std::fmt::Debug for LlamaServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaServer")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// Serializes llama-server subprocess spawns process-wide. Under parallel test
/// spawn an `execve` can transiently see a freshly-written script as "busy"
/// (ETXTBSY, os error 26); holding this lock while spawning makes each spawn
/// happen one at a time, which fully eliminates that race. Production impact is
/// nil (the node spawns at most one engine at boot, and the lock is held only
/// for the brief synchronous `Command::spawn`).
static ENGINE_SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl LlamaServer {
    /// Spawns the child without waiting for readiness (exposed for tests).
    pub fn start(binary: &Path, config: &RuntimeConfig) -> Result<Self> {
        let port = match config.port {
            Some(p) => p,
            None => allocate_port(&config.bind_host)?,
        };
        let args = server_args(config, port);
        info!(binary = %binary.display(), port, "starting llama-server");
        // Hold the process-wide spawn lock + retry briefly on ETXTBSY
        // (os error 26): under parallel test spawn a concurrent exec can
        // transiently see the executable/script as busy. Serializing the brief
        // spawn (plus a small backoff) eliminates the flake deterministically.
        let _guard = ENGINE_SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut child = None;
        for attempt in 0..5 {
            let spawn = Command::new(binary)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match spawn {
                Ok(c) => {
                    child = Some(c);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(26) && attempt < 4 => {
                    std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
                }
                Err(e) => return Err(e).context(format!("spawning {}", binary.display())),
            }
        }
        Ok(Self {
            child: child.expect("engine spawn retries exhausted"),
            host: config.bind_host.clone(),
            port,
        })
    }

    /// Spawns the server and blocks until the health endpoint answers.
    /// Kills the child if it never becomes ready.
    pub async fn spawn(binary: &Path, config: &RuntimeConfig) -> Result<Self> {
        let server = Self::start(binary, config)?;
        if let Err(e) = wait_until_ready(&server.host, server.port, config.ready_timeout).await {
            let _ = server.stop().await;
            return Err(e);
        }
        Ok(server)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Kills the child and waits for it to exit, returning its status.
    pub async fn stop(mut self) -> Result<ExitStatus> {
        self.child
            .start_kill()
            .context("failed to kill llama-server")?;
        let status = self
            .child
            .wait()
            .await
            .context("failed to reap llama-server")?;
        info!(port = self.port, "llama-server stopped");
        Ok(status)
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        // Backstop only: the process may already be dead after stop().
        let _ = self.child.start_kill();
    }
}

/// Tracks a running server and unloads it after an idle period.
/// The OpenAI-compatible proxy (M4c) calls [`ServeManager::note_activity`]
/// on every request; until then the CLI keeps the model in the foreground.
pub struct ServeManager {
    server: Option<LlamaServer>,
    idle_timeout: Duration,
    last_activity: Instant,
    /// Restart spec used by the M24 engine supervisor to respawn a crashed
    /// llama-server. `None` disables auto-restart.
    binary: Option<PathBuf>,
    restart_config: Option<RuntimeConfig>,
    /// Number of automatic restarts performed (observability).
    pub respawns: u32,
}

impl ServeManager {
    pub fn new(server: LlamaServer, idle_timeout: Duration) -> Self {
        Self {
            server: Some(server),
            idle_timeout,
            last_activity: Instant::now(),
            binary: None,
            restart_config: None,
            respawns: 0,
        }
    }

    /// A manager with no local engine (Q3 remote backend). Used when the
    /// model runs on a `serve start --backend http://host:port` and this
    /// node only keeps auth/tiers/queue/dashboard local. `base_url()` returns
    /// `None`, so the proxy falls back to `state.backend_url`; `is_loaded()`
    /// is `false` and the idle watcher exits immediately (no local model to
    /// unload).
    pub fn unloaded(idle_timeout: Duration) -> Self {
        Self {
            server: None,
            idle_timeout,
            last_activity: Instant::now(),
            binary: None,
            restart_config: None,
            respawns: 0,
        }
    }

    /// Supplies the binary + config needed to respawn the engine (M24 engine
    /// supervisor). Auto-restart stays disabled until this is set.
    pub fn set_restart_spec(&mut self, binary: PathBuf, config: RuntimeConfig) {
        self.binary = Some(binary);
        self.restart_config = Some(config);
    }

    /// Swaps the model in the restart spec (model selector). Returns `false`
    /// when there is no restart spec (e.g. remote-backend node) — the caller
    /// then knows a live respawn is not possible and only persistence applies.
    pub fn set_restart_model(&mut self, model_path: PathBuf) -> bool {
        match &mut self.restart_config {
            Some(cfg) => {
                cfg.model_path = model_path;
                true
            }
            None => false,
        }
    }

    /// Marks the model as actively serving; resets the idle clock.
    pub fn note_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_loaded(&self) -> bool {
        self.server.is_some()
    }

    /// Returns the path of the model currently managed by this instance,
    /// if one was configured (i.e. when M24 engine supervisor restarts are enabled).
    pub fn current_model_path(&self) -> Option<&Path> {
        self.restart_config.as_ref().map(|c| c.model_path.as_ref())
    }

    pub fn base_url(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.base_url())
    }

    pub fn idle_for(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Stops the server when it has been idle longer than the configured
    /// timeout. Returns true when an unload actually happened.
    pub async fn unload_if_idle(&mut self) -> Result<bool> {
        if self.idle_for() < self.idle_timeout {
            return Ok(false);
        }
        if let Some(server) = self.server.take() {
            info!(
                idle_for_ms = self.idle_for().as_millis(),
                "idle timeout reached, unloading model"
            );
            server.stop().await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Stops the server if one is still running. Borrows the manager so
    /// the handle stays usable for status checks afterwards.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(server) = self.server.take() {
            server.stop().await?;
        }
        Ok(())
    }

    /// M24 engine supervisor: probes the running engine's health endpoint and,
    /// if the engine has crashed or gone unreachable, respawns it from the
    /// stored restart spec. Returns `true` when the engine is healthy after the
    /// call (possibly after a restart), `false` when it could not be made
    /// healthy (e.g. no restart spec, or the binary failed to start).
    pub async fn ensure_healthy(&mut self) -> Result<bool> {
        // If we have a live handle, confirm it answers health.
        if let Some(server) = &self.server {
            let probe_host = server.host.clone();
            let probe_port = server.port();
            let ok = tokio::task::spawn_blocking(move || probe_health(&probe_host, probe_port))
                .await
                .unwrap_or(Err(anyhow::anyhow!("health probe task failed")));
            if ok.is_ok() {
                return Ok(true);
            }
            warn!(
                probe_port,
                "engine health probe failed; restarting llama-server (M24)"
            );
            // Engine is dead. Drop it (stop kills the child) and respawn.
            let dead = self.server.take().unwrap();
            let _ = dead.stop().await;
        } else if self.restart_config.is_none() {
            // No engine and no restart spec: nothing to supervise.
            return Ok(false);
        }

        let (Some(binary), Some(config)) = (&self.binary, &self.restart_config) else {
            return Ok(false);
        };
        self.respawns += 1; // count the restart attempt
        match LlamaServer::spawn(binary, config).await {
            Ok(server) => {
                info!(
                    port = server.port(),
                    respawn = self.respawns,
                    "restarted llama-server (M24 auto-recovery)"
                );
                self.server = Some(server);
                Ok(true)
            }
            Err(e) => {
                warn!(error = %e, "failed to restart llama-server");
                Ok(false)
            }
        }
    }
}

/// Polls the health endpoint until it answers HTTP 200 or `timeout` elapses.
pub async fn wait_until_ready(host: &str, port: u16, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let probe_host = host.to_string();
        let probe = tokio::task::spawn_blocking(move || probe_health(&probe_host, port))
            .await
            .context("health probe task failed")?;
        match probe {
            Ok(()) => {
                info!(
                    port,
                    elapsed_ms = start.elapsed().as_millis(),
                    "llama-server is ready"
                );
                return Ok(());
            }
            Err(e) => {
                if start.elapsed() >= timeout {
                    return Err(e.context(format!(
                        "llama-server on port {port} did not become ready within {timeout:?}"
                    )));
                }
                tokio::time::sleep(PROBE_INTERVAL).await;
            }
        }
    }
}

/// Single blocking health probe: `GET /health`, expecting an HTTP 200 status.
pub(crate) fn probe_health(host: &str, port: u16) -> Result<()> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .context("could not resolve health probe address")?;
    let mut stream = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT)?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut buf = [0u8; 512];
    let n = stream.read(&mut buf)?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let status = head.lines().next().unwrap_or_default();
    if status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        bail!("unexpected health response: {status}")
    }
}

/// Embedded TTS server script (Kokoro-82M ONNX, stdlib HTTP). Written into
/// `<data_dir>/tts/tts_server.py` by [`TtsServer::start`] so the binary stays
/// self-contained and the operator can inspect what runs on the loopback.
const TTS_SERVER_PY: &str = include_str!("tts_server.py");

/// The running Kokoro TTS subprocess (external engine — never FFI). Listens
/// on loopback only; the node proxies `/v1/tts` with Bearer auth.
pub struct TtsServer {
    child: tokio::process::Child,
    host: String,
    port: u16,
}

impl TtsServer {
    /// Writes the embedded script and spawns the Python venv interpreter.
    /// Fails fast when the venv or model files are missing so the caller can
    /// disable TTS gracefully (the node must not fail startup for voice).
    ///
    /// `voice` is a Piper voice id (`ro_RO-raluca-high`, `ro_RO-lili-high`,
    /// `ro_RO-mihai-medium`, …) whose `.onnx` + `.onnx.json` live in
    /// `<data_dir>/tts/models/piper-ro/`.
    pub fn start(data_dir: &Path, voice: &str, _speed: f64) -> Result<Self> {
        let tts_dir = data_dir.join("tts");
        let venv_python = tts_dir.join("venv").join("bin").join("python");
        let model = tts_dir
            .join("models")
            .join("piper-ro")
            .join(format!("{voice}.onnx"));
        let config = tts_dir
            .join("models")
            .join("piper-ro")
            .join(format!("{voice}.onnx.json"));
        let script = tts_dir.join("tts_server.py");
        for (what, path) in [
            ("python venv", &venv_python),
            ("model", &model),
            ("voice config", &config),
        ] {
            if !path.exists() {
                bail!(
                    "TTS {what} missing at {}: run `scripts/setup-tts.sh` or disable `tts.enabled`",
                    path.display()
                );
            }
        }
        fs::write(&script, TTS_SERVER_PY)
            .with_context(|| format!("writing TTS server script to {}", script.display()))?;
        let port = allocate_port("127.0.0.1")?;
        let site_packages = tts_dir
            .join("venv")
            .join("lib")
            .join("python3.13")
            .join("site-packages");
        // The venv interpreter resolves its own site-packages; pass PYTHONPATH
        // as a fallback for unusual layouts.
        let mut cmd = Command::new(&venv_python);
        cmd.args([
            script.to_string_lossy().as_ref(),
            "--model",
            model.to_string_lossy().as_ref(),
            "--config",
            config.to_string_lossy().as_ref(),
            "--port",
            &port.to_string(),
            "--voice",
            voice,
        ])
        .env("PYTHONPATH", site_packages.to_string_lossy().as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let child = cmd.spawn().context("spawning TTS server")?;
        Ok(Self {
            child,
            host: "127.0.0.1".to_string(),
            port,
        })
    }

    /// Spawns and waits for `/health` to answer 200 (Kokoro load + warmup
    /// can take several seconds on CPU). Kills the child on timeout.
    pub async fn spawn(data_dir: &Path, voice: &str, speed: f64) -> Result<Self> {
        let server = Self::start(data_dir, voice, speed)?;
        let port = server.port;
        if let Err(e) = wait_until_ready(&server.host, port, Duration::from_secs(60)).await {
            let _ = server.stop().await;
            return Err(e.context("TTS server did not become ready"));
        }
        Ok(server)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Kills the child and reaps it.
    pub async fn stop(mut self) -> Result<ExitStatus> {
        self.child
            .start_kill()
            .context("failed to kill TTS server")?;
        let status = self
            .child
            .wait()
            .await
            .context("failed to reap TTS server")?;
        info!(port = self.port, "TTS server stopped");
        Ok(status)
    }
}

impl Drop for TtsServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Holds the TTS subprocess + the configured voice/speed the proxy applies.
/// `None` server = TTS disabled; the dashboard hides the speak button.
pub struct TtsManager {
    server: Option<TtsServer>,
    pub voice: String,
    pub speed: f64,
}

impl TtsManager {
    pub fn new(server: Option<TtsServer>, voice: String, speed: f64) -> Self {
        Self {
            server,
            voice,
            speed,
        }
    }

    /// TTS disabled — used when the config omits `tts` or startup should not
    /// fail on missing model files (dashboard hides the speak button).
    pub fn disabled() -> Self {
        Self {
            server: None,
            voice: "ro_RO-raluca-high".to_string(),
            speed: 1.0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.server.is_some()
    }

    pub fn base_url(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.base_url())
    }

    /// Health probe for the dashboard /status endpoint.
    pub fn healthy(&self) -> bool {
        self.server
            .as_ref()
            .map(|_| probe_health("127.0.0.1", self.server.as_ref().unwrap().port()).is_ok())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_config::GpuPolicy;

    #[test]
    fn args_carry_model_context_and_parallelism() {
        let mut config = RuntimeConfig::new(PathBuf::from("/models/test.gguf"));
        config.ctx_size = 8192;
        config.parallel = 2;
        config.threads = Some(3);
        config.extra_args = vec!["--n-gpu-layers".to_string(), "99".to_string()];
        let args = server_args(&config, 8080);
        let joined = args.join(" ");
        assert!(joined.contains("--model /models/test.gguf"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 8080"));
        assert!(joined.contains("--ctx-size 8192"));
        assert!(joined.contains("--parallel 2"));
        assert!(joined.contains("--threads 3"));
        assert!(joined.contains("--flash-attn on"));
        assert!(joined.contains("--jinja"));
        assert!(joined.ends_with("--n-gpu-layers 99"));
    }

    #[test]
    fn runtime_config_port_defaults_to_auto_allocate() {
        let config = RuntimeConfig::new(PathBuf::from("/m.gguf"));
        assert_eq!(
            config.port, None,
            "default: auto-allocate an ephemeral port"
        );
        let mut fixed = RuntimeConfig::new(PathBuf::from("/m.gguf"));
        fixed.port = Some(8081);
        assert_eq!(fixed.port, Some(8081));
        let args = server_args(&fixed, 8081);
        assert!(args.join(" ").contains("--port 8081"));
    }

    #[test]
    fn threads_are_omitted_when_unset() {
        let config = RuntimeConfig::new(PathBuf::from("/models/test.gguf"));
        let joined = server_args(&config, 8080).join(" ");
        assert!(!joined.contains("--threads"));
        assert!(joined.contains("--flash-attn on"));
    }

    #[test]
    fn explicit_missing_binary_is_rejected() {
        let err = find_llama_server(Some(Path::new("/definitely/not/here"))).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[cfg(unix)]
    #[test]
    fn probe_common_locations_finds_home_build() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("llama.cpp/build/bin/llama-server");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
        let found = probe_common_locations(Some(dir.path().as_os_str().to_os_string())).unwrap();
        assert_eq!(found, target);
    }

    #[test]
    fn probe_common_locations_ignores_empty_home_dir() {
        // An empty $HOME must never produce a hit by itself. A Some value may
        // only come from a system-wide install (e.g. /usr/lib/ollama on hosts
        // with that package), so the found path must not live under the temp
        // dir used as $HOME.
        let dir = tempfile::tempdir().unwrap();
        let result = probe_common_locations(Some(dir.path().as_os_str().to_os_string()));
        if let Some(found) = result {
            assert!(
                !found.starts_with(dir.path()),
                "home probe must not match inside the empty temp dir"
            );
        }
    }

    #[test]
    fn allocate_port_returns_usable_port() {
        let port = allocate_port("127.0.0.1").unwrap();
        assert!(port > 0);
    }

    #[test]
    fn resolve_model_prefers_registry_records() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.gguf"), b"GGUF test").unwrap();
        let mut registry = ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        registry.scan_directory(dir.path()).unwrap();
        let resolved = resolve_model(&registry, "model.gguf").unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.is_file());
    }

    #[test]
    fn resolve_model_falls_back_to_direct_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("direct.gguf");
        std::fs::write(&file, b"GGUF test").unwrap();
        let registry = ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        let resolved = resolve_model(&registry, file.to_str().unwrap()).unwrap();
        assert_eq!(resolved, file);
    }

    #[test]
    fn resolve_model_rejects_unknown_models() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::new(dir.path().to_path_buf()).unwrap();
        assert!(resolve_model(&registry, "missing.gguf").is_err());
    }

    #[test]
    fn mode_never_disables_inference() {
        let err = check_inference_mode(InferenceMode::Never).unwrap_err();
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn mode_auto_and_always_pass_the_gate() {
        check_inference_mode(InferenceMode::Auto).unwrap();
        check_inference_mode(InferenceMode::Always).unwrap();
    }

    fn test_resources(policy: GpuPolicy) -> ResourceSection {
        ResourceSection {
            cpu_max_percent: 80,
            reserve_cpu_cores: 2,
            memory_max_percent: 80,
            reserve_ram_mb: 1024,
            gpu_enabled: policy,
            gpu_max_vram_percent: 75,
            reserve_vram_mb: 1024,
            stop_gpu_temperature_celsius: 83,
            max_upload_mbps: 20,
            max_download_mbps: 80,
        }
    }

    fn test_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            logical_cpus: 8,
            cpu_usage_percent: 10.0,
            total_memory_bytes: 32 * 1024 * 1024 * 1024,
            available_memory_bytes: 16 * 1024 * 1024 * 1024,
            used_swap_bytes: 0,
            total_disk_free_bytes: 500 * 1024 * 1024 * 1024,
            battery_percent: None,
        }
    }

    #[test]
    fn admission_rejects_missing_required_gpu() {
        let err = evaluate_admission(
            &test_snapshot(),
            &GpuProbeStatus::Unavailable("none".into()),
            &test_resources(GpuPolicy::Required),
            100,
            20,
        )
        .unwrap_err();
        assert!(err.to_string().contains("admission rejected"));
    }

    #[test]
    fn admission_accepts_healthy_system() {
        evaluate_admission(
            &test_snapshot(),
            &GpuProbeStatus::Unavailable("none".into()),
            &test_resources(GpuPolicy::Auto),
            100,
            20,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_accepts_http_200() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
            }
        });
        wait_until_ready("127.0.0.1", port, Duration::from_secs(5))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_times_out_on_error_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        });
        let err = wait_until_ready("127.0.0.1", port, Duration::from_millis(600))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not become ready"));
    }

    #[tokio::test]
    async fn wait_until_ready_times_out_when_nothing_listens() {
        let port = allocate_port("127.0.0.1").unwrap();
        let err = wait_until_ready("127.0.0.1", port, Duration::from_millis(400))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not become ready"));
    }

    #[cfg(unix)]
    fn write_fake_server(dir: &Path) -> PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-llama-server");
        // Write + sync + close BEFORE spawning, so a concurrent exec in another
        // test never sees a freshly-open-for-write script (the ETXTBSY flake
        // under parallel tests). Retry a few times on ETXTBSY just in case.
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
    #[tokio::test]
    async fn stop_kills_the_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let binary = write_fake_server(dir.path());
        let config = RuntimeConfig::new(dir.path().join("model.gguf"));
        let server = LlamaServer::start(&binary, &config).unwrap();
        let status = server.stop().await.unwrap();
        assert!(!status.success(), "killed process must not exit cleanly");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_kills_child_when_never_ready() {
        let dir = tempfile::tempdir().unwrap();
        let binary = write_fake_server(dir.path());
        let mut config = RuntimeConfig::new(dir.path().join("model.gguf"));
        config.ready_timeout = Duration::from_millis(500);
        let err = LlamaServer::spawn(&binary, &config).await.unwrap_err();
        assert!(err.to_string().contains("did not become ready"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn serve_manager_unloads_after_idle_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let binary = write_fake_server(dir.path());
        let config = RuntimeConfig::new(dir.path().join("model.gguf"));
        let server = LlamaServer::start(&binary, &config).unwrap();
        let mut manager = ServeManager::new(server, Duration::from_millis(100));
        assert!(manager.is_loaded());
        assert!(manager.base_url().is_some());
        assert!(!manager.unload_if_idle().await.unwrap());
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(manager.unload_if_idle().await.unwrap());
        assert!(!manager.is_loaded());
        assert!(manager.base_url().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn serve_manager_activity_resets_idle_clock() {
        let dir = tempfile::tempdir().unwrap();
        let binary = write_fake_server(dir.path());
        let config = RuntimeConfig::new(dir.path().join("model.gguf"));
        let server = LlamaServer::start(&binary, &config).unwrap();
        let mut manager = ServeManager::new(server, Duration::from_millis(300));
        tokio::time::sleep(Duration::from_millis(150)).await;
        manager.note_activity();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!manager.unload_if_idle().await.unwrap());
        assert!(manager.is_loaded());
        manager.shutdown().await.unwrap();
        assert!(!manager.is_loaded());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_healthy_attempts_restart_for_dead_engine() {
        let dir = tempfile::tempdir().unwrap();
        let binary = write_fake_server(dir.path());
        let mut config = RuntimeConfig::new(dir.path().join("model.gguf"));
        config.ready_timeout = Duration::from_millis(300);

        // A started-but-never-serving fake engine: it holds a port number but
        // does not bind it, so the health probe fails as if the engine crashed.
        let server = LlamaServer::start(&binary, &config).unwrap();
        let port = server.port();

        let mut manager = ServeManager::new(server, Duration::from_secs(60));
        // No restart spec -> supervisor cannot recover a dead engine.
        assert!(!manager.ensure_healthy().await.unwrap());

        // Simulate a crash: drop the live handle. Without a restart spec this
        // reports unhealthy.
        manager.server = None;
        assert!(!manager.ensure_healthy().await.unwrap());

        // With a restart spec, recovery is attempted. The fake never serves
        // HTTP 200, so spawn's ready-wait fails and ensure_healthy reports the
        // engine is still not healthy (and does not panic). This exercises the
        // full probe->stop->respawn path against a real child.
        manager.set_restart_spec(binary, config);
        manager.server = None;
        let healthy = manager.ensure_healthy().await.unwrap();
        assert!(!healthy, "fake server never becomes ready");
        assert_eq!(manager.respawns, 1, "one restart was attempted");
        let _ = port;
    }
}
