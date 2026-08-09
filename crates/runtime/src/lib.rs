//! Managed llama.cpp `llama-server` subprocess (M4a).
//!
//! The inference engine runs as an external process, not FFI bindings:
//! upgrades are simple binary swaps and a crash in inference never takes
//! the node down. This crate locates the binary, derives its arguments
//! from the node configuration, waits for the HTTP health endpoint, and
//! guarantees the child is killed when the manager goes away.

use anyhow::{Context, Result, bail};
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

/// Handle to a running llama-server child process.
/// Kills the child on drop as a backstop; prefer [`LlamaServer::stop`].
pub struct LlamaServer {
    child: Child,
    host: String,
    port: u16,
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

    /// Kills the child and waits for it to exit.
    pub async fn stop(mut self) -> Result<ExitStatus> {
        let status = self
            .child
            .kill()
            .await
            .context("failed to kill llama-server")?;
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
}
