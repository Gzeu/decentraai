//! Economic authorization — the FUTURE adapter seam (layer 5).
//!
//! The cognitive core never touches the economy directly: any proposal
//! carrying a non-`None` commitment must pass through
//! [`EconomicAuthorization`] first. In v0.1 exactly one implementation
//! exists ([`DenyAllEconomicAuthorization`]), which refuses everything —
//! so there is no path, however indirect, from a proposal to live funds.
//!
//! A future live adapter implements this trait (real balance checks,
//! escrow holds, multisig, human approval…) WITHOUT touching the core:
//! policy, sandbox and evidence keep compiling unchanged.

use crate::error::ProposalError;
use crate::risk::ResourceCommitment;

/// Why economic authorization refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconomicAuthError {
    /// No live adapter is wired (the only v0.1 outcome).
    AdaptersDisabled {
        /// Which commitment was requested.
        commitment: ResourceCommitment,
        /// Which proposal asked.
        proposal_id: String,
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
                "v0.1: no live economic adapter; commitment {commitment:?} \
                 for proposal {proposal_id} denied by construction"
            ),
        }
    }
}

impl std::error::Error for EconomicAuthError {}

/// Future live-economy gate. Core calls it; adapters implement it.
pub trait EconomicAuthorization: Send + Sync {
    /// Approve committing `commitment` for `proposal_id`, or refuse.
    /// v0.1: always refuses (see [`DenyAllEconomicAuthorization`]).
    fn authorize_commitment(
        &self,
        commitment: ResourceCommitment,
        proposal_id: &str,
    ) -> Result<(), EconomicAuthError>;
}

/// The only v0.1 implementation: deny everything, explicitly.
///
/// Existence proof that the seam is wired end-to-end (policy consults it)
/// while guaranteeing no economic effect is possible yet.
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

impl From<EconomicAuthError> for ProposalError {
    fn from(e: EconomicAuthError) -> Self {
        ProposalError::EconomicAuth(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
