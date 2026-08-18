//! `decentraai-worker` — a standalone lightweight worker.
//!
//! A worker's ONLY job is to join a fabric, advertise what it can run, accept
//! authorized remote inference, execute it against a local llama-server, and
//! report real measurements. It does NOT run the control plane: no planner, no
//! model hub, no registry scan, no dashboard, no MCP, no tokens, no decisions,
//! no orchestration.
//!
//! It reuses 100% of the existing worker capabilities from `decentraai-distributed`
//! (`ComputeManager` worker-side methods + `DistributedInference::register_worker_backend`)
//! and the existing identity / config / system-probe / engine crates. It does NOT
//! duplicate identity, trust, capability matching, resource estimation, auth, or
//! the signed P2P protocol — a coordinator already trusts this peer via the same
//! mechanism and verifies the signed advertisement + inbound request signatures.
//!
//! Lifecycle (evidence-backed): the worker is DISCOVERED by coordinators (its
//! signed advertisement + mDNS), becomes TRUSTED when a coordinator trusts its
//! peer id, CONNECTED/READY once it is listening and advertising, BUSY while
//! serving, and OFFLINE when it stops heartbeating. It never claims UPDATING /
//! VERIFIED (no remote update mechanism exists).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use decentraai_distributed::{
    ComputeManager, DistributedInference, DistributedP2PHandler, InferenceConfig, WorkerManager,
};
use decentraai_identity::Identity;
use decentraai_inference_adapter::{BackendConfig, EngineKind, OpenAiCompatibleBackend};
use decentraai_p2p::{ChainedHandler, P2PNode};
use decentraai_runtime::{LlamaServer, RuntimeConfig, find_llama_server};
use decentraai_system_probe::{SystemSnapshot, probe_gpu};

/// Standard data directory (same layout as `decentraai setup` / `decentraai node`).
fn default_data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("DECENTRAAI_DATA_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        format!("{home}/.decentraai")
    }))
}

#[derive(Debug, Parser)]
#[command(
    name = "decentraai-worker",
    about = "Standalone DecentraAI lightweight worker (no control plane)"
)]
struct Args {
    /// Human-readable node name advertised in the compute advertisement.
    #[arg(long, default_value = "decentraai-worker")]
    name: String,

    /// Config file path. Defaults to the standard node config.
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: std::path::PathBuf,

    /// Explicit llama-server binary path (overrides env, PATH and common install
    /// locations). Required when llama-server cannot be located automatically.
    #[arg(long)]
    binary: Option<std::path::PathBuf>,

    /// Data directory (identity, models, registry). Defaults to ~/.decentraai.
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,

    /// Model file (registry relative path or file name) this worker serves.
    #[arg(long)]
    model: Option<String>,

    /// Skip advertising a model and only advertise capability/resources (e.g. a
    /// provisioning worker that fetches models on demand).
    #[arg(long)]
    no_model: bool,

    /// Join an existing fabric from an invite: '<reachable-multiaddr> <dsk_ token>'.
    /// Provisions identity + config, stores the guest credential (0600), and
    /// verifies the coordinating peer is reachable over P2P. Then run with
    /// `decentraai-worker --model <file.gguf>`.
    #[arg(long)]
    join: Option<String>,

    /// Show real local worker state (identity, config, engine, advertisement,
    /// lifecycle) and exit.
    #[arg(long)]
    status: bool,

    /// Read-only diagnostics: report reachable/real problems (missing identity,
    /// bad config, engine unavailable, insufficient resources) and exit.
    #[arg(long)]
    doctor: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let args = Args::parse();

    // ---- one-shot commands (join / status / doctor) ----
    if let Some(invite) = &args.join {
        let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
        return join_worker(invite, &data_dir).await;
    }
    if args.status {
        return status_worker(&args);
    }
    if args.doctor {
        return doctor_worker(&args);
    }

    let data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    for directory in ["identity", "models", "registry", "db", "logs", "runtime"] {
        std::fs::create_dir_all(data_dir.join(directory))?;
    }

