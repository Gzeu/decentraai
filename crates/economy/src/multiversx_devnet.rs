//! MultiversX Devnet adapter (MX-8004) — READ/TEST only.
//!
//! # Verified facts (from the official skill.md, fetched 2026-08-23)
//!
//! - Devnet API base: `https://devnet-mx8004-api.multiversx.com`
//! - `GET /agents?from&size` lists registered agents
//! - `GET /agents/:nonce` returns one agent (soulbound NFT identity)
//! - `GET /reputations/agents/:nonce` returns `{agentNonce, average, count}`
//! - Registration (`POST /agents`) requires a wallet + hosted manifest —
//!   OUT OF SCOPE here: no wallets, no keys, no monetary transactions.
//! - Mainnet: "Coming soon" per the same source. We do not target it.
//!
//! # Posture
//!
//! READ-ONLY. The [`crate::settlement::BlockchainAdapter`] implementation is
//! attached but deliberately refuses writes until wallet key management
//! exists outside this repository. Reads are transport-injected so every
//! test runs offline against canned devnet-shaped JSON.

use crate::settlement::{BlockchainAdapter, SettlementError, SettlementReceipt, SettlementRecord};
use serde::{Deserialize, Serialize};

/// LIVE devnet API base — discovered from the official explorer bundle
/// (agents.multiversx.com calls THIS host; the skill.md-documented
/// `devnet-mx8004-api.multiversx.com` did not resolve from any of our
/// environments). Verified live 2026-08-23.
pub const DEVNET_API_BASE: &str = "https://devnet-taskclaw-api.multiversx.com";

/// Registry contract addresses on devnet — discovered by inspecting
/// successful on-chain transactions (see
/// docs/MULTIVERSX_DEVNET_ADDRESSES.md for hashes and method).
/// - Identity: corroborated by 2 independent register_agent txs.
/// - Validation: corroborated by 3 different functions.
/// - Reputation: single successful submit_feedback observed (PARTIALLY).
pub mod registry_addresses {
    pub const IDENTITY: &str = "erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x";
    pub const VALIDATION: &str = "erd1qqqqqqqqqqqqqpgqvax6z79cvyz9gkfwg57hqume352p7s7rd8ss4g3t43";
    /// PARTIALLY verified — one successful submit_feedback observed.
    pub const REPUTATION: &str = "erd1qqqqqqqqqqqqqpgqwhqpuzkrywc5j8q2ec6skqnejtzgjnzad8ssdmv962";
}

/// HTTP transport seam: production uses reqwest-blocking; tests inject a
/// fake. Keeping this trait tiny keeps the adapter honest and offline-testable.
pub trait MxHttp: Send + Sync {
    /// GETs `url` and parses the body as JSON.
    fn get_json(&self, url: &str) -> Result<serde_json::Value, String>;
}

/// Production transport: blocking reqwest with a hard timeout.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest blocking client"),
        }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MxHttp for ReqwestTransport {
    fn get_json(&self, url: &str) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(url)
            .send()
            .map_err(|e| format!("devnet request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("devnet returned {}", resp.status()));
        }
        resp.json::<serde_json::Value>()
            .map_err(|e| format!("devnet response is not JSON: {e}"))
    }
}

/// One registered MX-8004 agent, parsed leniently from devnet JSON.
/// Unknown/missing fields become `None` — we never invent values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MxAgentRecord {
    pub nonce: Option<u64>,
    pub name: Option<String>,
    /// Manifest URI (IPFS or HTTPS) hosting the agent's capability manifest.
    pub uri: Option<String>,
    /// The agent's Ed25519 public key hex (0x-prefixed), as registered.
    pub public_key: Option<String>,
}

/// Reputation summary as returned by `GET /reputations/agents/:nonce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MxReputation {
    pub agent_nonce: u64,
    /// Average score as reported by the devnet registry (0..=100).
    pub average: u32,
    /// Number of ratings backing that average.
    pub count: u32,
}

