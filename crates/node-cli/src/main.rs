use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use decentraai_config::NodeConfig;
use decentraai_identity::Identity;
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "decentraai", version, about = "DecentraAI node control CLI")]
struct Cli {
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
    Doctor(DoctorArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
    Swarm {
        #[command(subcommand)]
        command: SwarmCommand,
    },
    Serve {
        #[command(subcommand)]
        command: ServeCommand,
    },
    Pull(PullArgs),
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    Worker(WorkerArgs),
    Distributed(DistributedArgs),
}
#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = "~/.decentraai")]
    data_dir: String,
}
#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: PathBuf,
}
#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Validate {
        #[arg(long, default_value = "configs/node.example.yaml")]
        file: PathBuf,
    },
}
#[derive(Debug, Args)]
struct ScanArgs {
    #[arg(long, default_value = "~/.decentraai/models")]
    directory: String,
    #[arg(long, default_value = "~/.decentraai/db/registry.json")]
    registry: String,
}
#[derive(Debug, Subcommand)]
enum RegistryCommand {
    Scan(ScanArgs),
    List {
        #[arg(long, default_value = "~/.decentraai/db/registry.json")]
        registry: String,
    },
}
#[derive(Debug, Subcommand)]
enum SwarmCommand {
    Start {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
enum ServeCommand {
    Start {
        /// Model reference: registry relative path or direct file path.
        /// Omit it to pick interactively from the registry.
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Explicit llama-server binary path (overrides env and PATH search).
        #[arg(long)]
        binary: Option<PathBuf>,
    },
}
#[derive(Debug, Args)]
struct WorkerArgs {
    #[arg(long)]
    name: String,
}

#[derive(Debug, Args)]
struct DistributedArgs {
    /// Model reference: registry relative path or direct file path.
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: PathBuf,
    /// Explicit llama-server binary path (overrides env, PATH and common
    /// install locations). Required for worker mode when llama-server
    /// cannot be located automatically.
    #[arg(long)]
    binary: Option<PathBuf>,
    /// One-shot client mode: route this prompt to the best available worker
    /// and stream the response, then exit. Omit it to run as a persistent
    /// coordinator (or worker) node.
    #[arg(long)]
    prompt: Option<String>,
    /// Human-readable node name advertised in compute advertisements.
    #[arg(long, default_value = "decentraai-node")]
    name: String,
}
#[derive(Debug, Args)]
struct PullArgs {
    /// Peer address, e.g. /ip4/192.168.1.5/tcp/4001/p2p/<PEER_ID>
    #[arg(long)]
    from: String,
    /// Model file name or manifest id from the peer's catalog.
    #[arg(long)]
    model: Option<String>,
    /// Only list the peer's catalog, without downloading.
    #[arg(long)]
    list: bool,
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: PathBuf,
}
#[derive(Debug, Subcommand)]
enum TokenCommand {
    /// Issue a subscription token; printed once, stored only as a hash.
    Create {
        #[arg(long)]
        name: String,
        /// 1 = guest, 2 = contributor, 3 = core.
        #[arg(long)]
        tier: u8,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show every issued token (active and revoked).
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Revoke a token by name; it stops working immediately.
    Revoke {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(cli.log_level))
        .with_target(false)
        .init();
    match cli.command {
        Command::Init(args) => init(args),
        Command::Doctor(args) => doctor(args),
        Command::Config {
            command: ConfigCommand::Validate { file },
        } => validate_config(file),
        Command::Registry {
            command: RegistryCommand::Scan(args),
        } => scan(args),
        Command::Registry {
            command: RegistryCommand::List { registry },
        } => list_registry(registry),
        Command::Swarm {
            command: SwarmCommand::Start { config },
        } => swarm_start(config).await,
        Command::Serve {
            command:
                ServeCommand::Start {
                    model,
                    config,
                    binary,
                },
        } => serve_start(config, model, binary).await,
        Command::Pull(args) => pull(args).await,
        Command::Token { command } => token_command(command),
        Command::Worker(args) => worker_command(args),
        Command::Distributed(args) => distributed_command(args).await,
    }
}
fn init(args: InitArgs) -> Result<()> {
    let data_dir = expand_tilde(&args.data_dir);
    for directory in [
        "config",
        "identity",
        "cache/chunks",
        "cache/partial",
        "models",
        "quarantine",
        "db",
        "logs",
        "runtime",
    ] {
        fs::create_dir_all(data_dir.join(directory))
            .with_context(|| format!("creating {directory}"))?;
    }

    // Generate identity if it doesn't exist
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        let identity = Identity::generate();
        identity
            .save(&identity_path)
            .with_context(|| format!("saving identity to {}", identity_path.display()))?;
        info!(peer_id = %identity.peer_id(), "generated new identity");
        println!("Generated new identity with PeerId: {}", identity.peer_id());
    } else {
        let identity = Identity::load(&identity_path)
            .with_context(|| format!("loading identity from {}", identity_path.display()))?;
        info!(peer_id = %identity.peer_id(), "existing identity found");
        println!(
            "Existing identity found with PeerId: {}",
            identity.peer_id()
        );
    }

    info!(path = %data_dir.display(), "node data directories initialized");
    Ok(())
}
fn doctor(args: DoctorArgs) -> Result<()> {
    let config = NodeConfig::load(&args.config)
        .with_context(|| format!("loading {}", args.config.display()))?;

    // Load and display identity
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    let peer_id = if identity_path.exists() {
        let identity = Identity::load(&identity_path)
            .with_context(|| format!("loading identity from {}", identity_path.display()))?;
        identity.peer_id().to_string()
    } else {
        "not initialized (run 'decentraai init')".to_string()
    };

    let snapshot = SystemSnapshot::collect();
    let budget = snapshot.derive_budget(
        &config.resources,
        config.storage.max_cache_gb,
        config.storage.min_free_disk_gb,
    );
    let gpu = probe_gpu();
    let admission =
        snapshot.admit_inference(&budget, &gpu, config.resources.stop_gpu_temperature_celsius);
    let gpu_report = match &gpu {
        GpuProbeStatus::Nvidia(info) => format!(
            "{} | free VRAM: {} MiB | temperature: {} C",
            info.name, info.free_vram_mib, info.temperature_celsius
        ),
        GpuProbeStatus::Unavailable(reason) => format!("unavailable ({reason})"),
    };
    let verdict = match &admission {
        AdmissionDecision::Admit => "admit",
        AdmissionDecision::Reject(reason) => reason.as_str(),
    };
    println!(
        "DecentraAI node report\n  PeerId: {}\n  CPU threads budget: {}\n  RAM budget: {:.2} GiB\n  Cache budget: {:.2} GiB\n  GPU: {}\n  Inference admission: {}",
        peer_id,
        budget.max_cpu_threads,
        bytes_to_gib(budget.max_memory_bytes),
        bytes_to_gib(budget.max_cache_bytes),
        gpu_report,
        verdict
    );
    Ok(())
}
fn validate_config(file: PathBuf) -> Result<()> {
    let config =
        NodeConfig::load(&file).with_context(|| format!("validating {}", file.display()))?;
    info!(node = %config.node.name, "configuration validated");
    println!("Configuration is valid: {}", file.display());
    Ok(())
}
fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(rest);
    }
    PathBuf::from(value)
}
fn scan(args: ScanArgs) -> Result<()> {
    let scan_dir = expand_tilde(&args.directory);
    let registry_path = expand_tilde(&args.registry);
    let registry_dir = registry_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid registry path"))?;
    fs::create_dir_all(registry_dir)
        .with_context(|| format!("creating registry directory {}", registry_dir.display()))?;
    let mut registry = if registry_path.exists() {
        ModelRegistry::load(&registry_path).with_context(|| {
            format!("loading existing registry from {}", registry_path.display())
        })?
    } else {
        ModelRegistry::new(scan_dir.clone())
            .with_context(|| format!("creating new registry for {}", scan_dir.display()))?
    };
    let count = registry
        .scan_directory(&scan_dir)
        .with_context(|| format!("scanning directory {}", scan_dir.display()))?;
    registry
        .save(&registry_path)
        .with_context(|| format!("saving registry to {}", registry_path.display()))?;
    info!(models = count, path = %scan_dir.display(), "scan completed");
    println!("Scanned {} models from {}", count, scan_dir.display());
    println!("Registry saved to {}", registry_path.display());
    Ok(())
}
fn list_registry(registry: String) -> Result<()> {
    let registry_path = expand_tilde(&registry);
    if !registry_path.exists() {
        println!("Registry not found at {}", registry_path.display());
        println!("Run 'decentraai registry scan' to create a registry.");
        return Ok(());
    }
    let registry = ModelRegistry::load(&registry_path)
        .with_context(|| format!("loading registry from {}", registry_path.display()))?;
    let models = registry.list_models();
    println!(
        "Model registry ({} models, root: {})",
        models.len(),
        registry.root
    );
    for model in models {
        println!(
            "  {} ({} bytes, modified: {}, ext: {})",
            model.relative_path, model.size_bytes, model.modification_time, model.extension
        );
    }
    Ok(())
}

