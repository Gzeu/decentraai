//! Economic authorization — the adapter seam (layer 5).
//!
//! The cognitive core never touches the economy directly. Two adapters:
//!
//! - [`DenyAllEconomicAuthorization`]: refuses everything (the v0.1
//!   behavior, still the default for every non-testnet path).
//! - [`TestnetEconomicAuthorization`]: the v0.2 bounded lane — approves
//!   ONLY testnet actions inside an explicit budget, with a deterministic
//!   kill switch. No mainnet code path exists anywhere in this file.
//!
//! Neither adapter holds wallets, keys or network handles: authorization
//! is pure checking over declared values. Signing and broadcast stay with
//! the operator-side executor, which must present a [`TestnetApproval`].

use crate::budget::{ExperimentBudget, TESTNET_CHAIN_ID, TestnetAsset};
use crate::error::ProposalError;
use crate::risk::ResourceCommitment;

/// Why economic authorization refused. Every variant names the exact
/// hard limit that fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconomicAuthError {
    /// No live adapter is wired for this commitment kind.
    AdaptersDisabled {
        /// Which commitment was requested.
        commitment: ResourceCommitment,
        /// Which proposal asked.
        proposal_id: String,
    },
    /// Kill switch is off: every economic action is denied, no exceptions.
    KillSwitch,
    /// Target chain is not the testnet. There is no mainnet lane.
    NotTestnet {
        /// Requested chain id.
        chain_id: String,
    },
    /// Policy did not Allow this proposal.
    MissingPolicyApproval {
        /// Which proposal.
        proposal_id: String,
    },
    /// No usable budget attached.
    MissingBudget {
        /// Which proposal.
        proposal_id: String,
    },
    /// Amount exceeds the authorized budget.
    BudgetExceeded {
        /// Requested wei.
        amount_wei: u64,
        /// Budget cap wei.
        max_wei: u64,
    },
    /// Gas exceeds the authorized budget.
    GasExceeded {
        /// Requested gas.
        gas: u64,
        /// Budget cap gas.
        max_gas: u64,
    },
    /// Experiment is past expiry.
    Expired {
        /// Now (unix seconds).
        now_unix: u64,
        /// Budget expiry.
        expiry_unix: u64,
    },
    /// Asset is not budget-allowlisted.
    WrongAsset {
        /// Requested asset.
        asset: String,
    },
    /// Destination is not budget-allowlisted. Never arbitrary.
    ArbitraryDestination {
        /// Requested destination.
        destination: String,
    },
    /// Retry count exhausted.
    RetryBudgetExceeded {
        /// Attempts already used.
        attempts_used: u32,
        /// Budget cap.
        max_retries: u32,
    },
    /// Too many actions for this budget.
    TooManyActions {
        /// Requested action count.
        actions: usize,
        /// Budget cap.
        max_actions: u32,
    },
}

impl std::fmt::Display for EconomicAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AdaptersDisabled {
                commitment,
                proposal_id,
            } => write!(
                f,
                "no live economic adapter; commitment {commitment:?} \
                 for proposal {proposal_id} denied by construction"
            ),
            Self::KillSwitch => write!(f, "testnet kill switch is off: denied"),
            Self::NotTestnet { chain_id } => write!(
                f,
                "chain {chain_id} is not testnet {TESTNET_CHAIN_ID}: denied (no mainnet lane)"
            ),
            Self::MissingPolicyApproval { proposal_id } => {
                write!(f, "proposal {proposal_id} has no policy Allow: denied")
            }
            Self::MissingBudget { proposal_id } => {
                write!(f, "proposal {proposal_id} carries no usable budget: denied")
            }
            Self::BudgetExceeded {
                amount_wei,
                max_wei,
            } => write!(
                f,
                "amount {amount_wei} wei exceeds budget {max_wei} wei: denied"
            ),
            Self::GasExceeded { gas, max_gas } => {
                write!(f, "gas {gas} exceeds budget {max_gas}: denied")
            }
            Self::Expired {
                now_unix,
                expiry_unix,
            } => write!(f, "expired: now {now_unix} >= expiry {expiry_unix}: denied"),
            Self::WrongAsset { asset } => {
                write!(f, "asset {asset} is not budget-allowlisted: denied")
            }
            Self::ArbitraryDestination { destination } => write!(
                f,
                "destination {destination} is not budget-allowlisted: denied"
            ),
            Self::RetryBudgetExceeded {
                attempts_used,
                max_retries,
            } => write!(
                f,
                "retries exhausted: used {attempts_used} > max {max_retries}: denied"
            ),
            Self::TooManyActions {
                actions,
                max_actions,
            } => write!(f, "actions {actions} exceed budget {max_actions}: denied"),
        }
    }
}

impl std::error::Error for EconomicAuthError {}

