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
pub enum NodeMode { Conservative, Balanced, Contributor, Dedicated }

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
    pub gpu_enabled: String,
    pub gpu_max_vram_percent: u8,
    pub reserve_vram_mb: u32,
    pub stop_gpu_temperature_celsius: u8,
    pub max_upload_mbps: u32,
    pub max_download_mbps: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceSection {
    pub enabled: String,
    pub runtime: String,
    pub bind_address: String,
    pub api_auth_required: bool,
    pub allow_remote_inference: bool,
    pub max_concurrent_requests: u16,
    pub max_context_tokens: u32,
    pub max_generated_tokens: u32,
    pub request_timeout_seconds: u32,
    pub queue_max_requests: u16,
    pub idle_model_unload_minutes: u16,
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
    pub trust_mode: String,
    pub require_signed_announcements: bool,
    pub require_request_signatures: bool,
    pub ban_duration_minutes: u32,
    pub max_invalid_chunks_per_peer: u8,
}

impl NodeConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path)?;
        let config: Self = serde_yaml::from_str(&raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.node.name.trim().is_empty() { return Err(ConfigError::Validation("node.name must not be empty".into())); }
        if self.network.max_connections == 0 { return Err(ConfigError::Validation("network.max_connections must be greater than zero".into())); }
        if !(1..=64).contains(&self.storage.chunk_size_mb) { return Err(ConfigError::Validation("storage.chunk_size_mb must be between 1 and 64".into())); }
        if self.storage.hash_algorithm != "blake3" && self.storage.hash_algorithm != "sha256" { return Err(ConfigError::Validation("storage.hash_algorithm must be blake3 or sha256".into())); }
        for (name, value) in [("resources.cpu_max_percent", self.resources.cpu_max_percent), ("resources.memory_max_percent", self.resources.memory_max_percent), ("resources.gpu_max_vram_percent", self.resources.gpu_max_vram_percent)] {
            if value == 0 || value > 100 { return Err(ConfigError::Validation(format!("{name} must be between 1 and 100"))); }
        }
        if self.inference.bind_address != "127.0.0.1" && self.inference.bind_address != "::1" && !self.inference.api_auth_required { return Err(ConfigError::Validation("non-local inference bind_address requires api_auth_required: true".into())); }
        if self.inference.allow_remote_inference && !self.network.private_swarm { return Err(ConfigError::Validation("remote inference requires private_swarm in the initial release".into())); }
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
        file.write_all(include_bytes!("../../../configs/node.example.yaml")).unwrap();
        assert!(NodeConfig::load(file.path()).is_ok());
    }
}