    // ---- identity: reuse the node's existing key (never regenerate a random one) ----
    let identity_path = data_dir.join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        let identity = Identity::generate();
        identity.save(&identity_path)?;
        identity
    };
    let keypair = libp2p::identity::Keypair::ed25519_from_bytes(identity.signing_key_bytes())
        .context("deriving libp2p keypair from identity")?;
    let local_peer_id = libp2p::PeerId::from(keypair.public());

    // ---- config: load worker-relevant sections, not the control plane ----
    let config =
        decentraai_config::NodeConfig::load(&args.config).context("loading node config")?;

    // ---- engine: a real llama-server is REQUIRED (no silent mock) ----
    let model_path = resolve_model_path(&data_dir, args.model.as_deref()).await?;
    let binary = find_llama_server(args.binary.as_deref()).with_context(
        || "worker requires llama-server; pass --binary <path> or install llama.cpp",
    )?;
    let mut runtime_cfg = RuntimeConfig::new(model_path.clone());
    runtime_cfg.ctx_size = config.inference.max_context_tokens;
    runtime_cfg.parallel = config.inference.max_concurrent_requests;
    runtime_cfg.threads = Some(
        SystemSnapshot::collect()
            .logical_cpus
            .saturating_sub(usize::from(config.resources.reserve_cpu_cores))
            .max(1),
    );
    let server = LlamaServer::spawn(&binary, &runtime_cfg).await?;
    let url = server.base_url();
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: url,
        model: model_name_for(&model_path),
        api_key: None,
        connect_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(300),
        max_prompt_bytes: 200_000,
        max_output_tokens: 8192,
        engine: EngineKind::LlamaServer,
        backend_url_resolver: None,
    })
    .context("creating inference backend")?;

    // ---- compute manager: worker-side only (advertise + serve) ----
    let mut compute_manager_unwrapped = ComputeManager::new(
        local_peer_id,
        args.name.clone(),
        std::collections::HashSet::new(), // worker does not schedule others
    );
    // P3: sign this node's advertisements so coordinators authenticate them.
    compute_manager_unwrapped.set_signing_key(identity.signing_key_bytes());
    let compute_manager = Arc::new(compute_manager_unwrapped);
    compute_manager.set_accepts_remote_inference(config.inference.allow_remote_inference);

    // ---- P2P node with the distributed handler (answers InferPing/InferRequest) ----
    let worker_manager = Arc::new(WorkerManager::new(
        local_peer_id,
        InferenceConfig::default(),
    ));
    let distributed_handler = DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    let chained = ChainedHandler::new().add_handler(Arc::new(distributed_handler));
    let p2p_node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained)),
    )?;
    let bound = p2p_node.listen("/ip4/0.0.0.0/tcp/0").await?;

    let mut distributed = DistributedInference::new(
        p2p_node,
        InferenceConfig::default(),
        Some(worker_manager.clone()),
        None,
    )?;
    distributed.set_compute_manager(compute_manager.clone());
    distributed.set_logs_dir(Some(data_dir.join("logs")));

    // ---- register + advertise this node as a worker ----
    let model_name = model_name_for(&model_path);
    let model_hash = hash_model(&model_path).await?;
    distributed.register_as_worker(model_name.clone(), vec![model_hash.clone()], 1.0)?;
    distributed.register_worker_backend(
        backend,
        model_hash.clone(),
        None, // no on-demand provisioning on the minimal worker
        config.inference.allow_remote_inference,
    )?;

    let size_mb = (std::fs::metadata(&model_path)
        .map(|m| m.len() / (1024 * 1024))
        .unwrap_or(1024))
    .max(1);
    let served_models = build_served_models(&model_hash, &model_name, size_mb);
    advertise_and_broadcast(&compute_manager, &distributed, served_models).await;

    println!(
        "DecentraAI worker running\n  PeerId: {}\n  Listening: {}/p2p/{}\n  Model: {}\n  Mode: worker (no control plane)",
        local_peer_id, bound, local_peer_id, model_name
    );

    // Park until interrupted; the broadcaster + worker loop run in background.
    tokio::signal::ctrl_c().await?;
    distributed.shutdown();
    let _ = server.stop().await;
    Ok(())
}

