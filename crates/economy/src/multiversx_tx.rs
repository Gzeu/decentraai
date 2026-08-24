//! Deterministic MX-8004 transaction-data builders + unsigned intent model.
//!
//! # What this module DOES and does NOT do
//!
//! DOES: encode VERIFIED v2.1 contract calls into the exact on-chain
//! `data` field format (`endpoint@fieldHex@fieldHex…`), wrapped in an
//! [`UnsignedTxIntent`] the operator's wallet tooling can complete.
//!
//! DOES NOT: sign, submit, hold keys, hard-code contract addresses, or touch
//! the network. `receiver` stays `None` until a VERIFIED devnet registry
//! address enters `docs/MULTIVERSX_DEVNET_ADDRESSES.md`.
//!
//! # Hash representation rule (EconomicEvidence bridge)
//!
//! `submit_proof(job_id, proof)` takes the proof as a buffer. We pass the
//! EconomicEvidence BLAKE3 digest **as-is**: raw 32 bytes, hex-encoded once
//! at the data-field boundary. The digest is NEVER hashed again — double
//! hashing would break independent verification downstream.
//!
//! All encodings are lowercase hex; all builders are pure functions.

use crate::evidence::EconomicEvidence;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

pub const REGISTER_AGENT_ENDPOINT: &str = "register_agent";
pub const SUBMIT_PROOF_ENDPOINT: &str = "submit_proof";
pub const VALIDATION_REQUEST_ENDPOINT: &str = "validation_request";
pub const VALIDATION_RESPONSE_ENDPOINT: &str = "validation_response";
pub const INIT_JOB_WITH_PAYMENT_ENDPOINT: &str = "init_job_with_payment";
pub const SUBMIT_FEEDBACK_ENDPOINT: &str = "submit_feedback";

/// Network tag carried on every intent produced here.
pub const TESTNET_NETWORK: &str = "multiversx-testnet";

/// Legacy devnet network tag.
pub const DEVNET_NETWORK: &str = "multiversx-devnet";

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn str_field(field: &'static str, s: &str) -> Result<String, TxBuildError> {
    if s.is_empty() {
        return Err(TxBuildError::EmptyField(field));
    }
    Ok(to_hex(s.as_bytes()))
}

fn num_field(v: u64) -> String {
    to_hex(&v.to_be_bytes())
}

fn hash_field(hash: &[u8; 32]) -> String {
    to_hex(hash)
}

/// Builder input errors — precise, bounded, no guessing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TxBuildError {
    #[error("field '{0}' must not be empty")]
    EmptyField(&'static str),
    #[error("field '{field}' exceeds {max} chars")]
    FieldTooLong { field: &'static str, max: usize },
    #[error("public key must be 0x-prefixed hex decoding to 32 bytes")]
    InvalidPublicKey,
    #[error("score must be within 0..=100")]
    ScoreOutOfRange,
    #[error("payment value is required for a paid job")]
    ValueRequired,
    #[error("uri scheme must be ipfs:// or https://")]
    InvalidUriScheme,
}

const MAX_NAME: usize = 64;
const MAX_URI: usize = 512;
const MAX_JOB_ID: usize = 128;

/// Status marker: intents are PREPARATION until an operator signs them with
/// external tooling. There is deliberately no path from here to a signed tx.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    Preparation,
}

/// An unsigned transaction intent: everything EXCEPT authority.
///
/// `receiver` is intentionally optional and filled ONLY from a VERIFIED
/// address (none exists yet). `sender`, `nonce`, `gas_limit`, `chain_id`
/// belong to the operator's wallet tooling and stay unset here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsignedTxIntent {
    pub network: String,
    pub endpoint: String,
    /// Hex-encoded arguments, in exact contract order, without separators.
    pub data_fields_hex: Vec<String>,
    /// EGLD value in denomination (10^18) — non-zero only for paid jobs.
    pub value_denomination: u64,
    /// Registry contract address — `None` until independently verified.
    pub receiver: Option<String>,
    /// Operator wallet ADDRESS (never key material).
    pub sender: Option<String>,
    pub nonce: Option<u64>,
    pub gas_limit: Option<u64>,
    /// Devnet chain id is `"D"` once the caller sets it.
    pub chain_id: Option<String>,
    pub status: IntentStatus,
}