/// Runs the swarm: loads identity and config, serves every model in the
/// local registry to LAN peers, broadcasts signed announcements, reacts to
/// announcements from peers (auto-share), and drives the event loop until
/// interrupted.
async fn swarm_start(config_path: PathBuf) -> Result<()> {
    use decentraai_p2p::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, P2PNode, RegistryServer};
    use std::sync::Arc;

    let config = NodeConfig::load(&config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        anyhow::bail!(
            "identity not found at {}; run 'decentraai init' first",
            identity_path.display()
        );
    }
    let identity = Identity::load(&identity_path)?;

    // Load the registry if one exists; the node serves its models.
    let registry_path = data_dir.join("db/registry.json");
    let handler = if registry_path.exists() {
        let registry = ModelRegistry::load(&registry_path)
            .with_context(|| format!("loading registry from {}", registry_path.display()))?;
        Some(Arc::new(RegistryServer::new(registry)) as Arc<dyn decentraai_p2p::RequestHandler>)
    } else {
        info!("no registry found; node will not serve models");
        None
    };

    let mut node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        handler,
    )?;
    let bound = node.listen("/ip4/0.0.0.0/tcp/0").await?;

    // Announce every model we serve, signed with the node identity.
    let mut announced = 0usize;
    if registry_path.exists() {
        let registry = ModelRegistry::load(&registry_path)?;
        let server = RegistryServer::new(registry);
        for manifest in server.manifests() {
            let signature = decentraai_protocol::sign_manifest(&identity, &manifest);
            let payload = decentraai_protocol::announcement_bytes(
                &manifest,
                Some(signature.to_bytes().to_vec()),
            )?;
            node.announce(payload);
            announced += 1;
        }
    }

    // Auto-share worker: reactions to peer announcements run here so the
    // swarm event loop never blocks on a download. mDNS already auto-dials
    // peers on the same LAN, so two `swarm start` nodes exchange models
    // with no manual multiaddr handling. Every artifact is still verified
    // (per-chunk BLAKE3 + Merkle gate) before it becomes usable.
    let (ann_tx, ann_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let tx = ann_tx.clone();
        node.set_on_manifest_announcement(move |peer, manifest| {
            let _ = tx.send((peer, manifest));
        });
    }
    let share_mode = config.sharing.mode;
    let max_concurrent = config.sharing.max_concurrent_downloads as usize;
    let max_invalid_chunks = config.security.max_invalid_chunks_per_peer;
    let ban_duration =
        Duration::from_secs(u64::from(config.security.ban_duration_minutes) * 60);
    let worker = run_share_worker(
        ann_rx,
        node.clone(),
        ShareWorkerConfig {
            data_dir: data_dir.clone(),
            identity_path: identity_path.clone(),
            registry_path,
            share_mode,
            max_invalid_chunks,
            ban_duration,
        },
        max_concurrent,
    );
    tokio::spawn(async move {
        if let Err(e) = worker.await {
            warn!(error = %e, "share worker stopped");
        }
    });

    println!(
        "DecentraAI swarm running\n  PeerId (identity): {}\n  PeerId (libp2p): {}\n  Listening: {}/p2p/{}\n  Serving: {} model(s) announced\n  Press Ctrl+C to stop",
        identity.peer_id(),
        node.local_peer_id(),
        bound,
        node.local_peer_id(),
        announced
    );
    tokio::signal::ctrl_c().await?;
    node.shutdown();
    Ok(())
}

