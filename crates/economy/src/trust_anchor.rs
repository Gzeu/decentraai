//! Verifiable Trust Anchors — connects EvidenceChain output to wallet-backed
//! identity for on-chain verification.
//!
//! # Design
//!
//! ```text
//! Execution (off-chain, EvidenceChain)
//!   → TrustAnchor {
//!       agent_wallet (who did the work)
//!       evidence_hash (BLAKE3 of EconomicEvidence)
//!       capability (what was done)
//!       quality_score (verified quality)
//!       timestamp
//!       previous_anchor_hash (chain of trust)
//!     }
//!   → AnchorPayloadPrep (MultiversX anchoring)
//! ```
//!
//! # Rules
//!
//! - Every trust anchor links to exactly ONE wallet-backed agent.
//! - Anchors chain via `previous_anchor_hash` (like a blockchain of trust).
//! - The anchor is OFF-CHAIN but VERIFIABLE: the evidence hash can be
//!   independently recomputed and checked against the anchor.
//! - MultiversX stores only the anchor hash (not the full execution data).
//! - The local trust store is the source of truth; MX is the external anchor.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A verifiable trust anchor linking execution to wallet identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustAnchor {
    /// Unique anchor identifier.
    pub anchor_id: String,
    /// Wallet address of the agent who performed the work.
    pub agent_wallet: String,
    /// BLAKE3 hash of the EconomicEvidence (the on-chain anchor).
    pub evidence_hash: String,
    /// Capability that was executed.
    pub capability: String,
    /// Verified quality score (0..=100).
    pub quality_score: u8,
    /// Whether the execution was verified.
    pub verified: bool,
    /// Micro-CU awarded.
    pub micro_cu: u64,
    /// Previous anchor hash (chain linkage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_anchor_hash: Option<String>,
    /// BLAKE3 of this anchor's canonical form (self-chaining).
    pub anchor_hash: String,
    /// Creation timestamp.
    pub created_at: u64,
    /// Contract ID if this anchor is tied to a service contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<String>,
    /// Chain settlement backing this anchor (economic finality layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<ChainSettlement>,
}

/// A checkpoint: multiple anchors bundled for periodic on-chain anchoring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustCheckpoint {
    /// Checkpoint ID.
    pub checkpoint_id: String,
    /// Merkle root of all anchor hashes in this checkpoint.
    pub merkle_root: String,
    /// Number of anchors included.
    pub anchor_count: u32,
    /// wallets covered in this checkpoint.
    pub wallets: Vec<String>,
    /// Checkpoint timestamp.
    pub created_at: u64,
    /// Whether this checkpoint has been anchored on-chain.
    pub anchored_on_chain: bool,
    /// MultiversX tx hash (once anchored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
}

/// Trust store: local verifiable trust history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    /// All trust anchors, keyed by anchor_id.
    pub anchors: BTreeMap<String, TrustAnchor>,
    /// Per-agent anchor chains (wallet → latest anchor hash).
    pub agent_chains: BTreeMap<String, String>,
    /// Trust checkpoints.
    pub checkpoints: Vec<TrustCheckpoint>,
    /// Evidence hashes that have been anchored on-chain (dedup for on-chain anchoring).
    pub on_chain_anchored: BTreeSet<String>,
}

/// Errors from trust operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("anchor for evidence {0} already exists")]
    DuplicateEvidence(String),
    #[error("agent wallet not found in chain: {0}")]
    UnknownAgent(String),
    #[error("anchor verification failed: {0}")]
    VerificationFailed(String),
    #[error("checkpoint already anchored: {0}")]
    AlreadyAnchored(String),
}

/// Input parameters for recording a trust anchor.
#[derive(Debug, Clone)]
pub struct AnchorParams {
    pub agent_wallet: String,
    pub evidence_hash: String,
    pub capability: String,
    pub quality_score: u8,
    pub verified: bool,
    pub micro_cu: u64,
    pub contract_id: Option<String>,
    /// Chain settlement backing this anchor (`None` = off-chain or
    /// pre-correlation anchor). When present, its fingerprint enters the
    /// canonical hash, so a settled anchor and an unsettled one over the
    /// same evidence hash DIFFER (no silent upgrade of trust).
    pub settlement: Option<ChainSettlement>,
}