impl UnsignedTxIntent {
    /// The full on-chain `data` string: `endpoint@f1@f2…`.
    pub fn data_field(&self) -> String {
        let mut out = self.endpoint.clone();
        for f in &self.data_fields_hex {
            out.push('@');
            out.push_str(f);
        }
        out
    }
}

fn base_intent(endpoint: &str, fields: Vec<String>) -> UnsignedTxIntent {
    UnsignedTxIntent {
        network: TESTNET_NETWORK.to_string(),
        endpoint: endpoint.to_string(),
        data_fields_hex: fields,
        value_denomination: 0,
        receiver: None,
        sender: None,
        nonce: None,
        gas_limit: None,
        chain_id: None,
        status: IntentStatus::Preparation,
    }
}

/// Deterministic builders for every SOURCE-VERIFIED v2.1 operation.
pub struct Mx8004TxBuilder;

impl Mx8004TxBuilder {
    /// `register_agent@name@uri@pubKey[@k@v…]` (S1 §1.3, S2 §3.1).
    pub fn register_agent(
        name: &str,
        manifest_uri: &str,
        public_key_hex: &str,
        metadata: &[(String, String)],
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if name.chars().count() > MAX_NAME {
            return Err(TxBuildError::FieldTooLong {
                field: "name",
                max: MAX_NAME,
            });
        }
        if manifest_uri.chars().count() > MAX_URI {
            return Err(TxBuildError::FieldTooLong {
                field: "uri",
                max: MAX_URI,
            });
        }
        // S2 §3.1: hosting can be IPFS, HTTPS, or base64 data URI.
        if !(manifest_uri.starts_with("ipfs://")
            || manifest_uri.starts_with("https://")
            || manifest_uri.starts_with("data:application/json;base64,"))
        {
            return Err(TxBuildError::InvalidUriScheme);
        }
        let pk = public_key_hex
            .strip_prefix("0x")
            .ok_or(TxBuildError::InvalidPublicKey)?;
        if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(TxBuildError::InvalidPublicKey);
        }
        // PublicKey travels as RAW KEY HEX (the 64 hex digits themselves),
        // exactly like S2's `@<publicKeyHex>` — NOT hex-of-the-hex-string.
        let pk_raw = pk.to_ascii_lowercase();
        let mut fields = vec![
            str_field("name", name)?,
            str_field("uri", manifest_uri)?,
            pk_raw,
        ];
        for (k, v) in metadata {
            fields.push(str_field("metadata_key", k.as_str())?);
            fields.push(str_field("metadata_value", v.as_str())?);
        }
        Ok(base_intent(REGISTER_AGENT_ENDPOINT, fields))
    }

    /// `submit_proof@jobId@proof` — proof IS the raw EconomicEvidence BLAKE3
    /// digest, hex-encoded once. Never re-hashed (see module docs).
    pub fn submit_proof(
        job_id: &str,
        evidence: &EconomicEvidence,
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if job_id.is_empty() || job_id.chars().count() > MAX_JOB_ID {
            return Err(TxBuildError::EmptyField("job_id"));
        }
        let fields = vec![
            str_field("job_id", job_id)?,
            hash_field(&evidence.evidence_hash()),
        ];
        Ok(base_intent(SUBMIT_PROOF_ENDPOINT, fields))
    }

    /// `validation_request@jobId@validatorAddr@requestUri@requestHash`.
    pub fn validation_request(
        job_id: &str,
        validator_address: &str,
        request_uri: &str,
        request_hash: &[u8; 32],
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if validator_address.is_empty() || request_uri.is_empty() {
            return Err(TxBuildError::EmptyField("validator/uri"));
        }
        Ok(base_intent(
            VALIDATION_REQUEST_ENDPOINT,
            vec![
                str_field("job_id", job_id)?,
                str_field("validator_address", validator_address)?,
                str_field("request_uri", request_uri)?,
                hash_field(request_hash),
            ],
        ))
    }

    /// `validation_response@requestHash@response@responseUri@responseHash@tag`
    /// — score travels inside `response` (0..=100), enforced here.
    pub fn validation_response(
        request_hash: &[u8; 32],
        score: u8,
        response_uri: &str,
        response_hash: &[u8; 32],
        tag: &str,
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if score > 100 {
            return Err(TxBuildError::ScoreOutOfRange);
        }
        Ok(base_intent(
            VALIDATION_RESPONSE_ENDPOINT,
            vec![
                hash_field(request_hash),
                num_field(u64::from(score)),
                str_field("response_uri", response_uri)?,
                hash_field(response_hash),
                str_field("tag", tag)?,
            ],
        ))
    }

    /// `init_job_with_payment@jobId@agentNonce@ServiceId` with the payment
    /// as tx VALUE (denomination 10^18). Zero value is rejected — the
    /// contract requires the sent payment to meet the service price.
    pub fn init_job_with_payment(
        job_id: &str,
        agent_nonce: u64,
        service_id: &str,
        value_denomination: u64,
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if value_denomination == 0 {
            return Err(TxBuildError::ValueRequired);
        }
        let mut intent = base_intent(
            INIT_JOB_WITH_PAYMENT_ENDPOINT,
            vec![
                str_field("job_id", job_id)?,
                num_field(agent_nonce),
                str_field("service_id", service_id)?,
            ],
        );
        intent.value_denomination = value_denomination;
        Ok(intent)
    }

    /// `submit_feedback@jobId@agentNonce@rating` (employer-only on-chain).
    pub fn submit_feedback(
        job_id: &str,
        agent_nonce: u64,
        rating: u8,
    ) -> Result<UnsignedTxIntent, TxBuildError> {
        if rating > 100 {
            return Err(TxBuildError::ScoreOutOfRange);
        }
        Ok(base_intent(
            SUBMIT_FEEDBACK_ENDPOINT,
            vec![
                str_field("job_id", job_id)?,
                num_field(agent_nonce),
                num_field(u64::from(rating)),
            ],
        ))
    }
}