/// Parameters for the auto-share worker, gathered from the node config.
struct ShareWorkerConfig {
    data_dir: PathBuf,
    identity_path: PathBuf,
    registry_path: PathBuf,
    share_mode: decentraai_config::ShareMode,
    max_invalid_chunks: u8,
    ban_duration: Duration,
}

/// Background worker for `swarm start` auto-sharing: consumes manifest
/// announcements and downloads the announced models with full verification.
/// Runs as its own task so the swarm event loop never blocks on a transfer.
/// Downloads are serialized on one worker (reputation writes are not
/// concurrent), so `max_concurrent` caps the queue it drains per peer.
async fn run_share_worker(
    mut ann_rx: tokio::sync::mpsc::UnboundedReceiver<(decentraai_p2p::PeerId, decentraai_manifest::Manifest)>,
    node: decentraai_p2p::P2PNode,
    cfg: ShareWorkerConfig,
    max_concurrent: usize,
) -> Result<()> {
    use decentraai_config::ShareMode;
    use decentraai_p2p::reputation::ReputationStore;
    use decentraai_p2p::transfer::download_multi;
    use std::collections::HashSet;

    let ShareWorkerConfig {
        data_dir,
        identity_path,
        registry_path,
        share_mode,
        max_invalid_chunks,
        ban_duration,
    } = cfg;
    let models_dir = data_dir.join("models");
    let reputation = ReputationStore::load(
        &data_dir.join("db/reputation.json"),
        max_invalid_chunks,
        ban_duration,
    )?;
    let reputation = tokio::sync::Mutex::new(reputation);
    let mut registry = if registry_path.exists() {
        ModelRegistry::load(&registry_path)?
    } else {
        ModelRegistry::new(models_dir.clone())?
    };
    let mut in_flight: HashSet<String> = HashSet::new();
    // Downloads stay serialized by the reputation mutex; the semaphore is a
    // per-peer headroom guard for bursts of announcements.
    let semaphore = tokio::sync::Semaphore::new(max_concurrent);

    while let Some((peer, manifest)) = ann_rx.recv().await {
        if in_flight.contains(&manifest.model_id) {
            continue;
        }
        if models_dir.join(&manifest.file_name).exists() {
            info!(model = %manifest.file_name, "already present; skipping auto-download");
            continue;
        }
        let proceed = match share_mode {
            ShareMode::Auto => true,
            ShareMode::Ask => {
                use std::io::Write;
                print!(
                    "Peer {peer} shares '{}' ({} MiB). Download? [y/N] ",
                    manifest.file_name,
                    manifest.file_size / (1024 * 1024)
                );
                std::io::stdout().flush().ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).unwrap_or_default();
                matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            }
            ShareMode::Off => continue,
        };
        if !proceed {
            continue;
        }
        in_flight.insert(manifest.model_id.clone());
        let _permit = semaphore.acquire().await;
        info!(peer = %peer, model = %manifest.file_name, "auto-downloading announced model");
        let result = {
            let mut guard = reputation.lock().await;
            download_multi(
                &node,
                &[peer],
                &manifest.model_id,
                &data_dir,
                &mut guard,
            )
            .await
        };
        in_flight.remove(&manifest.model_id);
        match result {
            Ok(path) => {
                info!(path = %path.display(), "auto-downloaded and verified");
                if let Err(e) = registry.scan_directory(&models_dir) {
                    warn!(error = %e, "failed to index auto-downloaded model");
                } else if let Err(e) = registry.save(&registry_path) {
                    warn!(error = %e, "failed to persist registry");
                }
                // Re-announce the downloaded model, now signed by our
                // identity, so other peers can pull it from us too.
                if let Ok(identity) = Identity::load(&identity_path) {
                    let signature = decentraai_protocol::sign_manifest(&identity, &manifest);
                    if let Ok(payload) = decentraai_protocol::announcement_bytes(
                        &manifest,
                        Some(signature.to_bytes().to_vec()),
                    ) {
                        node.announce(payload);
                        info!(model = %manifest.file_name, "re-announced");
                    }
                }
            }
            Err(e) => warn!(
                peer = %peer,
                model = %manifest.file_name,
                error = %e,
                "auto-download failed"
            ),
        }
    }
    Ok(())
}

