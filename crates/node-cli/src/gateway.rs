//! M16 gateway operator ceremony (CLI-first — NO HTTP issuance in v0.1).
//!
//! Master-premise commands (run on the node machine by the operator):
//! `gateway issue` / `show` / `revoke` / `list`. The secret is shown
//! EXACTLY ONCE at issuance and zeroized after display; the store keeps
//! only its BLAKE3 hash. Every issuance/revocation appends an audit event
//! (ids and metadata only — never secrets).
//!
//! Pure parameter validation lives in [`validate_issuance`] (unit-tested);
//! this module only wires config + store + audit around it.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

/// Gateway credential operator commands (local CLI = operator premise).
#[derive(Debug, Subcommand)]
pub enum GatewayCommand {
    /// Issue a BYOA agent credential (secret shown once).
    Issue(GatewayIssueArgs),
    /// Show redacted metadata for one credential (never the secret).
    Show(GatewayShowArgs),
    /// Revoke a credential by id (immediate effect, audited).
    Revoke(GatewayRevokeArgs),
    /// List all gateway credentials (redacted metadata).
    List(GatewayListArgs),
}

#[derive(Debug, Args)]
pub struct GatewayIssueArgs {
    /// Agent display name (`1..=32 [a-zA-Z0-9_-]`).
    #[arg(long)]
    pub agent: String,
    /// Granted capabilities, comma-separated hub-taxonomy names (non-empty).
    #[arg(long)]
    pub capabilities: String,
    /// Owner account in the quota ledger (draws quota like `dca_`).
    #[arg(long)]
    pub account: String,
    /// Per-request quota ceiling (> 0, ≤ section max).
    #[arg(long)]
    pub quota: u64,
    /// Max requests per minute (> 0, ≤ section max).
    #[arg(long, default_value_t = 60)]
    pub rate: u32,
    /// TTL in days (`[1, 30]`, default 7; also capped by section expiry).
    #[arg(long, default_value_t = 7)]
    pub ttl_days: u64,
    /// Node config (data dir + `agent_gateway` section).
    #[arg(long, default_value = "configs/node.example.yaml")]
    pub config: PathBuf,
}

#[derive(Debug, Args)]
pub struct GatewayShowArgs {
    /// Credential id (`gk-…`).
    pub id: String,
    #[arg(long, default_value = "configs/node.example.yaml")]
    pub config: PathBuf,
}

#[derive(Debug, Args)]
pub struct GatewayRevokeArgs {
    /// Credential id (`gk-…`).
    pub id: String,
    #[arg(long, default_value = "configs/node.example.yaml")]
    pub config: PathBuf,
}

#[derive(Debug, Args)]
pub struct GatewayListArgs {
    /// Machine-readable output.
    #[arg(long, default_value_t = false)]
    pub json: bool,
    #[arg(long, default_value = "configs/node.example.yaml")]
    pub config: PathBuf,
}

/// Validated issuance parameters (pure output of [`validate_issuance`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuanceParams {
    /// Agent name (validated).
    pub agent: String,
    /// Granted taxonomy capabilities (validated, non-empty).
    pub capabilities: Vec<String>,
    /// Owner ledger account (validated non-empty).
    pub account: String,
    /// Quota ceiling (validated).
    pub quota: u64,
    /// Rate limit (validated).
    pub rate: u32,
    /// Expiry unix seconds (TTL + section cap applied).
    pub expires_at: u64,
}

/// Issuance request bundle (keeps [`validate_issuance`] honest: free-arg
/// lists hide coupling; a struct makes the ceremony explicit).
#[derive(Debug, Clone)]
pub struct IssuanceRequest<'a> {
    /// Agent display name.
    pub agent: &'a str,
    /// Granted capabilities, comma-separated taxonomy names.
    pub capabilities_csv: &'a str,
    /// Owner ledger account.
    pub account: &'a str,
    /// Per-request quota ceiling.
    pub quota: u64,
    /// Max requests per minute.
    pub rate: u32,
    /// TTL in days (`[1, 30]`).
    pub ttl_days: u64,
    /// Section caps (None = scope bounds only).
    pub section: Option<&'a decentraai_config::AgentGatewaySection>,
    /// Now (unix seconds, injected for deterministic tests).
    pub now_unix: u64,
}