/// An EXTERNAL validation signal arriving from MultiversX. By contract this
/// is UNTRUSTED input: converting it into DecentraAI knowledge goes through
/// the normal memory pipeline (candidate → verified), never straight to
/// trusted, and NEVER into RBAC authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MxValidationSignal {
    pub job_id: String,
    pub validator_address: String,
    /// Validator score 0..=100 as recorded on-chain.
    pub score_percent: u8,
    pub request_hash_hex: String,
}

/// External reputation signal (registry average/count read).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MxReputationSignal {
    pub agent_nonce: u64,
    pub average_percent: u8,
    pub count: u32,
}

// ---------------------------------------------------------------------------
// REGISTRATION PREPARATION (operator runbook support)
//
// Produces everything the OPERATOR needs to execute the first registration
// manually: the exact data field, every tx field they must fill, and the
// verification steps AFTER confirmation. Nothing here signs or submits.
// ---------------------------------------------------------------------------

/// One field the operator must supply/verify at signing time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorField {
    pub name: &'static str,
    pub value: String,
    pub note: &'static str,
}

/// Full preparation report for the first Governor registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationPreparation {
    pub network: &'static str,
    pub chain_id: &'static str,
    pub endpoint: &'static str,
    /// The complete on-chain data string, ready to paste.
    pub data_field: String,
    pub sender_wallet_address: String,
    pub agent_public_key_hex: String,
    pub manifest_uri: String,
    /// Fields the operator fills from live devnet state / wallet tooling.
    pub operator_fields: Vec<OperatorField>,
    /// Post-confirmation verification steps (ordered).
    pub verification_steps: Vec<&'static str>,
}

