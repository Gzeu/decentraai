//! SAES 0.5 Phase 3 — Agent Gateway / BYOA.
//!
//! The typed, deterministic **decision layer** for an external generic agent
//! to enter the fabric through a scoped `dca_` credential.
//!
//! ```text
//! external agent → scoped credential → onboarding → capability declaration
//!       → task entry → quota reserve → placement → execution → settlement → learning
//! ```
//!
//! * **No second gateway / quota system / task protocol.** Every gate reuses
//!   the existing primitives: `dca_` shape + scopes, `quota_ledger`
//!   `reserve/settle/release` (modelled here as a pure reservation decision;
//!   the runtime's `ConsumerQuotaGuard` owns the real ledger), Hub
//!   `Task → Bid → Team → Execute`, `placement::select_placement`, DFCP, and
//!   `EventBus` with a single `correlation_id` for the lifecycle.
//! * **Generic agent:** no hardcoded Cline/OpenClaw/Pylon/model. Capabilities
//!   are free-form `String` (hub taxonomy snake_case).
//! * **Pure:** every function is `&self`/`&T` → decision, no I/O, no libp2p.
//!   The runtime (`LocalAgentRuntime`) applies the decision, touches the
//!   ledger/Hub, and emits the correlated event.
//! * **Deterministic:** no randomness; tie-breaks identical to placement.
//! * **Fail-safe:** reservation is only emitted when `available>0` and
//!   `available.min(ceiling)>0`; settlement consumes real measured usage;
//!   failure always releases; every step emits an explainable event.

use serde::{Deserialize, Serialize};

use super::placement::{PlacementDecision, PlacementOffer, select_placement};
use super::pressure::CollaborationSignal;

/// Scoped credential an external agent presents. Mirrors the persisted
/// `ConsumerKeyRecord` fields the gateway cares about — no secret here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayCredential {
    /// Stable key id, e.g. `ck-ab12`.
    pub key_id: String,
    /// Owner account in the `QuotaLedger`.
    pub account: String,
    /// Scopes granted to this key, e.g. `["inference","hub"]`.
    pub scopes: Vec<String>,
    /// Per-request quota ceiling.
    pub quota_ceiling: u64,
    /// Rate limit per minute (informational; enforced by runtime).
    pub rate_limit_per_minute: u32,
}

/// Session established after onboarding. Carries the `correlation_id` that
/// threads the whole lifecycle `gateway → placement → execution → settlement`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySession {
    /// External agent's logical id (free-form, not a libp2p peer id).
    pub agent_id: String,
    pub key_id: String,
    pub account: String,
    pub scopes: Vec<String>,
    pub quota_ceiling: u64,
    /// Correlation id for the entire agent episode (one uuid per session).
    pub correlation_id: String,
    /// Capabilities declared by the agent during onboarding.
    pub declared_capabilities: Vec<String>,
    pub onboarded_at_ms: u64,
}

/// Why a gateway step was rejected — fully enumerated for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayRejection {
    InvalidKeyPrefix,
    EmptyAccount,
    ZeroCeiling,
    MissingScope { required: String },
    EmptyCapabilities,
    InvalidCapability { index: usize, reason: String },
    NoSpendableQuota { account: String },
    PlacementFailed { reasons: Vec<String> },
}

impl std::fmt::Display for GatewayRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKeyPrefix => write!(f, "key must start with dca_"),
            Self::EmptyAccount => write!(f, "account must not be empty"),
            Self::ZeroCeiling => write!(f, "quota_ceiling must be >0"),
            Self::MissingScope { required } => write!(f, "missing required scope: {required}"),
            Self::EmptyCapabilities => write!(f, "at least one capability required"),
            Self::InvalidCapability { index, reason } => {
                write!(f, "capability[{index}] invalid: {reason}")
            }
            Self::NoSpendableQuota { account } => {
                write!(f, "no spendable quota for account {account}")
            }
            Self::PlacementFailed { reasons } => {
                write!(f, "placement failed: {}", reasons.join(", "))
            }
        }
    }
}