/// Interactive model picker (Q1): lists every registry model with its
/// size and a memory-fit verdict from the live budget, then reads a
/// choice from stdin. Non-interactive runs (piped scripts) get a clear
/// error instead of hanging.
fn pick_model_interactively(registry: &ModelRegistry, config: &NodeConfig) -> Result<String> {
    use std::io::IsTerminal;

    let models = registry.list_models();
    if models.is_empty() {
        anyhow::bail!(
            "no models in the registry; run 'decentraai registry scan --directory <path>' first"
        );
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "--model is required in non-interactive mode; available: {}",
            models
                .iter()
                .map(|m| m.relative_path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let budget = SystemSnapshot::collect().derive_budget(
        &config.resources,
        config.storage.max_cache_gb,
        config.storage.min_free_disk_gb,
    );
    println!(
        "Available models (memory budget: {:.1} GiB):",
        bytes_to_gib(budget.max_memory_bytes)
    );
    for (i, model) in models.iter().enumerate() {
        let verdict = if model.size_bytes <= budget.max_memory_bytes {
            "fits"
        } else {
            "too large for the current budget"
        };
        println!(
            "  [{}] {} ({:.2} GiB, {})",
            i + 1,
            model.relative_path,
            bytes_to_gib(model.size_bytes),
            verdict
        );
    }
    print!("Choose a model [1-{}]: ", models.len());
    use std::io::Write;
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input
        .trim()
        .parse()
        .context("please answer with the model number")?;
    if choice == 0 || choice > models.len() {
        anyhow::bail!("choice {choice} is out of range 1..={}", models.len());
    }
    Ok(models[choice - 1].relative_path.clone())
}

/// Runs gated inference with the OpenAI-compatible API and the web
/// dashboard on inference.bind_address:api_port.
async fn serve_start(
    config_path: PathBuf,
    model: Option<String>,
    binary: Option<PathBuf>,
) -> Result<()> {
    use decentraai_runtime::api::{ApiState, DashboardInfo, ensure_api_token, serve_api};
    use decentraai_runtime::queue::InferenceQueue;
    use decentraai_runtime::{
        LlamaServer, RuntimeConfig, ServeManager, ensure_admitted, find_llama_server, resolve_model,
    };
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    let config = NodeConfig::load(&config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    ensure_admitted(&config)?;

    let data_dir = expand_tilde(&config.node.data_dir);
    let registry_path = data_dir.join("db/registry.json");
    if !registry_path.exists() {
        anyhow::bail!(
            "registry not found at {}; run 'decentraai registry scan' first",
            registry_path.display()
        );
    }
    let registry = ModelRegistry::load(&registry_path)
        .with_context(|| format!("loading registry from {}", registry_path.display()))?;
    let model = match model {
        Some(reference) => reference,
        None => pick_model_interactively(&registry, &config)?,
    };
    let model_path = resolve_model(&registry, &model)?;
    let model_name = model_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("model")
        .to_string();
    let model_size_bytes = std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0);
    let binary = find_llama_server(binary.as_deref())?;

    let mut runtime = RuntimeConfig::new(model_path.clone());
    runtime.ctx_size = config.inference.max_context_tokens;
    runtime.parallel = config.inference.max_concurrent_requests;
    // Thread budget for token generation: logical CPUs minus the
    // configured reserve (min 1). Oversubscribed threads are the most
    // common cause of slow CPU inference.
    runtime.threads = Some(
        SystemSnapshot::collect()
            .logical_cpus
            .saturating_sub(usize::from(config.resources.reserve_cpu_cores))
            .max(1),
    );

    let server = LlamaServer::spawn(&binary, &runtime).await?;
    let backend_url = server.base_url();
    let idle_timeout =
        Duration::from_secs(u64::from(config.inference.idle_model_unload_minutes) * 60);
    let manager = Arc::new(Mutex::new(ServeManager::new(server, idle_timeout)));

    // Idle watcher: unloads the model after the configured timeout.
    let watcher = manager.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let mut guard = watcher.lock().await;
            if !guard.is_loaded() {
                break;
            }
            let _ = guard.unload_if_idle().await;
        }
    });

    let token = if config.inference.api_auth_required {
        Some(ensure_api_token(&data_dir.join("runtime/api.token"))?)
    } else {
        None
    };
    let info = DashboardInfo {
        repo_root: data_dir.clone(),
        reputation_path: Some(data_dir.join("db/reputation.json")),
        max_invalid_chunks: config.security.max_invalid_chunks_per_peer,
        ban_duration: Duration::from_secs(u64::from(config.security.ban_duration_minutes) * 60),
        api_port: config.inference.api_port,
        model_name,
        model_size_bytes,
        generation: config.inference.generation.clone(),
    };
    let token_store_path = config
        .tiers
        .as_ref()
        .map(|_| data_dir.join("db/tokens.json"));
    // Q2: one request at a time reaches the backend with the machine's
    // full resources; the waiting room and wait limit come from config.
    let queue = InferenceQueue::new(
        usize::from(config.inference.queue_max_requests),
        Duration::from_secs(u64::from(config.inference.request_timeout_seconds)),
    );
    let state = ApiState::new(
        backend_url,
        token.clone(),
        manager.clone(),
        info,
        token_store_path,
        config.tiers.clone(),
        queue,
    );
    let api_addr = serve_api(
        state,
        &config.inference.bind_address,
        config.inference.api_port,
    )
    .await?;

    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        "inference_started",
        serde_json::json!({
            "model": model_path.display().to_string(),
            "api": api_addr.to_string(),
        }),
    );

    let auth_hint = match &token {
        Some(_) => format!(
            "master token: {}",
            data_dir.join("runtime/api.token").display()
        ),
        None => "no auth required by config".to_string(),
    };
    let tiers_hint = if config.tiers.is_some() {
        "tiers: on (decentraai token create --name <n> --tier 1..3)"
    } else {
        "tiers: off (add a tiers: section to the config to enable subscriptions)"
    };
    println!(
        "DecentraAI inference running\n  Model: {}\n  Threads: {} (logical CPUs minus reserve)\n  Queue: FIFO, {} waiting slots, {}s wait limit (dashboard shows it live)\n  Dashboard: http://{}/ (status, peers, share guide)\n  API: http://{}/v1 (OpenAI-compatible)\n  Auth: {}\n  Subscriptions: {}\n  Idle unload: {} min\n  Press Ctrl+C to stop",
        model_path.display(),
        runtime.threads.unwrap_or(0),
        config.inference.queue_max_requests,
        config.inference.request_timeout_seconds,
        api_addr,
        api_addr,
        auth_hint,
        tiers_hint,
        config.inference.idle_model_unload_minutes
    );
    tokio::signal::ctrl_c().await?;
    manager.lock().await.shutdown().await?;
    Ok(())
}

