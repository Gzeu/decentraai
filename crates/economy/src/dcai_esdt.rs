//! DCAI ESDT issuance data-field builder (pure — no network, no keys).
//!
//! This module supports the *only* issuance we accept today: **zero
//! circulating supply**. The transaction reserves the ticker and assigns
//! token management to the operating account. Minting itself happens later
//! through governed contracts, never ad-hoc from a wallet.
//!
//! The network-facing broadcast/sign/poll wrappers live in `node-cli` (they
//! need a signer, which is off-limits here). This crate contains only the
//! pure builders and validation rules.
//!
//! Docs: <https://docs.multiversx.com/tokens/fungible-tokens>

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

// ── Constants ───────────────────────────────────────────────────────────────

/// MultiversX ESDT system smart contract (built-in, not VM-executable).
pub const ESDT_SYSTEM_SC: &str = "erd1qqqqqqqqqqqqqqqpqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzllls8a5w6u";
/// Protocol cost of issuance (0.05 xEGLD).
pub const ISSUE_COST_WEI: &str = "50000000000000000";
/// Protocol gas limit for issuance transaction.
pub const ISSUE_GAS_LIMIT: u64 = 60_000_000;

// ── Parameters ──────────────────────────────────────────────────────────────

/// Parameters for one ESDT issuance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueParams {
    /// Token name: 3–20 chars, alphanumeric.
    pub name: String,
    /// Token ticker: 3–10 chars, UPPERCASE alphanumeric only.
    pub ticker: String,
    /// Initial supply in smallest units (e.g. wei-style). Pass 0 to issue a
    /// token with zero circulating supply (the default for DCAI).
    pub initial_supply: u64,
    /// Number of decimals (0–18).
    pub decimals: u8,
}

/// How DCAI COULD mint later, without inventing tokenomics now.
///
/// - M18escrow contract: when a verified contribution is settled, the
///   contract mints DCAI to the contributor.
/// - Provider bonds: locked bonds mint DCAI on verified proven compute.
///
/// Both paths live IN CONTRACTS, not in this issuance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MintingPath {
    /// No path yet — token sits at zero supply. The issuance is intent-only.
    Unconnected,
    /// M18 escrow contract will mint DCAI on verified proof-of-compute.
    M18EscrowContract,
    /// A provider bond will mint DCAI on verified successful delivery.
    ProviderBonds,
}

impl IssueParams {
    /// Validate fields against the MultiversX issuance spec.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.len() < 3 || self.name.len() > 20 {
            return Err(format!(
                "token name must be 3..20 chars (got {})",
                self.name.len()
            ));
        }
        if !self.name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("token name must be alphanumeric".into());
        }
        if self.ticker.len() < 3 || self.ticker.len() > 10 {
            return Err(format!(
                "ticker must be 3..10 chars (got {})",
                self.ticker.len()
            ));
        }
        if !self
            .ticker
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err("ticker must be UPPERCASE alphanumeric only".into());
        }
        if self.decimals > 18 {
            return Err("decimals must be 0..=18".into());
        }
        Ok(())
    }

    /// The token identifier will be TICKER-XXXXXX where XXXXXX is
    /// 3 random bytes (6 hex chars) chosen by the chain.
    pub fn expected_ticker_prefix(&self) -> String {
        self.ticker.clone()
    }

    /// Build the `data` field for the issuance transaction.
    ///
    /// Format: `issue@<hex(name)>@<hex(ticker)>@<hex(supply)>@<hex(decimals)>`
    /// followed by property pairs: `@<hex(canXyz)>@<hex("true"/"false")>`.
    pub fn build_data_field(&self) -> String {
        let supply = if self.initial_supply == 0 {
            // Supply 0 → hex "00" (not "" — protocol expects even length).
            "00".to_string()
        } else {
            format!("{:02x}", self.initial_supply)
        };
        let mut d = format!(
            "issue@{}@{}@{supply}@{:02x}",
            hex::encode(self.name.as_bytes()),
            hex::encode(self.ticker.as_bytes()),
            self.decimals,
        );
        // Explicitly set properties:
        //   canChangeOwner=true  → management transferable to a smart contract later
        //   canUpgrade=true      → properties upgradable later
        //   canAddSpecialRoles=true → needed to grant minting role to the
        //                             M18 escrow contract without giving
        //                             the manager wallet mint power
        //   canMint=false        → global mint path off (local mint goes through
        //                             the special role later)
        //   canBurn=false        → burning from holding account only; the manager
        //                             can't reduce other holders' balances
        //   canFreeze/canWipe/canPause=false
        d.push_str(&format!("@{}@74727565", hex::encode(b"canChangeOwner")));
        d.push_str(&format!("@{}@74727565", hex::encode(b"canUpgrade")));
        d.push_str(&format!("@{}@74727565", hex::encode(b"canAddSpecialRoles")));
        d.push_str(&format!("@{}@66616c7365", hex::encode(b"canMint")));
        d.push_str(&format!("@{}@66616c7365", hex::encode(b"canBurn")));
        d.push_str(&format!("@{}@66616c7365", hex::encode(b"canFreeze")));
        d.push_str(&format!("@{}@66616c7365", hex::encode(b"canWipe")));
        d.push_str(&format!("@{}@66616c7365", hex::encode(b"canPause")));
        d
    }
}

