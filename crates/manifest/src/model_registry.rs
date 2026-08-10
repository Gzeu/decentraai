//! Multi-model registry with version management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// Model with multiple versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub description: String,
    pub model_type: ModelType,
    pub versions: HashMap<String, ModelVersion>,
    pub aliases: HashMap<String, String>,  // alias -> version
    pub default_version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Model type classification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Llm,           // Large Language Model
    Embedding,     // Text embedding
    Vision,        // Image understanding
    Speech,        // Audio/speech
    Multimodal,    // Text + image + audio
}

/// Specific model version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVersion {
    pub version: String,           // e.g., "v1.0.0", "v2.1.0"
    pub model_hash: String,        // SHA256 of model weights
    pub file_size_bytes: u64,
    pub quantization: Quantization,
    pub context_length: u32,       // Max context tokens
    pub parameters: u64,           // Model parameters (e.g., 7B, 70B)
    pub architecture: String,      // e.g., "llama", "mistral"
    pub download_url: Option<String>,
    pub is_stable: bool,
    pub is_canary: bool,
    pub performance_metrics: PerformanceMetrics,
    pub uploaded_at: DateTime<Utc>,
}

/// Quantization level
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quantization {
    Fp16,    // 16-bit float
    Fp32,    // 32-bit float
    Int8,    // 8-bit integer
    Int4,    // 4-bit integer
    Q4_K_M,  // llama.cpp Q4_K_M
    Q4_K_S,  // llama.cpp Q4_K_S
    Q5_K_M,  // llama.cpp Q5_K_M
    Q8_0,    // llama.cpp Q8_0
}

/// Model performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub tokens_per_second: f32,    // Inference speed
    pub time_to_first_token_ms: f32,
    pub memory_usage_gb: f32,
    pub accuracy_score: f32,       // 0.0 - 1.0
    pub benchmark_dataset: String,
}

impl Model {
    pub fn new(name: String, model_type: ModelType) -> Self {
        let now = Utc::now();
        Self {
            name,
            description: String::new(),
            model_type,
            versions: HashMap::new(),
            aliases: HashMap::new(),
            default_version: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a new version
    pub fn add_version(&mut self, version: ModelVersion) {
        let version_str = version.version.clone();
        self.versions.insert(version_str.clone(), version);
        
        if self.default_version.is_empty() {
            self.default_version = version_str.clone();
        }
        
        self.updated_at = Utc::now();
    }

    /// Set version alias (e.g., "stable" -> "v1.2.0")
    pub fn set_alias(&mut self, alias: String, version: String) -> anyhow::Result<()> {
        if !self.versions.contains_key(&version) {
            return Err(anyhow::anyhow!("Version {} not found", version));
        }
        
        self.aliases.insert(alias, version);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Get version by alias or direct version
    pub fn resolve_version(&self, version_or_alias: &str) -> Option<&ModelVersion> {
        // Try direct version first
        if let Some(version) = self.versions.get(version_or_alias) {
            return Some(version);
        }
        
        // Try alias
        if let Some(version) = self.aliases.get(version_or_alias) {
            return self.versions.get(version);
        }
        
        // Default version
        self.versions.get(&self.default_version)
    }

    /// Get latest stable version
    pub fn get_stable(&self) -> Option<&ModelVersion> {
        self.versions
            .values()
            .find(|v| v.is_stable)
            .or_else(|| self.versions.get(&self.default_version))
    }

    /// Get latest canary version
    pub fn get_canary(&self) -> Option<&ModelVersion> {
        self.versions
            .values()
            .find(|v| v.is_canary)
    }

    /// List all versions
    pub fn list_versions(&self) -> Vec<&ModelVersion> {
        self.versions.values().collect()
    }
}

/// Model registry managing all models
pub struct ModelRegistry {
    models: HashMap<String, Model>,
    search_index: HashMap<String, Vec<String>>,  // tag -> model names
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            search_index: HashMap::new(),
        }
    }

    /// Register a new model
    pub fn register_model(&mut self, model: Model) {
        let name = model.name.clone();
        self.models.insert(name.clone(), model);
        
        // Add to search index
        self.search_index
            .entry("all".to_string())
            .or_insert_with(Vec::new)
            .push(name);
    }

    /// Get model by name
    pub fn get_model(&self, name: &str) -> Option<&Model> {
        self.models.get(name)
    }

    /// Get model with resolved version
    pub fn get_model_version(&self, name: &str, version_or_alias: &str) -> Option<&ModelVersion> {
        self.models.get(name)?.resolve_version(version_or_alias)
    }

    /// List all models
    pub fn list_models(&self) -> Vec<&Model> {
        self.models.values().collect()
    }

    /// Search models by tag
    pub fn search(&self, tag: &str) -> Vec<&Model> {
        self.search_index
            .get(tag)
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|name| self.models.get(name))
            .collect()
    }