/// Pulls a model from a peer: fetch its catalog, pick the model,
/// download with verification, resume, reputation, and quarantine.
async fn pull(args: PullArgs) -> Result<()> {
    use decentraai_p2p::reputation::ReputationStore;
    use decentraai_p2p::transfer::download;
    use decentraai_p2p::{
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode, PeerId,
    };
    use decentraai_protocol::{
        CURRENT_PROTOCOL_VERSION, CatalogRequest, CatalogResponse, deserialize_message,
        serialize_message,
    };
    use std::time::Duration;

    let config = NodeConfig::load(&args.config)
        .with_context(|| format!("loading {}", args.config.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        anyhow::bail!(
            "identity not found at {}; run 'decentraai init' first",
            identity_path.display()
        );
    }
    let identity = Identity::load(&identity_path)?;

    let peer_str = args
        .from
        .rsplit("/p2p/")
        .next()
        .filter(|s| !s.contains('/'))
        .context("--from must end with /p2p/<peer-id> (see swarm start output)")?;
    let peer_id: PeerId = peer_str.parse().context("invalid peer id in --from")?;

    let node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )?;
    node.dial(&args.from).await?;

    // Fetch the catalog; the fresh connection may need a moment.
    let catalog_request = serialize_message(&CatalogRequest {
        protocol_version: CURRENT_PROTOCOL_VERSION,
    })?;
    let raw = {
        let mut last_err = None;
        let mut result = None;
        for _ in 0..10 {
            match node.request(peer_id, catalog_request.clone()).await {
                Ok(bytes) => {
                    result = Some(bytes);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        match result {
            Some(bytes) => bytes,
            None => return Err(last_err.unwrap()),
        }
    };
    let catalog: CatalogResponse = deserialize_message(&raw, DEFAULT_MAX_MESSAGE_BYTES)?;
    if catalog.protocol_version != CURRENT_PROTOCOL_VERSION {
        anyhow::bail!(
            "peer answered with protocol version {}",
            catalog.protocol_version
        );
    }

    if args.list || args.model.is_none() {
        println!(
            "Peer {} serves {} model(s):",
            peer_id,
            catalog.manifests.len()
        );
        for manifest in &catalog.manifests {
            println!(
                "  {} ({:.2} GiB, id: {}...)",
                manifest.file_name,
                manifest.file_size as f64 / (1024.0 * 1024.0 * 1024.0),
                &manifest.model_id[..16.min(manifest.model_id.len())]
            );
        }
        if args.model.is_none() && !args.list {
            println!("\nUse --model <file_name> to download one.");
        }
        return Ok(());
    }

    let wanted = args.model.unwrap();
    let manifest = catalog
        .manifests
        .iter()
        .find(|m| m.file_name == wanted || m.model_id == wanted || m.model_id.starts_with(&wanted))
        .with_context(|| format!("model '{wanted}' not in the peer's catalog; try --list"))?;

    let mut reputation = ReputationStore::load(
        &data_dir.join("db/reputation.json"),
        config.security.max_invalid_chunks_per_peer,
        Duration::from_secs(u64::from(config.security.ban_duration_minutes) * 60),
    )?;

    println!(
        "Downloading {} ({} chunks)...",
        manifest.file_name,
        manifest.chunk_hashes.len()
    );
    let path = download(
        &node,
        peer_id,
        &manifest.model_id,
        &data_dir,
        &mut reputation,
    )
    .await?;
    println!("Downloaded and verified: {}", path.display());
    println!(
        "Index it with: decentraai registry scan --directory {}",
        data_dir.join("models").display()
    );
    Ok(())
}

/// Issues, lists, and revokes subscription tokens (P1). The admin is
/// whoever has local access to the data directory, mirroring the master
/// token file's security posture.
fn worker_command(args: WorkerArgs) -> Result<()> {
    use decentraai_discovery::{PairingCode, TrustStore};
    use decentraai_identity::Identity;
    use libp2p::PeerId;

    println!("Starting worker with name: {}", args.name);

    // Load or generate identity
    let data_dir = expand_tilde("~/.decentraai");
    let identity_path = std::path::PathBuf::from(&data_dir).join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        let identity = Identity::generate();
        identity.save(&identity_path)?;
        identity
    };

    let peer_id = identity.peer_id();
    println!("Worker PeerId: {}", peer_id.as_str());

    // Generate random libp2p PeerId for demo purposes
    // In production, this would be derived from the identity
    let worker_peer_id = PeerId::random();

    // Generate pairing code for controller
    let controller_peer_id = PeerId::random(); // In real scenario, this would be the actual controller
    let pairing = PairingCode::new(
        worker_peer_id,
        controller_peer_id,
        args.name.clone(),
        300, // 5 minutes TTL
    );

    let qr_data = pairing.to_qr_data()?;
    println!("Pairing QR code data: {}", qr_data);

    // Initialize trust store
    let trust_db_path = std::path::PathBuf::from(&data_dir).join("trust.db");
    let _trust_store = TrustStore::new(&trust_db_path)?;

    println!("Worker '{}' is ready for pairing", args.name);
    println!("Scan the QR code from the controller to complete pairing");

    Ok(())
}

fn token_command(command: TokenCommand) -> Result<()> {
    use decentraai_tokens::{Tier, TokenStore};

    let config_path = match &command {
        TokenCommand::Create { config, .. }
        | TokenCommand::List { config }
        | TokenCommand::Revoke { config, .. } => config,
    };
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let registry_path = data_dir.join("db/tokens.json");
    let mut store = TokenStore::load(&registry_path)
        .with_context(|| format!("loading token registry from {}", registry_path.display()))?;
    let logs_dir = data_dir.join("logs");

    match command {
        TokenCommand::Create { name, tier, .. } => {
            let tier = Tier::parse(tier)?;
            let plaintext = store.create(&name, tier)?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "token_created",
                serde_json::json!({"name": name, "tier": tier.0}),
            );
            println!(
                "Subscription token for '{name}' (tier {} — {}):",
                tier.0,
                tier.name()
            );
            println!("  {plaintext}");
            println!("Store it now: it is shown once and only its BLAKE3 hash is kept.");
            println!("Active at the next API request; no restart needed.");
        }
        TokenCommand::List { .. } => {
            let records = store.list();
            println!("Subscription tokens ({}):", records.len());
            for record in records {
                let status = if record.revoked { "revoked" } else { "active" };
                println!(
                    "  {} (tier {}, {}) — created {}",
                    record.name, record.tier, status, record.created_at
                );
            }
            if store.list().is_empty() {
                println!(
                    "  none yet — create one with: decentraai token create --name <n> --tier 1..3"
                );
            }
        }
        TokenCommand::Revoke { name, .. } => {
            store.revoke(&name)?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "token_revoked",
                serde_json::json!({"name": name}),
            );
            println!("Token '{name}' revoked; it stops working at the next API request.");
        }
    }
    Ok(())
}

