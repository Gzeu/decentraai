//! Provider control plane (Model Fabric).
//!
//! DecentraAI unifies three model sources under one catalog:
//!
//! - **local** — GGUF models verified and served on this node;
//! - **fabric** — models served by trusted remote workers (existing P2P);
//! - **provider** — externally hosted OpenAI-compatible providers
//!   (OpenRouter, OpenAI, Groq, Together, Fireworks, generic).
//!
//! This crate holds the *pure domain* of the provider half: typed records,
//! credential *references* (never secrets), health + circuit-breaker state,
//! sharing policy and cost/rate limits. It deliberately contains NO I/O: the
//! provider adapter is a thin HTTP layer over the existing backend-neutral
//! `decentraai-inference-adapter`, and the runtime owns persistence.

use decentraai_hub::capability::CapabilityKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which provider API family a connected provider speaks.
///
/// `GenericOpenAiCompatible` is the foundation: OpenRouter/OpenAI/Groq/
/// Together/Fireworks all expose the same `/v1/chat/completions` surface, so
/// provider-specific differences are limited to authentication, model
/// discovery and metadata. Native kinds are added only when the generic
/// adapter cannot represent the API correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenRouter,
    OpenAi,
    Groq,
    Together,
    Fireworks,
    GenericOpenAiCompatible,
}

impl ProviderKind {
    /// Wire/display kind. Lowercase snake_case like the rest of the repo.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenAi => "openai",
            Self::Groq => "groq",
            Self::Together => "together",
            Self::Fireworks => "fireworks",
            Self::GenericOpenAiCompatible => "generic_openai_compatible",
        }
    }

    /// Human display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::OpenAi => "OpenAI",
            Self::Groq => "Groq",
            Self::Together => "Together",
            Self::Fireworks => "Fireworks",
            Self::GenericOpenAiCompatible => "Generic OpenAI-compatible",
        }
    }

    /// Default base URL for well-known kinds. Generic has no default (the
    /// operator supplies one). Returns `None` for kinds without a canonical
    /// endpoint so the wizard can prompt for it.
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Groq => Some("https://api.groq.com/openai/v1"),
            Self::Together => Some("https://api.together.xyz/v1"),
            Self::Fireworks => Some("https://api.fireworks.ai/inference/v1"),
            Self::GenericOpenAiCompatible => None,
        }
    }

    /// Parse from a wire string; unknown values return `None` rather than
    /// inventing a provider.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "openrouter" => Some(Self::OpenRouter),
            "openai" => Some(Self::OpenAi),
            "groq" => Some(Self::Groq),
            "together" => Some(Self::Together),
            "fireworks" => Some(Self::Fireworks),
            "generic_openai_compatible" | "generic" | "openai-compatible" => {
                Some(Self::GenericOpenAiCompatible)
            }
            _ => None,
        }
    }
}

/// The source of a model in the unified catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Local,
    Fabric,
    Provider,
}

impl ModelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Fabric => "fabric",
            Self::Provider => "provider",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Fabric => "Fabric",
            Self::Provider => "Provider",
        }
    }
}

/// A stable handle an agent/API layer can reference instead of an API key.
///
/// Agents and untrusted peers receive a [`ModelHandle`] (or its string form)
/// when a provider-backed model is shared; the handle resolves through the
/// owning node's provider manager to the real credential, which NEVER leaves
/// the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelHandle {
    pub provider_id: String,
    pub model_id: String,
}

impl ModelHandle {
    /// Compact wire string: `provider:<provider_id>:<model_id>`. Safe to log,
    /// safe to put in P2P advertisements — contains no secret material.
    pub fn wire(&self) -> String {
        format!("provider:{}:{}", self.provider_id, self.model_id)
    }

    /// Parse a wire string back into a handle. Returns `None` for anything
    /// that does not start with the `provider:` prefix (legacy model names,
    /// local file names, remote worker handles).
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("provider:")?;
        let (provider_id, model_id) = rest.split_once(':')?;
        if provider_id.is_empty() || model_id.is_empty() {
            return None;
        }
        Some(Self {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        })
    }
}

/// Health of a provider as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    #[default]
    Unknown,
    Healthy,
    Degraded,
    Offline,
    Disabled,
}