fn parse_agent(v: &serde_json::Value) -> MxAgentRecord {
    // Live devnet field is `publicKeyHex`; skill.md documents `publicKey`.
    // Accept BOTH — lenient by contract.
    let pk = v
        .get("publicKeyHex")
        .or_else(|| v.get("publicKey"))
        .and_then(|x| x.as_str())
        .map(str::to_string);
    MxAgentRecord {
        nonce: v.get("nonce").and_then(|x| x.as_u64()),
        name: v.get("name").and_then(|x| x.as_str()).map(str::to_string),
        uri: v.get("uri").and_then(|x| x.as_str()).map(str::to_string),
        public_key: pk,
    }
}

/// Read-only MX-8004 devnet client over an injected transport.
#[derive(Debug, Clone)]
pub struct MxDevnetClient<T: MxHttp> {
    transport: T,
    base: String,
}

impl<T: MxHttp> MxDevnetClient<T> {
    /// A client pointed at the official devnet base.
    pub fn devnet(transport: T) -> Self {
        Self {
            transport,
            base: DEVNET_API_BASE.to_string(),
        }
    }

    /// A client pointed at a custom base (test doubles, future networks).
    pub fn with_base(transport: T, base: String) -> Self {
        Self { transport, base }
    }

    fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        self.transport.get_json(&format!("{}{}", self.base, path))
    }

    /// Lists registered agents (bounded page).
    pub fn list_agents(&self, from: u64, size: u64) -> Result<Vec<MxAgentRecord>, String> {
        let size = size.clamp(1, 1000);
        let v = self.get(&format!("/agents?from={from}&size={size}"))?;
        // Live wrapper wraps pages in {items:[...]}; bare arrays also accepted.
        let arr = v
            .get("items")
            .and_then(|x| x.as_array())
            .or_else(|| v.as_array())
            .ok_or_else(|| "unexpected /agents shape".to_string())?;
        Ok(arr.iter().map(parse_agent).collect())
    }

    /// Fetches one agent by its on-chain nonce.
    pub fn get_agent(&self, nonce: u64) -> Result<MxAgentRecord, String> {
        let v = self.get(&format!("/agents/{nonce}"))?;
        Ok(parse_agent(&v))
    }

    /// Caută un agent după cheia publică (hex 0x-…) paginând lista.
    /// Folosit DUPĂ registration pentru a descoperi nonce-ul on-chain
    /// atribuit identității noastre. `max_pages` limitează traversarea.
    pub fn find_agent_by_public_key(
        &self,
        public_key_hex: &str,
        max_pages: u64,
    ) -> Result<Option<MxAgentRecord>, String> {
        let mut from: u64 = 0;
        for _ in 0..max_pages {
            let page = self.list_agents(from, 100)?;
            if page.is_empty() {
                return Ok(None);
            }
            if let Some(hit) = page
                .iter()
                .find(|a| a.public_key.as_deref() == Some(public_key_hex))
            {
                return Ok(Some(hit.clone()));
            }
            from += 100;
        }
        Ok(None)
    }

    /// Fetches the registry's reputation summary for one agent.
    pub fn reputation(&self, nonce: u64) -> Result<MxReputation, String> {
        let v = self.get(&format!("/reputations/agents/{nonce}"))?;
        Ok(MxReputation {
            agent_nonce: v
                .get("agentNonce")
                .and_then(|x| x.as_u64())
                .unwrap_or(nonce),
            average: v
                .get("average")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32,
            count: v
                .get("count")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32,
        })
    }
}

/// Settlement adapter wrapper in READ-ONLY posture.
///
/// Attaches to the economy's [`BlockchainAdapter`] seam so the wiring exists
/// end-to-end, while writes are EXPLICITLY refused: anchoring/settlement on
/// MultiversX requires a funded wallet + transaction signing, which needs key
/// management outside this repository (Phase 7 future interfaces).
#[derive(Debug, Clone)]
pub struct MxDevnetSettlementAdapter<T: MxHttp> {
    pub client: MxDevnetClient<T>,
}