/// Runs a distributed inference node (M9)
///
/// This command starts a node that can act as both a worker (serving models)
/// and a client (routing requests to other workers).
async fn distributed_command(args: DistributedArgs) -> Result<()> {
    use decentraai_distributed::{DistributedInference, InferenceConfig};
    use decentraai_identity::Identity;
    use decentraai_inference_adapter::{BackendConfig, OpenAiCompatibleBackend};
    use decentraai_p2p::{
        ChainedHandler, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, P2PNode, RegistryServer,
    };
    use libp2p::PeerId as Libp2pPeerId;
    use libp2p::identity::Keypair as Libp2pKeypair;
    use std::sync::Arc;

    let config = NodeConfig::load(&args.config)
        .with_context(|| format!("loading {}", args.config.display()))?;

    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        anyhow::bail!(
            "identity not found at {}; run 'decentraai init' first",
            identity_path.display()
        );
    }
    let identity = Identity::load(&identity_path)?;

    // Derive the libp2p peer_id from the identity's signing key
    // This ensures we use the same peer_id that P2PNode will use
    let keypair = Libp2pKeypair::ed25519_from_bytes(identity.signing_key_bytes())
        .expect("Failed to create libp2p keypair from identity");
    let local_peer_id = Libp2pPeerId::from(keypair.public());

    // Load the registry if one exists; the node serves its models.
    let registry_path = data_dir.join("db/registry.json");

    // Track if we'll register as a worker
    let will_be_worker = args.model.is_some();

    // Optional running llama-server handle (spawned only when acting as a worker)
    let mut maybe_server: Option<decentraai_runtime::LlamaServer> = None;

    // Pre-load registry and prepare inference backend if we're a worker
    let (registry, model_hash, model_name, backend) = if will_be_worker {
        if !registry_path.exists() {
            anyhow::bail!(
                "registry not found at {}; run 'decentraai registry scan' first",
                registry_path.display()
            );
        }
        let registry = ModelRegistry::load(&registry_path)
            .with_context(|| format!("loading registry from {}", registry_path.display()))?;

        let model = args.model.as_ref().unwrap();
        let model_path = decentraai_runtime::resolve_model(&registry, model)?;
        let model_name = model_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("model")
            .to_string();

        // Calculate model hash
        let model_hash = blake3::hash(&std::fs::read(&model_path)?)
            .to_hex()
            .to_string();

        // Attempt to start a local llama-server for real inference. Worker
        // mode REQUIRES a real llama-server binary: no silent fallback to a
        // mock backend URL, so the distributed path always exercises real
        // inference. The binary is found via --binary, the DECENTRAAI_LLAMA_SERVER
        // env var, PATH, or common build/install locations.
        use decentraai_runtime::{LlamaServer, RuntimeConfig, find_llama_server};
        let binary = find_llama_server(args.binary.as_deref()).with_context(|| {
            "worker mode requires llama-server; pass --binary <path> or install llama.cpp"
        })?;

        // Spawn the server and wait until ready
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
        maybe_server = Some(server);

        // Create inference backend pointing at the chosen base URL
        let backend_config = BackendConfig {
            base_url: url,
            model: model_name.clone(),
            api_key: None,
            connect_timeout: std::time::Duration::from_secs(3),
            request_timeout: std::time::Duration::from_secs(300),
            max_prompt_bytes: 200_000,
            max_output_tokens: 8192,
        };

        let backend = OpenAiCompatibleBackend::new(backend_config)
            .map_err(|e| anyhow::anyhow!("Failed to create inference backend: {}", e))?;

        (
            Some(registry),
            Some(model_hash),
            Some(model_name),
            Some(backend),
        )
    } else {
        // Not a worker, load registry if it exists for RegistryServer
        let registry =
            if registry_path.exists() {
                Some(ModelRegistry::load(&registry_path).with_context(|| {
                    format!("loading registry from {}", registry_path.display())
                })?)
            } else {
                None
            };
        (registry, None, None, None)
    };

    // Create worker manager with the libp2p peer_id
    let worker_manager = Arc::new(decentraai_distributed::WorkerManager::new(
        local_peer_id,
        decentraai_distributed::InferenceConfig::default(),
    ));

    // Compute sharing (M11–M13): the coordinator trusts peers it has paired
    // with (the P5 pairing flow records them in trust.db). A node that has
    // never paired anyone starts with an empty trust set, so compute
    // selection stays off until operators explicitly trust workers.
    let mut compute_trusted = std::collections::HashSet::new();
    let trust_db_path = data_dir.join("trust.db");
    if trust_db_path.exists() {
        use decentraai_discovery::TrustStore;
        match TrustStore::new(&trust_db_path) {
            Ok(store) => {
                for record in store.list_trusted().unwrap_or_default() {
                    if let Ok(peer) = record.worker_peer_id.parse::<libp2p::PeerId>() {
                        compute_trusted.insert(peer);
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to open trust.db; compute trust set is empty"),
        }
    }
    let compute_manager = Arc::new(decentraai_distributed::ComputeManager::new(
        local_peer_id,
        args.name.clone(),
        compute_trusted,
    ));

    // Create a shared request tracker for streaming progress
    let tracker = Arc::new(decentraai_distributed::RequestTracker::new());

    // Create distributed P2P handler: worker announcements AND compute
    // advertisements are processed here on every node (worker or
    // coordinator). Inference serving is NOT handled through this chain
    // anymore — worker mode registers a streaming backend via
    // `register_worker_backend`, which installs the P2P-level on_infer
    // callback that enqueues + streams (queue → backend → progress).
    let mut distributed_handler =
        decentraai_distributed::DistributedP2PHandler::with_worker_manager(
            worker_manager.clone(),
        );
    distributed_handler.set_tracker(tracker.clone());
    distributed_handler.set_compute_manager(compute_manager.clone());
    let mut chained_handler = ChainedHandler::new().add_handler(Arc::new(distributed_handler));

    // Add registry handler if registry exists
    if let Some(registry) = registry {
        chained_handler = chained_handler.add_handler(Arc::new(RegistryServer::new(registry)));
    }

    // Create P2P node with chained handler
    let p2p_node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_handler)),
    )?;

    let bound = p2p_node.listen("/ip4/0.0.0.0/tcp/0").await?;

    // Create distributed inference coordinator with the shared worker manager
    let distributed_config = InferenceConfig::default();
    let mut distributed = DistributedInference::new(
        p2p_node,
        distributed_config,
        Some(worker_manager.clone()),
        Some(tracker.clone()),
    )?;
    distributed.set_compute_manager(compute_manager.clone());
    // If we have a model specified, register as a worker
    if will_be_worker {
        let model_hash = model_hash.expect("model_hash must be set for worker");
        let model_name = model_name.expect("model_name must be set for worker");

        // Register as worker
        distributed.register_as_worker(
            model_name.clone(),
            vec![model_hash.clone()],
            1.0, // Full capacity initially
        )?;

        // Register the local backend to handle inference and streaming
        if let Some(backend) = &backend {
            distributed.register_worker_backend(backend.clone(), model_hash.clone())?;
        }

        // Advertise compute capability from a real hardware probe so this
        // node can be selected through the capability-aware scheduler.
        let served_models = build_served_models(&registry_path, &model_hash, &model_name)?;
        let snapshot = SystemSnapshot::collect();
        let gpu = decentraai_system_probe::probe_gpu();
        let adv = compute_manager
            .advertise_local(snapshot, gpu, served_models)
            .await;
        info!(
            peer_id = %local_peer_id,
            node_name = %adv.node_name,
            models = ?adv.capability.served_models.iter().map(|m| &m.model_hash).collect::<Vec<_>>(),
            "registered as distributed compute worker"
        );
        spawn_compute_broadcaster(compute_manager.clone(), distributed.p2p_node().clone()).await?;

        info!(peer_id = %local_peer_id, model = %model_name, "registered as distributed worker");
    }

    // Start worker discovery
    distributed.start_worker_discovery().await?;

    let mode = if will_be_worker { "worker" } else { "client" };
    println!(
        "DecentraAI distributed node running\n  PeerId: {}\n  Listening: {}/p2p/{}\n  Mode: {}",
        local_peer_id,
        bound,
        local_peer_id,
        mode
    );

    // One-shot client mode: route the prompt to the best available worker,
    // stream the response, then exit. This exercises the REAL path end-to-end:
    // coordinator router → P2P InferRequest → worker queue → llama-server.
    if let Some(prompt) = args.prompt {
        run_distributed_ask(&distributed, prompt, &worker_manager, local_peer_id).await?;
    } else {
        // Persistent coordinator / worker: keep running until interrupted.
        tokio::signal::ctrl_c().await?;
    }

    // If we spawned a local llama-server for worker mode, stop it cleanly.
    if let Some(server) = maybe_server.take() {
        let _ = server.stop().await;
    }

    distributed.shutdown();
    Ok(())
}

