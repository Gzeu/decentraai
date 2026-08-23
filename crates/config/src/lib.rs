use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid YAML configuration: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("invalid configuration: {0}")]
    Validation(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub node: NodeSection,
    pub network: NetworkSection,
    pub storage: StorageSection,
    pub resources: ResourceSection,
    pub inference: InferenceSection,
    pub privacy: PrivacySection,
    pub security: SecuritySection,
    /// How inbound model announcements are handled (`swarm start`).
    /// Absent = Auto (download announced models with verification).
    #[serde(default)]
    pub sharing: SharingSection,
    /// Subscription tiers (P1). Absent = admin-token-only, which keeps
    /// existing installs unchanged.
    #[serde(default)]
    pub tiers: Option<TiersSection>,
    /// Local text-to-speech (Kokoro-82M ONNX subprocess). Absent = TTS is
    /// disabled; the dashboard chat shows no speak button and `/v1/tts`
    /// returns 404. Enabling requires the TTS venv + model files installed
    /// under the data dir (`tts/`).
    #[serde(default)]
    pub tts: Option<TtsSection>,
    /// Local OCR (RapidOCR onnxruntime subprocess). Absent = OCR is disabled;
    /// `/v1/ocr` returns 404. Enabling requires `<data_dir>/tools/ocr/venv`.
    #[serde(default)]
    pub ocr: Option<OcrSection>,
    /// Fabric Intelligence (the reasoning layer between a task and the
    /// deterministic planner). Absent = disabled; `/v1/intel/*` returns 404.
    #[serde(default)]
    pub fabric_intelligence: Option<FabricIntelligenceSection>,
    /// M15 Autonomous Compute Pressure. Absent = disabled; the node never
    /// requests assistance on its own.
    #[serde(default)]
    pub autonomous_assist: Option<AutonomousAssistSection>,
    /// M16 Agent Gateway (BYOA). Absent = gateway disabled; onboarding returns 404.
    #[serde(default)]
    pub agent_gateway: Option<AgentGatewaySection>,
    /// Local STT (faster-whisper subprocess). Absent = STT is disabled;
    /// `/v1/stt` returns 404. Enabling requires `<data_dir>/tools/stt/venv`.
    #[serde(default)]
    pub stt: Option<SttSection>,
    /// Local HF skills (small transformers pipelines subprocess). Absent =
    /// skills are disabled; `/v1/skills/<id>` returns 404. Enabling requires
    /// `<data_dir>/tools/skills/venv`.
    #[serde(default)]
    pub skills: Option<SkillsSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSection {
    pub name: String,
    pub mode: NodeMode,
    pub data_dir: String,
    /// Embedded dashboard to expose at `/`. `v1` remains the safe default;
    /// `v2` is an opt-in visual refresh available at `/ui2` either way.
    #[serde(default)]
    pub dashboard: DashboardVersion,
    /// Explicit GGUF model file name to serve (e.g.
    /// `Mistral-7B-Instruct-v0.3-Q4_K_M.gguf`). When set, the node serves this
    /// model instead of auto-detecting the first one in the models dir; a
    /// missing file is a hard error at startup. Optional (absent = auto-detect).
    #[serde(default)]
    pub model: Option<String>,
}

/// Which embedded dashboard is served at the root path.
///
/// Keeping this an enum makes invalid values a config-load failure instead of
/// silently selecting a UI the operator did not intend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DashboardVersion {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeMode {
    Conservative,
    Balanced,
    Contributor,
    Dedicated,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuPolicy {
    Auto,
    Required,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceRuntime {
    LlamaServer,
    Vllm,
    Sglang,
    Ollama,
    RemoteOpenAI,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustMode {
    Private,
    Open,
}

/// How a `swarm start` node reacts to manifest announcements from peers.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareMode {
    /// Download every announced model immediately. Every artifact is
    /// still verified (per-chunk BLAKE3 + Merkle gate) before use.
    Auto,
    /// Ask on stdin before downloading each announced model.
    Ask,
    /// Ignore announcements entirely.
    Off,
}

/// Automatic model sharing across the LAN swarm.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharingSection {
    pub mode: ShareMode,
    /// Upper bound on simultaneous auto downloads (disk/CPU guard).
    pub max_concurrent_downloads: u32,
    /// When true, a distributed worker that receives a workload for a model
    /// it does not yet hold fetches that model on demand from the requester
    /// through the verified-transfer pipeline (M14). The worker advertises
    /// `can_provision` only when this is set.
    pub provision_models_on_demand: bool,
    /// Compute Assist resource sharing (M14/M15 "Sharing is Caring").
    /// Absent = assist disabled; the node shares nothing by default.
    #[serde(default)]
    pub assist: Option<AssistSharingSection>,
}

impl Default for SharingSection {
    fn default() -> Self {
        Self {
            mode: ShareMode::Auto,
            max_concurrent_downloads: 2,
            provision_models_on_demand: true,
            // Compute Assist sharing is OPT-IN and conservative by default:
            // a node that never configures it shares NOTHING, exactly as
            // before the feature existed.
            assist: None,
        }
    }
}

/// Owner-controlled limits for Compute Assist resource sharing ("Sharing is
/// Caring", M14/M15 milestone 1). The node owner is AUTHORITATIVE: a remote
/// peer can never consume resources outside these limits, no matter what its
/// request asks for. Absent section = assist disabled entirely.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistSharingSection {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Maximum share of THIS node's CPU cores offered to assist workloads
    /// (percent of total cores, 1–100).
    #[serde(default = "default_assist_cpu_percent")]
    pub cpu_max_percent: u8,
    /// Maximum RAM (MiB) offered to assist workloads.
    #[serde(default = "default_assist_ram_mb")]
    pub ram_max_mb: u64,
    /// Hard lease ceiling in seconds — every lease is clamped to this and
    /// expires regardless of peer behavior.
    #[serde(default = "default_assist_lease_secs")]
    pub max_lease_seconds: u64,
    /// Capabilities this node is willing to assist with (hub taxonomy names).
    /// Empty list = ALL capabilities within the other limits.
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    /// Peer ids allowed to REQUEST assistance from this node. Empty = any
    /// TRUSTED peer (trust is still required on top of this list).
    #[serde(default)]
    pub allowed_peers: Vec<String>,
}

