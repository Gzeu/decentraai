//! ProviderManager — lifecycle + persistence for the provider control plane.
//!
//! Owns the in-memory provider registry, the credential store (Arc-shared
//! with the adapter boundary), per-provider/per-model health states and the
//! budget tracker. Persists provider records (NEVER secrets) to
//! `<data_dir>/db/providers.json` with atomic tmp+rename.
//!
//! CRITICAL SECURITY INVARIANT: `providers.json` contains only `credential_ref`
//! handles (e.g. `dcrypt_...`). The plaintext API keys live ONLY in the
//! in-memory `CredentialStore` and are re-entered by the operator after a
//! restart (an encrypted backing store is a future milestone).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::adapter::{ModelAdapter, OpenAICompatibleProvider, ProviderAdapter};
use crate::credential_store::CredentialStore;
use crate::health::{HealthConfig, ModelHealthState, ProviderHealthState};
use crate::{
    CircuitState, ConnectedModel, ModelHealth, ModelSource, Provider, ProviderError,
    ProviderErrorClass, ProviderHealth, ProviderKind, ProviderSummary, SharingPolicy,
};

/// Filesystem persistence structure (secrets excluded by construction).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    pub version: u32,
    pub providers: Vec<Provider>,
}

/// One entry in the unified model catalog (local + fabric + provider).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Stable model key (provider-backed: symbolic hash; local: file name;
    /// fabric: model hash).
    pub id: String,
    /// Display name.
    pub name: String,
    pub source: ModelSource,
    /// Provider id when source == Provider.
    pub provider_id: Option<String>,
    /// Upstream model id when source == Provider.
    pub upstream_model: Option<String>,
    /// Symbolic hash usable with the fabric planner.
    pub model_hash: Option<String>,
    /// Whether the model is enabled for routing.
    pub enabled: bool,
    /// Health when known.
    pub health: Option<String>,
    /// Latency (ms) when measured.
    pub latency_ms: Option<u64>,
    /// Capabilities when known (snake_case).
    pub capabilities: Vec<String>,
    /// Estimated/known context window.
    pub context_window: Option<u32>,
    /// Human share state ("shared" / "private").
    pub share_state: String,
}

/// Runtime cost/rate tracking, kept in-memory (never persisted).
#[derive(Debug, Clone, Default)]
pub struct ModelUsageTracker {
    /// (provider_id, model_id) -> usage counters.
    pub requests_last_minute: HashMap<(String, String), u32>,
    pub tokens_today: HashMap<(String, String), u64>,
    pub spend_today: HashMap<(String, String), f64>,
    pub day_key: String,
}

/// The provider control plane manager.
pub struct ProviderManager {
    data_dir: PathBuf,
    providers: Vec<Provider>,
    credential_store: Arc<Mutex<CredentialStore>>,
    provider_health: HashMap<String, ProviderHealthState>,
    model_health: HashMap<String, ModelHealthState>,
    health_config: HealthConfig,
    pub usage: ModelUsageTracker,
}