/// Builds the `ServedModel` list for compute advertising from the local
/// registry. Only the model chosen with `--model` is advertised, matching
/// what the worker actually loaded into llama-server.
///
/// Memory estimates are conservative: RAM ≈ model bytes/4 + 1 GiB (working
/// set + KV cache), VRAM ≈ full model bytes when a GPU is present, else 0.
/// Overestimating never risks double-booking; it only tightens eligibility.
fn build_served_models(
    registry_path: &std::path::Path,
    model_hash: &str,
    model_name: &str,
) -> Result<Vec<decentraai_compute::ServedModel>> {
    use decentraai_registry::ModelRegistry;

    if !registry_path.exists() {
        return Ok(vec![]);
    }
    let registry = ModelRegistry::load(registry_path)?;
    let record = registry
        .models
        .values()
        .find(|r| r.relative_path == model_name)
        .or_else(|| registry.models.values().find(|r| r.relative_path.ends_with(model_name)));
    let size_mb = match record {
        Some(record) => (record.size_bytes / (1024 * 1024)).max(1),
        None => {
            std::fs::metadata(std::path::Path::new(model_name))
                .map(|m| m.len() / (1024 * 1024))
                .unwrap_or(0)
                .max(1)
        }
    };
    let gpu_present = matches!(
        decentraai_system_probe::probe_gpu(),
        decentraai_system_probe::GpuProbeStatus::Nvidia(_)
    );
    Ok(vec![decentraai_compute::ServedModel {
        model_hash: model_hash.to_string(),
        file_name: model_name.to_string(),
        size_mb,
        est_ram_mb: size_mb / 4 + 1024,
        est_vram_mb: if gpu_present { size_mb } else { 0 },
    }])
}

/// Re-probes this node's hardware and re-broadcasts the compute
/// advertisement on the heartbeat interval, so coordinators never see this
/// worker go stale. Fire-and-forget; a failing probe just skips a beat.
async fn spawn_compute_broadcaster(
    compute_manager: std::sync::Arc<decentraai_distributed::ComputeManager>,
    p2p_node: decentraai_p2p::P2PNode,
) -> Result<()> {
    use decentraai_system_probe::{SystemSnapshot, probe_gpu};
    use decentraai_protocol::serialize_message;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            compute_manager.advertisement_interval_ms(),
        ));
        loop {
            interval.tick().await;
            let snapshot = SystemSnapshot::collect();
            let gpu = probe_gpu();
            // Advertise the latest probe; served_models come from the last
            // full advertisement stored in the manager.
            let workers = compute_manager.workers().await;
            let served_models = workers
                .iter()
                .find(|w| w.peer_id == compute_manager.local_peer())
                .map(|w| w.capability.served_models.clone())
                .unwrap_or_default();
            let adv = compute_manager
                .advertise_local(snapshot, gpu, served_models)
                .await;
            if let Ok(bytes) = serialize_message(&adv) {
                p2p_node.announce(bytes);
            }
        }
    });
    Ok(())
}

