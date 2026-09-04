//! Unified error type for the proposal protocol.
use thiserror::Error;

/// Every failure mode of the v0.1 protocol. Denials are values
/// ([`crate::policy::PolicyDecision::Deny`]), not errors: an error here means
/// malformed input or an internal invariant violation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProposalError {
    /// JSON does not parse or violates the closed schema.
    #[error("proposal parse error: {0}")]
    Parse(String),
    /// A field or collection exceeds its hard bound.
    #[error("proposal bound violated: {0}")]
    Bound(String),
    /// Execution was attempted without (or against) an Allow decision.
    #[error("execution refused: {0}")]
    ExecutionRefused(String),
    /// An economic action reached the sandbox executor boundary.
    /// Unreachable through policy; a hard stop if it ever happens.
    #[error("economic action at executor boundary: {0}")]
    EconomicAtBoundary(String),
    /// Evidence chain verification failed.
    #[error("evidence chain broken: {0}")]
    ChainBroken(String),
    /// Economic authorization seam refused (always, in v0.1).
    #[error("economic authorization: {0}")]
    EconomicAuth(String),
}