/// Health of a single connected model from a provider.
///
/// A provider can be healthy while one of its models is unavailable (e.g. the
/// upstream model is deprecated or the account lacks access); that must be
/// representable independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealth {
    #[default]
    Unknown,
    Healthy,
    Degraded,
    Offline,
    Disabled,
}

/// Circuit-breaker state for one provider (and, independently, one model).
///
/// HEALTHY → failures → DEGRADED → threshold → OPEN → cooldown → HALF_OPEN →
/// successful probe → HEALTHY. OPEN means the breaker refuses requests until
/// the cooldown elapses; HALF_OPEN lets a bounded probe through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    #[default]
    Healthy,
    Degraded,
    Open,
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

/// Configuration for the circuit breaker (per provider).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BreakerConfig {
    /// Consecutive failures after which the breaker opens.
    pub open_threshold: u32,
    /// Cooldown before HALF_OPEN (seconds).
    pub cooldown_secs: u64,
    /// Once HALF_OPEN, how many probe requests are allowed through.
    pub half_open_probe_limit: u32,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            open_threshold: 5,
            cooldown_secs: 30,
            half_open_probe_limit: 2,
        }
    }
}

/// Error classes for the provider layer (wire-safe, no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorClass {
    /// Invalid or expired credential (401/403).
    Auth,
    /// Rate limited (429); honor Retry-After where available.
    RateLimited,
    /// Quota/budget exhausted (e.g. 402 or provider budget metadata).
    QuotaExhausted,
    /// Model unavailable / unsupported / 404.
    ModelUnavailable,
    /// Request/response timeout or network-level failure.
    Timeout,
    /// Upstream 5xx.
    Upstream,
    /// Malformed response body.
    Protocol,
    /// Local policy denial (share revoked, budget exhausted, capacity).
    Policy,
    /// No known classification.
    Unknown,
}

impl ProviderErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::ModelUnavailable => "model_unavailable",
            Self::Timeout => "timeout",
            Self::Upstream => "upstream",
            Self::Protocol => "protocol",
            Self::Policy => "policy",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether a failure should be retried (used by the retry policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    Retryable,
    NonRetryable,
}

impl ProviderErrorClass {
    /// Classifies a class into retryable/non-retryable.
    ///
    /// NEVER retry: auth, quota, policy, model-unsupported, malformed protocol.
    /// Retry (bounded): timeout, upstream 5xx, and rate limiting ONLY when the
    /// caller applies Retry-After. Anything retryable is still bounded by the
    /// circuit breaker so a storm is impossible.
    pub fn retry_class(self) -> RetryClass {
        match self {
            Self::Auth | Self::QuotaExhausted | Self::Policy | Self::ModelUnavailable => {
                RetryClass::NonRetryable
            }
            Self::RateLimited | Self::Timeout | Self::Upstream | Self::Protocol | Self::Unknown => {
                RetryClass::Retryable
            }
        }
    }
}

/// Pricing metadata. Explicitly marked provider-reported / estimated: we never
/// invent pricing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pricing {
    /// USD per 1M input tokens, when the provider reports it.
    pub input_per_1m: Option<f64>,
    /// USD per 1M output tokens, when the provider reports it.
    pub output_per_1m: Option<f64>,
    /// Whether the numbers are provider-reported or estimated.
    pub provenance: PriceProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceProvenance {
    ProviderReported,
    Estimated,
    Unknown,
}

/// Sharing policy for a connected model. Sharing DEFAULTS OFF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharingPolicy {
    pub enabled: bool,
    /// Peer id allowlist. Empty = any trusted peer (subject to required trust
    /// level), per the fabric's `private_swarm` trust model.
    #[serde(default)]
    pub allowed_peers: Vec<String>,
    /// Minimum trust level required of a remote peer to use the model.
    #[serde(default = "default_required_trust")]
    pub required_trust_level: u8,
    /// Max concurrent shared requests routed through this model.
    #[serde(default)]
    pub max_concurrency: u32,
    /// Max shared requests per minute (0 = unlimited).
    #[serde(default)]
    pub requests_per_minute: u32,
    /// Daily token budget for shared use (0 = unlimited).
    #[serde(default)]
    pub daily_token_limit: u64,
    /// Daily cost budget in USD (f64; 0 = unlimited).
    #[serde(default)]
    pub daily_cost_limit: f64,
    /// Capabilities allowed to be routed through the shared model. Empty =
    /// any capability the model itself supports.
    #[serde(default)]
    pub allowed_capabilities: Vec<CapabilityKind>,
    /// Optional instant at which sharing expires (unix ms).
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
    /// Force authentication (a valid subscriber/consumer token) for shared use.
    #[serde(default = "default_true")]
    pub require_authentication: bool,
    /// Require the requesting peer to be in the node's trusted set.
    #[serde(default = "default_true")]
    pub require_trusted_peer: bool,
}