    /// Get popular models (by usage)
    pub fn get_popular(&self, limit: usize) -> Vec<&Model> {
        // TODO: Track usage and sort
        self.models.values().take(limit).collect()
    }

    /// Add model to search index
    pub fn add_to_index(&mut self, model_name: &str, tag: &str) {
        self.search_index
            .entry(tag.to_string())
            .or_insert_with(Vec::new)
            .push(model_name.to_string());
    }
}

/// Model router for request dispatch
pub struct ModelRouter {
    registry: ModelRegistry,
    default_model: Option<String>,
}

impl ModelRouter {
    pub fn new(registry: ModelRegistry) -> Self {
        Self {
            registry,
            default_model: None,
        }
    }

    /// Set default model
    pub fn set_default(&mut self, model_name: &str) {
        self.default_model = Some(model_name.to_string());
    }

    /// Route request to appropriate model
    pub fn route(&self, model_name: Option<&str>, version: &str) -> Option<&ModelVersion> {
        let name = model_name.or(self.default_model.as_deref())?;
        self.registry.get_model_version(name, version)
    }

    /// Get registry
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_versioning() {
        let mut model = Model::new("llama-3".to_string(), ModelType::Llm);
        
        let v1 = ModelVersion {
            version: "v1.0.0".to_string(),
            model_hash: "abc123".to_string(),
            file_size_bytes: 4_000_000_000,
            quantization: Quantization::Q4_K_M,
            context_length: 8192,
            parameters: 8_000_000_000,
            architecture: "llama".to_string(),
            download_url: None,
            is_stable: true,
            is_canary: false,
            performance_metrics: PerformanceMetrics {
                tokens_per_second: 50.0,
                time_to_first_token_ms: 100.0,
                memory_usage_gb: 8.0,
                accuracy_score: 0.95,
                benchmark_dataset: "mmlu".to_string(),
            },
            uploaded_at: Utc::now(),
        };
        
        model.add_version(v1);
        
        assert_eq!(model.default_version, "v1.0.0");
        assert!(model.get_stable().is_some());
    }

    #[test]
    fn test_version_aliases() {
        let mut model = Model::new("mistral".to_string(), ModelType::Llm);
        
        // Add versions
        model.add_version(ModelVersion {
            version: "v1.0.0".to_string(),
            model_hash: "abc".to_string(),
            file_size_bytes: 4_000_000_000,
            quantization: Quantization::Q4_K_M,
            context_length: 8192,
            parameters: 7_000_000_000,
            architecture: "mistral".to_string(),
            download_url: None,
            is_stable: true,
            is_canary: false,
            performance_metrics: PerformanceMetrics {
                tokens_per_second: 60.0,
                time_to_first_token_ms: 80.0,
                memory_usage_gb: 7.0,
                accuracy_score: 0.92,
                benchmark_dataset: "mmlu".to_string(),
            },
            uploaded_at: Utc::now(),
        });
        
        model.add_version(ModelVersion {
            version: "v2.0.0".to_string(),
            model_hash: "xyz".to_string(),
            file_size_bytes: 14_000_000_000,
            quantization: Quantization::Q4_K_M,
            context_length: 32768,
            parameters: 14_000_000_000,
            architecture: "mistral".to_string(),
            download_url: None,
            is_stable: false,
            is_canary: true,
            performance_metrics: PerformanceMetrics {
                tokens_per_second: 45.0,
                time_to_first_token_ms: 120.0,
                memory_usage_gb: 14.0,
                accuracy_score: 0.96,
                benchmark_dataset: "mmlu".to_string(),
            },
            uploaded_at: Utc::now(),
        });
        
        // Set aliases
        model.set_alias("stable".to_string(), "v1.0.0".to_string()).unwrap();
        model.set_alias("canary".to_string(), "v2.0.0".to_string()).unwrap();
        
        // Resolve aliases
        assert!(model.resolve_version("stable").is_some());
        assert!(model.resolve_version("canary").is_some());
        assert_eq!(model.resolve_version("stable").unwrap().version, "v1.0.0");
        assert_eq!(model.resolve_version("canary").unwrap().version, "v2.0.0");
    }
}