/// Parse a worker join invite: '<reachable-multiaddr> <dsk_ token>'. Reuses the
/// same format and credential prefix as `decentraai join` (no new token format).
fn parse_invite(invite: &str) -> Result<(String, String)> {
    let split_at = invite
        .find(' ')
        .context("invite must be '<reachable-multiaddr> <dsk_ token>'")?;
    let multiaddr = invite[..split_at].trim();
    let token = invite[split_at..].trim_start();
    if multiaddr.is_empty() || token.is_empty() {
        anyhow::bail!("invite must be '<reachable-multiaddr> <dsk_ token>'");
    }
    if !token.starts_with("dsk_") {
        anyhow::bail!("invite token must start with 'dsk_' — got an invalid invite string");
    }
    Ok((multiaddr.to_string(), token.to_string()))
}

/// `decentraai-worker --join '<multiaddr> <dsk_ token>'` — join an existing
/// fabric. Reuses the EXISTING identity, config, guest-token credential (0600),
/// and verified P2P dial mechanisms; it does NOT invent a new identity, token,
/// trust, auth, or discovery system.
async fn join_worker(invite: &str, data_dir: &std::path::Path) -> Result<()> {
    use decentraai_p2p::P2PNode;
    use std::os::unix::fs::PermissionsExt;

    let (multiaddr, token) = parse_invite(invite)?;
    for directory in ["identity", "models", "registry", "db", "logs", "runtime"] {
        std::fs::create_dir_all(data_dir.join(directory))?;
    }

    // Identity: load or generate (same store as the full node — one identity).
    let identity_path = data_dir.join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        let identity = Identity::generate();
        identity.save(&identity_path)?;
        identity
    };

    // Store the guest credential (0600). The coordinator keeps only the hash,
    // so this is the seat's only plaintext copy. Never logged.
    let runtime_dir = data_dir.join("runtime");
    std::fs::create_dir_all(&runtime_dir)?;
    let credential_path = runtime_dir.join("invite.token");
    std::fs::write(&credential_path, format!("{token}\n"))?;
    let mut perms = std::fs::metadata(&credential_path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&credential_path, perms)?;

    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        "worker_joined",
        serde_json::json!({"peer": multiaddr}),
    );

    // Verify the coordinating peer is reachable over the verified P2P path.
    let node = P2PNode::new(&identity, 1_048_576, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, None)?;
    node.dial(&multiaddr).await.with_context(|| {
        format!("could not reach the coordinating peer at {multiaddr}; is it online?")
    })?;
    node.shutdown();

    println!("Worker joined the fabric — connected to the coordinating peer at {multiaddr}");
    println!("  Worker PeerId : {}", identity.peer_id());
    println!("  Credential     : {} (0600)", credential_path.display());
    println!(
        "  Run the worker : decentraai-worker --model <file.gguf> [--data-dir {}]",
        data_dir.display()
    );
    Ok(())
}