/// Waits for at least one worker announcement, routes `prompt` to the best
/// available worker, prints the streamed response, and cancels the request on
/// Ctrl-C. Returns when a terminal InferResponse/InferFailed arrives.
async fn run_distributed_ask(
    distributed: &decentraai_distributed::DistributedInference,
    prompt: String,
    worker_manager: &decentraai_distributed::WorkerManager,
    local_peer_id: libp2p::PeerId,
) -> Result<()> {
    use decentraai_protocol::{InferMessage, InferRequest, serialize_message};
    use tokio::sync::mpsc;

    // Give the swarm a moment to settle and announcements to arrive via mDNS.
    let workers = wait_for_workers(worker_manager, Duration::from_secs(15)).await?;
    let worker = workers
        .into_iter()
        .find(|w| !w.loaded_models.is_empty())
        .ok_or_else(|| anyhow::anyhow!("no worker with a loaded model discovered"))?;
    let model_hash = worker
        .loaded_models
        .first()
        .ok_or_else(|| anyhow::anyhow!("discovered worker has no loaded model"))?
        .clone();
    info!(
        worker_peer_id = %worker.peer_id,
        node_name = %worker.node_name,
        "routing prompt to discovered worker"
    );

    let mut request = InferRequest::new(model_hash, prompt, 512);
    request = request.with_sender(local_peer_id);
    request = request.with_streaming(true);
    request.timeout_ms = 120_000;
    let request_id = request.request_id;

    // Spawn a printer for streamed chunks and route the request.
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let ask = async move {
        distributed.route_request_streamed(request, progress_tx).await
    };

    let printing = tokio::spawn(async move {
        while let Some(chunk) = progress_rx.recv().await {
            print!("{chunk}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    });

    let cancel = async {
        let _ = tokio::signal::ctrl_c().await;
        info!(%request_id, "user cancelled; sending InferCancel to worker");
        let msg = InferMessage::InferCancel {
            request_id,
            reason: "user abort".to_string(),
        };
        if let Ok(bytes) = serialize_message(&msg) {
            let _ = distributed
                .p2p_node()
                .request(worker.peer_id, bytes)
                .await;
        }
    };

    let result = tokio::select! {
        r = ask => r,
        _ = cancel => {
            printing.abort();
            println!();
            println!("--- cancelled by user ---");
            return Ok(());
        }
    };

    let _ = printing.await;
    println!();
    match result {
        Ok(resp) => {
            println!(
                "--- done (tokens={} elapsed_ms={} worker={}) ---",
                resp.tokens_used,
                resp.processing_time_ms,
                resp.worker_peer_id
            );
            Ok(())
        }
        Err(e) => {
            println!("--- failed: {e} ---");
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// Polls the worker manager until at least one worker announcement arrives
/// (from a periodic broadcast) or `timeout` elapses.
async fn wait_for_workers(
    worker_manager: &decentraai_distributed::WorkerManager,
    timeout: Duration,
) -> Result<Vec<decentraai_protocol::WorkerAnnouncement>> {
    use std::time::Instant;
    let start = Instant::now();
    loop {
        let workers = worker_manager.get_workers().await;
        if !workers.is_empty() {
            return Ok(workers);
        }
        if start.elapsed() >= timeout {
            anyhow::bail!(
                "timed out waiting {}s for a worker announcement; is a worker node running?",
                timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_scan_command() {
        let cli = Cli::try_parse_from(["decentraai", "registry", "scan"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Registry {
                command: RegistryCommand::Scan(_)
            }
        ));
    }

    #[test]
    fn parses_registry_list_command() {
        let cli = Cli::try_parse_from(["decentraai", "registry", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Registry {
                command: RegistryCommand::List { .. }
            }
        ));
    }

    #[test]
    fn rejects_legacy_top_level_scan() {
        assert!(Cli::try_parse_from(["decentraai", "scan"]).is_err());
    }

    #[test]
    fn parses_swarm_start_command() {
        let cli = Cli::try_parse_from(["decentraai", "swarm", "start"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Swarm {
                command: SwarmCommand::Start { .. }
            }
        ));
    }

    #[test]
    fn parses_serve_start_command() {
        let cli =
            Cli::try_parse_from(["decentraai", "serve", "start", "--model", "model.gguf"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Serve {
                command: ServeCommand::Start { .. }
            }
        ));
    }

    #[test]
    fn serve_start_model_is_optional() {
        // Without --model the picker opens (interactive); parsing must succeed.
        let cli = Cli::try_parse_from(["decentraai", "serve", "start"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Serve {
                command: ServeCommand::Start { .. }
            }
        ));
    }

    #[test]
    fn parses_pull_command() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "pull",
            "--from",
            "/ip4/127.0.0.1/tcp/4001/p2p/12D3KooWabc",
            "--model",
            "tiny.gguf",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Pull(_)));
    }

    #[test]
    fn parses_pull_list_only() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "pull",
            "--from",
            "/ip4/10.0.0.2/tcp/9999/p2p/12D3KooWxyz",
            "--list",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Pull(_)));
    }

    #[test]
    fn parses_token_create_command() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "token",
            "create",
            "--name",
            "alice",
            "--tier",
            "1",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Token { .. }));
    }

    #[test]
    fn parses_token_list_and_revoke_commands() {
        assert!(
            Cli::try_parse_from(["decentraai", "token", "list"]).is_ok(),
            "token list must parse"
        );
        assert!(
            Cli::try_parse_from(["decentraai", "token", "revoke", "--name", "alice"]).is_ok(),
            "token revoke must parse"
        );
    }
}