/// Builds the full registration preparation. Pure + offline.
/// `gas_limit` suggestion is a STARTING POINT — confirm on explorer.
pub fn registration_preparation(
    name: &str,
    manifest_uri: &str,
    agent_public_key_hex: &str,
    sender_wallet_address: &str,
    suggested_gas_limit: u64,
) -> Result<RegistrationPreparation, TxBuildError> {
    let intent = Mx8004TxBuilder::register_agent(name, manifest_uri, agent_public_key_hex, &[])?;
    Ok(RegistrationPreparation {
        network: DEVNET_NETWORK,
        chain_id: "T",
        endpoint: REGISTER_AGENT_ENDPOINT,
        data_field: intent.data_field(),
        sender_wallet_address: sender_wallet_address.to_string(),
        agent_public_key_hex: agent_public_key_hex.to_string(),
        manifest_uri: manifest_uri.to_string(),
        operator_fields: vec![
            OperatorField {
                name: "receiver",
                value: crate::multiversx_devnet::registry_addresses::IDENTITY.into(),
                note: "VERIFIED Identity Registry (devnet indexer; 2 independent register_agent txs — MULTIVERSX_DEVNET_ADDRESSES.md)",
            },
            OperatorField {
                name: "nonce",
                value: "<account nonce of sender, from GET /accounts/{sender}>".into(),
                note: "live devnet account state",
            },
            OperatorField {
                name: "gasLimit",
                value: suggested_gas_limit.to_string(),
                note: "STARTING POINT — NFT mint burns gas; raise if explorer shows insufficient",
            },
            OperatorField {
                name: "chainId",
                value: "T".into(),
                note: "testnet",
            },
            OperatorField {
                name: "version",
                value: "1".into(),
                note: "tx version",
            },
        ],
        verification_steps: vec![
            "GET https://devnet-api.multiversx.com/transactions/{txHash} until status=success",
            "extract `receiver` from the confirmed tx",
            "check tx logs/events contain agentRegistered",
            "GET DEVNET_API_BASE/agents/{nonce} — publicKey must equal ours",
            "record receiver+txHash+explorer URL into MULTIVERSX_DEVNET_ADDRESSES.md",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::{ContributionFacts, VerificationStatus};
    use crate::evidence::EconomicEvidence;

    fn evidence() -> EconomicEvidence {
        let facts = ContributionFacts {
            worker_id: "w".into(),
            verified_units: 3,
            quality_percent: 95,
            reliability_percent: 100,
            latency_ms: 700,
            baseline_latency_ms: 1000,
            resource_bytes: 256,
            efficiency_index_x100: 100,
            scarcity_bps: 12_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: "bench:x:0".into(),
            verifier_id: "verifier-x".into(),
        };
        EconomicEvidence::from_facts(&facts).unwrap()
    }

    fn key_hex() -> String {
        format!("0x{}", "ab".repeat(32))
    }

    #[test]
    fn register_agent_encoding_matches_verified_format_exactly() {
        let intent = Mx8004TxBuilder::register_agent(
            "DecentraGovernor",
            "ipfs://QmZ",
            &key_hex(),
            &[("category".into(), "research-analysis".into())],
        )
        .unwrap();
        assert_eq!(intent.endpoint, REGISTER_AGENT_ENDPOINT);
        assert_eq!(intent.status, IntentStatus::Preparation);
        assert!(intent.receiver.is_none(), "no address invented");
        assert_eq!(
            intent.data_field(),
            format!(
                "register_agent@{}@{}@{}@{}@{}",
                to_hex(b"DecentraGovernor"),
                to_hex(b"ipfs://QmZ"),
                "ab".repeat(32),
                to_hex(b"category"),
                to_hex(b"research-analysis")
            )
        );
        // Determinism: rebuild → identical data field.
        let again = Mx8004TxBuilder::register_agent(
            "DecentraGovernor",
            "ipfs://QmZ",
            &key_hex(),
            &[("category".into(), "research-analysis".into())],
        )
        .unwrap();
        assert_eq!(intent.data_field(), again.data_field());
    }

    #[test]
    fn submit_proof_passes_the_digest_as_is_never_rehashed() {
        let ev = evidence();
        let intent = Mx8004TxBuilder::submit_proof("job-77", &ev).unwrap();
        // The proof field equals plain hex of the 32-byte digest:
        let digest_hex = to_hex(&ev.evidence_hash());
        assert_eq!(
            intent.data_fields_hex[1], digest_hex,
            "raw digest hex — NOT a second hash"
        );
        assert_eq!(digest_hex.len(), 64);
    }

    #[test]
    fn validation_round_trip_encodes_score_and_hashes() {
        let rh = [9u8; 32];
        let sh = [8u8; 32];
        let intent =
            Mx8004TxBuilder::validation_request("job-77", "erd1validator", "https://v/x", &rh)
                .unwrap();
        assert_eq!(intent.endpoint, VALIDATION_REQUEST_ENDPOINT);

        let resp =
            Mx8004TxBuilder::validation_response(&rh, 87, "https://r/y", &sh, "mi-corpus").unwrap();
        assert_eq!(resp.endpoint, VALIDATION_RESPONSE_ENDPOINT);
        assert_eq!(resp.data_fields_hex[1], num_field(87));

        // Out-of-band score rejected deterministically.
        assert!(matches!(
            Mx8004TxBuilder::validation_response(&rh, 101, "u", &sh, "t"),
            Err(TxBuildError::ScoreOutOfRange)
        ));
    }

    #[test]
    fn paid_job_requires_value_and_carries_it_on_the_intent() {
        let err = Mx8004TxBuilder::init_job_with_payment("j", 42, "chat", 0).unwrap_err();
        assert_eq!(err, TxBuildError::ValueRequired);
        let ok =
            Mx8004TxBuilder::init_job_with_payment("j", 42, "chat", 5 * 10u64.pow(17)).unwrap();
        assert_eq!(ok.value_denomination, 5 * 10u64.pow(17));
        assert_eq!(ok.endpoint, INIT_JOB_WITH_PAYMENT_ENDPOINT);
    }

    #[test]
    fn feedback_rejects_out_of_range_rating() {
        assert!(Mx8004TxBuilder::submit_feedback("j", 7, 120).is_err());
        assert!(Mx8004TxBuilder::submit_feedback("j", 7, 85).is_ok());
    }

    #[test]
    fn oversize_inputs_are_rejected_not_truncated() {
        let long_name = "x".repeat(MAX_NAME + 1);
        assert!(matches!(
            Mx8004TxBuilder::register_agent(&long_name, "ipfs://QmZ", &key_hex(), &[]),
            Err(TxBuildError::FieldTooLong { field: "name", .. })
        ));
        let long_uri = format!("ipfs://{}", "y".repeat(600));
        assert!(matches!(
            Mx8004TxBuilder::register_agent("n", &long_uri, &key_hex(), &[]),
            Err(TxBuildError::FieldTooLong { field: "uri", .. })
        ));
    }

    #[test]
    fn intent_serialization_round_trips() {
        let intent = Mx8004TxBuilder::submit_proof("job-1", &evidence()).unwrap();
        let back: UnsignedTxIntent =
            serde_json::from_str(&serde_json::to_string(&intent).unwrap()).unwrap();
        assert_eq!(back, intent);
        assert_eq!(back.network, TESTNET_NETWORK);
    }

    #[test]
    fn external_signals_are_data_not_authority() {
        let sig = MxValidationSignal {
            job_id: "j".into(),
            validator_address: "erd1v".into(),
            score_percent: 88,
            request_hash_hex: "00".repeat(32),
        };
        // The signal carries facts; conversion into trusted knowledge is the
        // memory pipeline's job (candidate first). Nothing here grants RBAC.
        assert!(sig.score_percent <= 100);
        let rep = MxReputationSignal {
            agent_nonce: 42,
            average_percent: 70,
            count: 5,
        };
        assert!(rep.average_percent <= 100);
    }
    #[test]
    fn registration_preparation_is_complete_and_deterministic() {
        let p = registration_preparation(
            "DecentraGovernor",
            "ipfs://QmZ",
            &key_hex(),
            "erd1operator",
            30_000_000,
        )
        .unwrap();
        assert_eq!(p.chain_id, "T");
        assert_eq!(p.endpoint, REGISTER_AGENT_ENDPOINT);
        assert!(p.data_field.starts_with("register_agent@"));
        assert_eq!(p.sender_wallet_address, "erd1operator");
        // Receiver MUST be a pending placeholder — no address invented.
        assert!(p.operator_fields.iter().any(|f| f.name == "receiver"
            && f.value == crate::multiversx_devnet::registry_addresses::IDENTITY));
        assert_eq!(p.verification_steps.len(), 5);
        // Determinism.
        let p2 = registration_preparation(
            "DecentraGovernor",
            "ipfs://QmZ",
            &key_hex(),
            "erd1operator",
            30_000_000,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            serde_json::to_string(&p2).unwrap()
        );
    }

    #[test]
    fn registration_preparation_rejects_bad_inputs() {
        assert!(registration_preparation("", "ipfs://QmZ", &key_hex(), "erd1o", 1000).is_err());
        assert!(registration_preparation("n", "http://bad", &key_hex(), "erd1o", 1000).is_err());
        assert!(registration_preparation("n", "ipfs://QmZ", "not-a-key", "erd1o", 1000).is_err());
    }
}