fn default_assist_cpu_percent() -> u8 {
    40
}
fn default_assist_ram_mb() -> u64 {
    2048
}
fn default_assist_lease_secs() -> u64 {
    120
}

impl AssistSharingSection {
    /// Boot-time cross-field validation.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.cpu_max_percent) {
            return Err(format!(
                "sharing.assist.cpu_max_percent must be within [1,100], got {}",
                self.cpu_max_percent
            ));
        }
        if self.ram_max_mb == 0 {
            return Err("sharing.assist.ram_max_mb must be > 0".into());
        }
        if self.max_lease_seconds == 0 {
            return Err("sharing.assist.max_lease_seconds must be > 0".into());
        }
        Ok(())
    }

    /// Deterministic clamp of an incoming request against owner limits.
    /// Returns `None` when the capability or peer is not allowed at all.
    pub fn admit(
        &self,
        capability: &str,
        requesting_peer: &str,
        trusted: bool,
        requested_cpu: u16,
        requested_ram_mb: u64,
    ) -> Option<(u16, u64)> {
        if !self.enabled || !trusted {
            return None;
        }
        if !self.allowed_peers.is_empty()
            && !self.allowed_peers.iter().any(|p| p == requesting_peer)
        {
            return None;
        }
        if !self.allowed_capabilities.is_empty()
            && !self.allowed_capabilities.iter().any(|c| c == capability)
        {
            return None;
        }
        Some((requested_cpu.min(self.cpu_cap_cores()), requested_ram_mb.min(self.ram_max_mb)))
    }

    /// Cores this node may offer, derived from cpu_max_percent.
    pub fn cpu_cap_cores(&self) -> u16 {
        let total = std::thread::available_parallelism()
            .map(|n| n.get() as u64)
            .unwrap_or(1);
        ((total * u64::from(self.cpu_max_percent)) / 100).max(1) as u16
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSection {
    pub private_swarm: bool,
    pub lan_discovery: bool,
    pub dht_enabled: bool,
    pub relay_enabled: bool,
    pub bootstrap_peers: Vec<String>,
    pub max_connections: u16,
    pub max_message_bytes: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSection {
    pub chunk_size_mb: u16,
    pub hash_algorithm: String,
    pub max_cache_gb: u32,
    pub min_free_disk_gb: u32,
    pub verify_full_file_after_assembly: bool,
    pub allow_unsigned_models: bool,
    pub auto_seed_verified_models: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSection {
    pub cpu_max_percent: u8,
    pub reserve_cpu_cores: u16,
    pub memory_max_percent: u8,
    pub reserve_ram_mb: u32,
    pub gpu_enabled: GpuPolicy,
    pub gpu_max_vram_percent: u8,
    pub reserve_vram_mb: u32,
    pub stop_gpu_temperature_celsius: u8,
    pub max_upload_mbps: u32,
    pub max_download_mbps: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceSection {
    pub enabled: InferenceMode,
    pub runtime: InferenceRuntime,
    pub bind_address: String,
    pub api_auth_required: bool,
    pub allow_remote_inference: bool,
    pub max_concurrent_requests: u16,
    pub max_context_tokens: u32,
    pub max_generated_tokens: u32,
    pub request_timeout_seconds: u32,
    pub queue_max_requests: u16,
    pub idle_model_unload_minutes: u16,
    /// Fixed port for the OpenAI-compatible API; 0 means ephemeral.
    pub api_port: u16,
    /// Sampling defaults applied when a request omits them (Q1).
    #[serde(default)]
    pub generation: GenerationSection,
    /// Optional engine-kind override (M22). Wire values match the engine
    /// fabric's `EngineKind::as_str`: `llama-server`, `vllm`, `sglang`,
    /// `ollama`, `openai-compatible`. When set for a worker, it is advertised
    /// honestly so coordinators' planners reason engine-aware. `None`
    /// (default) keeps the llama-server runtime and behavior unchanged.
    #[serde(default)]
    pub engine: Option<String>,
    /// Optional remote OpenAI-compatible backend URL (M22). When set with a
    /// non-llama `engine`, `serve start` probes and serves this remote instead
    /// of a local llama-server. Must start with `http://` or `https://`.
    #[serde(default)]
    pub backend_url: Option<String>,
    /// Optional local OpenAI-compatible embeddings backend (a llama-server
    /// launched with `--embedding`, e.g. on `nomic-embed-text-v1.5`). When
    /// set, the node exposes `/v1/embeddings` for the RAG retrieval path.
    #[serde(default)]
    pub embeddings_backend_url: Option<String>,
    /// Distributed-fabric tuning (P14 consolidation). Each knob is OPTIONAL:
    /// `None` falls back to `decentraai_distributed::InferenceConfig::default`
    /// (the previous hard-coded behavior), so existing configs are unchanged.
    /// These were previously dead — the runtime always built
    /// `InferenceConfig::default()` regardless of the operator YAML.
    /// Maximum retry attempts for a routed request.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Backoff (ms) between retry attempts.
    #[serde(default)]
    pub retry_backoff_ms: Option<u64>,
    /// Worker announcement broadcast interval (ms).
    #[serde(default)]
    pub announcement_interval_ms: Option<u64>,
    /// Stale-worker check interval (ms).
    #[serde(default)]
    pub discovery_interval_ms: Option<u64>,
    /// Heartbeat gap after which a worker is stale/offline (ms).
    #[serde(default)]
    pub stale_worker_timeout_ms: Option<u64>,
    /// Max queue depth per worker before it is considered overloaded.
    #[serde(default)]
    pub max_queue_depth: Option<u32>,
    /// Min available capacity (0..1) for a worker to be eligible.
    #[serde(default)]
    pub min_available_capacity: Option<f32>,
}

/// Generation defaults injected into inference requests that do not
/// specify them. Small models answer far more coherently with sampling
/// parameters and a system line than with raw llama.cpp defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSection {
    pub temperature: f32,
    pub top_p: f32,
    #[serde(default)]
    pub top_k: Option<i32>,
    pub repeat_penalty: f32,
    /// Prepended as a system message when the conversation has none.
    /// Empty = no system prompt.
    #[serde(default)]
    pub system_prompt: String,
}

impl Default for GenerationSection {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: Some(40),
            repeat_penalty: 1.1,
            system_prompt: String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacySection {
    pub log_prompts: bool,
    pub log_outputs: bool,
    pub publish_exact_hardware: bool,
    pub telemetry_opt_in: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecuritySection {
    pub trust_mode: TrustMode,
    pub require_signed_announcements: bool,
    pub require_request_signatures: bool,
    pub ban_duration_minutes: u32,
    pub max_invalid_chunks_per_peer: u8,
}

/// One subscription tier: which models its tokens may use and how fast.
/// `models` is an allowlist of model file names; an empty list means
/// "every model the node serves" (the admin's own posture).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierPolicy {
    #[serde(default)]
    pub models: Vec<String>,
    pub rate_limit_per_minute: u32,
}

/// Subscription tiers (P1): guest / contributor / core.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TiersSection {
    pub tier1: TierPolicy,
    pub tier2: TierPolicy,
    pub tier3: TierPolicy,
}

impl TiersSection {
    pub fn policy(&self, tier: u8) -> Option<&TierPolicy> {
        match tier {
            1 => Some(&self.tier1),
            2 => Some(&self.tier2),
            3 => Some(&self.tier3),
            _ => None,
        }
    }
}

/// Local text-to-speech (TTS). Drives a Piper VITS Python subprocess
/// (external engine — never FFI) exposed to the dashboard chat as `/v1/tts`.
/// Piper supports Romanian natively; voices live in `<data_dir>/tts/models/
/// piper-ro/` and the Python venv in `<data_dir>/tts/venv/`. Absent section
/// = TTS off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsSection {
    /// Enable the TTS subprocess at node start. If true but the venv/model
    /// files are missing, the node logs a warning and serves without TTS
    /// (dashboard hides the speak button) instead of failing startup.
    #[serde(default)]
    pub enabled: bool,
    /// Piper voice id, e.g. `ro_RO-raluca-high` (female, Romanian),
    /// `ro_RO-lili-high` (female), `ro_RO-mihai-medium` (male). Defaults to
    /// the Romanian female voice `ro_RO-raluca-high` when absent.
    #[serde(default = "default_tts_voice")]
    pub voice: String,
    /// Speech rate multiplier (0.5 = half speed, 1.0 = normal, 1.5 = fast).
    #[serde(default = "default_tts_speed")]
    pub speed: f64,
}

fn default_tts_voice() -> String {
    "ro_RO-raluca-high".to_string()
}

fn default_tts_speed() -> f64 {
    1.0
}

impl Default for TtsSection {
    fn default() -> Self {
        Self {
            enabled: false,
            voice: default_tts_voice(),
            speed: default_tts_speed(),
        }
    }
}

/// Local optical character recognition (OCR). Drives a RapidOCR
/// (PP-OCRv4 on onnxruntime) Python subprocess — external engine, never FFI —
/// exposed as `/v1/ocr`. Models are bundled in the wheel; the venv lives in
/// `<data_dir>/tools/ocr/venv/`. Absent section = OCR off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrSection {
    /// Enable the OCR subprocess at node start. If true but the venv is
    /// missing, the node logs a warning and serves without OCR instead of
    /// failing startup.
    #[serde(default)]
    pub enabled: bool,
    /// Default recognition language passed to the engine (`en`, `ro`, …).
    #[serde(default = "default_ocr_lang")]
    pub lang: String,
}

fn default_ocr_lang() -> String {
    "en".to_string()
}

impl Default for OcrSection {
    fn default() -> Self {
        Self {
            enabled: false,
            lang: default_ocr_lang(),
        }
    }
}

/// Fabric Intelligence policy: how the intelligence layer chooses between
/// the local backend and an external OpenAI-compatible provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FabricIntelligencePolicy {
    /// Local backend first; external only as an allowed fallback when the
    /// local attempt fails. DEFAULT — user content stays on-node.
    #[default]
    LocalFirst,
    /// External first; local only as fallback.
    ExternalFirst,
    /// Never call an external provider (air-gapped deployments).
    LocalOnly,
    /// Always external; local never consulted.
    ExternalOnly,
    /// Whichever succeeds first, tried in local→external order.
    Fallback,
}

