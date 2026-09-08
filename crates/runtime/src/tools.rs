//! Tool Runtime — external capability subprocesses (OCR, STT, …).
//!
//! Mirrors the proven TTS pattern exactly: a Python script is embedded in the
//! binary (`include_str!`), written into `<data_dir>/tools/<name>/server.py`,
//! spawned as a **subprocess** on loopback with an ephemeral port, health-
//! probed until ready, and proxied through an authenticated `/v1/<name>`
//! endpoint. The engine is always an external process — never FFI.
//!
//! Each tool is a separate opt-in subprocess so the node keeps working when a
//! tool's venv/model files are missing (fails graceful, dashboard shows
//! disabled). Secrets/prompts are never logged; only security-relevant
//! events land in audit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};
use tracing::info;

/// Shared HTTP-server lifecycle for a tool subprocess.
///
/// The server itself is always a stdlib `http.server` Python script so the
/// tool venv needs only the ML dependency (onnxruntime, faster-whisper, …).
pub struct ToolServer {
    child: Child,
    host: String,
    port: u16,
}

impl ToolServer {
    /// Writes the embedded script and spawns the tool's venv interpreter.
    /// Fails fast when the venv or model files are missing so the caller can
    /// disable the tool gracefully (the node must not fail startup).
    ///
    /// `dir` is `<data_dir>/tools/<tool>/`; the script is written as
    /// `server.py` inside it. `args` are passed to the script after
    /// `--port <n>` (the script must bind loopback only).
    pub fn start(
        dir: &Path,
        venv_python: &Path,
        script: &str,
        args: &[String],
        setup_hint: &str,
    ) -> Result<Self> {
        if !venv_python.exists() {
            bail!(
                "tool python venv missing at {}: run {setup_hint} or disable the tool",
                venv_python.display()
            );
        }
        let script_path = dir.join("server.py");
        fs::write(&script_path, script)
            .with_context(|| format!("writing tool server script to {}", script_path.display()))?;
        let port = super::allocate_port("127.0.0.1")?;
        let site_packages = venv_python
            .parent()
            .and_then(|bin| bin.parent())
            .map(|lib| lib.join("python3.13").join("site-packages"))
            .filter(|p| p.exists())
            .or_else(|| {
                // Broader fallback: scan for the interpreter's site-packages.
                find_site_packages(venv_python)
            });
        let mut cmd = Command::new(venv_python);
        let mut full_args = vec![script_path.to_string_lossy().to_string()];
        full_args.push("--port".to_string());
        full_args.push(port.to_string());
        full_args.extend(args.iter().cloned());
        cmd.args(&full_args);
        if let Some(sp) = &site_packages {
            cmd.env("PYTHONPATH", sp.to_string_lossy().as_ref());
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().context("spawning tool server")?;
        Ok(Self {
            child,
            host: "127.0.0.1".to_string(),
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Kills the child and reaps it.
    pub async fn stop(mut self) -> Result<std::process::ExitStatus> {
        self.child
            .start_kill()
            .context("failed to kill tool server")?;
        let status = self
            .child
            .wait()
            .await
            .context("failed to reap tool server")?;
        info!(port = self.port, "tool server stopped");
        Ok(status)
    }
}

impl Drop for ToolServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Finds `<venv>/lib/python3.X/site-packages` by scanning one level down.
fn find_site_packages(venv_python: &Path) -> Option<PathBuf> {
    let bin = venv_python.parent()?;
    let lib = bin.parent()?.join("lib");
    let entries = fs::read_dir(&lib).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("python3") {
            let sp = lib.join(&name).join("site-packages");
            if sp.exists() {
                return Some(sp);
            }
        }
    }
    None
}

/// Health probe loop shared by all tools (like TTS).
pub async fn wait_until_ready(host: &str, port: u16, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let client = reqwest::Client::builder()
            .build()
            .context("building probe client")?;
        match client
            .get(format!("http://{host}:{port}/health"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {
                if tokio::time::Instant::now() >= deadline {
                    bail!("tool server did not become ready on {host}:{port}");
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OCR (RapidOCR — PP-OCRv4 on onnxruntime, CPU-friendly)
// ---------------------------------------------------------------------------

const OCR_SERVER_PY: &str = include_str!("ocr_server.py");

/// The running OCR subprocess + its tool directory. `None` = disabled.
pub struct OcrServer {
    server: ToolServer,
}

impl OcrServer {
    /// Spawns and waits for `/health` (model load can take seconds on CPU).
    /// Kills the child on timeout.
    pub async fn spawn(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("tools").join("ocr");
        let venv_python = dir.join("venv").join("bin").join("python");
        let args = vec!["--lang".to_string(), "en".to_string()];
        let server = ToolServer::start(
            &dir,
            &venv_python,
            OCR_SERVER_PY,
            &args,
            "scripts/setup-ocr.sh",
        )?;
        let port = server.port();
        if let Err(e) = wait_until_ready("127.0.0.1", port, Duration::from_secs(120)).await {
            let _ = server.stop().await;
            return Err(e.context("OCR server did not become ready"));
        }
        Ok(Self { server })
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }
}

/// Holds the OCR subprocess. `None` server = OCR disabled.
pub struct OcrManager {
    server: Option<OcrServer>,
}

impl OcrManager {
    pub fn new(server: Option<OcrServer>) -> Self {
        Self { server }
    }

    pub fn disabled() -> Self {
        Self { server: None }
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
            .map(|_| {
                super::probe_health("127.0.0.1", self.server.as_ref().unwrap().server.port())
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// STT (faster-whisper — CTranslate2, CPU)
// ---------------------------------------------------------------------------

const STT_SERVER_PY: &str = include_str!("stt_server.py");

/// The running STT subprocess + its tool directory. `None` = disabled.
pub struct SttServer {
    server: ToolServer,
}

impl SttServer {
    /// Spawns and waits for `/health` (model load can take seconds on CPU).
    /// Kills the child on timeout.
    pub async fn spawn(data_dir: &Path, model: &str) -> Result<Self> {
        let dir = data_dir.join("tools").join("stt");
        let venv_python = dir.join("venv").join("bin").join("python");
        let args = vec!["--model".to_string(), model.to_string()];
        let server = ToolServer::start(
            &dir,
            &venv_python,
            STT_SERVER_PY,
            &args,
            "scripts/setup-stt.sh",
        )?;
        let port = server.port();
        if let Err(e) = wait_until_ready("127.0.0.1", port, Duration::from_secs(120)).await {
            let _ = server.stop().await;
            return Err(e.context("STT server did not become ready"));
        }
        Ok(Self { server })
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }
}

/// Holds the STT subprocess. `None` server = STT disabled.
pub struct SttManager {
    server: Option<SttServer>,
    pub model: String,
}

impl SttManager {
    pub fn new(server: Option<SttServer>, model: String) -> Self {
        Self { server, model }
    }

    pub fn disabled() -> Self {
        Self {
            server: None,
            model: "base".to_string(),
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
            .map(|_| {
                super::probe_health("127.0.0.1", self.server.as_ref().unwrap().server.port())
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// HF Skills (small transformers pipelines — sentiment, NER, summarize, translate)
// ---------------------------------------------------------------------------

const HF_SKILL_SERVER_PY: &str = include_str!("hf_skill_server.py");
const TRANSFORMERS_SERVER_PY: &str = include_str!("transformers_server.py");

/// The running HF-skills subprocess. One server hosts all enabled skills
/// (pipelines load lazily on first call). `None` = disabled.
pub struct HfSkillsServer {
    server: ToolServer,
    skills: Vec<String>,
}

impl HfSkillsServer {
    /// Spawns and waits for `/health`. Loads no pipeline until a skill is
    /// called, so startup stays fast even with several skills enabled.
    pub async fn spawn(data_dir: &Path, skills: &[String]) -> Result<Self> {
        let dir = data_dir.join("tools").join("skills");
        let venv_python = dir.join("venv").join("bin").join("python");
        let args = vec!["--skills".to_string(), skills.join(",")];
        let server = ToolServer::start(
            &dir,
            &venv_python,
            HF_SKILL_SERVER_PY,
            &args,
            "scripts/setup-skills.sh",
        )?;
        let port = server.port();
        if let Err(e) = wait_until_ready("127.0.0.1", port, Duration::from_secs(120)).await {
            let _ = server.stop().await;
            return Err(e.context("HF skills server did not become ready"));
        }
        Ok(Self {
            server,
            skills: skills.to_vec(),
        })
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }

    pub fn skills(&self) -> &[String] {
        &self.skills
    }
}

/// Holds the HF-skills subprocess. `None` server = skills disabled.
pub struct HfSkillsManager {
    server: Option<HfSkillsServer>,
}

impl HfSkillsManager {
    pub fn new(server: Option<HfSkillsServer>) -> Self {
        Self { server }
    }

    pub fn disabled() -> Self {
        Self { server: None }
    }

    pub fn enabled(&self) -> bool {
        self.server.is_some()
    }

    pub fn base_url(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.base_url())
    }

    /// Skills this node actually runs (empty when disabled).
    pub fn skills(&self) -> Vec<String> {
        self.server
            .as_ref()
            .map(|s| s.skills().to_vec())
            .unwrap_or_default()
    }

    /// Health probe for the dashboard /status endpoint.
    pub fn healthy(&self) -> bool {
        self.server
            .as_ref()
            .map(|_| {
                super::probe_health("127.0.0.1", self.server.as_ref().unwrap().server.port())
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

// ─── Transformers Inference Backend ────────────────────────────────────────

/// The running Transformers inference subprocess. Exposes an OpenAI-compatible
/// `/v1/*` surface that the existing `InferenceBackend` adapter drives.
pub struct TransformersServer {
    server: ToolServer,
    model: String,
}

impl TransformersServer {
    /// Spawns the Python inference server and waits for `/health`.
    /// The model loads lazily on first request, so startup stays fast.
    pub async fn spawn(data_dir: &Path, model: &str, device: &str) -> Result<Self> {
        let dir = data_dir.join("tools").join("transformers");
        let venv_python = dir.join("venv").join("bin").join("python");
        let args = vec![
            "--model".to_string(),
            model.to_string(),
            "--device".to_string(),
            device.to_string(),
        ];
        let server = ToolServer::start(
            &dir,
            &venv_python,
            TRANSFORMERS_SERVER_PY,
            &args,
            "scripts/setup-transformers.sh",
        )?;
        let port = server.port();
        if let Err(e) = wait_until_ready("127.0.0.1", port, Duration::from_secs(120)).await {
            let _ = server.stop().await;
            return Err(e.context("Transformers server did not become ready"));
        }
        Ok(Self {
            server,
            model: model.to_string(),
        })
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Manages the Transformers inference subprocess. `None` = disabled.
pub struct TransformersManager {
    server: Option<TransformersServer>,
}

impl TransformersManager {
    pub fn new(server: Option<TransformersServer>) -> Self {
        Self { server }
    }

    pub fn disabled() -> Self {
        Self { server: None }
    }

    pub fn enabled(&self) -> bool {
        self.server.is_some()
    }

    pub fn base_url(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.base_url())
    }

    pub fn model(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.model().to_string())
    }

    /// Health probe for the dashboard /status endpoint.
    pub fn healthy(&self) -> bool {
        self.server
            .as_ref()
            .map(|_| {
                super::probe_health("127.0.0.1", self.server.as_ref().unwrap().server.port())
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// M22: Diffusion (Stable Diffusion text-to-image via diffusers)
// ---------------------------------------------------------------------------

const DIFFUSION_SERVER_PY: &str = include_str!("diffusion_server.py");

/// The running Diffusion subprocess + its tool directory. `None` = disabled.
pub struct DiffusionServer {
    server: ToolServer,
}

impl DiffusionServer {
    /// Spawns and waits for `/health` (model download + load can take minutes
    /// on first run). Kills the child on timeout.
    pub async fn spawn(data_dir: &Path, model: &str) -> Result<Self> {
        let dir = data_dir.join("tools").join("diffusion");
        let venv_python = dir.join("venv").join("bin").join("python");
        let args = vec!["--model".to_string(), model.to_string()];
        let server = ToolServer::start(
            &dir,
            &venv_python,
            DIFFUSION_SERVER_PY,
            &args,
            "scripts/setup-diffusion.sh",
        )?;
        let port = server.port();
        // Model download + load can take minutes; generous timeout.
        if let Err(e) = wait_until_ready("127.0.0.1", port, Duration::from_secs(300)).await {
            let _ = server.stop().await;
            return Err(e.context("Diffusion server did not become ready"));
        }
        Ok(Self { server })
    }

    pub fn base_url(&self) -> String {
        self.server.base_url()
    }
}

/// M22: maximum prompt length accepted by the diffusion surface (REST +
/// MCP). Mirrors the MCP extractor bound so both doors share one policy.
pub const DIFFUSION_MAX_PROMPT_CHARS: usize = 4000;

/// Holds the Diffusion subprocess. `None` server = Diffusion disabled.
/// `max_size` / `max_steps` are the operator caps from `DiffusionSection`
/// (defaults 1024/50); the REST handler clamps caller input against them
/// before proxying, so oversized requests are rejected cheaply instead of
/// burning minutes of CPU in the Python backend (which keeps its own
/// 1024/50 clamp as the last line of defense).
pub struct DiffusionManager {
    server: Option<DiffusionServer>,
    max_size: u32,
    max_steps: u32,
}

impl DiffusionManager {
    pub fn new(server: Option<DiffusionServer>) -> Self {
        Self {
            server,
            max_size: 1024,
            max_steps: 50,
        }
    }

    /// Applies the operator caps from config. Zero values are sanitized to
    /// 1 so a misconfigured cap can never clamp every request to 0.
    pub fn with_limits(mut self, max_size: u32, max_steps: u32) -> Self {
        self.max_size = max_size.max(1);
        self.max_steps = max_steps.max(1);
        self
    }

    pub fn disabled() -> Self {
        Self {
            server: None,
            max_size: 1024,
            max_steps: 50,
        }
    }

    pub fn max_size(&self) -> u32 {
        self.max_size
    }

    pub fn max_steps(&self) -> u32 {
        self.max_steps
    }

    /// Upper-bound clamp for one image dimension against the operator cap.
    pub fn clamp_dim(&self, v: u32) -> u32 {
        v.min(self.max_size)
    }

    /// Upper-bound clamp for inference steps against the operator cap.
    pub fn clamp_steps(&self, s: u32) -> u32 {
        s.min(self.max_steps)
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
            .map(|_| {
                super::probe_health("127.0.0.1", self.server.as_ref().unwrap().server.port())
                    .is_ok()
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_packages_finder_tolerates_missing_lib() {
        let missing = Path::new("/nonexistent/venv/bin/python");
        assert!(find_site_packages(missing).is_none());
    }

    #[test]
    fn diffusion_manager_default_caps_match_mcp_and_python() {
        // REST parity: defaults must equal the MCP extractor clamp
        // (1024/50 in mcp.rs) and the Python backend clamp.
        let m = DiffusionManager::disabled();
        assert_eq!(m.max_size(), 1024);
        assert_eq!(m.max_steps(), 50);
        assert_eq!(m.clamp_dim(4096), 1024);
        assert_eq!(m.clamp_dim(512), 512);
        assert_eq!(m.clamp_steps(100), 50);
        assert_eq!(m.clamp_steps(20), 20);
    }

    #[test]
    fn diffusion_manager_with_limits_honours_operator_caps() {
        let m = DiffusionManager::disabled().with_limits(512, 10);
        assert_eq!(m.max_size(), 512);
        assert_eq!(m.max_steps(), 10);
        assert_eq!(m.clamp_dim(1024), 512);
        assert_eq!(m.clamp_steps(50), 10);
    }

    #[test]
    fn diffusion_manager_with_limits_sanitizes_zero() {
        let m = DiffusionManager::disabled().with_limits(0, 0);
        assert_eq!(m.max_size(), 1);
        assert_eq!(m.max_steps(), 1);
    }
}
