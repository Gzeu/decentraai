//! Experimental Inference Credit Economy (research track only).
//!
//! OPTIONAL and crypto-agnostic. Not wired into production scheduler, discovery,
//! workers, P2P, or receipts. Does **not** replace existing primitives:
//!
//! - [`decentraai_compute::QuotaLedger`] — contribution→quota (EARNED→AVAILABLE→RESERVED→CONSUMED)
//! - [`decentraai_compute::CreditLedger`] — P14 synthetic credits from verified compute
//! - [`decentraai_compute::CompensationLedger`] — reputation-scaled compensation
//! - [`decentraai_compute::ReservationLedger`] — compute slot / VRAM admission
//! - P13 signed `VerifiedComputeReceipt` — cryptographic execution evidence
//!
//! This crate adds the missing economic layer required by the research track:
//!
//! ```text
//! TEMPORARY EXTERNAL RESOURCE  ≠  DECENTRAAI CONTRIBUTION CREDIT (CU)
//! (provider quota may expire)     (settled CU remain durable)
//! ```
//!
//! CU are awarded only from verified contribution and may later be spent on a
//! *different* eligible resource than the one that generated them.
//!
//! API keys never appear in advertisements or ledger events. Callers pass an
//! opaque `credential_ref` that stays local to the contributor node.
//!
//! # Crypto Readiness
//!
//! This crate provides the modular [`SettlementEngine`] and [`ExternalAssetAdapter`]
//! abstractions with an explicit escrow/lock state machine (`AVAILABLE → LOCKED → FINALIZED`
//! or `REFUNDED`). Blockchain logic, token issuance, and wallet private-key management
//! are strictly decoupled and deferred to external adapters.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Stable account identifier. Reuses existing DecentraAI identity strings
/// (libp2p peer id, operator account id). No new identity system.
pub type AccountId = String;

const MAX_EVENTS: usize = 8192;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Resource types and advertisements (no secrets)
// ---------------------------------------------------------------------------

/// Kind of contributed or consumed resource. CU are **not** permanently bound
/// to the origin type; this is provenance, not a spend restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResourceType {
    ApiQuota,
    GpuCompute,
    CpuCompute,
    Storage,
    Bandwidth,
}

/// How usage was obtained. Credits require a verified method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementMethod {
    /// Bound to an existing signed compute / usage receipt (P13).
    SignedReceipt,
    /// Provider-side accounting the contributor node observed locally.
    ProviderAccounting,
    /// Worker telemetry (insufficient alone for settlement).
    WorkerTelemetry,
    Unknown,
}

impl MeasurementMethod {
    pub fn can_settle(self) -> bool {
        matches!(self, Self::SignedReceipt | Self::ProviderAccounting)
    }
}

/// Public advertisement of available capacity. **Never** carries API keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceAdvertisement {
    pub advertisement_id: String,
    pub contributor: AccountId,
    pub resource_type: ResourceType,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub capacity_units: u64,
    pub available_from_ms: Option<u64>,
    pub available_until_ms: Option<u64>,
    pub rate_limit_per_minute: Option<u32>,
    pub concurrency_limit: Option<u32>,
    pub measurement: MeasurementMethod,
    pub region: Option<String>,
    pub capabilities: Vec<String>,
    /// Opaque local handle (e.g. env-var name). Must not look like a secret.
    pub credential_ref: Option<String>,
}

