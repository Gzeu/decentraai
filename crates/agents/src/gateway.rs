//! M16 Agent Gateway, step 1 — types + authorization invariant.
//!
//! PURE ONLY: no issuance, no storage, no HTTP, no MCP mutation, no
//! economic execution. This module answers ONE question deterministically:
//! given a gateway credential, a requested capability + operation class,
//! and the current time — Allow or Deny, and the exact stable reason.
//!
//! ```text
//! AI proposes → authorize() decides → (later steps) evidence + execution
//! ```
//!
//! Step-1 boundary discipline (see `docs/M16_AGENT_GATEWAY_SCOPE.md`):
//! - `GatewayCredential::new` is a shape validator, NOT issuance: no
//!   secrets are generated here, nothing is stored, no randomness.
//!   Issuance (secret generation + hashed store + CLI ceremony) is step 3.
//! - Only `Denied` / `ExpiredOrRevoked` can arise here; `RetryLater` /
//!   `Rejected` / `Failed` belong to execution steps (quota, Governor,
//!   runtime) and are defined now so the failure set stays closed.
//! - Testnet-economic and privileged classes can NEVER be allowed for
//!   agent credentials — structurally, regardless of grants.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Policy constants M16 v0.1 — security specification, NOT measured
/// values (see scope §3). Tunable only through a scope revision.
pub const GATEWAY_TTL_MIN_DAYS: u64 = 1;
/// Policy constants M16 v0.1 — see above.
pub const GATEWAY_TTL_DEFAULT_DAYS: u64 = 7;
/// Policy constants M16 v0.1 — see above.
pub const GATEWAY_TTL_MAX_DAYS: u64 = 30;

/// Seconds per day (TTL bound arithmetic).
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Stable denial reasons (exact strings are part of the contract:
/// expired and revoked share ONE reason — no oracle).
pub const REASON_INACTIVE: &str = "credential inactive or expired";
/// Stable denial reasons — see above.
pub const REASON_CLASS_DENIED: &str = "operation class not granted to agent credentials";
/// Stable denial reasons — see above.
pub const REASON_NOT_GRANTED: &str = "capability not granted";
/// Stable denial reasons — see above.
pub const REASON_CEILING: &str = "operation class above credential ceiling";

/// Operation classes (§4 of the scope). Ordered by privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    /// Reads, planning projections, self-inspection.
    ReadOnly,
    /// Quota-gated mutation through authorized capabilities.
    Compute,
    /// Signing/broadcasting/settling. NEVER reachable from the gateway.
    TestnetEconomic,
    /// Key/config/admin control planes. Master only, unchanged.
    Privileged,
}

/// Credential kinds — the strict ladder (§2). Each rung inherits NOTHING
/// upward; this enum only names rungs, it grants nothing by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// NEW BYOA agent credential (this module authorizes exactly these).
    Agent,
    /// Existing `dca_` consumer key (unchanged behavior).
    Consumer,
    /// Existing wallet session (read-only in MCP, unchanged).
    WalletSession,
    /// Operator/master (control plane, unchanged, never delegable).
    OperatorMaster,
}

impl CredentialKind {
    /// Highest operation class this kind may EVER reach. Agent credentials
    /// top out at Compute — economic and privileged are structurally out.
    #[must_use]
    pub fn ceiling(&self) -> OperationClass {
        match self {
            CredentialKind::Agent | CredentialKind::Consumer => OperationClass::Compute,
            CredentialKind::WalletSession => OperationClass::ReadOnly,
            CredentialKind::OperatorMaster => OperationClass::Privileged,
        }
    }
}

/// Closed failure set (§8). Step 1 produces only `Denied` and
/// `ExpiredOrRevoked`; the rest are defined now so later steps cannot
/// invent new failure modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayFailure {
    /// Policy/capability/ceiling denial (reason in `AuthDecision::Deny`).
    Denied,
    /// Expired OR revoked — one variant, one reason, no oracle.
    ExpiredOrRevoked,
    /// Rate/single-flight/Governor-Queue (later steps).
    RetryLater,
    /// Governor-Reject/overload (later steps).
    Rejected,
    /// Execution error with evidence (later steps).
    Failed,
}

/// A BYOA agent credential (shape only — no secret lives here; secrets
/// are generated and hashed at issuance in step 3, never in this type).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCredential {
    /// Stable credential id (revocation handle; safe to log).
    pub id: String,
    /// Agent display name (`1..=32`, `[a-zA-Z0-9_-]`).
    pub agent_name: String,
    /// Granted taxonomy capabilities (non-empty; each parses as
    /// `CapabilityKind`).
    pub capabilities: BTreeSet<String>,
    /// Quota ceiling (opaque units; enforced by the ledger in step 4).
    pub quota_ceiling: u64,
    /// Issuance time (unix seconds).
    pub issued_at: u64,
    /// Expiry time (unix seconds; `issued_at < expires_at`, duration
    /// within `[MIN, MAX]` days).
    pub expires_at: u64,
    /// Revocation flag (checked at auth time on every call).
    #[serde(default)]
    pub revoked: bool,
}