fn default_required_trust() -> u8 {
    1
}
fn default_true() -> bool {
    true
}

impl Default for SharingPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_peers: Vec::new(),
            required_trust_level: default_required_trust(),
            max_concurrency: 2,
            requests_per_minute: 0,
            daily_token_limit: 0,
            daily_cost_limit: 0.0,
            allowed_capabilities: Vec::new(),
            expires_at_ms: None,
            require_authentication: default_true(),
            require_trusted_peer: default_true(),
        }
    }
}

/// Rate/concurrency limits for a connected model (local use + sharing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelBudget {
    /// Requests per minute (0 = unlimited).
    #[serde(default)]
    pub requests_per_minute: u32,
    /// Max concurrent in-flight requests (0 = unlimited).
    #[serde(default)]
    pub max_concurrency: u32,
    /// Daily token budget (0 = unlimited).
    #[serde(default)]
    pub daily_token_limit: u64,
    /// Daily cost budget in USD (f64; 0 = unlimited).
    #[serde(default)]
    pub daily_cost_limit: f64,
    /// Optional monthly spend cap in USD (f64; 0 = unlimited).
    #[serde(default)]
    pub monthly_cost_limit: f64,
}

impl Default for ModelBudget {
    fn default() -> Self {
        Self {
            requests_per_minute: 0,
            max_concurrency: 4,
            daily_token_limit: 0,
            daily_cost_limit: 0.0,
            monthly_cost_limit: 0.0,
        }
    }
}

/// A provider-backed model as it appears in the unified catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectedModel {
    /// Stable id unique within the node (uuid).
    pub model_id: String,
    /// Provider id this model belongs to.
    pub provider_id: String,
    /// Upstream model id exactly as the provider expects it.
    pub upstream_model: String,
    /// Display name (defaults to upstream_model when not set).
    #[serde(default)]
    pub display_name: String,
    /// Capabilities this model supports. Never inferred from the name alone
    /// unless explicitly marked as heuristics.
    #[serde(default)]
    pub capabilities: Vec<CapabilityKind>,
    /// Whether capabilities are provider-verified / probed / manual-configured.
    /// Manual overrides must be visibly marked.
    #[serde(default)]
    pub capability_provenance: CapabilityProvenance,
    /// Context window if known.
    #[serde(default)]
    pub context_window: Option<u32>,
    /// Pricing metadata if available (provider-reported or estimated).
    #[serde(default)]
    pub pricing: Option<Pricing>,
    /// Whether routing may use this model.
    pub enabled: bool,
    /// Sharing policy — defaults OFF.
    #[serde(default)]
    pub sharing: SharingPolicy,
    /// Rate/concurrency/cost limits local + shared.
    #[serde(default)]
    pub budget: ModelBudget,
    /// Current health.
    #[serde(default)]
    pub health: ModelHealth,
    /// Latency of the last successful probe/request (ms). `None` = never
    /// measured.
    #[serde(default)]
    pub last_latency_ms: Option<u64>,
    /// Unix ms of the last successful request/probe.
    #[serde(default)]
    pub last_success_at_ms: Option<u64>,
    /// Unix ms of the last failure.
    #[serde(default)]
    pub last_failure_at_ms: Option<u64>,
    /// Consecutive failures since the last success.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Circuit state for this model (independent from provider-level state).
    #[serde(default)]
    pub circuit: CircuitState,
    /// Books usage for budget enforcement (tracked runtime-side).
    #[serde(default)]
    pub usage: ModelUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProvenance {
    /// Capabilities reported by the provider / verified by a live probe.
    #[default]
    Verified,
    /// Capabilities configured manually by the operator (marked as such).
    Manual,
    /// Heuristic (e.g. name-based) — always labeled.
    Heuristic,
    Unknown,
}