impl ResourceAdvertisement {
    /// Rejects refs that look like leaked API keys. The field is a handle, not a secret.
    pub fn validate_no_secrets(&self) -> Result<(), EconomyError> {
        if let Some(r) = &self.credential_ref {
            let lower = r.to_ascii_lowercase();
            if r.starts_with("sk-")
                || lower.contains("api_key")
                || lower.contains("apikey")
                || lower.contains("secret")
                || lower.contains("bearer ")
            {
                return Err(EconomyError::SecretInAdvertisement);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Measured usage (from existing receipts — not a replacement receipt type)
// ---------------------------------------------------------------------------

/// Already-verified usage extracted from existing DecentraAI receipts.
///
/// The cryptographic check (`signature_valid`) is performed by the existing
/// P13 verifier **before** this struct is built. This crate never re-implements
/// Ed25519 or `VerifiedComputeReceipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedUsage {
    pub receipt_id: String,
    pub execution_id: String,
    pub contributor: AccountId,
    pub consumer: AccountId,
    pub resource_type: ResourceType,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub gpu_ms: u64,
    pub cpu_ms: u64,
    pub storage_byte_hours: u64,
    pub bandwidth_bytes: u64,
    pub success: bool,
    /// Must be true: caller already verified the existing signed receipt.
    pub signature_valid: bool,
    pub measurement: MeasurementMethod,
    pub reservation_id: Option<String>,
    pub measured_at_ms: u64,
}

impl VerifiedUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

// ---------------------------------------------------------------------------
// Versioned credit policy (integer only; not 1 token = 1 CU)
// ---------------------------------------------------------------------------

/// Operator-settable, versioned policy. Authoritative accounting is integer CU.
/// Historical events keep the version that produced them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditPolicy {
    pub version: u32,
    pub name: String,
    pub input_token_weight: u64,
    pub output_token_weight: u64,
    pub gpu_ms_weight: u64,
    pub cpu_ms_weight: u64,
    pub storage_byte_hour_weight: u64,
    pub bandwidth_byte_weight: u64,
}

impl Default for CreditPolicy {
    fn default() -> Self {
        // Deliberately not 1 token = 1 CU: output tokens are weighted higher.
        Self {
            version: 1,
            name: "ice-v1".to_string(),
            input_token_weight: 1,
            output_token_weight: 2,
            gpu_ms_weight: 1,
            cpu_ms_weight: 1,
            storage_byte_hour_weight: 0,
            bandwidth_byte_weight: 0,
        }
    }
}

impl CreditPolicy {
    pub fn calculate(&self, usage: &VerifiedUsage) -> CreditCalculation {
        let input = usage.input_tokens.saturating_mul(self.input_token_weight);
        let output = usage.output_tokens.saturating_mul(self.output_token_weight);
        let gpu = usage.gpu_ms.saturating_mul(self.gpu_ms_weight);
        let cpu = usage.cpu_ms.saturating_mul(self.cpu_ms_weight);
        let storage = usage
            .storage_byte_hours
            .saturating_mul(self.storage_byte_hour_weight);
        let bandwidth = usage
            .bandwidth_bytes
            .saturating_mul(self.bandwidth_byte_weight);
        let mut breakdown = BTreeMap::new();
        breakdown.insert("input_tokens".into(), input);
        breakdown.insert("output_tokens".into(), output);
        breakdown.insert("gpu_ms".into(), gpu);
        breakdown.insert("cpu_ms".into(), cpu);
        breakdown.insert("storage".into(), storage);
        breakdown.insert("bandwidth".into(), bandwidth);
        let credits = input
            .saturating_add(output)
            .saturating_add(gpu)
            .saturating_add(cpu)
            .saturating_add(storage)
            .saturating_add(bandwidth);
        CreditCalculation {
            credits,
            policy_version: self.version,
            breakdown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditCalculation {
    pub credits: u64,
    pub policy_version: u32,
    pub breakdown: BTreeMap<String, u64>,
}

// ---------------------------------------------------------------------------
// Contribution lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContributionState {
    Pending,
    Verified,
    Settled,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRecord {
    pub contribution_id: String,
    pub contributor: AccountId,
    pub resource_type: ResourceType,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub advertisement_id: Option<String>,
    pub quota_id: Option<String>,
    pub state: ContributionState,
    pub usage: Option<VerifiedUsage>,
    pub calculation: Option<CreditCalculation>,
    pub reject_reason: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

// ---------------------------------------------------------------------------
// Temporary provider quota (never confused with the CU ledger)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderQuota {
    pub quota_id: String,
    pub contributor: AccountId,
    pub resource_type: ResourceType,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub available: u64,
    pub reserved: u64,
    pub consumed: u64,
    pub reset_at_ms: Option<u64>,
    pub expired: bool,
}

impl ProviderQuota {
    pub fn remaining(&self) -> u64 {
        if self.expired {
            0
        } else {
            self.available
        }
    }
}

// ---------------------------------------------------------------------------
// CU ledger events and balances
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditOp {
    Earn,
    Reserve,
    Release,
    Consume,
    Reject,
    LockSettlement,
    FinalizeSettlement,
    RefundSettlement,
}

/// Append-only provenance event. Answers who / what / receipt / policy / when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditEvent {
    pub op: CreditOp,
    pub account: AccountId,
    pub amount: u64,
    pub ref_id: String,
    pub contribution_id: Option<String>,
    pub receipt_id: Option<String>,
    pub execution_id: Option<String>,
    pub origin_resource: Option<ResourceType>,
    pub origin_provider: Option<String>,
    pub origin_model: Option<String>,
    pub consume_resource: Option<ResourceType>,
    pub consume_provider: Option<String>,
    pub consume_model: Option<String>,
    pub settlement_rail: Option<String>,
    pub external_ref: Option<String>,
    pub policy_version: u32,
    pub created_at_ms: u64,
    pub settled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CreditBalance {
    pub earned: u64,
    pub available: u64,
    pub reserved: u64,
    pub consumed: u64,
    pub locked_for_settlement: u64,
    pub pending: u64,
}

impl CreditBalance {
    /// Invariant: earned == available + reserved + consumed + locked_for_settlement (pending is not CU).
    pub fn check_invariant(&self) -> bool {
        self.earned
            == self
                .available
                .saturating_add(self.reserved)
                .saturating_add(self.consumed)
                .saturating_add(self.locked_for_settlement)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditReservation {
    pub reservation_id: String,
    pub account: AccountId,
    pub amount: u64,
    pub consume_resource: ResourceType,
    pub consume_provider: Option<String>,
    pub consume_model: Option<String>,
    pub settled: bool,
}

// ---------------------------------------------------------------------------
// Crypto Readiness / Modular Settlement Models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettlementStatus {
    Created,
    Locked,
    Committed,
    Finalized,
    Failed,
    Refunded,
}

/// Escrow / settlement intent for bridging or settling CU to an external rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementIntent {
    pub intent_id: String,
    pub account: AccountId,
    pub cu_amount: u64,
    pub target_rail: String,
    pub external_recipient: String,
    pub status: SettlementStatus,
    pub external_tx_id: Option<String>,
    pub created_at_ms: u64,
    pub finalized_at_ms: Option<u64>,
    pub failure_reason: Option<String>,
}

/// Optional settlement engine interface. Decoupled from core DecentraAI.
pub trait SettlementEngine {
    fn on_settled(&mut self, event: &CreditEvent) -> Result<(), EconomyError>;
}

/// Optional external asset adapter interface for quoting and verifying external rails.
pub trait ExternalAssetAdapter {
    fn quote_units(&self, cu_amount: u64, target_rail: &str) -> Result<u64, EconomyError>;
    fn validate_address(&self, target_rail: &str, address: &str) -> bool;
}

/// Default internal clearinghouse (no-op external bridge).
#[derive(Debug, Default)]
pub struct InternalSettlement;

impl SettlementEngine for InternalSettlement {
    fn on_settled(&mut self, _event: &CreditEvent) -> Result<(), EconomyError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomyError {
    SecretInAdvertisement,
    UnknownContribution,
    InvalidState,
    ForgedReceipt,
    DuplicateReceipt,
    DuplicateSettlement,
    UnverifiedMeasurement,
    InsufficientQuota { available: u64, requested: u64 },
    InsufficientCredits { available: u64, requested: u64 },
    UnknownReservation,
    AlreadySettled,
    QuotaExpired,
    QuotaExhausted,
    UnknownQuota,
    UnknownSettlementIntent,
    InvalidSettlementState,
    UnsupportedRail,
    InvalidExternalAddress,
    IoError,
    CorruptedState,
}

impl std::fmt::Display for EconomyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretInAdvertisement => write!(f, "advertisement must not contain API secrets"),
            Self::UnknownContribution => write!(f, "unknown contribution"),
            Self::InvalidState => write!(f, "invalid contribution state transition"),
            Self::ForgedReceipt => write!(f, "receipt signature is not valid"),
            Self::DuplicateReceipt => write!(f, "receipt already used"),
            Self::DuplicateSettlement => write!(f, "contribution already settled"),
            Self::UnverifiedMeasurement => write!(f, "measurement method cannot settle credits"),
            Self::InsufficientQuota { available, requested } => {
                write!(f, "insufficient provider quota: requested {requested}, available {available}")
            }
            Self::InsufficientCredits { available, requested } => {
                write!(f, "insufficient CU: requested {requested}, available {available}")
            }
            Self::UnknownReservation => write!(f, "unknown reservation"),
            Self::AlreadySettled => write!(f, "reservation already settled"),
            Self::QuotaExpired => write!(f, "provider quota expired"),
            Self::QuotaExhausted => write!(f, "provider quota exhausted"),
            Self::UnknownQuota => write!(f, "unknown provider quota"),
            Self::UnknownSettlementIntent => write!(f, "unknown settlement intent"),
            Self::InvalidSettlementState => write!(f, "invalid settlement state transition"),
            Self::UnsupportedRail => write!(f, "unsupported settlement rail"),
            Self::InvalidExternalAddress => write!(f, "invalid external recipient address"),
            Self::IoError => write!(f, "persistence I/O error"),
            Self::CorruptedState => write!(f, "corrupted ledger snapshot"),
        }
    }
}

impl std::error::Error for EconomyError {}

// ---------------------------------------------------------------------------
// Persistence snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomySnapshot {
    pub ads: BTreeMap<String, ResourceAdvertisement>,
    pub quotas: HashMap<String, ProviderQuota>,
    pub contributions: BTreeMap<String, ContributionRecord>,
    pub accounts: HashMap<AccountId, CreditBalance>,
    pub reservations: HashMap<String, CreditReservation>,
    pub settlement_intents: HashMap<String, SettlementIntent>,
    pub events: Vec<CreditEvent>,
    pub receipts: HashSet<String>,
    pub applied: HashSet<(String, String)>,
    pub policy: CreditPolicy,
    pub timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// Economy (mutex-protected; wrap, never await under the lock)
// ---------------------------------------------------------------------------

struct Inner {
    ads: BTreeMap<String, ResourceAdvertisement>,
    quotas: HashMap<String, ProviderQuota>,
    contributions: BTreeMap<String, ContributionRecord>,
    accounts: HashMap<AccountId, CreditBalance>,
    reservations: HashMap<String, CreditReservation>,
    settlement_intents: HashMap<String, SettlementIntent>,
    events: VecDeque<CreditEvent>,
    receipts: HashSet<String>,
    applied: HashSet<(String, String)>,
    policy: CreditPolicy,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            ads: BTreeMap::new(),
            quotas: HashMap::new(),
            contributions: BTreeMap::new(),
            accounts: HashMap::new(),
            reservations: HashMap::new(),
            settlement_intents: HashMap::new(),
            events: VecDeque::new(),
            receipts: HashSet::new(),
            applied: HashSet::new(),
            policy: CreditPolicy::default(),
        }
    }
}

/// Experimental credit economy. Optional. Not a cryptocurrency.
pub struct InferenceCreditEconomy {
    inner: Mutex<Inner>,
}

impl Default for InferenceCreditEconomy {
    fn default() -> Self {
        Self::new(CreditPolicy::default())
    }
}

impl InferenceCreditEconomy {
    pub fn new(policy: CreditPolicy) -> Self {
        Self {
            inner: Mutex::new(Inner {
                policy,
                ..Inner::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn policy(&self) -> CreditPolicy {
        self.lock().policy.clone()
    }

    pub fn set_policy(&self, policy: CreditPolicy) {
        self.lock().policy = policy;
    }

    pub fn snapshot(&self) -> EconomySnapshot {
        let g = self.lock();
        EconomySnapshot {
            ads: g.ads.clone(),
            quotas: g.quotas.clone(),
            contributions: g.contributions.clone(),
            accounts: g.accounts.clone(),
            reservations: g.reservations.clone(),
            settlement_intents: g.settlement_intents.clone(),
            events: g.events.iter().cloned().collect(),
            receipts: g.receipts.clone(),
            applied: g.applied.clone(),
            policy: g.policy.clone(),
            timestamp_ms: now_ms(),
        }
    }

    pub fn restore_snapshot(&self, snap: EconomySnapshot) -> Result<(), EconomyError> {
        for bal in snap.accounts.values() {
            if !bal.check_invariant() {
                return Err(EconomyError::CorruptedState);
            }
        }
        let mut g = self.lock();
        g.ads = snap.ads;
        g.quotas = snap.quotas;
        g.contributions = snap.contributions;
        g.accounts = snap.accounts;
        g.reservations = snap.reservations;
        g.settlement_intents = snap.settlement_intents;
        g.events = snap.events.into_iter().collect();
        g.receipts = snap.receipts;
        g.applied = snap.applied;
        g.policy = snap.policy;
        Ok(())
    }

    pub fn persist_to_disk(&self, path: &Path) -> Result<(), EconomyError> {
        let snap = self.snapshot();
        let serialized = serde_json::to_vec_pretty(&snap).map_err(|_| EconomyError::IoError)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| EconomyError::IoError)?;
        }
        let tmp_path = path.with_extension(format!("tmp.{}", now_ms()));
        {
            let mut file = File::create(&tmp_path).map_err(|_| EconomyError::IoError)?;
            file.write_all(&serialized).map_err(|_| EconomyError::IoError)?;
            file.sync_all().map_err(|_| EconomyError::IoError)?;
        }
        fs::rename(&tmp_path, path).map_err(|_| EconomyError::IoError)?;
        Ok(())
    }

    pub fn load_from_disk(path: &Path) -> Result<Self, EconomyError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut file = File::open(path).map_err(|_| EconomyError::IoError)?;
        let mut content = Vec::new();
        file.read_to_end(&mut content).map_err(|_| EconomyError::IoError)?;
        let snap: EconomySnapshot = serde_json::from_slice(&content).map_err(|_| EconomyError::CorruptedState)?;
        let eco = Self::new(snap.policy.clone());
        eco.restore_snapshot(snap)?;
        Ok(eco)
    }

    pub fn advertise(&self, ad: ResourceAdvertisement) -> Result<(), EconomyError> {
        ad.validate_no_secrets()?;
        let mut g = self.lock();
        g.ads.insert(ad.advertisement_id.clone(), ad);
        Ok(())
    }

    pub fn advertisement(&self, id: &str) -> Option<ResourceAdvertisement> {
        self.lock().ads.get(id).cloned()
    }

    pub fn register_quota(&self, quota: ProviderQuota) {
        let mut g = self.lock();
        g.quotas.insert(quota.quota_id.clone(), quota);
    }

    pub fn quota(&self, id: &str) -> Option<ProviderQuota> {
        self.lock().quotas.get(id).cloned()
    }

    pub fn expire_quota(&self, quota_id: &str) -> Result<(), EconomyError> {
        let mut g = self.lock();
        let q = g.quotas.get_mut(quota_id).ok_or(EconomyError::UnknownQuota)?;
        q.expired = true;
        q.available = 0;
        q.reserved = 0;
        Ok(())
    }

    pub fn submit_contribution(
        &self,
        contribution_id: impl Into<String>,
        contributor: impl Into<AccountId>,
        resource_type: ResourceType,
        provider: Option<String>,
        model: Option<String>,
        advertisement_id: Option<String>,
        quota_id: Option<String>,
    ) -> ContributionRecord {
        let now = now_ms();
        let rec = ContributionRecord {
            contribution_id: contribution_id.into(),
            contributor: contributor.into(),
            resource_type,
            provider,
            model,
            advertisement_id,
            quota_id,
            state: ContributionState::Pending,
            usage: None,
            calculation: None,
            reject_reason: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let mut g = self.lock();
        g.contributions
            .insert(rec.contribution_id.clone(), rec.clone());
        rec
    }

    pub fn verify_contribution(
        &self,
        contribution_id: &str,
        usage: VerifiedUsage,
    ) -> Result<ContributionRecord, EconomyError> {
        let mut g = self.lock();
        if g.receipts.contains(&usage.receipt_id) {
            return Err(EconomyError::DuplicateReceipt);
        }
        let rec = g
            .contributions
            .get_mut(contribution_id)
            .ok_or(EconomyError::UnknownContribution)?;
        if rec.state != ContributionState::Pending {
            return Err(EconomyError::InvalidState);
        }
        if !usage.signature_valid {
            rec.state = ContributionState::Rejected;
            rec.reject_reason = Some("forged or unverified receipt".into());
            rec.updated_at_ms = now_ms();
            return Err(EconomyError::ForgedReceipt);
        }
        if !usage.measurement.can_settle() {
            rec.state = ContributionState::Rejected;
            rec.reject_reason = Some("measurement cannot settle".into());
            rec.updated_at_ms = now_ms();
            return Err(EconomyError::UnverifiedMeasurement);
        }
        if !usage.success {
            rec.state = ContributionState::Rejected;
            rec.usage = Some(usage);
            rec.reject_reason = Some("failed execution".into());
            rec.updated_at_ms = now_ms();
            let out = rec.clone();
            g.record_reject(&out);
            return Ok(out);
        }
        if usage.contributor != rec.contributor {
            rec.state = ContributionState::Rejected;
            rec.reject_reason = Some("contributor mismatch".into());
            rec.updated_at_ms = now_ms();
            return Err(EconomyError::InvalidState);
        }
        g.receipts.insert(usage.receipt_id.clone());
        rec.usage = Some(usage);
        rec.state = ContributionState::Verified;
        rec.updated_at_ms = now_ms();
        Ok(rec.clone())
    }

    pub fn settle_contribution(
        &self,
        contribution_id: &str,
    ) -> Result<CreditCalculation, EconomyError> {
        self.settle_with(contribution_id, &mut InternalSettlement)
    }

    pub fn settle_with(
        &self,
        contribution_id: &str,
        settlement: &mut dyn SettlementEngine,
    ) -> Result<CreditCalculation, EconomyError> {
        let mut g = self.lock();
        if !g.applied.insert(("settle".into(), contribution_id.to_string())) {
            let rec = g
                .contributions
                .get(contribution_id)
                .ok_or(EconomyError::UnknownContribution)?;
            if rec.state == ContributionState::Settled {
                return rec
                    .calculation
                    .clone()
                    .ok_or(EconomyError::DuplicateSettlement);
            }
            return Err(EconomyError::DuplicateSettlement);
        }
        let policy = g.policy.clone();
        let rec = g
            .contributions
            .get(contribution_id)
            .ok_or(EconomyError::UnknownContribution)?
            .clone();
        if rec.state != ContributionState::Verified {
            g.applied.remove(&("settle".into(), contribution_id.to_string()));
            return Err(EconomyError::InvalidState);
        }
        let usage = rec.usage.as_ref().ok_or(EconomyError::InvalidState)?;
        if let Some(qid) = &rec.quota_id {
            let q = g.quotas.get_mut(qid).ok_or(EconomyError::UnknownQuota)?;
            if q.expired {
                g.applied.remove(&("settle".into(), contribution_id.to_string()));
                return Err(EconomyError::QuotaExpired);
            }
            let used = match rec.resource_type {
                ResourceType::ApiQuota => usage.total_tokens(),
                ResourceType::GpuCompute => usage.gpu_ms,
                ResourceType::CpuCompute => usage.cpu_ms,
                ResourceType::Storage => usage.storage_byte_hours,
                ResourceType::Bandwidth => usage.bandwidth_bytes,
            };
            if q.available < used {
                g.applied.remove(&("settle".into(), contribution_id.to_string()));
                if q.available == 0 {
                    return Err(EconomyError::QuotaExhausted);
                }
                return Err(EconomyError::InsufficientQuota {
                    available: q.available,
                    requested: used,
                });
            }
            q.available = q.available.saturating_sub(used);
            q.consumed = q.consumed.saturating_add(used);
        }
        let calc = policy.calculate(usage);
        if calc.credits == 0 {
            let rec_mut = g.contributions.get_mut(contribution_id).unwrap();
            rec_mut.state = ContributionState::Rejected;
            rec_mut.reject_reason = Some("zero credit under policy".into());
            rec_mut.updated_at_ms = now_ms();
            return Err(EconomyError::InvalidState);
        }
        let acc = g.accounts.entry(rec.contributor.clone()).or_default();
        acc.earned = acc.earned.saturating_add(calc.credits);
        acc.available = acc.available.saturating_add(calc.credits);
        let event = CreditEvent {
            op: CreditOp::Earn,
            account: rec.contributor.clone(),
            amount: calc.credits,
            ref_id: contribution_id.to_string(),
            contribution_id: Some(contribution_id.to_string()),
            receipt_id: Some(usage.receipt_id.clone()),
            execution_id: Some(usage.execution_id.clone()),
            origin_resource: Some(rec.resource_type),
            origin_provider: rec.provider.clone(),
            origin_model: rec.model.clone(),
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: None,
            external_ref: None,
            policy_version: calc.policy_version,
            created_at_ms: now_ms(),
            settled: true,
        };
        g.push_event(event.clone());
        let rec_mut = g.contributions.get_mut(contribution_id).unwrap();
        rec_mut.state = ContributionState::Settled;
        rec_mut.calculation = Some(calc.clone());
        rec_mut.updated_at_ms = now_ms();
        drop(g);
        settlement.on_settled(&event)?;
        Ok(calc)
    }

    pub fn contribution(&self, id: &str) -> Option<ContributionRecord> {
        self.lock().contributions.get(id).cloned()
    }

    pub fn balance(&self, account: &str) -> CreditBalance {
        self.lock()
            .accounts
            .get(account)
            .copied()
            .unwrap_or_default()
    }

    pub fn events(&self) -> Vec<CreditEvent> {
        self.lock().events.iter().cloned().collect()
    }

    pub fn reserve(
        &self,
        account: &str,
        reservation_id: &str,
        amount: u64,
        consume_resource: ResourceType,
        consume_provider: Option<String>,
        consume_model: Option<String>,
    ) -> Result<CreditReservation, EconomyError> {
        let mut g = self.lock();
        if let Some(existing) = g.reservations.get(reservation_id) {
            return Ok(existing.clone());
        }
        let acc = g.accounts.entry(account.to_string()).or_default();
        if acc.available < amount {
            return Err(EconomyError::InsufficientCredits {
                available: acc.available,
                requested: amount,
            });
        }
        acc.available = acc.available.saturating_sub(amount);
        acc.reserved = acc.reserved.saturating_add(amount);
        let res = CreditReservation {
            reservation_id: reservation_id.to_string(),
            account: account.to_string(),
            amount,
            consume_resource,
            consume_provider: consume_provider.clone(),
            consume_model: consume_model.clone(),
            settled: false,
        };
        g.reservations
            .insert(reservation_id.to_string(), res.clone());
        g.push_event(CreditEvent {
            op: CreditOp::Reserve,
            account: account.to_string(),
            amount,
            ref_id: reservation_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: Some(consume_resource),
            consume_provider,
            consume_model,
            settlement_rail: None,
            external_ref: None,
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: false,
        });
        Ok(res)
    }

    pub fn release(&self, reservation_id: &str) -> Result<(), EconomyError> {
        let mut g = self.lock();
        let res = g
            .reservations
            .get_mut(reservation_id)
            .ok_or(EconomyError::UnknownReservation)?;
        if res.settled {
            return Ok(());
        }
        res.settled = true;
        let account = res.account.clone();
        let amount = res.amount;
        let acc = g.accounts.entry(account.clone()).or_default();
        acc.reserved = acc.reserved.saturating_sub(amount);
        acc.available = acc.available.saturating_add(amount);
        g.push_event(CreditEvent {
            op: CreditOp::Release,
            account,
            amount,
            ref_id: reservation_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: None,
            external_ref: None,
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: true,
        });
        Ok(())
    }

    pub fn consume(
        &self,
        reservation_id: &str,
        used: u64,
    ) -> Result<u64, EconomyError> {
        let mut g = self.lock();
        let res = g
            .reservations
            .get_mut(reservation_id)
            .ok_or(EconomyError::UnknownReservation)?;
        if res.settled {
            return Err(EconomyError::AlreadySettled);
        }
        res.settled = true;
        let account = res.account.clone();
        let amount = res.amount;
        let consume_resource = res.consume_resource;
        let consume_provider = res.consume_provider.clone();
        let consume_model = res.consume_model.clone();
        let used = used.min(amount);
        let released = amount.saturating_sub(used);
        let acc = g.accounts.entry(account.clone()).or_default();
        acc.reserved = acc.reserved.saturating_sub(amount);
        acc.consumed = acc.consumed.saturating_add(used);
        acc.available = acc.available.saturating_add(released);
        g.push_event(CreditEvent {
            op: CreditOp::Consume,
            account,
            amount: used,
            ref_id: reservation_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: Some(consume_resource),
            consume_provider,
            consume_model,
            settlement_rail: None,
            external_ref: None,
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: true,
        });
        Ok(used)
    }

    // -----------------------------------------------------------------------
    // Crypto Readiness: Escrow / Lock State Machine for External Settlement
    // -----------------------------------------------------------------------

    /// Creates and locks CU in escrow for external settlement (e.g., bridge or off-ramp).
    pub fn create_settlement_intent(
        &self,
        intent_id: &str,
        account: &str,
        cu_amount: u64,
        target_rail: &str,
        external_recipient: &str,
    ) -> Result<SettlementIntent, EconomyError> {
        let mut g = self.lock();
        if g.settlement_intents.contains_key(intent_id) {
            return Err(EconomyError::InvalidSettlementState);
        }
        let acc = g.accounts.entry(account.to_string()).or_default();
        if acc.available < cu_amount {
            return Err(EconomyError::InsufficientCredits {
                available: acc.available,
                requested: cu_amount,
            });
        }
        acc.available = acc.available.saturating_sub(cu_amount);
        acc.locked_for_settlement = acc.locked_for_settlement.saturating_add(cu_amount);

        let intent = SettlementIntent {
            intent_id: intent_id.to_string(),
            account: account.to_string(),
            cu_amount,
            target_rail: target_rail.to_string(),
            external_recipient: external_recipient.to_string(),
            status: SettlementStatus::Locked,
            external_tx_id: None,
            created_at_ms: now_ms(),
            finalized_at_ms: None,
            failure_reason: None,
        };
        g.settlement_intents.insert(intent_id.to_string(), intent.clone());
        g.push_event(CreditEvent {
            op: CreditOp::LockSettlement,
            account: account.to_string(),
            amount: cu_amount,
            ref_id: intent_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: Some(target_rail.to_string()),
            external_ref: Some(external_recipient.to_string()),
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: false,
        });
        Ok(intent)
    }

    /// Finalizes an external settlement once external transaction confirmation is proven.
    /// Deducts locked CU permanently into consumed state.
    pub fn finalize_settlement(
        &self,
        intent_id: &str,
        external_tx_id: &str,
    ) -> Result<SettlementIntent, EconomyError> {
        let mut g = self.lock();
        let intent = g
            .settlement_intents
            .get_mut(intent_id)
            .ok_or(EconomyError::UnknownSettlementIntent)?;
        if intent.status != SettlementStatus::Locked && intent.status != SettlementStatus::Committed {
            return Err(EconomyError::InvalidSettlementState);
        }
        intent.status = SettlementStatus::Finalized;
        intent.external_tx_id = Some(external_tx_id.to_string());
        intent.finalized_at_ms = Some(now_ms());

        let account = intent.account.clone();
        let amount = intent.cu_amount;
        let rail = intent.target_rail.clone();
        let acc = g.accounts.entry(account.clone()).or_default();
        acc.locked_for_settlement = acc.locked_for_settlement.saturating_sub(amount);
        acc.consumed = acc.consumed.saturating_add(amount);

        let out = intent.clone();
        g.push_event(CreditEvent {
            op: CreditOp::FinalizeSettlement,
            account,
            amount,
            ref_id: intent_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: Some(rail),
            external_ref: Some(external_tx_id.to_string()),
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: true,
        });
        Ok(out)
    }

    /// Refunds locked CU back to spendable available pool if external settlement fails or times out.
    pub fn refund_settlement(
        &self,
        intent_id: &str,
        reason: &str,
    ) -> Result<SettlementIntent, EconomyError> {
        let mut g = self.lock();
        let intent = g
            .settlement_intents
            .get_mut(intent_id)
            .ok_or(EconomyError::UnknownSettlementIntent)?;
        if intent.status != SettlementStatus::Locked && intent.status != SettlementStatus::Committed {
            return Err(EconomyError::InvalidSettlementState);
        }
        intent.status = SettlementStatus::Refunded;
        intent.failure_reason = Some(reason.to_string());

        let account = intent.account.clone();
        let amount = intent.cu_amount;
        let rail = intent.target_rail.clone();
        let acc = g.accounts.entry(account.clone()).or_default();
        acc.locked_for_settlement = acc.locked_for_settlement.saturating_sub(amount);
        acc.available = acc.available.saturating_add(amount);

        let out = intent.clone();
        g.push_event(CreditEvent {
            op: CreditOp::RefundSettlement,
            account,
            amount,
            ref_id: intent_id.to_string(),
            contribution_id: None,
            receipt_id: None,
            execution_id: None,
            origin_resource: None,
            origin_provider: None,
            origin_model: None,
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: Some(rail),
            external_ref: Some(reason.to_string()),
            policy_version: g.policy.version,
            created_at_ms: now_ms(),
            settled: true,
        });
        Ok(out)
    }

    pub fn settlement_intent(&self, intent_id: &str) -> Option<SettlementIntent> {
        self.lock().settlement_intents.get(intent_id).cloned()
    }
}

impl Inner {
    fn push_event(&mut self, event: CreditEvent) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn record_reject(&mut self, rec: &ContributionRecord) {
        self.push_event(CreditEvent {
            op: CreditOp::Reject,
            account: rec.contributor.clone(),
            amount: 0,
            ref_id: rec.contribution_id.clone(),
            contribution_id: Some(rec.contribution_id.clone()),
            receipt_id: rec.usage.as_ref().map(|u| u.receipt_id.clone()),
            execution_id: rec.usage.as_ref().map(|u| u.execution_id.clone()),
            origin_resource: Some(rec.resource_type),
            origin_provider: rec.provider.clone(),
            origin_model: rec.model.clone(),
            consume_resource: None,
            consume_provider: None,
            consume_model: None,
            settlement_rail: None,
            external_ref: None,
            policy_version: self.policy.version,
            created_at_ms: now_ms(),
            settled: false,
        });
    }
}

// ---------------------------------------------------------------------------
// Tests — contribution → reward → balance → reservation → consumption → settlement
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn usage(receipt: &str, exec: &str, contributor: &str, input: u64, output: u64, ok: bool) -> VerifiedUsage {
        VerifiedUsage {
            receipt_id: receipt.into(),
            execution_id: exec.into(),
            contributor: contributor.into(),
            consumer: "consumer-b".into(),
            resource_type: ResourceType::ApiQuota,
            provider: Some("deepseek".into()),
            model: Some("deepseek-chat".into()),
            input_tokens: input,
            output_tokens: output,
            gpu_ms: 0,
            cpu_ms: 0,
            storage_byte_hours: 0,
            bandwidth_bytes: 0,
            success: ok,
            signature_valid: true,
            measurement: MeasurementMethod::SignedReceipt,
            reservation_id: None,
            measured_at_ms: 1,
        }
    }

    fn earn_api(eco: &InferenceCreditEconomy, id: &str, tokens_in: u64, tokens_out: u64) -> u64 {
        eco.submit_contribution(
            id,
            "node-a",
            ResourceType::ApiQuota,
            Some("deepseek".into()),
            Some("deepseek-chat".into()),
            Some("ad-1".into()),
            Some("q-1".into()),
        );
        eco.verify_contribution(id, usage(&format!("r-{id}"), id, "node-a", tokens_in, tokens_out, true))
            .unwrap();
        eco.settle_contribution(id).unwrap().credits
    }

    #[test]
    fn crypto_readiness_escrow_and_finalize_flow() {
        let eco = InferenceCreditEconomy::default();
        earn_api(&eco, "c1", 10_000, 0); // 10,000 CU
        
        // Lock 4,000 CU for external settlement
        let intent = eco
            .create_settlement_intent("intent-1", "node-a", 4_000, "multiversx", "erd1mockaddress")
            .unwrap();
        assert_eq!(intent.status, SettlementStatus::Locked);
        
        let bal = eco.balance("node-a");
        assert_eq!(bal.available, 6_000);
        assert_eq!(bal.locked_for_settlement, 4_000);
        assert!(bal.check_invariant());
        
        // Finalize settlement with external tx hash
        let fin = eco.finalize_settlement("intent-1", "0xdeadbeef1234").unwrap();
        assert_eq!(fin.status, SettlementStatus::Finalized);
        assert_eq!(fin.external_tx_id.as_deref(), Some("0xdeadbeef1234"));
        
        let bal_after = eco.balance("node-a");
        assert_eq!(bal_after.available, 6_000);
        assert_eq!(bal_after.locked_for_settlement, 0);
        assert_eq!(bal_after.consumed, 4_000);
        assert!(bal_after.check_invariant());
    }

    #[test]
    fn crypto_readiness_escrow_refund_on_failure() {
        let eco = InferenceCreditEconomy::default();
        earn_api(&eco, "c1", 10_000, 0);
        
        eco.create_settlement_intent("intent-2", "node-a", 3_000, "evm", "0x0000address").unwrap();
        assert_eq!(eco.balance("node-a").available, 7_000);
        
        // Timeout / chain failure -> refund to available
        let ref_intent = eco.refund_settlement("intent-2", "bridge timeout").unwrap();
        assert_eq!(ref_intent.status, SettlementStatus::Refunded);
        
        let bal = eco.balance("node-a");
        assert_eq!(bal.available, 10_000);
        assert_eq!(bal.locked_for_settlement, 0);
        assert!(bal.check_invariant());
    }
}