/// Pure issuance validation: shapes, taxonomy membership, section caps
/// (`allowed_capabilities` intersect when non-empty, quota/rate ceilings,
/// TTL `[1,30]` days AND section `max_expiry_seconds` when > 0).
pub fn validate_issuance(request: &IssuanceRequest<'_>) -> Result<IssuanceParams> {
    use decentraai_agents::gateway::validate_ttl_days;
    let agent = request.agent.trim();
    if agent.is_empty()
        || agent.len() > 32
        || !agent
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("agent name must be 1..=32 chars [a-zA-Z0-9_-]");
    }
    let capabilities: Vec<String> = request
        .capabilities_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if capabilities.is_empty() {
        anyhow::bail!("capabilities must not be empty (deny-by-default needs a grant)");
    }
    for cap in &capabilities {
        if cap
            .parse::<decentraai_hub::capability::CapabilityKind>()
            .is_err()
        {
            anyhow::bail!("unknown capability '{cap}' (must exist in the hub taxonomy)");
        }
    }
    let account = request.account.trim();
    if account.is_empty() {
        anyhow::bail!("owner account must not be empty");
    }
    validate_ttl_days(request.ttl_days).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut expires_at =
        request.now_unix + request.ttl_days * decentraai_agents::gateway::SECONDS_PER_DAY;
    if let Some(section) = request.section {
        if !section.allowed_capabilities.is_empty() {
            for cap in &capabilities {
                if !section.allowed_capabilities.iter().any(|a| a == cap) {
                    anyhow::bail!(
                        "capability '{cap}' is not in agent_gateway.allowed_capabilities"
                    );
                }
            }
        }
        if request.quota == 0 || request.quota > section.max_quota_ceiling {
            anyhow::bail!("quota must be within [1, {}]", section.max_quota_ceiling);
        }
        if request.rate == 0 || request.rate > section.max_rate_limit {
            anyhow::bail!("rate must be within [1, {}]", section.max_rate_limit);
        }
        if section.max_expiry_seconds > 0 {
            let cap = request.now_unix + section.max_expiry_seconds;
            if expires_at > cap {
                anyhow::bail!(
                    "ttl exceeds agent_gateway.max_expiry_seconds ({}s); raise the cap or shorten --ttl-days",
                    section.max_expiry_seconds
                );
            }
            expires_at = expires_at.min(cap);
        }
    } else if request.quota == 0 {
        anyhow::bail!("quota must be > 0");
    } else if request.rate == 0 {
        anyhow::bail!("rate must be > 0");
    }
    Ok(IssuanceParams {
        agent: agent.to_string(),
        capabilities,
        account: account.to_string(),
        quota: request.quota,
        rate: request.rate,
        expires_at,
    })
}

fn gateway_store_path(
    config_path: &PathBuf,
) -> Result<(
    PathBuf,
    Option<decentraai_config::AgentGatewaySection>,
    PathBuf,
)> {
    use decentraai_config::NodeConfig;
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = super::expand_tilde(&config.node.data_dir);
    let store_path = data_dir.join("db/gateway_keys.json");
    let logs_dir = data_dir.join("logs");
    Ok((store_path, config.agent_gateway.clone(), logs_dir))
}

