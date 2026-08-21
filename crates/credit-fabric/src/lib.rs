//! Mutual-credit inference fabric (research track).
//!
//! Layered **on top of** `decentraai-credit-economy`. Not a cryptocurrency,
//! not wired into production scheduler/P2P/runtime.
//!
//! ```text
//! OpenCode / any OpenAI client
//!        ↓
//! Gateway envelope (this crate)
//!        ↓
//! Credit check + estimate
//!        ↓
//! Resource planner (eligible catalog)
//!        ↓
//! Session: reserve CU → execute elsewhere → receipt
//!        ↓
//! Two-sided settlement
//!   consumer: CU consumed
//!   contributor: CU earned  (durable, origin-agnostic)
//! ```
//!
//! The economic object is verified contribution, not provider quota.
//! Quota may expire; settled CU remain and may be spent on another resource.

pub mod provider_bridge;

pub use provider_bridge::{
    ConnectedProviderModel, CredentialVault, ProviderCreditBridge, ProviderExecutionOutcome,
    ProviderKind,
};

use decentraai_credit_economy::{
    AccountId, ContributionState, CreditBalance, CreditPolicy, EconomyError,
    InferenceCreditEconomy, MeasurementMethod, ProviderQuota, ResourceAdvertisement,
    ResourceType, VerifiedUsage,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Catalog — advertised capacity, never secrets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub advertisement: ResourceAdvertisement,
    pub quota_id: Option<String>,
    /// 0..=10_000; 10_000 = fully healthy.
    pub health_bps: u64,
    pub load_percent: u8,
    pub eligible_for_spend: bool,
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadNeed {
    pub account: AccountId,
    pub preferred_resource: ResourceType,
    pub preferred_provider: Option<String>,
    pub preferred_model: Option<String>,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub estimated_gpu_ms: u64,
    pub estimated_cpu_ms: u64,
    pub allow_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedResource {
    pub advertisement_id: String,
    pub contributor: AccountId,
    pub resource_type: ResourceType,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub quota_id: Option<String>,
    pub estimated_cu: u64,
    pub score: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRejection {
    pub advertisement_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePlan {
    pub selected: Option<PlannedResource>,
    pub rejected: Vec<PlanRejection>,
    pub estimated_cu: u64,
}

pub struct ResourcePlanner;

impl ResourcePlanner {
    pub fn estimate_cu(policy: &CreditPolicy, need: &WorkloadNeed) -> u64 {
        let synthetic = VerifiedUsage {
            receipt_id: "estimate".into(),
            execution_id: "estimate".into(),
            contributor: "estimate".into(),
            consumer: need.account.clone(),
            resource_type: need.preferred_resource,
            provider: need.preferred_provider.clone(),
            model: need.preferred_model.clone(),
            input_tokens: need.estimated_input_tokens,
            output_tokens: need.estimated_output_tokens,
            gpu_ms: need.estimated_gpu_ms,
            cpu_ms: need.estimated_cpu_ms,
            storage_byte_hours: 0,
            bandwidth_bytes: 0,
            success: true,
            signature_valid: true,
            measurement: MeasurementMethod::SignedReceipt,
            reservation_id: None,
            measured_at_ms: 0,
        };
        policy.calculate(&synthetic).credits.max(1)
    }

    pub fn plan(
        catalog: &[CatalogEntry],
        policy: &CreditPolicy,
        need: &WorkloadNeed,
        budget: CreditBalance,
    ) -> ResourcePlan {
        let estimated_cu = Self::estimate_cu(policy, need);
        let mut rejected = Vec::new();
        let mut candidates: Vec<PlannedResource> = Vec::new();

        for e in catalog {
            if !e.eligible_for_spend {
                rejected.push(PlanRejection {
                    advertisement_id: e.advertisement.advertisement_id.clone(),
                    reason: "not eligible for CU spend".into(),
                });
                continue;
            }
            if e.health_bps < 2_000 {
                rejected.push(PlanRejection {
                    advertisement_id: e.advertisement.advertisement_id.clone(),
                    reason: "unhealthy".into(),
                });
                continue;
            }
            let ad = &e.advertisement;
            let model_hit = need
                .preferred_model
                .as_ref()
                .map(|m| ad.model.as_ref() == Some(m))
                .unwrap_or(false);
            let provider_hit = need
                .preferred_provider
                .as_ref()
                .map(|p| ad.provider.as_ref() == Some(p))
                .unwrap_or(false);
            let type_hit = ad.resource_type == need.preferred_resource;
            if !type_hit && !need.allow_fallback {
                rejected.push(PlanRejection {
                    advertisement_id: ad.advertisement_id.clone(),
                    reason: "resource type mismatch".into(),
                });
                continue;
            }
            if !type_hit && !model_hit && !provider_hit && !need.allow_fallback {
                continue;
            }
            let mut score: u64 = 1_000;
            if type_hit {
                score += 5_000;
            }
            if model_hit {
                score += 8_000;
            }
            if provider_hit {
                score += 3_000;
            }
            score += e.health_bps / 10;
            score = score.saturating_sub(u64::from(e.load_percent) * 20);
            if ad.capacity_units > 0 {
                score += (ad.capacity_units.min(10_000)) / 100;
            }
            candidates.push(PlannedResource {
                advertisement_id: ad.advertisement_id.clone(),
                contributor: ad.contributor.clone(),
                resource_type: ad.resource_type,
                provider: ad.provider.clone(),
                model: ad.model.clone(),
                quota_id: e.quota_id.clone(),
                estimated_cu,
                score,
                reason: if model_hit {
                    "preferred model".into()
                } else if type_hit {
                    "preferred resource class".into()
                } else {
                    "fallback eligible resource".into()
                },
            });
        }
        candidates.sort_by(|a, b| b.score.cmp(&a.score));
        let selected = candidates.into_iter().next();
        if selected.is_none() {
            rejected.push(PlanRejection {
                advertisement_id: "*".into(),
                reason: "no eligible resource".into(),
            });
        } else if budget.available < estimated_cu {
            rejected.push(PlanRejection {
                advertisement_id: selected.as_ref().unwrap().advertisement_id.clone(),
                reason: format!(
                    "insufficient CU: need {estimated_cu}, available {}",
                    budget.available
                ),
            });
            return ResourcePlan {
                selected: None,
                rejected,
                estimated_cu,
            };
        }
        ResourcePlan {
            selected,
            rejected,
            estimated_cu,
        }
    }
}

// ---------------------------------------------------------------------------
// Sessions — estimate → reserve CU → execute → two-sided settle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Estimated,
    CreditReserved,
    Executing,
    Settled,
    Failed,
    Held,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSession {
    pub session_id: String,
    pub consumer: AccountId,
    pub contributor: AccountId,
    pub planned: PlannedResource,
    pub credit_reservation_id: String,
    pub contribution_id: String,
    pub state: SessionState,
    pub estimated_cu: u64,
    pub earned_cu: u64,
    pub spent_cu: u64,
    pub created_at_ms: u64,
    pub hold_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReceipt {
    pub session_id: String,
    pub consumer: AccountId,
    pub contributor: AccountId,
    pub spent_cu: u64,
    pub earned_cu: u64,
    pub origin_resource: ResourceType,
    pub origin_provider: Option<String>,
    pub consume_resource: ResourceType,
    pub consume_provider: Option<String>,
}

// ---------------------------------------------------------------------------
// Gateway envelope (OpenAI-compatible conceptually, not an HTTP server)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayChatNeed {
    pub account: AccountId,
    pub model: String,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPlan {
    pub session_id: String,
    pub model: String,
    pub provider: Option<String>,
    pub resource_type: ResourceType,
    pub estimated_cu: u64,
    pub reservation_id: String,
    pub rejected: Vec<PlanRejection>,
}

// ---------------------------------------------------------------------------
// Abuse
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbusePolicy {
    pub deny_self_deal: bool,
    pub max_cu_per_session: u64,
    pub max_open_sessions_per_account: u32,
}

impl Default for AbusePolicy {
    fn default() -> Self {
        Self {
            deny_self_deal: true,
            max_cu_per_session: 1_000_000,
            max_open_sessions_per_account: 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FabricError {
    Economy(EconomyError),
    NoEligibleResource,
    InsufficientCredits,
    SelfDealingDenied,
    SessionLimit,
    SessionCuCap,
    UnknownSession,
    InvalidSessionState,
    AmbiguousHold,
}

impl From<EconomyError> for FabricError {
    fn from(e: EconomyError) -> Self {
        match e {
            EconomyError::InsufficientCredits { .. } => Self::InsufficientCredits,
            other => Self::Economy(other),
        }
    }
}

impl std::fmt::Display for FabricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Economy(e) => write!(f, "{e}"),
            Self::NoEligibleResource => write!(f, "no eligible resource"),
            Self::InsufficientCredits => write!(f, "insufficient CU"),
            Self::SelfDealingDenied => write!(f, "self-dealing denied"),
            Self::SessionLimit => write!(f, "too many open sessions"),
            Self::SessionCuCap => write!(f, "session CU cap exceeded"),
            Self::UnknownSession => write!(f, "unknown session"),
            Self::InvalidSessionState => write!(f, "invalid session state"),
            Self::AmbiguousHold => write!(f, "session held for reconciliation"),
        }
    }
}

impl std::error::Error for FabricError {}

// ---------------------------------------------------------------------------
// Fabric
// ---------------------------------------------------------------------------

pub struct CreditFabric {
    economy: InferenceCreditEconomy,
    catalog: Mutex<Vec<CatalogEntry>>,
    sessions: Mutex<HashMap<String, ExecutionSession>>,
    abuse: AbusePolicy,
}

impl CreditFabric {
    pub fn new(policy: CreditPolicy, abuse: AbusePolicy) -> Self {
        Self {
            economy: InferenceCreditEconomy::new(policy),
            catalog: Mutex::new(Vec::new()),
            sessions: Mutex::new(HashMap::new()),
            abuse,
        }
    }

    pub fn economy(&self) -> &InferenceCreditEconomy {
        &self.economy
    }

    fn catalog_lock(&self) -> std::sync::MutexGuard<'_, Vec<CatalogEntry>> {
        self.catalog.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn sessions_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ExecutionSession>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Offer capacity. Secrets in credential_ref are rejected by the ledger.
    pub fn offer_capacity(
        &self,
        ad: ResourceAdvertisement,
        quota_available: u64,
        reset_at_ms: Option<u64>,
    ) -> Result<(), FabricError> {
        let quota_id = format!("quota-{}", ad.advertisement_id);
        self.economy.advertise(ad.clone())?;
        self.economy.register_quota(ProviderQuota {
            quota_id: quota_id.clone(),
            contributor: ad.contributor.clone(),
            resource_type: ad.resource_type,
            provider: ad.provider.clone(),
            model: ad.model.clone(),
            available: quota_available,
            reserved: 0,
            consumed: 0,
            reset_at_ms,
            expired: false,
        });
        self.catalog_lock().push(CatalogEntry {
            advertisement: ad,
            quota_id: Some(quota_id),
            health_bps: 10_000,
            load_percent: 0,
            eligible_for_spend: true,
        });
        Ok(())
    }

    /// Provider daily window reset. Does **not** touch settled CU.
    pub fn reset_provider_window(
        &self,
        quota_id: &str,
        new_available: u64,
        reset_at_ms: Option<u64>,
    ) -> Result<(), FabricError> {
        let Some(existing) = self.economy.quota(quota_id) else {
            return Err(FabricError::Economy(EconomyError::UnknownQuota));
        };
        self.economy.register_quota(ProviderQuota {
            available: new_available,
            reserved: 0,
            consumed: 0,
            expired: false,
            reset_at_ms,
            ..existing
        });
        Ok(())
    }

    pub fn plan(&self, need: &WorkloadNeed) -> ResourcePlan {
        let cat = self.catalog_lock().clone();
        let policy = self.economy.policy();
        let budget = self.economy.balance(&need.account);
        ResourcePlanner::plan(&cat, &policy, need, budget)
    }

    fn open_count(&self, account: &str) -> u32 {
        self.sessions_lock()
            .values()
            .filter(|s| {
                s.consumer == account
                    && matches!(
                        s.state,
                        SessionState::CreditReserved | SessionState::Executing | SessionState::Held
                    )
            })
            .count() as u32
    }

    /// Estimate, plan, reserve consumer CU. Execution happens outside this crate.
    pub fn open_session(&self, session_id: &str, need: WorkloadNeed) -> Result<ExecutionSession, FabricError> {
        if self.open_count(&need.account) >= self.abuse.max_open_sessions_per_account {
            return Err(FabricError::SessionLimit);
        }
        let plan = self.plan(&need);
        let selected = plan.selected.clone().ok_or(FabricError::NoEligibleResource)?;
        if selected.estimated_cu > self.abuse.max_cu_per_session {
            return Err(FabricError::SessionCuCap);
        }
        if self.abuse.deny_self_deal && selected.contributor == need.account {
            return Err(FabricError::SelfDealingDenied);
        }
        self.economy.reserve(
            &need.account,
            session_id,
            selected.estimated_cu,
            selected.resource_type,
            selected.provider.clone(),
            selected.model.clone(),
        )?;
        let sess = ExecutionSession {
            session_id: session_id.into(),
            consumer: need.account.clone(),
            contributor: selected.contributor.clone(),
            credit_reservation_id: session_id.into(),
            contribution_id: format!("contrib-{session_id}"),
            planned: selected.clone(),
            state: SessionState::CreditReserved,
            estimated_cu: selected.estimated_cu,
            earned_cu: 0,
            spent_cu: 0,
            created_at_ms: now_ms(),
            hold_reason: None,
        };
        self.sessions_lock()
            .insert(session_id.to_string(), sess.clone());
        Ok(sess)
    }

    pub fn mark_executing(&self, session_id: &str) -> Result<(), FabricError> {
        let mut g = self.sessions_lock();
        let s = g.get_mut(session_id).ok_or(FabricError::UnknownSession)?;
        if s.state != SessionState::CreditReserved {
            return Err(FabricError::InvalidSessionState);
        }
        s.state = SessionState::Executing;
        Ok(())
    }

    pub fn fail_session(&self, session_id: &str) -> Result<(), FabricError> {
        {
            let mut g = self.sessions_lock();
            let s = g.get_mut(session_id).ok_or(FabricError::UnknownSession)?;
            if matches!(s.state, SessionState::Settled | SessionState::Failed) {
                return Ok(());
            }
            s.state = SessionState::Failed;
        }
        let _ = self.economy.release(session_id);
        Ok(())
    }

    /// Two-sided settlement from a verified receipt projection.
    /// Contributor earns durable CU; consumer spends CU. Origin ≠ spend target.
    pub fn complete_session(
        &self,
        session_id: &str,
        mut usage: VerifiedUsage,
    ) -> Result<SessionReceipt, FabricError> {
        let sess = self
            .sessions_lock()
            .get(session_id)
            .cloned()
            .ok_or(FabricError::UnknownSession)?;
        if !matches!(
            sess.state,
            SessionState::CreditReserved | SessionState::Executing
        ) {
            return Err(FabricError::InvalidSessionState);
        }
        if self.abuse.deny_self_deal && usage.contributor == sess.consumer {
            self.fail_session(session_id)?;
            return Err(FabricError::SelfDealingDenied);
        }
        if !usage.success {
            self.fail_session(session_id)?;
            return Err(FabricError::InvalidSessionState);
        }
        usage.consumer = sess.consumer.clone();
        usage.contributor = sess.contributor.clone();
        usage.resource_type = sess.planned.resource_type;
        usage.provider = sess.planned.provider.clone();
        usage.model = sess.planned.model.clone();
        usage.reservation_id = Some(sess.credit_reservation_id.clone());

        self.economy.submit_contribution(
            &sess.contribution_id,
            &sess.contributor,
            sess.planned.resource_type,
            sess.planned.provider.clone(),
            sess.planned.model.clone(),
            Some(sess.planned.advertisement_id.clone()),
            sess.planned.quota_id.clone(),
        );
        match self.economy.verify_contribution(&sess.contribution_id, usage.clone()) {
            Ok(rec) if rec.state == ContributionState::Verified => {}
            Ok(_) => {
                self.fail_session(session_id)?;
                return Err(FabricError::InvalidSessionState);
            }
            Err(e) => {
                self.fail_session(session_id)?;
                return Err(e.into());
            }
        }
        let earned = match self.economy.settle_contribution(&sess.contribution_id) {
            Ok(c) => c.credits,
            Err(e) => {
                self.fail_session(session_id)?;
                return Err(e.into());
            }
        };
        if earned > sess.estimated_cu {
            let mut g = self.sessions_lock();
            if let Some(s) = g.get_mut(session_id) {
                s.state = SessionState::Held;
                s.earned_cu = earned;
                s.hold_reason = Some("actual CU exceeded reservation".into());
            }
            return Err(FabricError::AmbiguousHold);
        }
        let spent = self.economy.consume(session_id, earned)?;
        let receipt = SessionReceipt {
            session_id: session_id.into(),
            consumer: sess.consumer.clone(),
            contributor: sess.contributor.clone(),
            spent_cu: spent,
            earned_cu: earned,
            origin_resource: sess.planned.resource_type,
            origin_provider: sess.planned.provider.clone(),
            consume_resource: sess.planned.resource_type,
            consume_provider: sess.planned.provider.clone(),
        };
        let mut g = self.sessions_lock();
        if let Some(s) = g.get_mut(session_id) {
            s.state = SessionState::Settled;
            s.earned_cu = earned;
            s.spent_cu = spent;
        }
        Ok(receipt)
    }

    /// OpenAI-compatible conceptual entry: credit check + plan + reserve.
    pub fn gateway_chat(&self, req: GatewayChatNeed) -> Result<GatewayPlan, FabricError> {
        let need = WorkloadNeed {
            account: req.account,
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: None,
            preferred_model: Some(req.model.clone()),
            estimated_input_tokens: req.estimated_input_tokens,
            estimated_output_tokens: req.estimated_output_tokens,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: true,
        };
        let session_id = format!("gw-{}", now_ms());
        let sess = self.open_session(&session_id, need)?;
        Ok(GatewayPlan {
            session_id: sess.session_id,
            model: req.model,
            provider: sess.planned.provider,
            resource_type: sess.planned.resource_type,
            estimated_cu: sess.estimated_cu,
            reservation_id: sess.credit_reservation_id,
            rejected: Vec::new(),
        })
    }

    pub fn session(&self, id: &str) -> Option<ExecutionSession> {
        self.sessions_lock().get(id).cloned()
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.catalog_lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_credit_economy::CreditPolicy;

    fn ad(
        id: &str,
        contributor: &str,
        rt: ResourceType,
        provider: &str,
        model: &str,
        cap: u64,
    ) -> ResourceAdvertisement {
        ResourceAdvertisement {
            advertisement_id: id.into(),
            contributor: contributor.into(),
            resource_type: rt,
            provider: Some(provider.into()),
            model: Some(model.into()),
            capacity_units: cap,
            available_from_ms: None,
            available_until_ms: None,
            rate_limit_per_minute: Some(60),
            concurrency_limit: Some(4),
            measurement: MeasurementMethod::SignedReceipt,
            region: Some("eu".into()),
            capabilities: vec!["chat".into()],
            credential_ref: Some("env:PROVIDER_KEY".into()),
        }
    }

    fn usage(receipt: &str, exec: &str, contributor: &str, input: u64, output: u64) -> VerifiedUsage {
        VerifiedUsage {
            receipt_id: receipt.into(),
            execution_id: exec.into(),
            contributor: contributor.into(),
            consumer: "consumer".into(),
            resource_type: ResourceType::ApiQuota,
            provider: Some("deepseek".into()),
            model: Some("deepseek-chat".into()),
            input_tokens: input,
            output_tokens: output,
            gpu_ms: 0,
            cpu_ms: 0,
            storage_byte_hours: 0,
            bandwidth_bytes: 0,
            success: true,
            signature_valid: true,
            measurement: MeasurementMethod::SignedReceipt,
            reservation_id: None,
            measured_at_ms: 1,
        }
    }

    fn seed_consumer(fabric: &CreditFabric, account: &str, input: u64) {
        // Bootstrap CU by a prior verified GPU contribution (different resource).
        fabric
            .economy()
            .submit_contribution("boot", account, ResourceType::GpuCompute, None, None, None, None);
        let mut u = usage("boot-r", "boot", account, 0, 0);
        u.resource_type = ResourceType::GpuCompute;
        u.gpu_ms = input;
        u.provider = Some("local".into());
        fabric.economy().verify_contribution("boot", u).unwrap();
        fabric.economy().settle_contribution("boot").unwrap();
    }

    #[test]
    fn two_sided_session_deepseek_to_qwen_later() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("ds", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, Some(9_999))
            .unwrap();
        seed_consumer(&fabric, "node-b", 80_000);

        let need = WorkloadNeed {
            account: "node-b".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 1_000,
            estimated_output_tokens: 500,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        let sess = fabric.open_session("s1", need).unwrap();
        assert_eq!(sess.state, SessionState::CreditReserved);
        assert_eq!(sess.contributor, "node-a");
        fabric.mark_executing("s1").unwrap();
        let rec = fabric
            .complete_session("s1", usage("r-s1", "e-s1", "node-a", 1_000, 500))
            .unwrap();
        assert_eq!(rec.earned_cu, 2_000); // 1000*1 + 500*2
        assert_eq!(rec.spent_cu, 2_000);
        assert_eq!(fabric.economy().balance("node-a").available, 2_000);
        assert!(fabric.economy().balance("node-b").consumed >= 2_000);

        // Quota expires; A's CU remain and can buy a different resource.
        fabric.economy().expire_quota("quota-ds").unwrap();
        fabric
            .offer_capacity(ad("qwen", "node-c", ResourceType::ApiQuota, "qwen", "qwen-plus", 50_000), 50_000, None)
            .unwrap();
        let spend = WorkloadNeed {
            account: "node-a".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("qwen".into()),
            preferred_model: Some("qwen-plus".into()),
            estimated_input_tokens: 100,
            estimated_output_tokens: 50,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        let s2 = fabric.open_session("s2", spend).unwrap();
        assert_eq!(s2.planned.provider.as_deref(), Some("qwen"));
        fabric
            .complete_session("s2", usage("r-s2", "e-s2", "node-c", 100, 50))
            .unwrap();
        assert!(fabric.economy().balance("node-c").earned > 0);
        assert_eq!(fabric.session("s2").unwrap().state, SessionState::Settled);
    }

    #[test]
    fn gpu_earn_spent_on_remote_gpu() {
        let mut abuse = AbusePolicy::default();
        abuse.deny_self_deal = true;
        let fabric = CreditFabric::new(CreditPolicy::default(), abuse);
        fabric
            .offer_capacity(ad("gpu-a", "node-a", ResourceType::GpuCompute, "local", "llama", 1_000), 1_000, None)
            .unwrap();
        fabric
            .offer_capacity(ad("gpu-b", "node-b", ResourceType::GpuCompute, "remote", "llama", 1_000), 1_000, None)
            .unwrap();
        seed_consumer(&fabric, "node-c", 5_000);
        let need = WorkloadNeed {
            account: "node-c".into(),
            preferred_resource: ResourceType::GpuCompute,
            preferred_provider: Some("remote".into()),
            preferred_model: Some("llama".into()),
            estimated_input_tokens: 0,
            estimated_output_tokens: 0,
            estimated_gpu_ms: 400,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        fabric.open_session("g1", need).unwrap();
        let mut u = usage("rg", "g1", "node-b", 0, 0);
        u.resource_type = ResourceType::GpuCompute;
        u.gpu_ms = 400;
        u.provider = Some("remote".into());
        let rec = fabric.complete_session("g1", u).unwrap();
        assert_eq!(rec.earned_cu, 400);
        assert_eq!(rec.consume_provider.as_deref(), Some("remote"));
    }

    #[test]
    fn planner_falls_back_and_rejects_unhealthy() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("a", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 10), 10, None)
            .unwrap();
        {
            let mut cat = fabric.catalog_lock();
            cat[0].health_bps = 100;
        }
        fabric
            .offer_capacity(ad("b", "node-b", ResourceType::GpuCompute, "remote", "llama", 10), 10, None)
            .unwrap();
        seed_consumer(&fabric, "node-c", 10_000);
        let need = WorkloadNeed {
            account: "node-c".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 10,
            estimated_output_tokens: 0,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: true,
        };
        let plan = fabric.plan(&need);
        assert!(plan.selected.is_some());
        assert_eq!(plan.selected.unwrap().advertisement_id, "b");
    }

    #[test]
    fn self_deal_denied() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("a", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 10), 10, None)
            .unwrap();
        seed_consumer(&fabric, "node-a", 10_000);
        let need = WorkloadNeed {
            account: "node-a".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 10,
            estimated_output_tokens: 0,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        let err = fabric.open_session("x", need).unwrap_err();
        assert_eq!(err, FabricError::SelfDealingDenied);
    }

    #[test]
    fn failed_execution_releases_consumer_cu() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("a", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 10_000), 10_000, None)
            .unwrap();
        seed_consumer(&fabric, "node-b", 10_000);
        let before = fabric.economy().balance("node-b").available;
        let need = WorkloadNeed {
            account: "node-b".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 10,
            estimated_output_tokens: 0,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        fabric.open_session("f1", need).unwrap();
        let mut u = usage("rf", "f1", "node-a", 10, 0);
        u.success = false;
        assert!(fabric.complete_session("f1", u).is_err());
        assert_eq!(fabric.economy().balance("node-b").available, before);
        assert_eq!(fabric.economy().balance("node-a").earned, 0);
        assert_eq!(fabric.session("f1").unwrap().state, SessionState::Failed);
    }

    #[test]
    fn quota_reset_does_not_erase_cu() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("a", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100), 100, None)
            .unwrap();
        seed_consumer(&fabric, "node-b", 10_000);
        let need = WorkloadNeed {
            account: "node-b".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 10,
            estimated_output_tokens: 0,
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        fabric.open_session("q1", need).unwrap();
        fabric
            .complete_session("q1", usage("rq", "q1", "node-a", 10, 0))
            .unwrap();
        let cu = fabric.economy().balance("node-a").available;
        assert!(cu > 0);
        fabric.reset_provider_window("quota-a", 100, Some(123)).unwrap();
        assert_eq!(fabric.economy().quota("quota-a").unwrap().available, 100);
        assert_eq!(fabric.economy().balance("node-a").available, cu);
    }

    #[test]
    fn gateway_chat_reserves_before_execution() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        fabric
            .offer_capacity(ad("a", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, None)
            .unwrap();
        seed_consumer(&fabric, "node-b", 50_000);
        let plan = fabric
            .gateway_chat(GatewayChatNeed {
                account: "node-b".into(),
                model: "deepseek-chat".into(),
                estimated_input_tokens: 20,
                estimated_output_tokens: 10,
                stream: true,
            })
            .unwrap();
        assert_eq!(plan.resource_type, ResourceType::ApiQuota);
        assert!(plan.estimated_cu > 0);
        assert_eq!(fabric.session(&plan.session_id).unwrap().state, SessionState::CreditReserved);
    }

    #[test]
    fn advertisement_still_rejects_raw_keys() {
        let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
        let mut bad = ad("x", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 1);
        bad.credential_ref = Some("sk-secret".into());
        assert!(fabric.offer_capacity(bad, 1, None).is_err());
    }
}