/// External OpenAI-compatible provider settings. The API key is NEVER a
/// config field: it is read from the environment variable named by
/// `api_key_env` at call time, so nothing secret persists in YAML, backups
/// or `/status` output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FabricIntelExternalSection {
    pub base_url: String,
    pub api_key_env: String,
    pub model: String,
}

/// M15 — Autonomous Compute Pressure: the node observes ITS OWN pressure
/// signals and may trigger an assist request through the EXISTING DFCP flow.
/// The agent/pressure layer can only PROPOSE; routing stays with the
/// deterministic planner. Opt-in; absent = disabled.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutonomousAssistSection {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Seconds between pressure evaluations.
    #[serde(default = "default_tick_secs")]
    pub tick_seconds: u64,
    /// Minimum seconds between two consecutive assist requests (hysteresis
    /// cooldown on top of the score-based state machine).
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_seconds: u64,
    #[serde(default)]
    pub thresholds: PressureThresholdsSection,
    /// The assist profile executed when pressure fires: which capability to
    /// offload and what payload template to send.
    #[serde(default)]
    pub profile: Option<AssistProfileSection>,
}

fn default_tick_secs() -> u64 {
    15
}
fn default_cooldown_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PressureThresholdsSection {
    #[serde(default = "default_queue_high")]
    pub queue_depth_high: u32,
    #[serde(default = "default_latency_high")]
    pub latency_ms_high: u64,
    #[serde(default = "default_cpu_high")]
    pub cpu_percent_high: f32,
    #[serde(default = "default_ram_high")]
    pub ram_percent_high: f32,
}

impl Default for PressureThresholdsSection {
    fn default() -> Self {
        Self {
            queue_depth_high: default_queue_high(),
            latency_ms_high: default_latency_high(),
            cpu_percent_high: default_cpu_high(),
            ram_percent_high: default_ram_high(),
        }
    }
}