/// Runtime usage bookkeeping for budget enforcement (not persisted).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    /// Requests in the current minute window.
    pub requests_this_minute: u32,
    /// Window start unix ms for the requests-per-minute counter.
    pub window_started_at_ms: u64,
    /// Tokens consumed today.
    pub tokens_today: u64,
    /// Day bucket (unix ms of local midnight / a stable day key).
    pub day_key: u64,
    /// Estimated/priced spend today (USD, f64 keep simple).
    pub spend_today: f64,
}

impl ConnectedModel {
    /// Stable symbolic model hash for the fabric routing layer.
    ///
    /// Provider models are NOT GGUF artifacts: they have no file, no Merkle
    /// root, no chunks. To keep the existing `ExecutionPlanner`,
    /// `resolve_model_hash` and admission gates intact, each connected model
    /// derives a deterministic virtual hash from `(provider_id, upstream)`.
    /// This lets the router select provider-backed execution through the same
    /// "model_hash" seam without pretending the model is a local artifact.
    pub fn symbolic_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"decentraai-provider-model-v1\0");
        hasher.update(self.provider_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.upstream_model.as_bytes());
        let digest = hasher.finalize();
        // First 24 hex chars (96 bits) is plenty for routing disambiguation.
        let hex_str: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let short: String = hex_str.chars().take(24).collect();
        format!("prov-{short}")
    }
}

/// A connected provider record. Persisted WITHOUT the credential — the store
/// keeps the actual secret separately and the record only references it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    /// Stable id unique within the node (uuid).
    pub provider_id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub base_url: String,
    /// Reference into the CredentialStore (e.g. a key id). NEVER the raw
    /// secret itself.
    pub credential_ref: String,
    pub enabled: bool,
    /// Milliseconds this node has failed to reach the provider (from the last
    /// health check).
    #[serde(default)]
    pub health: ProviderHealth,
    #[serde(default)]
    pub last_health_check_at_ms: Option<u64>,
    #[serde(default)]
    pub last_latency_ms: Option<u64>,
    #[serde(default)]
    pub failure_count: u32,
    #[serde(default)]
    pub last_error_class: Option<ProviderErrorClass>,
    #[serde(default)]
    pub last_success_at_ms: Option<u64>,
    #[serde(default)]
    pub circuit: CircuitState,
    /// Optional provider-reported budget metadata (e.g. OpenRouter credits).
    #[serde(default)]
    pub budget_metadata: Option<serde_json::Value>,
    /// Connected models owned by this provider.
    #[serde(default)]
    pub models: Vec<ConnectedModel>,
}

impl Provider {
    pub fn new(
        provider_id: impl Into<String>,
        kind: ProviderKind,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
        credential_ref: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            kind,
            display_name: display_name.into(),
            base_url: base_url.into(),
            credential_ref: credential_ref.into(),
            enabled: true,
            health: ProviderHealth::Unknown,
            last_health_check_at_ms: None,
            last_latency_ms: None,
            failure_count: 0,
            last_error_class: None,
            last_success_at_ms: None,
            circuit: CircuitState::Healthy,
            budget_metadata: None,
            models: Vec::new(),
        }
    }

    pub fn model(&self, model_id: &str) -> Option<&ConnectedModel> {
        self.models.iter().find(|m| m.model_id == model_id)
    }

    pub fn model_mut(&mut self, model_id: &str) -> Option<&mut ConnectedModel> {
        self.models.iter_mut().find(|m| m.model_id == model_id)
    }

    /// Models shareable to remote peers (sharing.enabled). This is the ONLY
    /// surface exposed over the P2P advertisement: handles, never credentials.
    pub fn shared_models(&self) -> impl Iterator<Item = &ConnectedModel> {
        self.models.iter().filter(|m| m.sharing.enabled)
    }

    /// Whether any model is shared.
    pub fn has_shared_models(&self) -> bool {
        self.shared_models().next().is_some()
    }
}