/// Pure reservation decision — what the ledger *would* book, without touching it.
/// The runtime turns this into a real `ConsumerQuotaGuard::reserve` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaReservationPlan {
    pub reservation_id: String,
    pub account: String,
    pub amount: u64,
    /// Whether the plan is bookable (amount>0). `false` means DENIED, not an error.
    pub bookable: bool,
}

/// Execution settlement outcome (pure). `consumed` is the measured usage the
/// caller observed (e.g. token count). The runtime settles the guard with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementKind {
    Settled { consumed: u64, released: u64 },
    Released,
}

/// The full gateway execution plan: credential → session → reservation → placement.
/// Produced purely so tests drive it with synthetics; the runtime applies it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayExecutionPlan {
    pub session: GatewaySession,
    pub reservation: QuotaReservationPlan,
    pub placement: PlacementDecision,
}

/// Validate a `dca_` credential shape (no secret checked here — that is
/// `ConsumerKeyStore::lookup` in the runtime). Pure and fast.
pub fn validate_credential_shape(plaintext: &str) -> Result<(), GatewayRejection> {
    if !plaintext.starts_with("dca_") {
        return Err(GatewayRejection::InvalidKeyPrefix);
    }
    if plaintext.len() < 8 {
        return Err(GatewayRejection::InvalidKeyPrefix);
    }
    Ok(())
}

pub fn validate_credential(cred: &GatewayCredential) -> Result<(), GatewayRejection> {
    if cred.account.trim().is_empty() {
        return Err(GatewayRejection::EmptyAccount);
    }
    if cred.quota_ceiling == 0 {
        return Err(GatewayRejection::ZeroCeiling);
    }
    if cred.key_id.trim().is_empty() {
        return Err(GatewayRejection::InvalidKeyPrefix);
    }
    Ok(())
}

pub fn require_scope(cred: &GatewayCredential, scope: &str) -> Result<(), GatewayRejection> {
    if cred.scopes.iter().any(|s| s == scope) {
        Ok(())
    } else {
        Err(GatewayRejection::MissingScope {
            required: scope.to_string(),
        })
    }
}

pub fn validate_capabilities(caps: &[String]) -> Result<(), GatewayRejection> {
    if caps.is_empty() {
        return Err(GatewayRejection::EmptyCapabilities);
    }
    for (i, c) in caps.iter().enumerate() {
        if c.trim().is_empty() {
            return Err(GatewayRejection::InvalidCapability {
                index: i,
                reason: "empty".into(),
            });
        }
        if c.len() > 128 {
            return Err(GatewayRejection::InvalidCapability {
                index: i,
                reason: "too long".into(),
            });
        }
    }
    Ok(())
}

/// Create a `GatewaySession` from a validated credential. The caller supplies
/// `now_ms` and `correlation_id` so the decision stays pure.
pub fn create_session(
    agent_id: &str,
    cred: &GatewayCredential,
    declared_capabilities: Vec<String>,
    correlation_id: String,
    now_ms: u64,
) -> Result<GatewaySession, GatewayRejection> {
    validate_credential(cred)?;
    validate_capabilities(&declared_capabilities)?;
    Ok(GatewaySession {
        agent_id: agent_id.to_string(),
        key_id: cred.key_id.clone(),
        account: cred.account.clone(),
        scopes: cred.scopes.clone(),
        quota_ceiling: cred.quota_ceiling,
        correlation_id,
        declared_capabilities,
        onboarded_at_ms: now_ms,
    })
}