impl PressureThresholdsSection {
    /// Boot-time sanity mirroring `compute::pressure::PressureThresholds`.
    pub fn validate(&self) -> Result<(), String> {
        if self.queue_depth_high == 0 {
            return Err("queue_depth_high must be > 0".into());
        }
        if self.latency_ms_high == 0 {
            return Err("latency_ms_high must be > 0".into());
        }
        if !(1.0..=100.0).contains(&self.cpu_percent_high) {
            return Err(format!("cpu_percent_high must be within [1,100], got {}", self.cpu_percent_high));
        }
        if !(1.0..=100.0).contains(&self.ram_percent_high) {
            return Err(format!("ram_percent_high must be within [1,100], got {}", self.ram_percent_high));
        }
        Ok(())
    }
}

fn default_queue_high() -> u32 {
    2
}
fn default_latency_high() -> u64 {
    5_000
}
fn default_cpu_high() -> f32 {
    90.0
}
fn default_ram_high() -> f32 {
    85.0
}

/// What to offload when pressure fires. `payload_template` is a JSON object
/// sent verbatim as the assist task payload (e.g. an OpenAI-shaped chat body
/// or an embeddings request); placeholders like {input} are filled by the
/// caller of the assist endpoint, not by the engine.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistProfileSection {
    pub capability: String,
    pub payload_template: serde_json::Value,
}

/// Fabric Intelligence configuration. Absent section = disabled; the node
/// behaves exactly as before this feature existed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FabricIntelligenceSection {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default)]
    pub policy: FabricIntelligencePolicy,
    /// Plans with confidence below this are treated as low-quality: under
    /// `local_first` they may trigger the external fallback (if configured).
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    /// Preferred local intelligence model name (advisory — the node's served
    /// model answers if it differs; the node owns its engine).
    #[serde(default)]
    pub local_model: Option<String>,
    /// External provider; optional even when the section itself is enabled.
    #[serde(default)]
    pub external: Option<FabricIntelExternalSection>,
    /// Auto-provisioning artifact ceiling for intelligence-recommended
    /// models. Hard cap is [`MAX_FABRIC_ARTIFACT_BYTES`] (2 GiB); config may
    /// only LOWER it.
    #[serde(default = "default_max_artifact_bytes")]
    pub max_artifact_bytes: u64,
}

fn default_false() -> bool {
    false
}
fn default_min_confidence() -> f32 {
    0.5
}
fn default_max_artifact_bytes() -> u64 {
    crate::MAX_FABRIC_ARTIFACT_BYTES
}

/// The absolute ceiling any `max_artifact_bytes` may take (2 GiB). Enforced
/// in validation, not just convention.
pub const MAX_FABRIC_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

impl Default for FabricIntelligenceSection {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: FabricIntelligencePolicy::LocalFirst,
            min_confidence: default_min_confidence(),
            local_model: None,
            external: None,
            max_artifact_bytes: MAX_FABRIC_ARTIFACT_BYTES,
        }
    }
}

impl FabricIntelligenceSection {
    /// Cross-field validation at load time, so a broken intel config fails
    /// at boot instead of on the first request.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(format!(
                "fabric_intelligence.min_confidence must be within [0,1], got {}",
                self.min_confidence
            ));
        }
        // An EXTERNAL-ONLY policy with no external endpoint can never answer:
        // fail at boot, not silently per request.
        if self.policy == FabricIntelligencePolicy::ExternalOnly && self.external.is_none() {
            return Err(
                "fabric_intelligence.policy=external_only requires an external section"
                    .to_string(),
            );
        }
        if let Some(ext) = &self.external {
            if ext.base_url.trim().is_empty()
                || ext.api_key_env.trim().is_empty()
                || ext.model.trim().is_empty()
            {
                return Err(
                    "fabric_intelligence.external requires non-empty base_url, api_key_env and model"
                        .to_string(),
                );
            }
        }
        if self.max_artifact_bytes > MAX_FABRIC_ARTIFACT_BYTES {
            return Err(format!(
                "fabric_intelligence.max_artifact_bytes exceeds the hard limit of {MAX_FABRIC_ARTIFACT_BYTES} bytes"
            ));
        }
        if self.max_artifact_bytes == 0 {
            return Err("fabric_intelligence.max_artifact_bytes must be > 0".to_string());
        }
        Ok(())
    }
}

/// Agent Gateway (M16): scoped identities for external agents (BYOA).
/// Absent = gateway disabled; onboarding endpoint returns 404.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGatewaySection {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Max quota ceiling an onboarded agent may receive.
    #[serde(default = "default_gateway_quota")]
    pub max_quota_ceiling: u64,
    /// Max rate limit an onboarded agent may receive (req/min).
    #[serde(default = "default_gateway_rate")]
    pub max_rate_limit: u32,
    /// Max expiry in seconds for an onboarded credential (0 = no expiry cap).
    #[serde(default = "default_gateway_expiry")]
    pub max_expiry_seconds: u64,
    /// Capabilities allowed for onboarding. Empty = any hub taxonomy capability.
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    /// Conservative free-starter preset applied when the caller requests starter.
    #[serde(default)]
    pub free_starter: FreeStarterSection,
}

fn default_gateway_quota() -> u64 {
    1000
}
fn default_gateway_rate() -> u32 {
    60
}
fn default_gateway_expiry() -> u64 {
    86400
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeStarterSection {
    #[serde(default = "default_starter_quota")]
    pub quota_ceiling: u64,
    #[serde(default = "default_starter_rate")]
    pub rate_limit: u32,
    #[serde(default = "default_starter_scopes")]
    pub scopes: Vec<String>,
}

fn default_starter_quota() -> u64 {
    100
}
fn default_starter_rate() -> u32 {
    10
}
fn default_starter_scopes() -> Vec<String> {
    vec!["inference".to_string(), "embeddings".to_string()]
}

impl Default for AgentGatewaySection {
    fn default() -> Self {
        Self {
            enabled: false,
            max_quota_ceiling: default_gateway_quota(),
            max_rate_limit: default_gateway_rate(),
            max_expiry_seconds: default_gateway_expiry(),
            allowed_capabilities: vec![],
            free_starter: FreeStarterSection::default(),
        }
    }
}

impl Default for FreeStarterSection {
    fn default() -> Self {
        Self {
            quota_ceiling: default_starter_quota(),
            rate_limit: default_starter_rate(),
            scopes: default_starter_scopes(),
        }
    }
}

impl AgentGatewaySection {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.max_quota_ceiling == 0 {
            return Err("agent_gateway.max_quota_ceiling must be > 0".into());
        }
        if self.max_rate_limit == 0 {
            return Err("agent_gateway.max_rate_limit must be > 0".into());
        }
        Ok(())
    }
}