/// Everything the dashboard needs to render a provider record WITHOUT the
/// credential: identity, health, models, share/circuit state and a masked
/// fingerprint (shown once at creation only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider_id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub base_url: String,
    pub enabled: bool,
    pub health: ProviderHealth,
    pub last_health_check_at_ms: Option<u64>,
    pub last_latency_ms: Option<u64>,
    pub failure_count: u32,
    pub last_error_class: Option<ProviderErrorClass>,
    pub last_success_at_ms: Option<u64>,
    pub circuit: CircuitState,
    pub budget_metadata: Option<serde_json::Value>,
    pub model_count: usize,
    pub shared_model_count: usize,
    /// Masked credential fingerprint, e.g. `••••a91f`. NEVER the full secret.
    pub credential_fingerprint: String,
}

impl Provider {
    /// Builds the safe public summary. `fingerprint` must be supplied by the
    /// CredentialStore (a masked view); this method never touches the secret.
    pub fn summary(&self, fingerprint: String) -> ProviderSummary {
        ProviderSummary {
            provider_id: self.provider_id.clone(),
            kind: self.kind,
            display_name: self.display_name.clone(),
            base_url: self.base_url.clone(),
            enabled: self.enabled,
            health: self.health,
            last_health_check_at_ms: self.last_health_check_at_ms,
            last_latency_ms: self.last_latency_ms,
            failure_count: self.failure_count,
            last_error_class: self.last_error_class.clone(),
            last_success_at_ms: self.last_success_at_ms,
            circuit: self.circuit,
            budget_metadata: self.budget_metadata.clone(),
            model_count: self.models.len(),
            shared_model_count: self.shared_models().count(),
            credential_fingerprint: fingerprint,
        }
    }
}

/// Top-level errors of the provider domain.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider {0} not found")]
    NotFound(String),
    #[error("model {0} not found on provider {1}")]
    ModelNotFound(String, String),
    #[error("provider is disabled")]
    Disabled,
    #[error("credential store error: {0}")]
    Credential(String),
    #[error("invalid provider kind: {0}")]
    InvalidKind(String),
    #[error("sharing is disabled for this model")]
    SharingDisabled,
    #[error("peer is not allowed to use this shared model")]
    PeerNotAllowed,
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
    #[error("circuit breaker open: cooldown until {0:?}")]
    CircuitOpen(Option<u64>),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("adapter error: {0}")]
    Adapter(String),
}

// ─── Module declarations ──────────────────────────────────────────────

mod credential_store;
pub use credential_store::CredentialStore;

pub mod adapter;
pub use adapter::{ModelAdapter, OpenAICompatibleProvider, ProviderAdapter};

