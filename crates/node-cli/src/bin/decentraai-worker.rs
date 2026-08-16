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
    std::path::PathBuf::from(
        std::env::var("DECENTRAAI_DATA_DIR").unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            format!("{home}/.decentraai")
        }),
    )
}

#[derive(Debug, Parser)]
#[command(name = "decentraai-worker", about = "Standalone DecentraAI lightweight worker (no control plane)")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
    ).init();

    let args = Args::parse();
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
    let config = decentraai_config::NodeConfig::load(&args.config)
        .context("loading node config")?;

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
    let distributed_handler =
        DistributedP2PHandler::with_worker_manager(worker_manager.clone());
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

    let size_mb = (std::fs::metadata(&model_path).map(|m| m.len() / (1024 * 1024)).unwrap_or(1024))
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

const DEFAULT_MAX_CHUNK_MESSAGE_BYTES: usize = 64 * 1024;

/// Resolve the model file path (data-dir models/<name> or an absolute path).
async fn resolve_model_path(data_dir: &std::path::Path, model: Option<&str>) -> Result<std::path::PathBuf> {
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
            found.into_iter().next().context("no GGUF model found in models/; pass --model <file>")
        }
    }
}

fn model_name_for(path: &std::path::Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "model.gguf".to_string())
}

async fn hash_model(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Build the served-model list (single model) for the advertisement.
fn build_served_models(model_hash: &str, model_name: &str, size_mb: u64) -> Vec<decentraai_compute::ServedModel> {
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
                .map(|w| (w.capability.served_models.clone(), w.capability.available_models.clone()))
                .unwrap_or((served_models.clone(), vec![]));
            let a = cm.advertise_local(snap, g, served, available, false).await;
            if let Ok(bytes) = cm.advertisement_wire_bytes(&a) {
                p2p.announce(bytes);
            }
        }
    });
}