// ── Identifier extraction ───────────────────────────────────────────────────

/// Extract the issued token identifier from a confirmed issuance transaction.
///
/// After successful processing, the ESDT system SC emits a
/// `ESDTTransfer@<token>@<value>` event in a Smart Contract Result. We:
/// 1. iterate all `smartContractResults`
/// 2. base64-decode their `data` field
/// 3. look for the `ESDTTransfer` marker
/// 4. return the first topic that matches the requested ticker prefix.
pub fn extract_token_identifier(
    tx: &serde_json::Value,
    ticker_prefix: &str,
) -> Option<String> {
    let results = tx.get("smartContractResults")?.as_array()?;
    for res in results {
        let data = res.get("data")?.as_str()?;
        let decoded = B64.decode(data).ok()?;
        let txt = String::from_utf8(decoded).ok()?;
        // Look for a @-delimited token identifier inside the event data.
        // e.g. "ESDTTransfer@DCAI-a1b2c3@00"
        for part in txt.split('@') {
            if part.starts_with(ticker_prefix) && part.len() > ticker_prefix.len() {
                // Ticker prefix itself is not enough — we require the random
                // 6-char suffix (e.g. "-a1b2c3").  Accept either full or short.
                return Some(part.to_string());
            }
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn params_zero() -> IssueParams {
        IssueParams {
            name: "DecentraAI".to_string(),
            ticker: "DCAI".to_string(),
            initial_supply: 0,
            decimals: 18,
        }
    }

    #[test]
    fn validate_rejects_bad_names() {
        assert!(IssueParams {
            name: "AB".to_string(),
            ticker: "AB".to_string(),
            initial_supply: 0,
            decimals: 18,
        }
        .validate()
        .is_err());
        assert!(IssueParams {
            name: "DecentraAI".to_string(),
            ticker: "dcai".to_string(), // lowercase
            initial_supply: 0,
            decimals: 18,
        }
        .validate()
        .is_err());
        assert!(IssueParams {
            name: "DecentraAI".to_string(),
            ticker: "DCAI".to_string(),
            initial_supply: 0,
            decimals: 19,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn data_field_encodes_known_vector() {
        let got = params_zero().build_data_field();
        // name    "DecentraAI"   = hex 446563656e7472614149
        // ticker  "DCAI"         = hex 44434149
        // supply  0              = hex 00
        // decimals 18            = hex 12
        assert!(got.starts_with("issue@446563656e7472614149@44434149@00@12"));
        assert!(got.contains("@63616e4368616e67654f776e6572@74727565")); // canChangeOwner=true
        assert!(got.contains("@63616e55706772616465@74727565")); // canUpgrade=true
        assert!(got.contains("@63616e4164645370656369616c526f6c6573@74727565")); // canAddSpecialRoles=true
        assert!(got.contains("@63616e4d696e74@66616c7365")); // canMint=false
        assert!(got.contains("@63616e4275726e@66616c7365")); // canBurn=false
        assert!(got.contains("@63616e467265657a65@66616c7365")); // canFreeze=false
        assert!(got.contains("@63616e57697065@66616c7365")); // canWipe=false
        assert!(got.contains("@63616e5061757365@66616c7365")); // canPause=false
    }

    #[test]
    fn extract_rejects_empty() {
        let tx = serde_json::json!({"smartContractResults": []});
        assert!(extract_token_identifier(&tx, "DCAI").is_none());
        let tx = serde_json::json!({});
        assert!(extract_token_identifier(&tx, "DCAI").is_none());
    }

    #[test]
    fn extract_finds_identifier_from_scr() {
        // bytes of "ESDTTransfer@DCAI-a1b2c3@00", base64-encoded.
        let raw = "ESDTTransfer@DCAI-a1b2c3@00";
        let tx = serde_json::json!({
            "smartContractResults": [
                { "data": base64::engine::general_purpose::STANDARD.encode(raw) }
            ]
        });
        assert_eq!(
            extract_token_identifier(&tx, "DCAI"),
            Some("DCAI-a1b2c3".to_string())
        );
        assert_eq!(extract_token_identifier(&tx, "XXXX"), None);
    }
}