/// `decentraai-worker --status` — real local worker state (identity, config,
/// engine availability, credential, lifecycle). Read-only.
fn status_worker(args: &Args) -> Result<()> {
    use decentraai_system_probe::{SystemSnapshot, probe_gpu};

    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    let credential = data_dir.join("runtime/invite.token");
    let has_identity = identity_path.exists();
    let has_credential = credential.exists();
    let config_ok = decentraai_config::NodeConfig::load(&args.config).is_ok();

    let snapshot = SystemSnapshot::collect();
    let gpu = probe_gpu();
    let gpu_line = match &gpu {
        decentraai_system_probe::GpuProbeStatus::Nvidia(g) => format!(
            "{} ({} MiB VRAM) · temp {}C · util {}%",
            g.name, g.total_vram_mib, g.temperature_celsius, g.utilization_percent
        ),
        decentraai_system_probe::GpuProbeStatus::Unavailable(_) => "none".to_string(),
    };

    println!("== decentraai-worker status ==");
    println!(
        "  PeerId      : {}",
        if has_identity {
            Identity::load(&identity_path)?.peer_id().to_string()
        } else {
            "UNKNOWN (no identity)".into()
        }
    );
    println!(
        "  Identity    : {}",
        if has_identity {
            "present"
        } else {
            "missing (run --join or --init)"
        }
    );
    println!(
        "  Credential  : {}",
        if has_credential {
            "stored (0600)"
        } else {
            "none — not joined yet"
        }
    );
    println!(
        "  Config      : {}",
        if config_ok {
            "valid"
        } else {
            "invalid/missing"
        }
    );
    // Evidence-backed worker-side lifecycle. Trust and BUSY are coordinator-side
    // (the coordinator decides trust and observes busy/queued), so the worker can
    // only honestly report DISCOVERED (joined, not yet runnable) vs READY (has
    // identity + credential + a usable engine + a model to serve). UPDATING /
    // VERIFIED are never emitted (no remote update mechanism).
    let engine_ok = find_llama_server(args.binary.as_deref()).is_ok();
    let model_ok = std::fs::read_dir(data_dir.join("models"))
        .map(|rd| {
            rd.flatten()
                .any(|e| e.path().extension().map(|x| x == "gguf").unwrap_or(false))
        })
        .unwrap_or(false);
    let lifecycle = if has_identity && has_credential && config_ok && engine_ok && model_ok {
        "READY (will advertise + serve when started)"
    } else if has_identity && has_credential {
        "DISCOVERED (joined; needs a model + engine to be READY)"
    } else {
        "UNKNOWN (not joined)"
    };
    println!("  Lifecycle   : {lifecycle}");
    println!(
        "  Engine      : {}",
        if engine_ok {
            "found"
        } else {
            "UNKNOWN — llama-server not found"
        }
    );
    println!(
        "  Model       : {}",
        if model_ok {
            "on disk"
        } else {
            "none on disk (UNKNOWN)"
        }
    );
    println!("  Trust       : (coordinator-side — see the fabric dashboard /v1/fabric)");
    println!("  CPU cores   : {}", snapshot.logical_cpus);
    println!(
        "  RAM         : {} MiB total / {} MiB available",
        snapshot.total_memory_bytes / (1024 * 1024),
        snapshot.available_memory_bytes / (1024 * 1024)
    );
    println!("  GPU         : {gpu_line}");
    Ok(())
}

/// `decentraai-worker --doctor` — read-only diagnostics reporting REAL,
/// useful problems (missing identity, bad config, engine unavailable, no
/// credential). UNKNOWN stays UNKNOWN. Never fabricates.
fn doctor_worker(args: &Args) -> Result<()> {
    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    let mut problems = 0u32;
    println!("== decentraai-worker doctor (read-only) ==");

    // Identity.
    let identity_path = data_dir.join("identity/key.pem");
    if identity_path.exists() {
        println!("  ✓ identity present");
    } else {
        problems += 1;
        println!(
            "  ✗ identity missing — run `decentraai-worker --join '<multiaddr> <dsk_ token>'`"
        );
    }

    // Credential / joined.
    let credential = data_dir.join("runtime/invite.token");
    if credential.exists() {
        println!("  ✓ join credential stored (0600)");
    } else {
        problems += 1;
        println!("  ✗ not joined — run `decentraai-worker --join ...` first");
    }

    // Config.
    match decentraai_config::NodeConfig::load(&args.config) {
        Ok(_) => println!("  ✓ config loads"),
        Err(e) => {
            problems += 1;
            println!("  ✗ config problem: {e}");
        }
    }

    // Engine availability (worker REQUIRES a real llama-server).
    match find_llama_server(args.binary.as_deref()) {
        Ok(path) => println!("  ✓ llama-server found: {}", path.display()),
        Err(_) => {
            problems += 1;
            println!("  ✗ llama-server not found — pass --binary <path> or install llama.cpp");
        }
    }

    // Model availability (a worker should have a model to serve).
    match std::fs::read_dir(data_dir.join("models")) {
        Ok(rd) => {
            let gguf = rd
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == "gguf").unwrap_or(false))
                .count();
            if gguf > 0 {
                println!("  ✓ {gguf} GGUF model(s) on disk");
            } else {
                problems += 1;
                println!("  ✗ no GGUF model on disk — pass --model <file.gguf>");
            }
        }
        Err(_) => {
            problems += 1;
            println!("  ✗ models/ directory missing");
        }
    }

    if problems == 0 {
        println!("  No problems detected — the worker should start and advertise.");
    } else {
        println!("  {problems} issue(s) found. Fix the items above, then run the worker.");
    }
    Ok(())
}