/// Construction/validation failures. Variants name the FIELD, never secrets
/// (this type carries none, but the rule holds for step 3 reuse).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    /// Credential id is empty.
    #[error("credential id must not be empty")]
    EmptyId,
    /// Agent name violates `1..=32 [a-zA-Z0-9_-]`.
    #[error("agent name must be 1..=32 chars [a-zA-Z0-9_-]")]
    InvalidName,
    /// No capabilities granted (default-deny needs a non-empty grant).
    #[error("capability set must not be empty")]
    EmptyCapabilities,
    /// A granted name is not in the `CapabilityKind` taxonomy.
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    /// Quota ceiling is zero.
    #[error("quota ceiling must be > 0")]
    InvalidQuota,
    /// Expiry is not after issuance.
    #[error("expires_at must be after issued_at")]
    InvalidExpiry,
    /// TTL duration outside `[MIN, MAX]` days policy bounds.
    #[error("ttl days must be within [1, 30]")]
    TtlOutOfRange,
}

impl GatewayCredential {
    /// Shape-validating constructor (NOT issuance: no secrets, no store,
    /// no randomness — step 3 owns all of that).
    pub fn new(
        id: &str,
        agent_name: &str,
        capabilities: BTreeSet<String>,
        quota_ceiling: u64,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Self, GatewayError> {
        if id.is_empty() {
            return Err(GatewayError::EmptyId);
        }
        if agent_name.is_empty()
            || agent_name.len() > 32
            || !agent_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(GatewayError::InvalidName);
        }
        if capabilities.is_empty() {
            return Err(GatewayError::EmptyCapabilities);
        }
        for cap in &capabilities {
            if cap
                .parse::<decentraai_hub::capability::CapabilityKind>()
                .is_err()
            {
                return Err(GatewayError::UnknownCapability(cap.clone()));
            }
        }
        if quota_ceiling == 0 {
            return Err(GatewayError::InvalidQuota);
        }
        if expires_at <= issued_at {
            return Err(GatewayError::InvalidExpiry);
        }
        let ttl_days = (expires_at - issued_at) / SECONDS_PER_DAY;
        validate_ttl_days(ttl_days)?;
        Ok(Self {
            id: id.to_string(),
            agent_name: agent_name.to_string(),
            capabilities,
            quota_ceiling,
            issued_at,
            expires_at,
            revoked: false,
        })
    }
}

/// Validate a TTL in whole days against the v0.1 policy bounds.
pub fn validate_ttl_days(days: u64) -> Result<(), GatewayError> {
    if (GATEWAY_TTL_MIN_DAYS..=GATEWAY_TTL_MAX_DAYS).contains(&days) {
        Ok(())
    } else {
        Err(GatewayError::TtlOutOfRange)
    }
}

/// The authorization verdict. `Allow` carries exactly what was granted;
/// `Deny` carries a STABLE reason string (contract, tested).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// Request granted: this capability, this class.
    Allow {
        /// Granted capability name.
        capability: String,
        /// Granted operation class.
        class: OperationClass,
    },
    /// Request denied: stable machine-readable reason.
    Deny {
        /// One of the `REASON_*` constants.
        reason: &'static str,
    },
}