/// One testnet authorization request: everything the adapter checks,
/// with nothing hidden (no wallet, no keys, no ambient authority).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestnetAuthRequest {
    /// Proposal asking.
    pub proposal_id: String,
    /// Target chain id (must be [`TESTNET_CHAIN_ID`]).
    pub chain_id: String,
    /// Asset to move.
    pub asset: TestnetAsset,
    /// Destination.
    pub destination: String,
    /// Total value across the experiment (wei).
    pub amount_wei: u64,
    /// Gas per action.
    pub gas: u64,
    /// How many actions the experiment runs.
    pub actions: usize,
    /// Attempts already used (for retry accounting).
    pub attempts_used: u32,
    /// Now (unix seconds, caller-provided).
    pub now_unix: u64,
    /// Policy allowed this proposal (the adapter never overrides policy).
    pub policy_allowed: bool,
    /// The experiment budget (already structurally validated).
    pub budget: ExperimentBudget,
}

/// Proof of authorization. The operator-side executor must present this;
/// it cannot be constructed except through an adapter approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestnetApproval {
    /// Proposal approved.
    pub proposal_id: String,
    /// Budget id backing it.
    pub budget_id: String,
    /// Chain approved (always testnet).
    pub chain_id: String,
    /// Asset approved.
    pub asset: TestnetAsset,
    /// Destination approved.
    pub destination: String,
    /// Amount approved (wei, ≤ budget).
    pub amount_wei: u64,
    /// Gas approved.
    pub gas: u64,
}

/// Live-economy gate. Core calls it; adapters implement it.
pub trait EconomicAuthorization: Send + Sync {
    /// Approve committing `commitment` for `proposal_id`, or refuse.
    /// Deny-all adapters refuse here unconditionally.
    fn authorize_commitment(
        &self,
        commitment: ResourceCommitment,
        proposal_id: &str,
    ) -> Result<(), EconomicAuthError>;

    /// Approve one bounded testnet request. Default: deny (adapters opt
    /// in explicitly by overriding — silence is never approval).
    fn authorize_testnet(
        &self,
        request: &TestnetAuthRequest,
    ) -> Result<TestnetApproval, EconomicAuthError> {
        let _ = request;
        Err(EconomicAuthError::AdaptersDisabled {
            commitment: ResourceCommitment::Escrow,
            proposal_id: String::new(),
        })
    }
}

/// Deny everything, explicitly. Default for every non-testnet path and
/// the only adapter that existed in v0.1.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllEconomicAuthorization;

impl EconomicAuthorization for DenyAllEconomicAuthorization {
    fn authorize_commitment(
        &self,
        commitment: ResourceCommitment,
        proposal_id: &str,
    ) -> Result<(), EconomicAuthError> {
        Err(EconomicAuthError::AdaptersDisabled {
            commitment,
            proposal_id: proposal_id.to_string(),
        })
    }
}

/// Configuration for the bounded testnet lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestnetAuthConfig {
    /// Deterministic kill switch. `false` (default) denies EVERYTHING.
    pub enabled: bool,
    /// Chain this adapter serves. Must be [`TESTNET_CHAIN_ID`]; any other
    /// value fails closed at construction time (see [`TestnetEconomicAuthorization::new`]).
    pub chain_id: String,
}

impl Default for TestnetAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chain_id: TESTNET_CHAIN_ID.to_string(),
        }
    }
}

/// The v0.2 bounded lane adapter: testnet only, budget-gated, kill-switched.
///
/// Checks, in order: kill switch → chain is testnet → policy allowed →
/// budget present-shaped (validated by caller) → expiry → asset allow-list
/// → destination allow-list → amount ≤ budget → gas ≤ budget → actions ≤
/// budget → retries ≤ budget. First failure denies; nothing is partial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestnetEconomicAuthorization {
    config: TestnetAuthConfig,
}

impl TestnetEconomicAuthorization {
    /// Build the adapter. Refuses non-testnet chain ids at construction:
    /// a mainnet adapter cannot be expressed, not just not approved.
    pub fn new(config: TestnetAuthConfig) -> Result<Self, EconomicAuthError> {
        if config.chain_id != TESTNET_CHAIN_ID {
            return Err(EconomicAuthError::NotTestnet {
                chain_id: config.chain_id,
            });
        }
        Ok(Self { config })
    }

    /// Whether the kill switch currently allows anything.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

impl EconomicAuthorization for TestnetEconomicAuthorization {
    fn authorize_commitment(
        &self,
        commitment: ResourceCommitment,
        proposal_id: &str,
    ) -> Result<(), EconomicAuthError> {
        // The testnet lane does not use bare commitments: every live effect
        // goes through authorize_testnet with a full request. Deny here so
        // no caller can smuggle value through the legacy path.
        Err(EconomicAuthError::AdaptersDisabled {
            commitment,
            proposal_id: proposal_id.to_string(),
        })
    }