/// Local speech-to-text (STT). Drives a faster-whisper (CTranslate2) Python
/// subprocess — external engine, never FFI — exposed as `/v1/stt`. Models
/// download on first use (or are pre-placed in `<data_dir>/tools/stt/models`
/// via HF_HOME). Absent section = STT off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SttSection {
    /// Enable the STT subprocess at node start. If true but the venv is
    /// missing, the node logs a warning and serves without STT instead of
    /// failing startup.
    #[serde(default)]
    pub enabled: bool,
    /// faster-whisper model size: `tiny`, `base`, `small`, `medium`, `large-v3`.
    #[serde(default = "default_stt_model")]
    pub model: String,
}

fn default_stt_model() -> String {
    "base".to_string()
}

impl Default for SttSection {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_stt_model(),
        }
    }
}

/// Local HF skills (small transformers pipelines — sentiment, NER, summarize,
/// translate ro↔en). One Python subprocess hosts all enabled pipelines,
/// exposed as `/v1/skills/<id>`. Models download on first use into
/// `<data_dir>/tools/skills/models`. Absent section = skills off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsSection {
    /// Enable the skills subprocess at node start. If true but the venv is
    /// missing, the node logs a warning and serves without skills instead of
    /// failing startup.
    #[serde(default)]
    pub enabled: bool,
    /// Skill ids to run: `sentiment`, `ner`, `summarize`, `translate_ro_en`,
    /// `translate_en_ro`. Unknown ids are rejected at config time.
    #[serde(default = "default_skills")]
    pub list: Vec<String>,
}

fn default_skills() -> Vec<String> {
    vec![
        "sentiment".to_string(),
        "ner".to_string(),
        "summarize".to_string(),
        "translate_ro_en".to_string(),
        "translate_en_ro".to_string(),
    ]
}

impl Default for SkillsSection {
    fn default() -> Self {
        Self {
            enabled: false,
            list: default_skills(),
        }
    }
}