/// The deterministic authorization boundary (§5). Pure: credential +
/// request + time in, verdict out. Deny is the default branch — every
/// allow path below lists ALL of its conditions.
///
/// Order (tested): inactivity (revoked/expired, one reason) → class ban
/// (economic/privileged can never pass) → grant membership → ceiling.
#[must_use]
pub fn authorize(
    credential: &GatewayCredential,
    capability: &str,
    class: OperationClass,
    now_unix: u64,
) -> AuthDecision {
    if credential.revoked || now_unix >= credential.expires_at {
        return AuthDecision::Deny {
            reason: REASON_INACTIVE,
        };
    }
    if class == OperationClass::TestnetEconomic || class == OperationClass::Privileged {
        return AuthDecision::Deny {
            reason: REASON_CLASS_DENIED,
        };
    }
    if !credential.capabilities.contains(capability) {
        return AuthDecision::Deny {
            reason: REASON_NOT_GRANTED,
        };
    }
    if class > CredentialKind::Agent.ceiling() {
        return AuthDecision::Deny {
            reason: REASON_CEILING,
        };
    }
    AuthDecision::Allow {
        capability: capability.to_string(),
        class,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> GatewayCredential {
        GatewayCredential::new(
            "cred-1",
            "ext-agent",
            ["ocr".to_string()].into_iter().collect(),
            100,
            1_700_000_000,
            1_700_000_000 + 7 * SECONDS_PER_DAY,
        )
        .unwrap()
    }

    #[test]
    fn fresh_grant_allows_exactly_what_was_granted() {
        let c = credential();
        assert_eq!(
            authorize(&c, "ocr", OperationClass::ReadOnly, 1_700_000_100),
            AuthDecision::Allow {
                capability: "ocr".to_string(),
                class: OperationClass::ReadOnly
            }
        );
        assert_eq!(
            authorize(&c, "ocr", OperationClass::Compute, 1_700_000_100),
            AuthDecision::Allow {
                capability: "ocr".to_string(),
                class: OperationClass::Compute
            }
        );
        // Granted one capability: neighbors deny (least privilege).
        assert_eq!(
            authorize(&c, "chat", OperationClass::ReadOnly, 1_700_000_100),
            AuthDecision::Deny {
                reason: REASON_NOT_GRANTED
            }
        );
        assert_eq!(
            authorize(&c, "", OperationClass::ReadOnly, 1_700_000_100),
            AuthDecision::Deny {
                reason: REASON_NOT_GRANTED
            }
        );
    }

    #[test]
    fn economic_and_privileged_never_pass_for_agents() {
        let c = credential();
        // Even the GRANTED capability is denied in banned classes.
        for class in [OperationClass::TestnetEconomic, OperationClass::Privileged] {
            assert_eq!(
                authorize(&c, "ocr", class, 1_700_000_100),
                AuthDecision::Deny {
                    reason: REASON_CLASS_DENIED
                }
            );
        }
    }

    #[test]
    fn expired_and_revoked_share_one_reason_no_oracle() {
        let mut expired = credential();
        let at = expired.expires_at;
        // Exactly at expiry: dead (same boundary the store enforces).
        let d_expired = authorize(&expired, "ocr", OperationClass::ReadOnly, at);
        expired.revoked = true;
        let d_revoked = authorize(&expired, "ocr", OperationClass::ReadOnly, at - 10);
        assert_eq!(d_expired, d_revoked);
        assert_eq!(
            d_expired,
            AuthDecision::Deny {
                reason: REASON_INACTIVE
            }
        );
    }

    #[test]
    fn constructor_rejects_bad_shapes() {
        let caps: BTreeSet<String> = ["ocr".to_string()].into_iter().collect();
        assert_eq!(
            GatewayCredential::new("", "a", caps.clone(), 1, 10, 20).unwrap_err(),
            GatewayError::EmptyId
        );
        assert_eq!(
            GatewayCredential::new("id", "", caps.clone(), 1, 10, 20).unwrap_err(),
            GatewayError::InvalidName
        );
        assert_eq!(
            GatewayCredential::new("id", "bad name!", caps.clone(), 1, 10, 20).unwrap_err(),
            GatewayError::InvalidName
        );
        assert_eq!(
            GatewayCredential::new("id", "a", BTreeSet::new(), 1, 10, 20).unwrap_err(),
            GatewayError::EmptyCapabilities
        );
        assert_eq!(
            GatewayCredential::new(
                "id",
                "a",
                ["teleport".to_string()].into_iter().collect(),
                1,
                10,
                20
            )
            .unwrap_err(),
            GatewayError::UnknownCapability("teleport".to_string())
        );
        assert_eq!(
            GatewayCredential::new("id", "a", caps.clone(), 0, 10, 20).unwrap_err(),
            GatewayError::InvalidQuota
        );
        assert_eq!(
            GatewayCredential::new("id", "a", caps.clone(), 1, 20, 20).unwrap_err(),
            GatewayError::InvalidExpiry
        );
        // 40-day duration exceeds the absolute max.
        assert_eq!(
            GatewayCredential::new("id", "a", caps, 1, 0, 40 * SECONDS_PER_DAY).unwrap_err(),
            GatewayError::TtlOutOfRange
        );
    }

    #[test]
    fn ttl_policy_bounds_are_exact() {
        assert!(validate_ttl_days(0).is_err());
        assert!(validate_ttl_days(1).is_ok());
        assert!(validate_ttl_days(7).is_ok());
        assert!(validate_ttl_days(30).is_ok());
        assert!(validate_ttl_days(31).is_err());
        assert_eq!(
            GATEWAY_TTL_DEFAULT_DAYS, 7,
            "policy constant v0.1 — change needs scope revision"
        );
        assert_eq!(
            GATEWAY_TTL_MAX_DAYS, 30,
            "policy constant v0.1 — change needs scope revision"
        );
    }

    #[test]
    fn ladder_ceilings_hold() {
        assert_eq!(CredentialKind::Agent.ceiling(), OperationClass::Compute);
        assert_eq!(CredentialKind::Consumer.ceiling(), OperationClass::Compute);
        assert_eq!(
            CredentialKind::WalletSession.ceiling(),
            OperationClass::ReadOnly
        );
        assert_eq!(
            CredentialKind::OperatorMaster.ceiling(),
            OperationClass::Privileged
        );
    }

    #[test]
    fn credential_round_trips_for_future_store() {
        // Step 3 will persist this shape; the closed schema must hold now.
        let c = credential();
        let back: GatewayCredential =
            serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
        assert!(serde_json::to_string(&c).unwrap().contains("\"ocr\""));
    }
}