pub fn gateway_command(command: GatewayCommand) -> Result<()> {
    use decentraai_tokens::GatewayKeyStore;
    match command {
        GatewayCommand::Issue(args) => {
            let (store_path, section, logs_dir) = gateway_store_path(&args.config)?;
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let params = validate_issuance(&IssuanceRequest {
                agent: &args.agent,
                capabilities_csv: &args.capabilities,
                account: &args.account,
                quota: args.quota,
                rate: args.rate,
                ttl_days: args.ttl_days,
                section: section.as_ref(),
                now_unix,
            })?;
            if section.as_ref().is_none_or(|s| !s.enabled) {
                println!(
                    "warn: agent_gateway is not enabled — this credential will NOT authenticate until enabled=true"
                );
            }
            let mut store = GatewayKeyStore::load(&store_path).with_context(|| {
                format!("loading gateway registry from {}", store_path.display())
            })?;
            let plaintext = store.create(
                &params.agent,
                params.capabilities.clone(),
                &params.account,
                params.quota,
                params.rate,
                params.expires_at,
            )?;
            let key_id = store
                .lookup(&plaintext)
                .map(|r| r.key_id.clone())
                .unwrap_or_default();
            decentraai_audit::record_best_effort(
                &logs_dir,
                "gateway_credential_issued",
                serde_json::json!({
                    "key_id": key_id,
                    "agent": params.agent,
                    "capabilities": params.capabilities,
                    "account": params.account,
                    "quota_ceiling": params.quota,
                    "rate_limit_per_minute": params.rate,
                    "expires_at": params.expires_at,
                }),
            );
            println!("Gateway credential for agent '{}':", params.agent);
            // Shown EXACTLY ONCE, then zeroized (never stored, never logged).
            let mut bytes = plaintext.into_bytes();
            println!("  {}", String::from_utf8_lossy(&bytes));
            bytes.fill(0);
            println!("Store it now: only its BLAKE3 hash is kept (id {key_id}).");
            println!(
                "Active at the next API request iff agent_gateway.enabled=true; no restart needed."
            );
            Ok(())
        }
        GatewayCommand::Show(args) => {
            let (store_path, _, _) = gateway_store_path(&args.config)?;
            let store = GatewayKeyStore::load(&store_path)?;
            let rec = store
                .list()
                .into_iter()
                .find(|r| r.key_id == args.id)
                .with_context(|| format!("no gateway credential '{}'", args.id))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "key_id": rec.key_id,
                    "agent": rec.agent_name,
                    "capabilities": rec.capabilities,
                    "account": rec.owner_account,
                    "quota_ceiling": rec.quota_ceiling,
                    "rate_limit_per_minute": rec.rate_limit_per_minute,
                    "created_at": rec.created_at,
                    "last_used_at": rec.last_used_at,
                    "revoked": rec.revoked,
                    "expires_at": rec.expires_at,
                    "prefix": rec.prefix,
                }))
                .unwrap()
            );
            Ok(())
        }
        GatewayCommand::Revoke(args) => {
            let (store_path, _, logs_dir) = gateway_store_path(&args.config)?;
            let mut store = GatewayKeyStore::load(&store_path)?;
            store.revoke(&args.id)?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "gateway_credential_revoked",
                serde_json::json!({"key_id": args.id}),
            );
            println!("Revoked {} (immediate effect).", args.id);
            Ok(())
        }
        GatewayCommand::List(args) => {
            let (store_path, _, _) = gateway_store_path(&args.config)?;
            let store = GatewayKeyStore::load(&store_path)?;
            let records = store.list();
            if args.json {
                println!("{}", serde_json::to_string(&records)?);
            } else {
                println!("Gateway credentials ({}):", records.len());
                for r in &records {
                    let status = if r.revoked { "revoked" } else { "active" };
                    println!(
                        "  {} {} caps=[{}] quota={} exp={} ({status})",
                        r.key_id,
                        r.agent_name,
                        r.capabilities.join(","),
                        r.quota_ceiling,
                        r.expires_at
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> decentraai_config::AgentGatewaySection {
        decentraai_config::AgentGatewaySection {
            enabled: true,
            max_quota_ceiling: 1000,
            max_rate_limit: 60,
            max_expiry_seconds: 30 * 86_400,
            allowed_capabilities: vec!["ocr".to_string(), "embeddings".to_string()],
            free_starter: Default::default(),
        }
    }

    fn check(
        agent: &str,
        csv: &str,
        account: &str,
        quota: u64,
        rate: u32,
        ttl: u64,
        section: Option<&decentraai_config::AgentGatewaySection>,
    ) -> Result<IssuanceParams> {
        validate_issuance(&IssuanceRequest {
            agent,
            capabilities_csv: csv,
            account,
            quota,
            rate,
            ttl_days: ttl,
            section,
            now_unix: 0,
        })
    }

    #[test]
    fn valid_issuance_computes_expiry() {
        let p = validate_issuance(&IssuanceRequest {
            agent: "ext-1",
            capabilities_csv: "ocr,embeddings",
            account: "acct",
            quota: 100,
            rate: 60,
            ttl_days: 7,
            section: Some(&section()),
            now_unix: 1_000_000,
        })
        .unwrap();
        assert_eq!(p.expires_at, 1_000_000 + 7 * 86_400);
        assert_eq!(
            p.capabilities,
            vec!["ocr".to_string(), "embeddings".to_string()]
        );
    }

    #[test]
    fn ttl_bounds_are_exact() {
        assert!(check("a", "ocr", "ac", 1, 1, 0, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 1, 1, 1, Some(&section())).is_ok());
        assert!(check("a", "ocr", "ac", 1, 1, 30, Some(&section())).is_ok());
        assert!(check("a", "ocr", "ac", 1, 1, 31, Some(&section())).is_err());
    }

    #[test]
    fn taxonomy_and_section_caps_reject() {
        assert!(check("a", "teleport", "ac", 1, 1, 7, Some(&section())).is_err());
        assert!(check("a", "chat", "ac", 1, 1, 7, Some(&section())).is_err());
        assert!(check("a", "", "ac", 1, 1, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 0, 1, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 1001, 1, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 1, 0, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 1, 61, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "ac", 1, 1, 7, Some(&section())).is_ok());
    }

    #[test]
    fn section_expiry_cap_binds() {
        let mut tight = section();
        tight.max_expiry_seconds = 86_400;
        assert!(check("a", "ocr", "ac", 1, 1, 7, Some(&tight)).is_err());
        assert!(check("a", "ocr", "ac", 1, 1, 1, Some(&tight)).is_ok());
    }

    #[test]
    fn names_and_accounts_validate() {
        assert!(check("", "ocr", "ac", 1, 1, 7, Some(&section())).is_err());
        assert!(check("bad name!", "ocr", "ac", 1, 1, 7, Some(&section())).is_err());
        assert!(check("a", "ocr", "", 1, 1, 7, Some(&section())).is_err());
    }

    #[test]
    fn no_section_means_scope_bounds_only() {
        assert!(check("a", "ocr", "ac", 1, 1, 7, None).is_ok());
        assert!(check("a", "ocr", "ac", 0, 1, 7, None).is_err());
    }
}
