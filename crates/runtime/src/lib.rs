//! Managed llama.cpp `llama-server` subprocess plus the admission gate
//! and serve lifecycle (M4a/M4b).
//!
//! The inference engine runs as an external process, not FFI bindings:
//! upgrades are simple binary swaps and a crash in inference never takes
//! the node down. Before any model loads, the admission gate checks the
//! config mode and the live hardware budgets from the system probe.

use anyhow::{Context, Result, bail};
use decentraai_config::{InferenceMode, NodeConfig, ResourceSection};
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tracing::info;

/// Environment variable that overrides the llama-server binary location.
pub const LLAMA_SERVER_ENV: &str = "DECENTRAAI_LLAMA_SERVER";

/// Candidate binary names searched on PATH.
const BINARY_NAMES: [&str; 2] = ["llama-server", "llama-server.exe"];

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
    /// How long to wait for the server to become ready (model load time).
    pub ready_timeout: Duration,
    /// Extra arguments passed through verbatim (e.g. `--n-gpu-layers 99`).
    pub extra_args: Vec<String>,
}

impl RuntimeConfig {
    pub fn new(model_path: PathBuf) -> Self {
        Self {
            model_path,
            bind_host: "127.0.0.1".to_string(),
            ctx_size: 4096,
            parallel: 4,
            ready_timeout: Duration::from_secs(120),
            extra_args: Vec::new(),
        }
    }
}

/// Builds the llama-server CLI arguments. Pure function for easy testing.
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
    bail!("llama-server not found on PATH; install llama.cpp or set {LLAMA_SERVER_ENV}")
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
pub fn ensure_admitted(config: &NodeConfig) -> Result<()> {
    check_inference_mode(config.inference.enabled)?;
    evaluate_admission(
        &SystemSnapshot::collect(),
        &probe_gpu(),
        &config.resources,
        config.storage.max_cache_gb,
        config.storage.min_free_disk_gb,
    )
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

impl LlamaServer {
    /// Spawns the child without waiting for readiness (exposed for tests).
    pub fn start(binary: &Path, config: &RuntimeConfig) -> Result<Self> {
        let port = allocate_port(&config.bind_host)?;
        let args = server_args(config, port);
        info!(binary = %binary.display(), port, "starting llama-server");
        let child = Command::new(binary)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning {}", binary.display()))?;
        Ok(Self {
            child,
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
}

impl ServeManager {
    pub fn new(server: LlamaServer, idle_timeout: Duration) -> Self {
        Self {
            server: Some(server),
            idle_timeout,
            last_activity: Instant::now(),
        }
    }

    /// Marks the model as actively serving; resets the idle clock.
    pub fn note_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_loaded(&self) -> bool {
        self.server.is_some()
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
            info!(idle_for_ms = self.idle_for().as_millis(), "idle timeout reached, unloading model");
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
                info!(port, elapsed_ms = start.elapsed().as_millis(), "llama-server is ready");
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
fn probe_health(host: &str, port: u16) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_config::GpuPolicy;

    #[test]
    fn args_carry_model_context_and_parallelism() {
        let mut config = RuntimeConfig::new(PathBuf::from("/models/test.gguf"));
        config.ctx_size = 8192;
        config.parallel = 2;
        config.extra_args = vec!["--n-gpu-layers".to_string(), "99".to_string()];
        let args = server_args(&config, 8080);
        let joined = args.join(" ");
        assert!(joined.contains("--model /models/test.gguf"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 8080"));
        assert!(joined.contains("--ctx-size 8192"));
        assert!(joined.contains("--parallel 2"));
        assert!(joined.ends_with("--n-gpu-layers 99"));
    }

    #[test]
    fn explicit_missing_binary_is_rejected() {
        let err = find_llama_server(Some(Path::new("/definitely/not/here"))).unwrap_err();
        assert!(err.to_string().contains("not found"));
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
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-llama-server");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
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
}