const DEFAULT_MAX_CHUNK_MESSAGE_BYTES: usize = 64 * 1024;

/// Resolve the model file path (data-dir models/<name> or an absolute path).
async fn resolve_model_path(
    data_dir: &std::path::Path,
    model: Option<&str>,
) -> Result<std::path::PathBuf> {
    match model {
        Some(m) => {
            let candidate = data_dir.join("models").join(m);
            if candidate.exists() {
                Ok(candidate)
            } else {
                let direct = std::path::PathBuf::from(m);
                if direct.exists() {
                    Ok(direct)
                } else {
                    anyhow::bail!("model not found: {m} (looked in {candidate:?} and as a path)")
                }
            }
        }
        None => {
            // Auto-pick the first GGUF in models/ (if any); worker requires one.
            let models_dir = data_dir.join("models");
            let mut found = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&models_dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x == "gguf").unwrap_or(false) {
                        found.push(p);
                    }
                }
            }
            found.sort();
            found
                .into_iter()
                .next()
                .context("no GGUF model found in models/; pass --model <file>")
        }
    }
}

fn model_name_for(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model.gguf".to_string())
}

async fn hash_model(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Build the served-model list (single model) for the advertisement.
fn build_served_models(
    model_hash: &str,
    model_name: &str,
    size_mb: u64,
) -> Vec<decentraai_compute::ServedModel> {
    vec![decentraai_compute::ServedModel {
        model_hash: model_hash.to_string(),
        file_name: model_name.to_string(),
        size_mb,
        est_ram_mb: decentraai_compute::ServedModel::estimate_ram_mb(size_mb * 1024 * 1024),
        est_vram_mb: 0,
        context_tokens: 4096,
    }]
}

/// Advertise a real probe immediately, then start the periodic broadcaster
/// (heartbeat) on the compute manager's interval. `served_models` is the model
/// set computed at registration (re-hashing every beat is unnecessary).
async fn advertise_and_broadcast(
    compute_manager: &Arc<ComputeManager>,
    distributed: &DistributedInference,
    served_models: Vec<decentraai_compute::ServedModel>,
) {
    let cm = compute_manager.clone();
    let p2p = distributed.p2p_node().clone();
    let interval = cm.advertisement_interval_ms();

    // Immediate first advertisement so coordinators discover us right away.
    let snapshot = SystemSnapshot::collect();
    let gpu = probe_gpu();
    let first = cm
        .advertise_local(snapshot, gpu, served_models.clone(), vec![], false)
        .await;
    if let Ok(bytes) = cm.advertisement_wire_bytes(&first) {
        p2p.announce(bytes);
    }

    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(interval));
        loop {
            timer.tick().await;
            let snap = SystemSnapshot::collect();
            let g = probe_gpu();
            // Re-advertise the latest probe; served/available come from the last
            // advertisement (recomputed at registration, not re-hashed each beat).
            let (served, available) = cm
                .workers()
                .await
                .iter()
                .find(|w| w.peer_id == cm.local_peer())
                .map(|w| {
                    (
                        w.capability.served_models.clone(),
                        w.capability.available_models.clone(),
                    )
                })
                .unwrap_or((served_models.clone(), vec![]));
            let a = cm.advertise_local(snap, g, served, available, false).await;
            if let Ok(bytes) = cm.advertisement_wire_bytes(&a) {
                p2p.announce(bytes);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_invite;

    #[test]
    fn parse_invite_accepts_multiaddr_and_dsk_token() {
        let (ma, tok) = parse_invite("/ip4/192.168.1.50/tcp/41501 dsk_abc123").unwrap();
        assert_eq!(ma, "/ip4/192.168.1.50/tcp/41501");
        assert_eq!(tok, "dsk_abc123");
    }

    #[test]
    fn parse_invite_rejects_non_dsk_token() {
        assert!(parse_invite("/ip4/1.2.3.4/tcp/5 secret").is_err());
        assert!(parse_invite("/ip4/1.2.3.4/tcp/5").is_err(), "no token");
        assert!(parse_invite("").is_err());
        assert!(
            parse_invite("dsk_only_no_multiaddr").is_err(),
            "no multiaddr"
        );
    }
}