// ─── Tests (domain types) ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_defaults_round_trip() {
        for (kind, wire) in [
            (ProviderKind::OpenRouter, "openrouter"),
            (ProviderKind::OpenAi, "openai"),
            (ProviderKind::Groq, "groq"),
            (ProviderKind::Together, "together"),
            (ProviderKind::Fireworks, "fireworks"),
            (
                ProviderKind::GenericOpenAiCompatible,
                "generic_openai_compatible",
            ),
        ] {
            assert_eq!(kind.as_str(), wire);
            assert_eq!(ProviderKind::parse(wire), Some(kind));
            assert_eq!(ProviderKind::parse("bogus"), None);
        }
        assert_eq!(
            ProviderKind::OpenRouter.default_base_url().unwrap(),
            "https://openrouter.ai/api/v1"
        );
        assert!(
            ProviderKind::GenericOpenAiCompatible
                .default_base_url()
                .is_none()
        );
    }

    #[test]
    fn model_source_round_trip() {
        for (src, wire) in [
            (ModelSource::Local, "local"),
            (ModelSource::Fabric, "fabric"),
            (ModelSource::Provider, "provider"),
        ] {
            assert_eq!(src.as_str(), wire);
        }
    }

    #[test]
    fn model_handle_wire_round_trip() {
        let h = ModelHandle {
            provider_id: "prov-1".into(),
            model_id: "qwen/qwen3-coder".into(),
        };
        let wire = h.wire();
        assert!(wire.starts_with("provider:prov-1:qwen/qwen3-coder"));
        assert_eq!(ModelHandle::parse(&wire), Some(h));
        assert!(ModelHandle::parse("llama-3.2-1b.gguf").is_none());
        assert!(ModelHandle::parse("provider:").is_none());
        assert!(ModelHandle::parse("provider:a:").is_none());
        assert!(ModelHandle::parse("provider::").is_none());
        assert!(ModelHandle::parse("provider::model").is_none());
        assert!(ModelHandle::parse("provider-of-models").is_none());
    }

    #[test]
    fn sharing_defaults_off() {
        let p = SharingPolicy::default();
        assert!(!p.enabled);
        assert!(p.require_authentication);
        assert!(p.require_trusted_peer);
        assert_eq!(p.required_trust_level, 1);
        assert_eq!(p.max_concurrency, 2);
        assert_eq!(p.requests_per_minute, 0);
        assert!(p.expires_at_ms.is_none());
    }

    #[test]
    fn symbolic_hash_is_stable_and_distinct() {
        let m1 = ConnectedModel {
            model_id: "m1".into(),
            provider_id: "p1".into(),
            upstream_model: "qwen/qwen3-coder".into(),
            display_name: String::new(),
            capabilities: vec![],
            capability_provenance: CapabilityProvenance::Unknown,
            context_window: None,
            pricing: None,
            enabled: true,
            sharing: SharingPolicy::default(),
            budget: ModelBudget::default(),
            health: ModelHealth::Unknown,
            last_latency_ms: None,
            last_success_at_ms: None,
            last_failure_at_ms: None,
            consecutive_failures: 0,
            circuit: CircuitState::Healthy,
            usage: ModelUsage::default(),
        };
        let m2 = ConnectedModel {
            upstream_model: "anthropic/claude-3.5-sonnet".into(),
            ..m1.clone()
        };
        let h1 = m1.symbolic_hash();
        let h11 = m1.symbolic_hash();
        let h2 = m2.symbolic_hash();
        assert_eq!(h1, h11, "same model must hash deterministically");
        assert_ne!(h1, h2, "different upstream must hash differently");
        assert!(
            h1.starts_with("prov-"),
            "symbolic hash carries the prov- marker"
        );
    }

    #[test]
    fn retry_classification_is_safe() {
        assert_eq!(
            ProviderErrorClass::Auth.retry_class(),
            RetryClass::NonRetryable
        );
        assert_eq!(
            ProviderErrorClass::QuotaExhausted.retry_class(),
            RetryClass::NonRetryable
        );
        assert_eq!(
            ProviderErrorClass::ModelUnavailable.retry_class(),
            RetryClass::NonRetryable
        );
        assert_eq!(
            ProviderErrorClass::Policy.retry_class(),
            RetryClass::NonRetryable
        );
        assert_eq!(
            ProviderErrorClass::Timeout.retry_class(),
            RetryClass::Retryable
        );
        assert_eq!(
            ProviderErrorClass::Upstream.retry_class(),
            RetryClass::Retryable
        );
    }

    #[test]
    fn provider_summary_never_exposes_credential() {
        let provider = Provider::new("p1", ProviderKind::OpenRouter, "OR", "https://x", "cred-1");
        let summary = provider.summary("••••a91f".into());
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("sk-"));
        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("cred-1"));
        assert!(json.contains("a91f"));
    }

    #[test]
    fn shared_models_iteration_only_returns_enabled() {
        let mut provider = Provider::new("p1", ProviderKind::Groq, "Groq", "https://x", "cred-1");
        let mut m1 = ConnectedModel {
            model_id: "m1".into(),
            provider_id: "p1".into(),
            upstream_model: "llama-3.3-70b".into(),
            display_name: String::new(),
            capabilities: vec![],
            capability_provenance: CapabilityProvenance::Unknown,
            context_window: None,
            pricing: None,
            enabled: true,
            sharing: SharingPolicy::default(),
            budget: ModelBudget::default(),
            health: ModelHealth::Unknown,
            last_latency_ms: None,
            last_success_at_ms: None,
            last_failure_at_ms: None,
            consecutive_failures: 0,
            circuit: CircuitState::Healthy,
            usage: ModelUsage::default(),
        };
        m1.sharing.enabled = true;
        provider.models.push(m1);
        assert_eq!(provider.shared_models().count(), 1);
        assert!(provider.has_shared_models());
        provider.models[0].sharing.enabled = false;
        assert_eq!(provider.shared_models().count(), 0);
        assert!(!provider.has_shared_models());
    }
}