/// Pure reservation sizing: `amount = min(available, ceiling)`; bookable iff >0.
pub fn plan_reservation(
    session: &GatewaySession,
    task_id: &str,
    available: u64,
) -> QuotaReservationPlan {
    let amount = available.min(session.quota_ceiling);
    let reservation_id = format!("consumer:{}:{}", session.key_id, task_id);
    QuotaReservationPlan {
        reservation_id,
        account: session.account.clone(),
        amount,
        bookable: amount > 0,
    }
}

/// Pure gateway + placement planning: credential → session → reservation → placement.
/// Returns the full plan; the runtime decides to book or deny from `reservation.bookable`
/// and from `placement.placed`.
pub fn plan_gateway_execution(
    session: &GatewaySession,
    task_id: &str,
    task_capability: &str,
    available: u64,
    offers: &[PlacementOffer],
) -> GatewayExecutionPlan {
    let reservation = plan_reservation(session, task_id, available);
    // Build a minimal CollaborationSignal for placement — the gateway reuses
    // the placement decision layer verbatim (no second engine).
    let signal = CollaborationSignal {
        agent_id: session.agent_id.clone(),
        capability: task_capability.to_string(),
        reasons: vec!["gateway".to_string()],
        urgency: super::pressure::Urgency::Elevated,
        correlation_id: session.correlation_id.clone(),
        cpu_cores: 0,
        ram_mb: 0,
        max_lease_seconds: 30,
    };
    let placement = select_placement(&signal, offers.iter());
    GatewayExecutionPlan {
        session: session.clone(),
        reservation,
        placement,
    }
}

/// Pure settlement sizing: how much of the reservation was consumed vs released.
pub fn plan_settlement(reservation_amount: u64, measured_consumed: u64) -> SettlementKind {
    if reservation_amount == 0 {
        return SettlementKind::Released;
    }
    let consumed = measured_consumed.min(reservation_amount);
    let released = reservation_amount.saturating_sub(consumed);
    SettlementKind::Settled { consumed, released }
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("gw-{:016x}-{:04x}", now, now & 0xffff)
}