impl NodeConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path)?;
        let config: Self = serde_yaml::from_str(&raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.node.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "node.name must not be empty".into(),
            ));
        }
        if let Some(intel) = &self.fabric_intelligence {
            intel.validate().map_err(ConfigError::Validation)?;
        }
        if let Some(assist) = &self.autonomous_assist {
            assist.thresholds.validate().map_err(|e| {
                ConfigError::Validation(format!("autonomous_assist: {e}"))
            })?;
            if assist.enabled && assist.profile.is_none() {
                return Err(ConfigError::Validation(
                    "autonomous_assist.enabled requires a `profile` (capability + payload_template)"
                        .into(),
                ));
            }
            if let Some(profile) = &assist.profile {
                if profile.capability.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "autonomous_assist.profile.capability must not be empty".into(),
                    ));
                }
                if !profile.payload_template.is_object() {
                    return Err(ConfigError::Validation(
                        "autonomous_assist.profile.payload_template must be a JSON object".into(),
                    ));
                }
            }
        }
        if let Some(gw) = &self.agent_gateway {
            gw.validate().map_err(ConfigError::Validation)?;
        }
        if self.network.max_connections == 0 {
            return Err(ConfigError::Validation(
                "network.max_connections must be greater than zero".into(),
            ));
        }
        if self.sharing.max_concurrent_downloads == 0 {
            return Err(ConfigError::Validation(
                "sharing.max_concurrent_downloads must be greater than zero".into(),
            ));
        }
        if !(1..=64).contains(&self.storage.chunk_size_mb) {
            return Err(ConfigError::Validation(
                "storage.chunk_size_mb must be between 1 and 64".into(),
            ));
        }
        if self.storage.hash_algorithm != "blake3" && self.storage.hash_algorithm != "sha256" {
            return Err(ConfigError::Validation(
                "storage.hash_algorithm must be blake3 or sha256".into(),
            ));
        }
        for (name, value) in [
            ("resources.cpu_max_percent", self.resources.cpu_max_percent),
            (
                "resources.memory_max_percent",
                self.resources.memory_max_percent,
            ),
            (
                "resources.gpu_max_vram_percent",
                self.resources.gpu_max_vram_percent,
            ),
        ] {
            if value == 0 || value > 100 {
                return Err(ConfigError::Validation(format!(
                    "{name} must be between 1 and 100"
                )));
            }
        }
        if self.inference.bind_address != "127.0.0.1"
            && self.inference.bind_address != "::1"
            && !self.inference.api_auth_required
        {
            return Err(ConfigError::Validation(
                "non-local inference bind_address requires api_auth_required: true".into(),
            ));
        }
        // Subscription tiers must never be silently disabled: when tiers are
        // configured but api_auth_required is false, classify() treats every
        // caller as Auth::Open and the per-tier model allowlist + rate limits
        // are bypassed entirely. Require auth so the tier boundary is real.
        if self.tiers.is_some() && !self.inference.api_auth_required {
            return Err(ConfigError::Validation(
                "subscription tiers require inference.api_auth_required: true (otherwise tier model allowlists and rate limits are silently disabled)"
                    .into(),
            ));
        }
        // HF skills: unknown ids are a config typo, never a silent no-op.
        if let Some(skills) = &self.skills {
            if skills.enabled {
                const KNOWN_SKILLS: [&str; 5] = [
                    "sentiment",
                    "ner",
                    "summarize",
                    "translate_ro_en",
                    "translate_en_ro",
                ];
                for id in &skills.list {
                    if !KNOWN_SKILLS.contains(&id.as_str()) {
                        return Err(ConfigError::Validation(format!(
                            "skills.list contains unknown skill '{id}' (known: {KNOWN_SKILLS:?})"
                        )));
                    }
                }
            }
        }
        if self.inference.allow_remote_inference && !self.network.private_swarm {
            return Err(ConfigError::Validation(
                "remote inference requires private_swarm in the initial release".into(),
            ));
        }
        if self.inference.api_port != 0 && self.inference.api_port < 1024 {
            return Err(ConfigError::Validation(
                "inference.api_port must be 0 (ephemeral) or at least 1024".into(),
            ));
        }
        if let Some(engine) = &self.inference.engine {
            if !is_known_engine(engine) {
                return Err(ConfigError::Validation(format!(
                    "inference.engine must be one of llama-server, vllm, sglang, ollama, openai-compatible (got {engine})"
                )));
            }
        }
        if let Some(url) = &self.inference.backend_url {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ConfigError::Validation(format!(
                    "inference.backend_url must start with http:// or https:// (got {url})"
                )));
            }
        }
        let generation = &self.inference.generation;
        if !(0.0..=2.0).contains(&generation.temperature) {
            return Err(ConfigError::Validation(
                "inference.generation.temperature must be between 0.0 and 2.0".into(),
            ));
        }
        if generation.top_p <= 0.0 || generation.top_p > 1.0 {
            return Err(ConfigError::Validation(
                "inference.generation.top_p must be in (0.0, 1.0]".into(),
            ));
        }
        if !(0.0..=2.0).contains(&generation.repeat_penalty) {
            return Err(ConfigError::Validation(
                "inference.generation.repeat_penalty must be between 0.0 and 2.0".into(),
            ));
        }
        if let Some(tiers) = &self.tiers {
            for (name, policy) in [
                ("tiers.tier1", &tiers.tier1),
                ("tiers.tier2", &tiers.tier2),
                ("tiers.tier3", &tiers.tier3),
            ] {
                if policy.rate_limit_per_minute == 0 {
                    return Err(ConfigError::Validation(format!(
                        "{name}.rate_limit_per_minute must be greater than zero"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Whether `s` is a recognized engine-kind wire string (M22). This is strict,
/// mirroring the engine fabric's known kinds: an unknown engine is a config
/// error at load time rather than silently degrading to `openai-compatible`,
/// so a typo'd engine is caught early instead of mis-advertising a node's real
/// runtime.
pub fn is_known_engine(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "llama-server"
            | "llama_server"
            | "llamacpp"
            | "llama.cpp"
            | "vllm"
            | "sglang"
            | "sglang_server"
            | "ollama"
            | "openai-compatible"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Fail-closed pin: `require_signed_announcements` has NO serde default.
    /// A config that omits it must FAIL to parse — never silently fall back
    /// to `false` (which would let the node accept unsigned mDNS model
    /// announcements). The safe default lives in the example/init templates,
    /// not in a silent deserialization fallback.
    #[test]
    fn omitting_require_signed_announcements_is_a_parse_error() {
        let yaml = r#"
node:
  name: t
  mode: balanced
  data_dir: /tmp/decentraai-test
network:
  private_swarm: true
storage:
  chunk_size_mb: 4
  hash_algorithm: blake3
  max_cache_gb: 10
  min_free_disk_gb: 1
resources:
  cpu_max_percent: 80
  memory_max_percent: 80
  reserve_cpu_cores: 1
  reserve_ram_mb: 512
inference:
  enabled: auto
  runtime: llama_server
  bind_address: 127.0.0.1
  api_auth_required: true
  allow_remote_inference: false
  max_concurrent_requests: 1
  max_context_tokens: 2048
  max_generated_tokens: 256
  request_timeout_seconds: 120
  queue_max_requests: 4
  idle_model_unload_minutes: 10
  api_port: 0
privacy:
  log_prompts: false
  log_outputs: false
  publish_exact_hardware: false
  telemetry_opt_in: false
security:
  trust_mode: private
  require_request_signatures: true
  ban_duration_minutes: 60
  max_invalid_chunks_per_peer: 2
"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        let result = NodeConfig::load(file.path());
        assert!(
            result.is_err(),
            "omitting require_signed_announcements must be a parse error (fail-closed)"
        );
    }

    #[test]
    fn example_configuration_is_valid() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let result = NodeConfig::load(file.path());
        if let Err(e) = &result {
            eprintln!("Config load error: {}", e);
        }
        assert!(result.is_ok());
    }

    #[test]
    fn example_generation_defaults_are_sane() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let generation = &config.inference.generation;
        assert_eq!(generation.temperature, 0.7);
        assert_eq!(generation.top_p, 0.9);
        assert_eq!(generation.repeat_penalty, 1.1);
        assert!(!generation.system_prompt.is_empty());
    }

    #[test]
    fn dashboard_version_accepts_v1_and_v2_and_rejects_other_values() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();

        let v1 = NodeConfig::load(file.path()).unwrap();
        assert_eq!(v1.node.dashboard, DashboardVersion::V1);

        std::fs::write(
            file.path(),
            raw.replace("dashboard: \"v1\"", "dashboard: \"v2\""),
        )
        .unwrap();
        let v2 = NodeConfig::load(file.path()).unwrap();
        assert_eq!(v2.node.dashboard, DashboardVersion::V2);

        std::fs::write(
            file.path(),
            raw.replace("dashboard: \"v1\"", "dashboard: \"neon\""),
        )
        .unwrap();
        assert!(NodeConfig::load(file.path()).is_err());
    }

    #[test]
    fn out_of_range_temperature_is_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace("temperature: 0.7", "temperature: 9.0");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("temperature"));
    }

    #[test]
    fn example_tiers_parse_and_gate_models() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let tiers = config.tiers.expect("example config defines tiers");
        assert_eq!(tiers.tier1.rate_limit_per_minute, 10);
        assert_eq!(tiers.policy(1).unwrap().models.len(), 1);
        assert_eq!(
            tiers.policy(3).unwrap().models.len(),
            0,
            "empty allowlist = all models"
        );
        assert!(tiers.policy(4).is_none());
    }

    #[test]
    fn zero_tier_rate_limit_is_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace("rate_limit_per_minute: 10", "rate_limit_per_minute: 0");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("rate_limit_per_minute"));
    }

    #[test]
    fn tiers_require_api_auth() {
        // The example config is valid (tiers + api_auth_required: true).
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        // Flip auth off while tiers remain configured: must be rejected so the
        // tier allowlist + rate limits can never be silently disabled.
        let bad = raw.replace("api_auth_required: true", "api_auth_required: false");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(
            err.to_string().contains("tiers require"),
            "tiers without api_auth_required must be rejected, got: {err}"
        );
    }

    #[test]
    fn ocr_and_stt_are_optional_and_off_by_default() {
        // The example config has no ocr/stt sections: they must parse as
        // absent (= disabled) and never break existing installs.
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert!(config.ocr.is_none());
        assert!(config.stt.is_none());
    }

    #[test]
    fn ocr_section_parses_and_unknown_fields_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let with_ocr = format!(
            "{raw}\nocr:\n  enabled: true\n  lang: \"ro\"\n"
        );
        std::fs::write(file.path(), with_ocr).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let ocr = config.ocr.expect("ocr section parsed");
        assert!(ocr.enabled);
        assert_eq!(ocr.lang, "ro");

        // deny_unknown_fields: a typo must fail validation, not be ignored.
        let bad = format!("{raw}\nocr:\n  enabled: true\n  lenguage: \"en\"\n");
        std::fs::write(file.path(), bad).unwrap();
        assert!(NodeConfig::load(file.path()).is_err());
    }

    #[test]
    fn stt_section_parses_and_unknown_fields_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let with_stt = format!("{raw}\nstt:\n  enabled: true\n  model: \"tiny\"\n");
        std::fs::write(file.path(), with_stt).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let stt = config.stt.expect("stt section parsed");
        assert!(stt.enabled);
        assert_eq!(stt.model, "tiny");

        // deny_unknown_fields: a typo must fail validation, not be ignored.
        let bad = format!("{raw}\nstt:\n  enabled: true\n  modle: \"base\"\n");
        std::fs::write(file.path(), bad).unwrap();
        assert!(NodeConfig::load(file.path()).is_err());
    }

    #[test]
    fn skills_section_parses_and_unknown_skill_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let with_skills = format!(
            "{raw}\nskills:\n  enabled: true\n  list: [\"sentiment\", \"ner\"]\n"
        );
        std::fs::write(file.path(), with_skills).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let skills = config.skills.expect("skills section parsed");
        assert!(skills.enabled);
        assert_eq!(skills.list, vec!["sentiment", "ner"]);

        // An unknown skill id is a typo — must be rejected, never a no-op.
        let bad = format!("{raw}\nskills:\n  enabled: true\n  list: [\"magic\"]\n");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(
            err.to_string().contains("unknown skill"),
            "unknown skill must be rejected, got: {err}"
        );

        // Disabled skills may carry an unknown id without failing (they never run).
        let off = format!("{raw}\nskills:\n  enabled: false\n  list: [\"magic\"]\n");
        std::fs::write(file.path(), off).unwrap();
        assert!(NodeConfig::load(file.path()).is_ok());
    }

    #[test]
    fn invalid_gpu_policy_fails_to_parse() {
        let yaml = r#"
node:
  name: test
  mode: balanced
  data_dir: /tmp
network:
  private_swarm: true
  lan_discovery: true
  dht_enabled: true
  relay_enabled: true
  bootstrap_peers: []
  max_connections: 50
  max_message_bytes: 1048576
storage:
  chunk_size_mb: 16
  hash_algorithm: blake3
  max_cache_gb: 100
  min_free_disk_gb: 20
  verify_full_file_after_assembly: true
  allow_unsigned_models: false
  auto_seed_verified_models: true
resources:
  cpu_max_percent: 80
  reserve_cpu_cores: 2
  memory_max_percent: 80
  reserve_ram_mb: 1024
  gpu_enabled: require
  gpu_max_vram_percent: 75
  reserve_vram_mb: 1024
  stop_gpu_temperature_celsius: 83
  max_upload_mbps: 20
  max_download_mbps: 80
inference:
  enabled: auto
  runtime: llama_server
  bind_address: 127.0.0.1
  api_auth_required: true
  allow_remote_inference: false
  max_concurrent_requests: 4
  max_context_tokens: 4096
  max_generated_tokens: 2048
  request_timeout_seconds: 120
  queue_max_requests: 10
  idle_model_unload_minutes: 10
  api_port: 0
privacy:
  log_prompts: false
  log_outputs: false
  publish_exact_hardware: false
  telemetry_opt_in: false
security:
  trust_mode: private
  require_signed_announcements: true
  require_request_signatures: true
  ban_duration_minutes: 60
  max_invalid_chunks_per_peer: 10
"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        assert!(NodeConfig::load(file.path()).is_err());
    }

    #[test]
    fn privileged_api_port_is_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace("api_port: 8080", "api_port: 80");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("api_port"));
    }

    #[test]
    fn example_sharing_section_parses() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert_eq!(config.sharing.mode, ShareMode::Auto);
        assert_eq!(config.sharing.max_concurrent_downloads, 2);
        assert!(config.sharing.provision_models_on_demand);
    }

    #[test]
    fn missing_tts_section_defaults_to_disabled() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert!(config.tts.is_none(), "example config must stay TTS-free");
    }

    #[test]
    fn tts_section_parses_with_voice_and_speed() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let with_tts =
            format!("{raw}\ntts:\n  enabled: true\n  voice: ro_RO-raluca-high\n  speed: 1.15\n");
        std::fs::write(file.path(), with_tts).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let tts = config.tts.as_ref().expect("tts section should parse");
        assert!(tts.enabled);
        assert_eq!(tts.voice, "ro_RO-raluca-high");
        assert_eq!(tts.speed, 1.15);
    }

    #[test]
    fn missing_sharing_section_defaults_to_auto() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let stripped = raw
            .replace(
                "sharing:\n  mode: \"auto\"\n  max_concurrent_downloads: 2\n  provision_models_on_demand: true\n",
                "",
            )
            .replace(
                "sharing:\n  mode: \"auto\"\n  max_concurrent_downloads: 2\n  provision_models_on_demand: true\n\n",
                "",
            );
        std::fs::write(file.path(), stripped).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert_eq!(config.sharing.mode, ShareMode::Auto);
        assert!(config.sharing.provision_models_on_demand);
    }

    /// Fabric Intelligence is OPT-IN: a config without the section loads
    /// fine and the layer stays disabled (backward compatibility).
    #[test]
    fn missing_fabric_intelligence_section_means_disabled() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert!(
            config.fabric_intelligence.is_none(),
            "example yaml must not silently enable the feature"
        );
    }

    #[test]
    fn fabric_intelligence_section_parses_with_external() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        std::fs::write(
            file.path(),
            format!(
                "{raw}\nfabric_intelligence:\n  enabled: true\n  policy: local_first\n  min_confidence: 0.6\n  local_model: qwen3-0.6b\n  external:\n    base_url: https://api.openai.com/v1\n    api_key_env: OPENAI_API_KEY\n    model: gpt-4o-mini\n"
            ),
        )
        .unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        let intel = config.fabric_intelligence.expect("section present");
        assert!(intel.enabled);
        assert_eq!(
            intel.policy,
            FabricIntelligencePolicy::LocalFirst,
            "local_first must be the default posture"
        );
        assert_eq!(intel.min_confidence, 0.6);
        let ext = intel.external.as_ref().expect("external present");
        assert_eq!(ext.api_key_env, "OPENAI_API_KEY");
        // The external section carries only the ENV NAME — never a key value.
        assert!(!ext.api_key_env.to_lowercase().contains("sk-"));
    }

    #[test]
    fn fabric_intelligence_rejects_artifact_limit_above_hard_cap() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        std::fs::write(
            file.path(),
            format!("{raw}\nfabric_intelligence:\n  enabled: true\n  max_artifact_bytes: 9999999999\n"),
        )
        .unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err().to_string();
        assert!(
            err.contains("hard limit"),
            "oversized artifact ceiling must fail validation, got: {err}"
        );
    }

    #[test]
    fn fabric_intelligence_rejects_external_only_without_endpoint() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        std::fs::write(
            file.path(),
            format!("{raw}\nfabric_intelligence:\n  enabled: true\n  policy: external_only\n"),
        )
        .unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err().to_string();
        assert!(
            err.contains("external_only"),
            "external-only without endpoint must fail at boot, got: {err}"
        );
    }

    #[test]
    fn provision_off_parses() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace(
            "provision_models_on_demand: true",
            "provision_models_on_demand: false",
        );
        std::fs::write(file.path(), bad).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert!(!config.sharing.provision_models_on_demand);
    }

    #[test]
    fn ask_mode_parses() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace("mode: \"auto\"", "mode: \"ask\"");
        std::fs::write(file.path(), bad).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert_eq!(config.sharing.mode, ShareMode::Ask);
    }

    #[test]
    fn zero_max_concurrent_downloads_is_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let bad = raw.replace("max_concurrent_downloads: 2", "max_concurrent_downloads: 0");
        std::fs::write(file.path(), bad).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("max_concurrent_downloads"));
    }

    #[test]
    fn new_runtime_variants_parse() {
        let yaml = r#"
node:
  name: test
  mode: balanced
  data_dir: /tmp
network:
  private_swarm: true
  lan_discovery: true
  dht_enabled: true
  relay_enabled: true
  bootstrap_peers: []
  max_connections: 50
  max_message_bytes: 1048576
storage:
  chunk_size_mb: 16
  hash_algorithm: blake3
  max_cache_gb: 100
  min_free_disk_gb: 20
  verify_full_file_after_assembly: true
  allow_unsigned_models: false
  auto_seed_verified_models: true
resources:
  cpu_max_percent: 80
  reserve_cpu_cores: 2
  memory_max_percent: 80
  reserve_ram_mb: 1024
  gpu_enabled: auto
  gpu_max_vram_percent: 75
  reserve_vram_mb: 1024
  stop_gpu_temperature_celsius: 83
  max_upload_mbps: 20
  max_download_mbps: 80
inference:
  enabled: auto
  runtime: vllm
  bind_address: 127.0.0.1
  api_auth_required: true
  allow_remote_inference: false
  max_concurrent_requests: 4
  max_context_tokens: 4096
  max_generated_tokens: 2048
  request_timeout_seconds: 120
  queue_max_requests: 10
  idle_model_unload_minutes: 10
  api_port: 0
privacy:
  log_prompts: false
  log_outputs: false
  publish_exact_hardware: false
  telemetry_opt_in: false
security:
  trust_mode: private
  require_signed_announcements: true
  require_request_signatures: true
  ban_duration_minutes: 60
  max_invalid_chunks_per_peer: 10
"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert_eq!(config.inference.runtime, InferenceRuntime::Vllm);
    }

    #[test]
    fn engine_and_backend_url_round_trip() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        // Insert opt-in M22 fields inside the inference section, before the
        // nested `generation:` block that terminates it.
        let patched = raw.replace(
            "  generation:",
            "  engine: \"sglang\"\n  backend_url: \"http://192.168.1.50:8000\"\n  generation:",
        );
        std::fs::write(file.path(), patched).unwrap();
        let config = NodeConfig::load(file.path()).unwrap();
        assert_eq!(config.inference.engine.as_deref(), Some("sglang"));
        assert_eq!(
            config.inference.backend_url.as_deref(),
            Some("http://192.168.1.50:8000")
        );
        assert_eq!(config.inference.runtime, InferenceRuntime::LlamaServer);
    }

    #[test]
    fn backend_url_without_scheme_is_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let patched = raw.replace(
            "  generation:",
            "  backend_url: \"192.168.1.50:8000\"\n  generation:",
        );
        std::fs::write(file.path(), patched).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("backend_url"));
    }

    #[test]
    fn unknown_engine_is_rejected_not_silently_remote() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(include_bytes!("../../../configs/node.example.yaml"))
            .unwrap();
        let raw = std::fs::read_to_string(file.path()).unwrap();
        let patched = raw.replace("  generation:", "  engine: baz-engine\n  generation:");
        std::fs::write(file.path(), patched).unwrap();
        let err = NodeConfig::load(file.path()).unwrap_err();
        assert!(err.to_string().contains("engine"));
    }

    #[test]
    fn known_engine_wire_values_recognized() {
        for known in [
            "llama-server",
            "vllm",
            "sglang",
            "ollama",
            "openai-compatible",
        ] {
            assert!(is_known_engine(known), "{known} should be known");
        }
        assert!(!is_known_engine("baz-engine"));
    }
}

mod helpers;
pub use helpers::{
    backend_request_timeout, backend_request_timeout_from, ensure_mode_0600,
    DEFAULT_BACKEND_TIMEOUT_SECS,
};