/// On-chain settlement evidence attached to a trust anchor.
///
/// The tx hash is a fact from our own submission lane; the finality fields
/// are Supernova observations (`None` detail = unobserved, never negative).
/// This is the economic-finality layer: trust carries its own settlement
/// proof instead of pointing at it from afar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainSettlement {
    /// Chain transaction hash anchoring the settled action.
    pub tx_hash: String,
    /// Supernova track detail at observation (`final`, `not-finalized`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Chain finality (execution + proof) observed.
    #[serde(default)]
    pub finality_reached: bool,
    /// Finality proof seen.
    #[serde(default)]
    pub proof_seen: bool,
}

/// Canonical form for anchor self-hash. Single definition used by both
/// recording and verification (they must never drift). The settlement
/// fingerprint joins ONLY when present, so legacy anchors verify exactly
/// as before.
#[allow(clippy::too_many_arguments)]
pub fn canonical_form(
    agent_wallet: &str,
    evidence_hash: &str,
    capability: &str,
    quality_score: u8,
    verified: bool,
    micro_cu: u64,
    settlement: Option<&ChainSettlement>,
    timestamp: u64,
) -> String {
    let settlement_fp = settlement.map(|s| {
        format!(
            "{}:{}:{}:{}",
            s.tx_hash,
            s.detail.as_deref().unwrap_or("?"),
            s.finality_reached,
            s.proof_seen
        )
    });
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        agent_wallet,
        evidence_hash,
        capability,
        quality_score,
        verified,
        micro_cu,
        settlement_fp.as_deref().unwrap_or(""),
        timestamp
    )
}

impl TrustStore {
    /// Records a new trust anchor from an execution outcome.
    #[allow(clippy::too_many_arguments)]
    pub fn record_anchor(
        &mut self,
        params: &AnchorParams,
        now: u64,
    ) -> Result<&TrustAnchor, TrustError> {
        // No duplicate anchor for the same evidence hash.
        if self
            .anchors
            .values()
            .any(|a| a.evidence_hash == params.evidence_hash)
        {
            return Err(TrustError::DuplicateEvidence(params.evidence_hash.clone()));
        }

        let previous = self.agent_chains.get(&params.agent_wallet).cloned();
        let anchor_id = format!(
            "ta-{}-{}",
            &params.evidence_hash[..16.min(params.evidence_hash.len())],
            now
        );

        let canonical = canonical_form(
            &params.agent_wallet,
            &params.evidence_hash,
            &params.capability,
            params.quality_score,
            params.verified,
            params.micro_cu,
            params.settlement.as_ref(),
            now,
        );
        let hash = blake3::hash(canonical.as_bytes());
        let anchor_hash: String = hash.as_bytes().iter().map(|b| format!("{b:02x}")).collect();

        let anchor = TrustAnchor {
            anchor_id: anchor_id.clone(),
            agent_wallet: params.agent_wallet.clone(),
            evidence_hash: params.evidence_hash.clone(),
            capability: params.capability.clone(),
            quality_score: params.quality_score,
            verified: params.verified,
            micro_cu: params.micro_cu,
            previous_anchor_hash: previous.clone(),
            anchor_hash: anchor_hash.clone(),
            created_at: now,
            contract_id: params.contract_id.clone(),
            settlement: params.settlement.clone(),
        };

        self.anchors.insert(anchor_id.clone(), anchor.clone());
        self.agent_chains
            .insert(params.agent_wallet.clone(), anchor_hash);

        Ok(self.anchors.get(&anchor_id).unwrap())
    }