pub fn new_correlation_id() -> String {
    uuid_simple()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saes::placement::PlacementOffer;

    fn cred() -> GatewayCredential {
        GatewayCredential {
            key_id: "ck-ab12".into(),
            account: "ext-agent".into(),
            scopes: vec!["hub".into(), "inference".into()],
            quota_ceiling: 100,
            rate_limit_per_minute: 60,
        }
    }

    fn session() -> GatewaySession {
        GatewaySession {
            agent_id: "agent-ext-1".into(),
            key_id: "ck-ab12".into(),
            account: "ext-agent".into(),
            scopes: vec!["hub".into()],
            quota_ceiling: 100,
            correlation_id: "gw-test-1".into(),
            declared_capabilities: vec!["embeddings".into()],
            onboarded_at_ms: 0,
        }
    }

    #[test]
    fn credential_shape_validation() {
        assert!(validate_credential_shape("dca_abcdef1234567890").is_ok());
        assert_eq!(
            validate_credential_shape("dsk_abc"),
            Err(GatewayRejection::InvalidKeyPrefix)
        );
        assert!(validate_credential_shape("").is_err());
    }

    #[test]
    fn validate_credential_rejects_bad() {
        let mut c = cred();
        c.account = "".into();
        assert_eq!(validate_credential(&c), Err(GatewayRejection::EmptyAccount));
        let mut c2 = cred();
        c2.quota_ceiling = 0;
        assert_eq!(validate_credential(&c2), Err(GatewayRejection::ZeroCeiling));
    }

    #[test]
    fn scope_gate() {
        let c = cred();
        assert!(require_scope(&c, "hub").is_ok());
        assert_eq!(
            require_scope(&c, "admin"),
            Err(GatewayRejection::MissingScope {
                required: "admin".into()
            })
        );
    }

    #[test]
    fn capabilities_validation() {
        assert!(validate_capabilities(&["a".into()]).is_ok());
        assert_eq!(
            validate_capabilities(&[]),
            Err(GatewayRejection::EmptyCapabilities)
        );
        assert!(matches!(
            validate_capabilities(&["".into()]),
            Err(GatewayRejection::InvalidCapability { .. })
        ));
    }

    #[test]
    fn create_session_ok() {
        let s = create_session("agent-1", &cred(), vec!["ocr".into()], "gw-1".into(), 0).unwrap();
        assert_eq!(s.agent_id, "agent-1");
        assert_eq!(s.correlation_id, "gw-1");
    }

    #[test]
    fn plan_reservation_caps_at_ceiling() {
        let s = session();
        let r = plan_reservation(&s, "task-1", 500);
        assert_eq!(r.amount, 100); // capped
        assert!(r.bookable);
        assert_eq!(r.reservation_id, "consumer:ck-ab12:task-1");
        let r2 = plan_reservation(&s, "task-1", 0);
        assert!(!r2.bookable);
        assert_eq!(r2.amount, 0);
        let r3 = plan_reservation(&s, "task-1", 20);
        assert_eq!(r3.amount, 20);
    }

    #[test]
    fn gateway_execution_reuses_placement() {
        let s = session();
        let offers = vec![
            PlacementOffer {
                peer_id: "aaa".into(),
                capability: "embeddings".into(),
                cpu_cores: 2,
                ram_mb: 512,
                lease_seconds: 60,
                sampled_ago_secs: 5,
                queue_depth: 0,
                contribution_balance: 0,
                has_recent_failure: false,
            },
            PlacementOffer {
                peer_id: "zzz-giver".into(),
                capability: "embeddings".into(),
                cpu_cores: 2,
                ram_mb: 512,
                lease_seconds: 60,
                sampled_ago_secs: 5,
                queue_depth: 0,
                contribution_balance: 200,
                has_recent_failure: false,
            },
        ];
        let plan = plan_gateway_execution(&s, "task-1", "embeddings", 100, &offers);
        assert!(plan.reservation.bookable);
        assert!(plan.placement.placed);
        assert_eq!(plan.placement.selected_peer.as_deref(), Some("zzz-giver"));
        assert_eq!(plan.placement.correlation_id, "gw-test-1");
        // Wrong capability → placement fails but reservation still bookable (decoupled)
        let plan2 = plan_gateway_execution(&s, "task-2", "ocr", 100, &offers);
        assert!(!plan2.placement.placed);
    }

    #[test]
    fn settlement_math() {
        assert_eq!(
            plan_settlement(100, 40),
            SettlementKind::Settled {
                consumed: 40,
                released: 60
            }
        );
        assert_eq!(
            plan_settlement(100, 200),
            SettlementKind::Settled {
                consumed: 100,
                released: 0
            }
        );
        assert_eq!(plan_settlement(0, 10), SettlementKind::Released);
    }

    #[test]
    fn correlation_id_generator() {
        let id = new_correlation_id();
        assert!(id.starts_with("gw-"));
    }

    /// Full lifecycle E2E via LocalAgentRuntime: onboard → reserve+place → settle → EventBus.
    #[tokio::test]
    async fn gateway_full_lifecycle_via_runtime() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use decentraai_event_bus::{EventBus, EventFilter, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus.clone(), obs);

        let cred = GatewayCredential {
            key_id: "ck-test".into(),
            account: "ext-generic".into(),
            scopes: vec!["hub".into(), "inference".into()],
            quota_ceiling: 50,
            rate_limit_per_minute: 60,
        };
        // Onboard generic external agent with dca_ key shape
        let session = runtime
            .gateway_onboard(
                "agent-generic-1",
                "dca_abcdef1234567890abcdef",
                cred,
                vec!["embeddings".into()],
                "hub",
            )
            .await
            .expect("onboard must succeed");
        assert_eq!(session.agent_id, "agent-generic-1");
        assert!(session.correlation_id.starts_with("gw-"));

        // Reserve + place (available 100 → capped at 50, two offers, giver wins)
        let offers = vec![
            PlacementOffer {
                peer_id: "aaa".into(),
                capability: "embeddings".into(),
                cpu_cores: 2,
                ram_mb: 512,
                lease_seconds: 60,
                sampled_ago_secs: 5,
                queue_depth: 0,
                contribution_balance: 0,
                has_recent_failure: false,
            },
            PlacementOffer {
                peer_id: "zzz-giver".into(),
                capability: "embeddings".into(),
                cpu_cores: 2,
                ram_mb: 512,
                lease_seconds: 60,
                sampled_ago_secs: 2,
                queue_depth: 0,
                contribution_balance: 150,
                has_recent_failure: false,
            },
        ];
        let plan = runtime
            .gateway_reserve_and_place(&session, "task-42", "embeddings", 100, offers)
            .await;
        assert!(plan.reservation.bookable);
        assert_eq!(plan.reservation.amount, 50);
        assert!(plan.placement.placed);
        assert_eq!(plan.placement.selected_peer.as_deref(), Some("zzz-giver"));
        assert_eq!(plan.placement.correlation_id, session.correlation_id);
        assert_eq!(plan.reservation.reservation_id, "consumer:ck-test:task-42");

        // Simulate execution succeeded with 30 tokens consumed → settle 30, release 20
        let kind = runtime
            .gateway_settle(&session, &plan.reservation, 30, true)
            .await;
        assert_eq!(
            kind,
            SettlementKind::Settled {
                consumed: 30,
                released: 20
            }
        );

        // Failure path releases everything
        let kind2 = runtime
            .gateway_settle(&session, &plan.reservation, 0, false)
            .await;
        assert_eq!(kind2, SettlementKind::Released);

        // EventBus contains the whole chain with same correlation_id
        let events = bus.get_events(EventFilter::default(), 50).await.unwrap();
        let types: Vec<_> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"agent.gateway.onboarded"));
        assert!(types.contains(&"agent.gateway.reserved"));
        assert!(types.contains(&"agent.gateway.placed"));
        assert!(types.contains(&"agent.gateway.settled"));
        assert!(types.contains(&"agent.gateway.released"));
        for ev in events
            .iter()
            .filter(|e| e.event_type.starts_with("agent.gateway"))
        {
            assert_eq!(
                ev.metadata.correlation_id.as_deref(),
                Some(session.correlation_id.as_str()),
                "every gateway event shares the session correlation_id"
            );
        }

        // Quota denied when no spendable
        let denied = runtime
            .gateway_reserve_and_place(&session, "task-43", "embeddings", 0, vec![])
            .await;
        assert!(!denied.reservation.bookable);
        assert_eq!(denied.reservation.amount, 0);
    }

    #[tokio::test]
    async fn gateway_rejects_missing_scope_and_empty_caps() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use decentraai_event_bus::{EventBus, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus, obs);

        let cred = GatewayCredential {
            key_id: "ck-test".into(),
            account: "ext-generic".into(),
            scopes: vec!["inference".into()], // missing hub
            quota_ceiling: 50,
            rate_limit_per_minute: 60,
        };
        let err = runtime
            .gateway_onboard(
                "agent-generic-2",
                "dca_validkey123456",
                cred,
                vec!["embeddings".into()],
                "hub",
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            GatewayRejection::MissingScope {
                required: "hub".into()
            }
        );

        let cred2 = GatewayCredential {
            key_id: "ck-test".into(),
            account: "ext-generic".into(),
            scopes: vec!["hub".into()],
            quota_ceiling: 50,
            rate_limit_per_minute: 60,
        };
        let err2 = runtime
            .gateway_onboard(
                "agent-generic-2",
                "dca_validkey123456",
                cred2,
                vec![], // empty caps
                "hub",
            )
            .await
            .unwrap_err();
        assert_eq!(err2, GatewayRejection::EmptyCapabilities);
    }
}
