use serde::Deserialize;
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSection {
    pub name: String,
    pub mode: NodeMode,
    pub data_dir: String,
    /// Explicit GGUF model file name to serve (e.g.
    /// `Mistral-7B-Instruct-v0.3-Q4_K_M.gguf`). When set, the node serves this
    /// model instead of auto-detecting the first one in the models dir; a
    /// missing file is a hard error at startup. Optional (absent = auto-detect).
    #[serde(default)]
    pub model: Option<String>,
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
}

impl Default for SharingSection {
    fn default() -> Self {
        Self {
            mode: ShareMode::Auto,
            max_concurrent_downloads: 2,
            provision_models_on_demand: true,
        }
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
pub use helpers::ensure_mode_0600;