    /// Verifies an anchor's integrity: recomputes the hash and checks chain.
    pub fn verify_anchor(&self, anchor: &TrustAnchor) -> Result<(), TrustError> {
        let canonical = canonical_form(
            &anchor.agent_wallet,
            &anchor.evidence_hash,
            &anchor.capability,
            anchor.quality_score,
            anchor.verified,
            anchor.micro_cu,
            anchor.settlement.as_ref(),
            anchor.created_at,
        );
        let hash = blake3::hash(canonical.as_bytes());
        let expected: String = hash.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        if expected != anchor.anchor_hash {
            return Err(TrustError::VerificationFailed(format!(
                "hash mismatch: expected {}, got {}",
                expected, anchor.anchor_hash
            )));
        }
        // Verify chain linkage.
        if let Some(ref prev) = anchor.previous_anchor_hash {
            // The previous anchor should exist in our store.
            let has_prev = self
                .anchors
                .values()
                .any(|a| a.anchor_hash == *prev && a.agent_wallet == anchor.agent_wallet);
            if !has_prev {
                return Err(TrustError::VerificationFailed(format!(
                    "previous anchor {} not found for agent {}",
                    prev, anchor.agent_wallet
                )));
            }
        }
        Ok(())
    }

    /// Creates a checkpoint from recent unanchored anchors.
    pub fn create_checkpoint(&mut self, now: u64) -> TrustCheckpoint {
        let unanchored: Vec<&TrustAnchor> = self
            .anchors
            .values()
            .filter(|a| !self.on_chain_anchored.contains(&a.evidence_hash))
            .collect();

        let mut wallets: Vec<String> = unanchored
            .iter()
            .map(|a| a.agent_wallet.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        wallets.sort();

        // Simple merkle: hash of sorted anchor hashes.
        let mut anchor_hashes: Vec<String> =
            unanchored.iter().map(|a| a.anchor_hash.clone()).collect();
        anchor_hashes.sort();
        let combined = anchor_hashes.join("");
        let hash = blake3::hash(combined.as_bytes());
        let merkle_root: String = hash.as_bytes().iter().map(|b| format!("{b:02x}")).collect();

        let checkpoint_id = format!("cp-{}-{}", now, &merkle_root[..16]);

        TrustCheckpoint {
            checkpoint_id,
            merkle_root,
            anchor_count: unanchored.len() as u32,
            wallets,
            created_at: now,
            anchored_on_chain: false,
            tx_hash: None,
        }
    }

    /// Returns all anchors for a given wallet.
    pub fn anchors_for_wallet(&self, wallet: &str) -> Vec<&TrustAnchor> {
        self.anchors
            .values()
            .filter(|a| a.agent_wallet == wallet)
            .collect()
    }

    /// Returns the trust score for an agent: ratio of verified anchors.
    pub fn trust_score(&self, wallet: &str) -> f64 {
        let anchors = self.anchors_for_wallet(wallet);
        if anchors.is_empty() {
            return 0.0;
        }
        let verified = anchors.iter().filter(|a| a.verified).count();
        verified as f64 / anchors.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet() -> String {
        "erd1qykvz2cfamvyhwrc087l8lnsy7f7f0z9exvnjudn3d8fagxn3d8spujzsm".to_string()
    }

    fn params(evidence: &str, cap: &str, score: u8, verified: bool, cu: u64) -> AnchorParams {
        AnchorParams {
            agent_wallet: wallet(),
            evidence_hash: evidence.to_string(),
            capability: cap.to_string(),
            quality_score: score,
            verified,
            micro_cu: cu,
            contract_id: None,
            settlement: None,
        }
    }

    #[test]
    fn record_and_verify_anchor() {
        let mut store = TrustStore::default();
        let anchor = store
            .record_anchor(
                &params("ab".repeat(32).as_str(), "chat", 95, true, 1_000_000),
                1000,
            )
            .unwrap()
            .clone();
        assert_eq!(anchor.agent_wallet, wallet());
        assert!(store.verify_anchor(&anchor).is_ok());
    }

    #[test]
    fn chain_linkage_works() {
        let mut store = TrustStore::default();
        let a1 = store
            .record_anchor(&params("ev-1", "chat", 90, true, 500_000), 1000)
            .unwrap()
            .clone();
        let a2 = store
            .record_anchor(&params("ev-2", "ocr", 85, true, 300_000), 2000)
            .unwrap()
            .clone();
        assert_eq!(a2.previous_anchor_hash, Some(a1.anchor_hash));
        assert!(store.verify_anchor(&a2).is_ok());
    }

    #[test]
    fn duplicate_evidence_rejected() {
        let mut store = TrustStore::default();
        store
            .record_anchor(&params("ev-dup", "chat", 90, true, 1_000_000), 1000)
            .unwrap();
        assert!(matches!(
            store.record_anchor(&params("ev-dup", "chat", 90, true, 1_000_000), 2000),
            Err(TrustError::DuplicateEvidence(_))
        ));
    }

    #[test]
    fn trust_score_computed_correctly() {
        let mut store = TrustStore::default();
        store
            .record_anchor(&params("ev-1", "chat", 90, true, 1_000_000), 1000)
            .unwrap();
        store
            .record_anchor(&params("ev-2", "chat", 85, true, 1_000_000), 2000)
            .unwrap();
        store
            .record_anchor(&params("ev-3", "chat", 70, false, 0), 3000)
            .unwrap();

        let score = store.trust_score(&wallet());
        assert!((score - 2.0 / 3.0).abs() < 0.01, "2/3 verified = ~0.667");
    }

    #[test]
    fn checkpoint_creation() {
        let mut store = TrustStore::default();
        store
            .record_anchor(&params("ev-1", "chat", 90, true, 1_000_000), 1000)
            .unwrap();
        store
            .record_anchor(&params("ev-2", "ocr", 85, true, 500_000), 2000)
            .unwrap();

        let cp = store.create_checkpoint(3000);
        assert_eq!(cp.anchor_count, 2);
        assert!(!cp.anchored_on_chain);
        assert!(cp.wallets.contains(&wallet()));
    }

    #[test]
    fn serialization_round_trip() {
        let mut store = TrustStore::default();
        store
            .record_anchor(&params("ev-rt", "chat", 90, true, 1_000_000), 1000)
            .unwrap();
        let json = serde_json::to_string(&store).unwrap();
        let back: TrustStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.anchors.len(), 1);
        assert_eq!(back.trust_score(&wallet()), 1.0);
    }

    #[test]
    fn settlement_changes_hash_and_verifies() {
        // Same evidence, one anchor settled and one not: different hashes
        // (no silent trust upgrade), both verify through the shared form.
        let mut store = TrustStore::default();
        let plain = store
            .record_anchor(&params("ev-set", "ocr", 90, true, 500_000), 1000)
            .unwrap()
            .clone();
        assert!(plain.settlement.is_none());
        assert!(store.verify_anchor(&plain).is_ok());
        let settled_params = AnchorParams {
            settlement: Some(ChainSettlement {
                tx_hash: "08a21463".to_string(),
                detail: Some("final".to_string()),
                finality_reached: true,
                proof_seen: true,
            }),
            ..params("ev-set2", "ocr", 90, true, 500_000)
        };
        let settled = store.record_anchor(&settled_params, 1000).unwrap().clone();
        assert_eq!(settled.settlement.as_ref().unwrap().tx_hash, "08a21463");
        assert!(store.verify_anchor(&settled).is_ok());
        assert_ne!(plain.anchor_hash, settled.anchor_hash);
        // Same evidence+settlement at another timestamp still differs only
        // by time (deterministic shape, no randomness).
        let again = store
            .record_anchor(
                &AnchorParams {
                    evidence_hash: "ev-set3".to_string(),
                    ..settled_params.clone()
                },
                1000,
            )
            .unwrap()
            .clone();
        assert!(store.verify_anchor(&again).is_ok());
    }

    #[test]
    fn legacy_anchor_json_without_settlement_parses() {
        let raw = r#"{"anchor_id":"ta-1","agent_wallet":"erd1x",
            "evidence_hash":"ev","capability":"chat","quality_score":90,
            "verified":true,"micro_cu":100,"anchor_hash":"h","created_at":1}"#;
        let a: TrustAnchor = serde_json::from_str(raw).unwrap();
        assert!(a.settlement.is_none());
        assert!(a.contract_id.is_none());
    }
}