impl<T: MxHttp> BlockchainAdapter for MxDevnetSettlementAdapter<T> {
    fn name(&self) -> &'static str {
        "multiversx-devnet"
    }

    fn submit_settlement(
        &self,
        _record: &SettlementRecord,
    ) -> Result<SettlementReceipt, SettlementError> {
        Err(SettlementError::Rejected(
            "read-only devnet posture: settlement writes require wallet signing \
             (planned via TransactionSigner); evidence reads are available now"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Offline fake transport serving devnet-shaped JSON per URL fragment.
    #[derive(Default)]
    struct FakeMx {
        routes: BTreeMap<&'static str, serde_json::Value>,
    }

    impl FakeMx {
        fn with(mut self, frag: &'static str, v: serde_json::Value) -> Self {
            self.routes.insert(frag, v);
            self
        }
    }

    impl MxHttp for FakeMx {
        fn get_json(&self, url: &str) -> Result<serde_json::Value, String> {
            // Most specific (longest) fragment wins — "/reputations/agents/42"
            // must beat "/agents/42" even though one contains the other.
            let mut hits: Vec<(&&'static str, &serde_json::Value)> = self
                .routes
                .iter()
                .filter(|(frag, _)| url.contains(*frag))
                .collect();
            hits.sort_by_key(|(frag, _)| frag.len());
            if let Some((_, v)) = hits.last() {
                return Ok((*v).clone());
            }
            Err(format!("no fake route for {url}"))
        }
    }

    #[test]
    fn reads_parse_devnet_shaped_json_leniently() {
        let fake = FakeMx::default()
            .with(
                "/agents?",
                serde_json::json!({"items": [
                    {"nonce": 42, "name": "DecentraGovernor",
                     "uri": "ipfs://QmX", "publicKeyHex": "0x04aa"},
                    {"weird": true}
                ]}),
            )
            .with(
                "/agents/42",
                serde_json::json!({"nonce": 42, "name": "DecentraGovernor", "uri": "ipfs://QmX"}),
            )
            .with(
                "/reputations/agents/42",
                serde_json::json!({"agentNonce": 42, "average": 85, "count": 12}),
            );
        let c = MxDevnetClient::devnet(fake);

        let agents = c.list_agents(0, 100).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].nonce, Some(42));
        assert_eq!(agents[0].public_key.as_deref(), Some("0x04aa"));
        // Lenient: malformed row parses to all-None instead of erroring.
        assert_eq!(agents[1].nonce, None);

        let a = c.get_agent(42).unwrap();
        assert_eq!(a.name.as_deref(), Some("DecentraGovernor"));

        let rep = c.reputation(42).unwrap();
        assert_eq!((rep.average, rep.count), (85, 12));
    }

    #[test]
    fn settlement_writes_are_refused_in_read_only_posture() {
        let adapter = MxDevnetSettlementAdapter {
            client: MxDevnetClient::devnet(FakeMx::default()),
        };
        let rec = SettlementRecord {
            worker_id: "w".into(),
            amount_micro_cu: 1_000,
            evidence_hash: [7u8; 32],
            cu_version: 2,
            epoch: 1,
        };
        let err = BlockchainAdapter::submit_settlement(&adapter, &rec).expect_err("writes refused");
        assert!(err.to_string().contains("read-only"));
        assert_eq!(BlockchainAdapter::name(&adapter), "multiversx-devnet");
    }

    #[test]
    fn base_url_is_the_published_devnet_api() {
        let c = MxDevnetClient::devnet(FakeMx::default());
        assert_eq!(c.base, DEVNET_API_BASE);
        assert!(
            DEVNET_API_BASE.starts_with("https://devnet-"),
            "never mainnet"
        );
    }
}