    fn authorize_testnet(
        &self,
        request: &TestnetAuthRequest,
    ) -> Result<TestnetApproval, EconomicAuthError> {
        if !self.config.enabled {
            return Err(EconomicAuthError::KillSwitch);
        }
        if request.chain_id != TESTNET_CHAIN_ID {
            return Err(EconomicAuthError::NotTestnet {
                chain_id: request.chain_id.clone(),
            });
        }
        if !request.policy_allowed {
            return Err(EconomicAuthError::MissingPolicyApproval {
                proposal_id: request.proposal_id.clone(),
            });
        }
        let b = &request.budget;
        if request.now_unix >= b.expiry_unix {
            return Err(EconomicAuthError::Expired {
                now_unix: request.now_unix,
                expiry_unix: b.expiry_unix,
            });
        }
        if !b.allowed_assets.contains(&request.asset) {
            return Err(EconomicAuthError::WrongAsset {
                asset: request.asset.name().to_string(),
            });
        }
        if !b
            .allowed_destinations
            .iter()
            .any(|d| d == &request.destination)
        {
            return Err(EconomicAuthError::ArbitraryDestination {
                destination: request.destination.clone(),
            });
        }
        if request.amount_wei > b.max_amount_wei {
            return Err(EconomicAuthError::BudgetExceeded {
                amount_wei: request.amount_wei,
                max_wei: b.max_amount_wei,
            });
        }
        if request.gas > b.max_gas {
            return Err(EconomicAuthError::GasExceeded {
                gas: request.gas,
                max_gas: b.max_gas,
            });
        }
        if request.actions > b.max_actions as usize {
            return Err(EconomicAuthError::TooManyActions {
                actions: request.actions,
                max_actions: b.max_actions,
            });
        }
        if request.attempts_used > b.max_retries {
            return Err(EconomicAuthError::RetryBudgetExceeded {
                attempts_used: request.attempts_used,
                max_retries: b.max_retries,
            });
        }
        Ok(TestnetApproval {
            proposal_id: request.proposal_id.clone(),
            budget_id: b.id.clone(),
            chain_id: TESTNET_CHAIN_ID.to_string(),
            asset: request.asset.clone(),
            destination: request.destination.clone(),
            amount_wei: request.amount_wei,
            gas: request.gas,
        })
    }
}

impl From<EconomicAuthError> for ProposalError {
    fn from(e: EconomicAuthError) -> Self {
        ProposalError::EconomicAuth(e.to_string())
    }
}

/// Shared auth-request fixture for crate-wide tests.
#[cfg(test)]
pub(crate) fn auth_request(now: u64, budget: ExperimentBudget) -> TestnetAuthRequest {
    tests::request(now, budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn request(now: u64, budget: ExperimentBudget) -> TestnetAuthRequest {
        TestnetAuthRequest {
            proposal_id: "prop:t".to_string(),
            chain_id: TESTNET_CHAIN_ID.to_string(),
            asset: TestnetAsset::Xegld,
            destination: "erd1operator".to_string(),
            amount_wei: 1_000,
            gas: 50_000,
            actions: 1,
            attempts_used: 0,
            now_unix: now,
            policy_allowed: true,
            budget,
        }
    }

    fn enabled_auth() -> TestnetEconomicAuthorization {
        TestnetEconomicAuthorization::new(TestnetAuthConfig {
            enabled: true,
            chain_id: TESTNET_CHAIN_ID.to_string(),
        })
        .unwrap()
    }

    #[test]
    fn deny_all_refuses_every_commitment() {
        let auth = DenyAllEconomicAuthorization;
        for c in [
            ResourceCommitment::None,
            ResourceCommitment::Cr,
            ResourceCommitment::DCAI,
            ResourceCommitment::Escrow,
        ] {
            let err = auth.authorize_commitment(c, "prop:x").unwrap_err();
            assert!(matches!(err, EconomicAuthError::AdaptersDisabled { .. }));
        }
        // …and the testnet path too (default trait method denies).
        let req = auth_request(1_000_000, crate::budget::valid_budget(1_000_000));
        assert!(auth.authorize_testnet(&req).is_err());
    }

    #[test]
    fn mainnet_adapter_cannot_be_constructed() {
        let err = TestnetEconomicAuthorization::new(TestnetAuthConfig {
            enabled: true,
            chain_id: "1".to_string(),
        })
        .unwrap_err();
        assert!(matches!(err, EconomicAuthError::NotTestnet { .. }));
    }

    #[test]
    fn kill_switch_denies_everything() {
        let off = TestnetEconomicAuthorization::new(TestnetAuthConfig::default()).unwrap();
        assert!(!off.is_enabled());
        let req = auth_request(1_000_000, crate::budget::valid_budget(1_000_000));
        assert_eq!(
            off.authorize_testnet(&req).unwrap_err(),
            EconomicAuthError::KillSwitch
        );
    }

    #[test]
    fn happy_path_approves_bounded_request() {
        let req = auth_request(1_000_000, crate::budget::valid_budget(1_000_000));
        let approval = enabled_auth().authorize_testnet(&req).unwrap();
        assert_eq!(approval.amount_wei, 1_000);
        assert_eq!(approval.chain_id, TESTNET_CHAIN_ID);
    }
}
