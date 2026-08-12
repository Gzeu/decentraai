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
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustMode {
    Private,
    Open,
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

#[derive(Debug, Deserialize)]
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
}

mod helpers;
pub use helpers::ensure_mode_0600;