impl ProviderManager {
    /// Load from `db/providers.json` if present; empty manager otherwise.
    /// The CredentialStore starts empty — providers with credentials must be
    /// re-authenticated after restart (documented security tradeoff).
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let path = data_dir.join("db/providers.json");
        let providers = load_persisted(&path).unwrap_or_default();
        Self {
            data_dir,
            providers,
            credential_store: Arc::new(Mutex::new(CredentialStore::new())),
            provider_health: HashMap::new(),
            model_health: HashMap::new(),
            health_config: HealthConfig::default(),
            usage: ModelUsageTracker::default(),
        }
    }

    pub fn credential_store(&self) -> Arc<Mutex<CredentialStore>> {
        self.credential_store.clone()
    }

    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub fn provider(&self, provider_id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.provider_id == provider_id)
    }

    pub fn provider_mut(&mut self, provider_id: &str) -> Option<&mut Provider> {
        self.providers
            .iter_mut()
            .find(|p| p.provider_id == provider_id)
    }

    /// Check the circuit state for a provider.
    pub fn provider_circuit(&self, provider_id: &str) -> CircuitState {
        self.provider_health
            .get(provider_id)
            .map(|h| h.circuit)
            .unwrap_or(CircuitState::Healthy)
    }

    /// Check the circuit state for a model.
    pub fn model_circuit(&self, provider_id: &str, model_id: &str) -> CircuitState {
        self.model_health
            .get(&model_key(provider_id, model_id))
            .map(|h| h.circuit)
            .unwrap_or(CircuitState::Healthy)
    }

    // ── Provider CRUD ────────────────────────────────────────────────

    /// Add a provider. The API key is stored in the CredentialStore; only the
    /// key_id handle lands in the persisted record.
    pub fn add_provider(
        &mut self,
        kind: ProviderKind,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<String, ProviderError> {
        let display_name = display_name.into();
        let base_url = base_url.into();
        if base_url.trim().is_empty() {
            return Err(ProviderError::Provider("base_url must not be empty".into()));
        }
        let key_id = {
            let mut creds = self
                .credential_store
                .lock()
                .map_err(|_| ProviderError::Credential("credential store poisoned".into()))?;
            creds.add(api_key)
        };
        let provider_id = format!("prov_{}", uuid_fragment());
        let provider = Provider::new(&provider_id, kind, display_name, &base_url, &key_id);
        self.provider_health.insert(
            provider_id.clone(),
            ProviderHealthState::new(&self.health_config),
        );
        self.providers.push(provider);
        self.save()?;
        Ok(provider_id)
    }

    /// Remove a provider (and its models + credential handle). The plaintext
    /// secret dies with the in-memory CredentialStore entry.
    pub fn remove_provider(&mut self, provider_id: &str) -> Result<(), ProviderError> {
        // Clone the fields we need first, to avoid mutating while borrowed.
        let (cred_ref, model_ids) = {
            let provider = self
                .providers
                .iter()
                .find(|p| p.provider_id == provider_id)
                .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?;
            (
                provider.credential_ref.clone(),
                provider
                    .models
                    .iter()
                    .map(|m| m.model_id.clone())
                    .collect::<Vec<_>>(),
            )
        };
        {
            let mut creds = self
                .credential_store
                .lock()
                .map_err(|_| ProviderError::Credential("credential store poisoned".into()))?;
            creds.remove(&cred_ref);
        }
        self.providers.retain(|p| p.provider_id != provider_id);
        self.provider_health.remove(provider_id);
        for m in model_ids {
            self.model_health.remove(&model_key(provider_id, &m));
        }
        self.save()?;
        Ok(())
    }

    /// Public summaries (masked fingerprints, NEVER secrets).
    pub fn list_provider_summaries(&self) -> Vec<ProviderSummary> {
        let creds = self
            .credential_store
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        self.providers
            .iter()
            .map(|p| p.summary(creds.fingerprint(&p.credential_ref)))
            .collect()
    }

    // ── Connection / discovery ───────────────────────────────────────

    /// Test the provider credential (via providers adapter). Updates health
    /// state on success/failure. Returns (latency_ms, model_count).
    pub async fn test_connection(
        &mut self,
        provider_id: &str,
    ) -> Result<(u64, usize), crate::adapter::ProviderConnError> {
        let Some(provider) = self.provider(provider_id).cloned() else {
            return Err(crate::adapter::ProviderConnError::Transport(
                "provider not found".into(),
            ));
        };
        let api_key = self
            .credential_store
            .lock()
            .map_err(|_| crate::adapter::ProviderConnError::Transport("credential lock".into()))?
            .get_secret(&provider.credential_ref)
            .ok_or_else(|| {
                crate::adapter::ProviderConnError::InvalidCredentials(format!(
                    "credential {0} not found",
                    provider.credential_ref
                ))
            })?
            .to_string();

        let adapter = OpenAICompatibleProvider::new(provider.kind);
        let result = adapter.test_connection(&provider.base_url, &api_key).await;

        let now = unix_ms();
        let health = self
            .provider_health
            .entry(provider_id.to_string())
            .or_insert_with(|| ProviderHealthState::new(&self.health_config));
        match &result {
            Ok((latency_ms, _)) => {
                health.record_success(now, &self.health_config);
                if let Some(p) = self.provider_mut(provider_id) {
                    p.health = ProviderHealth::Healthy;
                    p.last_health_check_at_ms = Some(now);
                    p.last_latency_ms = Some(*latency_ms);
                    p.last_success_at_ms = Some(now);
                    p.failure_count = 0;
                    p.last_error_class = None;
                }
            }
            Err(e) => {
                health.record_failure(now, &self.health_config);
                if let Some(p) = self.provider_mut(provider_id) {
                    p.failure_count += 1;
                    p.last_error_class = Some(classify_conn_error(e));
                    p.last_health_check_at_ms = Some(now);
                }
            }
        }
        self.save().ok();
        result
    }

    /// Discover available models from the provider.
    pub async fn discover_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::adapter::ProviderModelInfo>, crate::adapter::ProviderConnError> {
        let Some(provider) = self.provider(provider_id) else {
            return Err(crate::adapter::ProviderConnError::Transport(
                "provider not found".into(),
            ));
        };
        let api_key = self
            .credential_store
            .lock()
            .map_err(|_| crate::adapter::ProviderConnError::Transport("credential lock".into()))?
            .get_secret(&provider.credential_ref)
            .ok_or_else(|| {
                crate::adapter::ProviderConnError::InvalidCredentials("credential not found".into())
            })?
            .to_string();
        let adapter = OpenAICompatibleProvider::new(provider.kind);
        adapter.discover_models(&provider.base_url, &api_key).await
    }

    /// Selective model connection from discovery results.
    pub fn connect_model(
        &mut self,
        provider_id: &str,
        upstream_model: &str,
        display_name: Option<String>,
    ) -> Result<String, ProviderError> {
        // Check duplicate first (immutable borrow), then push (mutable).
        if self
            .provider(provider_id)
            .map(|p| p.models.iter().any(|m| m.upstream_model == upstream_model))
            .unwrap_or(false)
        {
            return Err(ProviderError::Provider(format!(
                "model {upstream_model} already connected"
            )));
        }
        let model_id = format!("mod_{}", uuid_fragment());
        let model = ConnectedModel {
            model_id: model_id.clone(),
            provider_id: provider_id.to_string(),
            upstream_model: upstream_model.to_string(),
            display_name: display_name.unwrap_or_else(|| upstream_model.to_string()),
            capabilities: Vec::new(),
            capability_provenance: crate::CapabilityProvenance::Unknown,
            context_window: None,
            pricing: None,
            enabled: true,
            sharing: SharingPolicy::default(),
            budget: Default::default(),
            health: ModelHealth::Unknown,
            last_latency_ms: None,
            last_success_at_ms: None,
            last_failure_at_ms: None,
            consecutive_failures: 0,
            circuit: CircuitState::Healthy,
            usage: Default::default(),
        };
        let key = model_key(provider_id, &model_id);
        self.model_health.insert(key, ModelHealthState::new());
        if let Some(provider) = self.provider_mut(provider_id) {
            provider.models.push(model);
        }
        self.save()?;
        Ok(model_id)
    }

    pub fn delete_model(&mut self, provider_id: &str, model_id: &str) -> Result<(), ProviderError> {
        let Some(provider) = self.provider_mut(provider_id) else {
            return Err(ProviderError::NotFound(provider_id.into()));
        };
        let before = provider.models.len();
        provider.models.retain(|m| m.model_id != model_id);
        if provider.models.len() == before {
            return Err(ProviderError::ModelNotFound(
                model_id.into(),
                provider_id.into(),
            ));
        }
        self.model_health.remove(&model_key(provider_id, model_id));
        self.save()?;
        Ok(())
    }

    pub fn set_model_enabled(
        &mut self,
        provider_id: &str,
        model_id: &str,
        enabled: bool,
    ) -> Result<(), ProviderError> {
        let Some(provider) = self.provider_mut(provider_id) else {
            return Err(ProviderError::NotFound(provider_id.into()));
        };
        let Some(model) = provider.model_mut(model_id) else {
            return Err(ProviderError::ModelNotFound(
                model_id.into(),
                provider_id.into(),
            ));
        };
        model.enabled = enabled;
        self.save()?;
        Ok(())
    }

    pub fn set_sharing(
        &mut self,
        provider_id: &str,
        model_id: &str,
        policy: SharingPolicy,
    ) -> Result<(), ProviderError> {
        let Some(provider) = self.provider_mut(provider_id) else {
            return Err(ProviderError::NotFound(provider_id.into()));
        };
        let Some(model) = provider.model_mut(model_id) else {
            return Err(ProviderError::ModelNotFound(
                model_id.into(),
                provider_id.into(),
            ));
        };
        model.sharing = policy;
        self.save()?;
        Ok(())
    }

    pub fn set_capabilities(
        &mut self,
        provider_id: &str,
        model_id: &str,
        capabilities: Vec<crate::CapabilityKind>,
        provenance: crate::CapabilityProvenance,
    ) -> Result<(), ProviderError> {
        let Some(provider) = self.provider_mut(provider_id) else {
            return Err(ProviderError::NotFound(provider_id.into()));
        };
        let Some(model) = provider.model_mut(model_id) else {
            return Err(ProviderError::ModelNotFound(
                model_id.into(),
                provider_id.into(),
            ));
        };
        model.capabilities = capabilities;
        model.capability_provenance = provenance;
        self.save()?;
        Ok(())
    }

    /// Look up a connected model by its symbolic hash (used by the router).
    pub fn model_by_symbolic_hash(
        &self,
        symbolic_hash: &str,
    ) -> Option<(&Provider, &ConnectedModel)> {
        for provider in &self.providers {
            for model in &provider.models {
                if model.symbolic_hash() == symbolic_hash {
                    return Some((provider, model));
                }
            }
        }
        None
    }

    pub fn model_by_id(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Option<(&Provider, &ConnectedModel)> {
        let provider = self.provider(provider_id)?;
        let model = provider.model(model_id)?;
        Some((provider, model))
    }

    /// Whether requests may route to this provider (circuit + enabled).
    pub fn allows_provider_requests(&self, provider_id: &str) -> bool {
        let Some(p) = self.provider(provider_id) else {
            return false;
        };
        if !p.enabled {
            return false;
        }
        self.provider_health
            .get(provider_id)
            .map(|h| h.allows_requests())
            .unwrap_or(true)
    }

    /// Whether requests may route to this model (provider + model gates).
    pub fn allows_model_requests(&self, provider_id: &str, model_id: &str) -> bool {
        if !self.allows_provider_requests(provider_id) {
            return false;
        }
        let Some((_, model)) = self.model_by_id(provider_id, model_id) else {
            return false;
        };
        if !model.enabled {
            return false;
        }
        self.model_health
            .get(&model_key(provider_id, model_id))
            .map(|h| h.allows_requests())
            .unwrap_or(true)
    }

    /// When an OPEN breaker will transition to HALF_OPEN (unix ms). `None`
    /// when not open.
    pub fn next_allowed_at_ms(&self, provider_id: &str) -> Option<u64> {
        self.provider_health
            .get(provider_id)
            .and_then(|h| h.half_open_at_ms)
            .filter(|_| matches!(self.provider_circuit(provider_id), CircuitState::Open))
    }

    // ── Health probing ───────────────────────────────────────────────

    /// Run a live health probe for a provider (cheap: /v1/models).
    pub async fn health_check_provider(
        &mut self,
        provider_id: &str,
    ) -> Result<(), crate::adapter::ProviderConnError> {
        let _ = self.test_connection(provider_id).await?;
        Ok(())
    }

    /// Run a live health probe for a connected model (1-token chat probe).
    pub async fn health_check_model(
        &mut self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<(), crate::adapter::ProviderInferError> {
        let (base_url, upstream, cred_ref) = {
            let Some((provider, model)) = self.model_by_id(provider_id, model_id) else {
                return Err(crate::adapter::ProviderInferError::CredentialNotFound(
                    "model not found".into(),
                ));
            };
            (
                provider.base_url.clone(),
                model.upstream_model.clone(),
                provider.credential_ref.clone(),
            )
        };
        let adapter =
            ModelAdapter::new(base_url, upstream, self.credential_store.clone(), cred_ref);
        let start = std::time::Instant::now();
        let result = adapter.health().await;
        let latency = start.elapsed().as_millis() as u64;
        let now = unix_ms();

        let key = model_key(provider_id, model_id);
        match &result {
            Ok(()) => {
                if let Some(h) = self.model_health.get_mut(&key) {
                    h.record_success(now, Some(latency));
                    h.health = crate::ModelHealth::Healthy;
                }
                if let Some(p) = self.provider_mut(provider_id) {
                    if let Some(m) = p.model_mut(model_id) {
                        m.health = ModelHealth::Healthy;
                        m.last_latency_ms = Some(latency);
                        m.last_success_at_ms = Some(now);
                        m.consecutive_failures = 0;
                    }
                }
            }
            Err(e) => {
                if let Some(h) = self.model_health.get_mut(&key) {
                    h.record_failure(now);
                }
                if let Some(p) = self.provider_mut(provider_id) {
                    if let Some(m) = p.model_mut(model_id) {
                        m.health = ModelHealth::Offline;
                        m.last_failure_at_ms = Some(now);
                        m.consecutive_failures += 1;
                    }
                }
                // Credential failures must also degrade the provider.
                if matches!(
                    e,
                    crate::adapter::ProviderInferError::ProviderError(ProviderErrorClass::Auth, _)
                ) {
                    if let Some(ph) = self.provider_health.get_mut(provider_id) {
                        ph.record_failure(now, &self.health_config);
                    }
                }
            }
        }
        self.save().ok();
        result
    }

    // ── Persistence ──────────────────────────────────────────────────

    pub fn save(&self) -> Result<(), ProviderError> {
        let db_dir = self.data_dir.join("db");
        fs::create_dir_all(&db_dir)
            .map_err(|e| ProviderError::Credential(format!("mkdir db: {e}")))?;
        let path = db_dir.join("providers.json");
        let state = PersistedState {
            version: 1,
            providers: self.providers.clone(),
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| ProviderError::Credential(format!("serialize: {e}")))?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    // ── Unified catalog ──────────────────────────────────────────────

    /// Build the unified catalog snapshot (local + fabric + provider).
    /// `local_models` and `fabric_models` are supplied by the runtime so this
    /// crate stays I/O-free.
    pub fn catalog(
        &self,
        local_models: Vec<LocalModelView>,
        fabric_models: Vec<FabricModelView>,
    ) -> Vec<CatalogEntry> {
        let mut entries = Vec::new();

        for m in local_models {
            entries.push(CatalogEntry {
                id: m.name.clone(),
                name: m.name.clone(),
                source: ModelSource::Local,
                provider_id: None,
                upstream_model: None,
                model_hash: m.hash.clone(),
                enabled: true,
                health: Some("healthy".into()),
                latency_ms: None,
                capabilities: m.capabilities,
                context_window: m.context_window,
                share_state: "local".into(),
            });
        }

        for m in fabric_models {
            entries.push(CatalogEntry {
                id: format!("fabric:{}", m.hash),
                name: m.file_name,
                source: ModelSource::Fabric,
                provider_id: None,
                upstream_model: None,
                model_hash: Some(m.hash),
                enabled: true,
                health: m.health,
                latency_ms: m.latency_ms,
                capabilities: m.capabilities,
                context_window: m.context_window,
                share_state: "private".into(),
            });
        }

        for provider in &self.providers {
            for model in &provider.models {
                let shared = model.sharing.enabled;
                entries.push(CatalogEntry {
                    id: model.model_id.clone(),
                    name: if model.display_name.is_empty() {
                        model.upstream_model.clone()
                    } else {
                        model.display_name.clone()
                    },
                    source: ModelSource::Provider,
                    provider_id: Some(provider.provider_id.clone()),
                    upstream_model: Some(model.upstream_model.clone()),
                    model_hash: Some(model.symbolic_hash()),
                    enabled: model.enabled,
                    health: Some(match model.health {
                        ModelHealth::Healthy => "healthy".into(),
                        ModelHealth::Degraded => "degraded".into(),
                        ModelHealth::Offline => "offline".into(),
                        ModelHealth::Disabled => "disabled".into(),
                        ModelHealth::Unknown => "unknown".into(),
                    }),
                    latency_ms: model.last_latency_ms,
                    capabilities: model
                        .capabilities
                        .iter()
                        .map(|c| c.label().to_string())
                        .collect(),
                    context_window: model.context_window,
                    share_state: if shared {
                        "shared".into()
                    } else {
                        "private".into()
                    },
                });
            }
        }

        entries
    }

    /// Get a ModelAdapter for a connected model (credential resolved at call
    /// time). Used by the runtime to execute provider-backed inference.
    pub fn model_adapter(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ModelAdapter, ProviderError> {
        let Some((provider, model)) = self.model_by_id(provider_id, model_id) else {
            return Err(ProviderError::ModelNotFound(
                model_id.into(),
                provider_id.into(),
            ));
        };
        Ok(ModelAdapter::new(
            provider.base_url.clone(),
            model.upstream_model.clone(),
            self.credential_store.clone(),
            provider.credential_ref.clone(),
        ))
    }

    /// Public list of shared provider models: the ONLY info a remote peer
    /// may ever see (handles + metadata, never credentials).
    pub fn shared_models_handles(&self) -> Vec<SharedModelHandle> {
        let mut out = Vec::new();
        for provider in &self.providers {
            if !provider.enabled {
                continue;
            }
            for model in provider.shared_models() {
                out.push(SharedModelHandle {
                    provider_id: provider.provider_id.clone(),
                    model_id: model.model_id.clone(),
                    upstream_model: model.upstream_model.clone(),
                    provider_kind: provider.kind.as_str().to_string(),
                    model_hash: model.symbolic_hash(),
                    capabilities: model
                        .capabilities
                        .iter()
                        .map(|c| c.label().to_string())
                        .collect(),
                    owner_peer: String::new(), // filled by the runtime
                    credentialed: true,
                });
            }
        }
        out
    }
}

/// A view of a local model for the catalog (supplied by the runtime).
#[derive(Debug, Clone)]
pub struct LocalModelView {
    pub name: String,
    pub hash: Option<String>,
    pub capabilities: Vec<String>,
    pub context_window: Option<u32>,
}

/// A view of a fabric model for the catalog (supplied by the runtime).
#[derive(Debug, Clone)]
pub struct FabricModelView {
    pub hash: String,
    pub file_name: String,
    pub health: Option<String>,
    pub latency_ms: Option<u64>,
    pub capabilities: Vec<String>,
    pub context_window: Option<u32>,
}

/// The safe handle set a remote peer may receive for shared provider models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedModelHandle {
    pub provider_id: String,
    pub model_id: String,
    pub upstream_model: String,
    pub provider_kind: String,
    pub model_hash: String,
    pub capabilities: Vec<String>,
    pub owner_peer: String,
    pub credentialed: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────

fn model_key(provider_id: &str, model_id: &str) -> String {
    format!("{provider_id}:{model_id}")
}

fn classify_conn_error(e: &crate::adapter::ProviderConnError) -> ProviderErrorClass {
    match e {
        crate::adapter::ProviderConnError::InvalidCredentials(_) => ProviderErrorClass::Auth,
        crate::adapter::ProviderConnError::Network(_) => ProviderErrorClass::Timeout,
        crate::adapter::ProviderConnError::Transport(_) => ProviderErrorClass::Timeout,
        crate::adapter::ProviderConnError::Timeout(_) => ProviderErrorClass::Timeout,
        crate::adapter::ProviderConnError::Protocol(_) => ProviderErrorClass::Protocol,
        crate::adapter::ProviderConnError::HttpError { error_class, .. } => error_class.clone(),
    }
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn uuid_fragment() -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"decentraai-provider-id\0");
    hasher.update(unix_ms().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn load_persisted(path: &Path) -> Option<Vec<Provider>> {
    let raw = fs::read_to_string(path).ok()?;
    let state: PersistedState = serde_json::from_str(&raw).ok()?;
    Some(state.providers)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProviderError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| ProviderError::Credential(format!("write tmp: {e}")))?;
    // Best-effort 0600 on the sensitive file before rename (no secrets in
    // this file today, but the pattern is right for future encrypted stores).
    let _ = set_0600(&tmp);
    fs::rename(&tmp, path).map_err(|e| ProviderError::Credential(format!("rename: {e}")))?;
    Ok(())
}

#[cfg(unix)]
fn set_0600(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_0600(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CapabilityKind, CapabilityProvenance};

    fn test_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("dca-providers-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn empty_manager_loads_and_saves() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let id = mgr
            .add_provider(
                ProviderKind::OpenRouter,
                "OR",
                "https://openrouter.ai/api/v1",
                "sk-test-123",
            )
            .unwrap();
        assert!(id.starts_with("prov_"));
        assert_eq!(mgr.providers().len(), 1);
        // Reload from disk: provider record persists (with its credential_ref
        // handle), but the raw secret is NOT in the file.
        drop(mgr);
        let reloaded = ProviderManager::new(&dir);
        assert_eq!(reloaded.providers().len(), 1);
        // The credential_ref handle survives (it points at the CredentialStore).
        assert!(!reloaded.providers()[0].credential_ref.is_empty());
        // The raw API key must never be inside the persisted file.
        let raw = fs::read_to_string(dir.join("db/providers.json")).unwrap();
        assert!(!raw.contains("sk-test-123"), "secret must not persist");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_record_never_contains_secret() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let _ = mgr
            .add_provider(
                ProviderKind::Groq,
                "Groq",
                "https://api.groq.com/openai/v1",
                "sk-super-secret-value",
            )
            .unwrap();
        let json = fs::read_to_string(dir.join("db/providers.json")).unwrap();
        assert!(!json.contains("sk-super-secret"));
        assert!(!json.contains("super-secret"));
        assert!(json.contains("credential_ref"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn connect_and_enable_model() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let pid = mgr
            .add_provider(
                ProviderKind::OpenAi,
                "OpenAI",
                "https://api.openai.com/v1",
                "sk-x",
            )
            .unwrap();
        let mid = mgr
            .connect_model(&pid, "gpt-4o", Some("GPT-4o".into()))
            .unwrap();
        assert!(mid.starts_with("mod_"));

        // Model enabled by default.
        assert!(mgr.allows_model_requests(&pid, &mid));

        // Disable → routing refuses.
        mgr.set_model_enabled(&pid, &mid, false).unwrap();
        assert!(!mgr.allows_model_requests(&pid, &mid));

        // Re-enable.
        mgr.set_model_enabled(&pid, &mid, true).unwrap();
        assert!(mgr.allows_model_requests(&pid, &mid));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sharing_policy_updates_and_revokes() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let pid = mgr
            .add_provider(
                ProviderKind::Together,
                "Together",
                "https://api.together.xyz/v1",
                "sk-x",
            )
            .unwrap();
        let mid = mgr
            .connect_model(&pid, "meta-llama/Llama-3.3-70B-Instruct-Turbo", None)
            .unwrap();
        // Default: no shared models.
        assert_eq!(mgr.shared_models_handles().len(), 0);
        // Enable sharing → handle appears.
        let policy = SharingPolicy {
            enabled: true,
            ..SharingPolicy::default()
        };
        mgr.set_sharing(&pid, &mid, policy).unwrap();
        let handles = mgr.shared_models_handles();
        assert_eq!(handles.len(), 1);
        assert!(handles[0].model_hash.starts_with("prov-"));
        // Revoke → handle gone.
        mgr.set_sharing(&pid, &mid, SharingPolicy::default())
            .unwrap();
        assert_eq!(mgr.shared_models_handles().len(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn symbolic_hash_lookup_round_trips() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let pid = mgr
            .add_provider(
                ProviderKind::OpenRouter,
                "OR",
                "https://openrouter.ai/api/v1",
                "sk-x",
            )
            .unwrap();
        let mid = mgr.connect_model(&pid, "qwen/qwen3-coder", None).unwrap();
        let (p, m) = mgr.model_by_id(&pid, &mid).unwrap();
        let hash = m.symbolic_hash();
        let (found_p, found_m) = mgr.model_by_symbolic_hash(&hash).unwrap();
        assert_eq!(found_p.provider_id, p.provider_id);
        assert_eq!(found_m.model_id, m.model_id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_capabilities_recorded() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let pid = mgr
            .add_provider(
                ProviderKind::Fireworks,
                "Fireworks",
                "https://api.fireworks.ai/inference/v1",
                "sk-x",
            )
            .unwrap();
        let mid = mgr
            .connect_model(&pid, "accounts/fireworks/models/qwen3-32b", None)
            .unwrap();
        mgr.set_capabilities(
            &pid,
            &mid,
            vec![CapabilityKind::Coding, CapabilityKind::ToolCalling],
            CapabilityProvenance::Verified,
        )
        .unwrap();
        let (_, m) = mgr.model_by_id(&pid, &mid).unwrap();
        assert_eq!(m.capabilities.len(), 2);
        assert_eq!(m.capability_provenance, CapabilityProvenance::Verified);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn catalog_merges_all_sources() {
        let dir = test_dir();
        let mut mgr = ProviderManager::new(&dir);
        let pid = mgr
            .add_provider(
                ProviderKind::Groq,
                "Groq",
                "https://api.groq.com/openai/v1",
                "sk-x",
            )
            .unwrap();
        mgr.connect_model(&pid, "llama-3.3-70b-versatile", None)
            .unwrap();

        let local = vec![LocalModelView {
            name: "Llama-3.2-1B.gguf".into(),
            hash: Some("abc".into()),
            capabilities: vec!["chat".into()],
            context_window: Some(4096),
        }];
        let fabric = vec![FabricModelView {
            hash: "def".into(),
            file_name: "Qwen2.5-Coder-7B.gguf".into(),
            health: Some("healthy".into()),
            latency_ms: Some(120),
            capabilities: vec!["coding".into()],
            context_window: Some(16384),
        }];
        let entries = mgr.catalog(local, fabric);
        assert_eq!(entries.len(), 3, "one per source");
        assert!(entries.iter().any(|e| e.source == ModelSource::Local));
        assert!(entries.iter().any(|e| e.source == ModelSource::Fabric));
        assert!(entries.iter().any(|e| e.source == ModelSource::Provider));
        let _ = fs::remove_dir_all(&dir);
    }
}
