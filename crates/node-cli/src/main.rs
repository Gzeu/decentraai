use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use decentraai_config::NodeConfig;
use decentraai_identity::Identity;
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "decentraai", version, about = "DecentraAI node control CLI")]
struct Cli {
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
    /// Log output format: 'text' (default) or 'json' (structured, H8).
    #[arg(long, global = true, default_value = "text")]
    log_format: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Init(InitArgs),
    /// One-command fresh-node onboarding: detects hardware, generates an
    /// identity, auto-selects a model, writes a validated config and prints
    /// readiness — no manual path/worker/port/topology tuning required (Q4).
    Setup(SetupArgs),
    Doctor(DoctorArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
    /// Search and download models from the HuggingFace Hub (verified
    /// downloads: the Hub's SHA-256 is enforced before the file lands in the
    /// local registry).
    Model {
        #[command(subcommand)]
        command: ModelCommand,
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
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
    Tier {
        #[command(subcommand)]
        command: TierCommand,
    },
    /// Manage consumer API keys (`dca_…`) for the Compute Contribution &
    /// Quota consumer path (Q2): an access credential + quota ceiling, never
    /// an admin credential.
    ConsumerKey {
        #[command(subcommand)]
        command: ConsumerKeyCommand,
    },
    /// Run the full node as a background daemon — LAN/P2P discovery, model
    /// serving and the dashboard all at once, the way the desktop app / systemd
    /// service drives it. Detects and provisions a model automatically; the
    /// node is usable without any manual topology or port configuration.
    Node(NodeArgs),
    /// Open the running node's dashboard in the default browser.
    Open(OpenArgs),
    /// Issue a join invite for a newcomer (P5): creates a Tier-1 Guest token
    /// and shows a copy-pastable `<reachable-multiaddr> <token>` string that a
    /// fresh node can pass to `decentraai join <invite>`.
    Invite(InviteArgs),
    /// Join a private swarm from an invite produced by `decentraai invite`
    /// (P5): parse the `<reachable-multiaddr> <token>` string, auto-provision
    /// identity + config, store the guest token as the node's credential, and
    /// verify it can reach the coordinating peer.
    Join(JoinArgs),
}
#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = "~/.decentraai")]
    data_dir: String,
}
#[derive(Debug, Args)]
struct SetupArgs {
    #[arg(long, default_value = "~/.decentraai")]
    data_dir: String,
    /// Where to write the generated validated config.
    #[arg(long, default_value = "~/.decentraai/node.yaml")]
    config: PathBuf,
    /// A directory to scan for a model to auto-select (defaults to the
    /// node's `models/` directory).
    #[arg(long)]
    models_dir: Option<PathBuf>,
    /// Human-readable node name; defaults to `<hostname>-node`.
    #[arg(long)]
    name: Option<String>,
}
#[derive(Debug, Args)]
struct NodeArgs {
    #[arg(long, default_value = "~/.decentraai/node.yaml")]
    config: PathBuf,
    /// Run a single routed inference through this node's fabric planner, then
    /// exit. The node still brings up identity/config/distributed first, so
    /// the request takes the real scheduler → reservation → P2P → worker path
    /// (executing locally only if that is the planner's decision).
    #[arg(long)]
    prompt: Option<String>,
    /// Optional session id (M20). Reusing the same session across invocations
    /// exercises continuation affinity: the coordinator steers the request
    /// back to the worker that already holds that session's KV prefix.
    #[arg(long)]
    session: Option<String>,
}
#[derive(Debug, Args)]
struct OpenArgs {
    /// The port of the node dashboard (matches config `inference.api_port`).
    #[arg(long, default_value = "8080")]
    port: u16,
}
#[derive(Debug, Args)]
struct InviteArgs {
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: PathBuf,
    /// This node's reachable (dialable) address for the newcomer, WITHOUT the
    /// `/p2p/<peer-id>` suffix — the peer id is derived from this node's
    /// identity. Example: `/ip4/192.168.1.5/tcp/4001`.
    #[arg(long)]
    addr: String,
    /// Invite lifetime in minutes. Past this the guest token stops working
    /// (H3). Default 0 = no expiry.
    #[arg(long, default_value = "0")]
    ttl: u64,
}
#[derive(Debug, Args)]
struct JoinArgs {
    /// The invite string printed by `decentraai invite`: a reachable
    /// multiaddr followed by a space and the Tier-1 Guest token. Quote it on
    /// the shell: `decentraai join "/ip4/192.168.1.5/tcp/4001 dsk_..."`
    #[arg()]
    invite: String,
    #[arg(long, default_value = "~/.decentraai")]
    data_dir: String,
    #[arg(long, default_value = "~/.decentraai/node.yaml")]
    config: PathBuf,
}
#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long, default_value = "configs/node.example.yaml")]
    config: PathBuf,
    /// Run a live, non-destructive connectivity self-check: probe the
    /// configured OpenAI/API port with a short TCP connect.
    #[arg(long)]
    online: bool,
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
enum ModelCommand {
    /// Search the HuggingFace Hub for GGUF models. Results can be filtered by
    /// pipeline category (text-generation, text-to-image, ...) with --category,
    /// or listed by category with --categories.
    Search {
        /// Search query, e.g. "Qwen2.5" or "mistral".
        query: String,
        /// Filter by pipeline category/tool, e.g. --category text-generation.
        #[arg(long)]
        category: Option<String>,
        /// List the distinct categories (pipeline tags) found for the query
        /// instead of the models.
        #[arg(long)]
        categories: bool,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Download a model reference from the Hub into the local models/ dir and
    /// refresh the registry. Accepts `hf:org/repo` (auto-picks the largest
    /// GGUF) or `hf:org/repo:file.gguf` (pins a specific file).
    Pull {
        /// Hub reference: `hf:org/repo` or `hf:org/repo:file.gguf`.
        reference: String,
        #[arg(long, default_value = "~/.decentraai/models")]
        models_dir: String,
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
        /// Q3: a remote OpenAI-compatible backend URL (e.g.
        /// http://192.168.1.50:8080) instead of a local llama-server. This
        /// station keeps auth/tiers/queue/dashboard while the model runs on
        /// the stronger machine. Overrides `--model`/`--binary` (no local
        /// engine is spawned or needed).
        #[arg(long)]
        backend: Option<String>,
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
    /// Loopback HTTP port that serves live compute metrics (workers, load,
    /// reservations, tokens/sec, latency) as JSON at /v1/compute (M16).
    #[arg(long)]
    metrics_port: Option<u16>,
}
#[derive(Debug, Subcommand)]
enum TrustCommand {
    /// Trust a worker peer so the capability-aware scheduler can route
    /// workloads to it. Writes the coordinator's trust.db.
    Add {
        /// Worker peer id, e.g. 12D3KooW...
        #[arg(long)]
        peer: String,
        /// Human-readable name for the worker.
        #[arg(long, default_value = "worker")]
        name: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// List every trusted worker.
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Stop trusting a worker peer.
    Remove {
        /// Worker peer id, e.g. 12D3KooW...
        #[arg(long)]
        peer: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
enum TierCommand {
    /// Suggest a subscription tier for each known worker from its measured
    /// compute contribution (M17): hardware × online hours × verified
    /// requests, reliability-adjusted. Reads the live contribution ledger
    /// the coordinator maintains while `swarm start`/`distributed start` runs.
    Suggest {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Apply contribution-suggested tiers to the token registry (P4). Pairs
    /// each worker's suggested tier to the token of the same name, so a token
    /// `name` must equal the worker's `node_name`. Dry-run by default; pass
    /// `--yes` to actually reassign tiers (each change records a
    /// `tier_changed` audit event).
    Apply {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Execute the changes (records `tier_changed` audit events). Without
        /// it, only prints what would change.
        #[arg(long)]
        yes: bool,
    },
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
        /// client (inference only) or operator (read-only operational views).
        #[arg(long, default_value = "client")]
        role: String,
        /// Unix-seconds expiry; the token stops authenticating after this.
        #[arg(long)]
        expires_at: Option<u64>,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show every issued token (active and revoked).
    List {
        /// Emit the records as a compact JSON array instead of the human table.
        #[arg(long)]
        json: bool,
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

/// Consumer API key management (Q2): create/revoke/list `dca_…` keys for the
/// Compute Contribution & Quota consumer path.
#[derive(Debug, Subcommand)]
enum ConsumerKeyCommand {
    /// Issue a consumer API key for an account; shown once, stored only as a
    /// hash. The key is an access credential + quota ceiling, never admin.
    Create {
        /// Owner account in the quota ledger (e.g. the worker/contributor name).
        #[arg(long)]
        account: String,
        /// Per-request quota ceiling in quota units (> 0).
        #[arg(long)]
        quota_ceiling: u64,
        /// Per-key rate limit (requests per minute, > 0).
        #[arg(long)]
        rate_limit_per_minute: u32,
        /// Comma-separated permission scopes (e.g. "inference").
        #[arg(long, default_value = "inference")]
        scopes: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// List every consumer key's metadata (never the plaintext secret).
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Revoke a consumer key by key id; it stops authenticating immediately.
    Revoke {
        #[arg(long)]
        key_id: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.log_format.eq_ignore_ascii_case("json") {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(cli.log_level))
            .with_target(false)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(cli.log_level))
            .with_target(false)
            .init();
    }
    match cli.command {
        Command::Init(args) => init(args),
        Command::Setup(args) => setup(args),
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
        Command::Model {
            command: ModelCommand::Search {
                query,
                category,
                categories,
                limit,
            },
        } => model_search(query, category, categories, limit).await,
        Command::Model {
            command: ModelCommand::Pull {
                reference,
                models_dir,
                registry,
            },
        } => model_pull(reference, models_dir, registry).await,
        Command::Swarm {
            command: SwarmCommand::Start { config },
        } => swarm_start(config).await,
        Command::Serve {
            command:
                ServeCommand::Start {
                    model,
                    config,
                    binary,
                    backend,
                },
        } => serve_start(config, model, binary, backend).await,
        Command::Pull(args) => pull(args).await,
        Command::Token { command } => token_command(command),
        Command::Worker(args) => worker_command(args),
        Command::Distributed(args) => distributed_command(args).await,
        Command::Trust { command } => trust_command(command),
        Command::Tier { command } => tier_command(command),
        Command::ConsumerKey { command } => consumer_key_command(command),
        Command::Node(args) => node_start(args).await,
        Command::Open(args) => open_dashboard(args),
        Command::Invite(args) => invite(args),
        Command::Join(args) => join(args).await,
    }
}
/// One-command fresh-node onboarding (Q4): detect hardware, generate an
/// identity, auto-select a model, write a validated config, and print
/// readiness — no manual path/worker/port/topology tuning required.
///
/// The wizard can be re-run safely: it never overwrites an identity and it
/// regenerates the config with current detected hardware.
fn setup(args: SetupArgs) -> Result<()> {
    use decentraai_system_probe::{SystemSnapshot, probe_gpu};
    use libp2p::PeerId as Libp2pPeerId;
    use libp2p::identity::Keypair as Libp2pKeypair;

    let data_dir = expand_tilde(&args.data_dir);
    let config_path = expand_tilde(&args.config.to_string_lossy());

    // 1. Make the standard directory layout (same as `decentraai init`).
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
        fs::create_dir_all(data_dir.join(directory))?;
    }

    // 2. Identity: reuse an existing one or generate a fresh key. The
    //    identity also produces the node's default name (`dca-…`), so a
    //    freshly generated node is already distinct on the fabric — no
    //    manual naming needed.
    let identity_path = data_dir.join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)
            .with_context(|| format!("loading identity from {}", identity_path.display()))?
    } else {
        let identity = Identity::generate();
        identity.save(&identity_path)?;
        identity
    };
    let peer_id = identity.peer_id().to_string();

    // 3. Auto-detect hardware (real probe, not mocked).
    let snapshot = SystemSnapshot::collect();
    let gpu = probe_gpu();
    let gpu_line = match &gpu {
        decentraai_system_probe::GpuProbeStatus::Nvidia(info) => {
            format!("{} ({} MiB VRAM free)", info.name, info.free_vram_mib)
        }
        _ => "no NVIDIA GPU detected (CPU-only)".to_string(),
    };
    println!(
        "Hardware detected: {} logical cores, {:.1} GiB RAM available",
        snapshot.logical_cpus,
        snapshot.available_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!("GPU: {gpu_line}");

    // 4. Auto-select a model: first GGUF under the models dir (or explicit
    //    --models-dir). The node is still fully functional without one.
    let models_dir = args
        .models_dir
        .clone()
        .unwrap_or_else(|| data_dir.join("models"));
    let model_name = auto_detect_model(&models_dir)?;
    let model_label = detect_model_label(&models_dir, &model_name).0;

    // 5. Derive a config from what we detected. RAM-driven context, an
    //    identity-derived node name (the node's own `dca-…` ID), loopback
    //    API, auto GPU policy.
    let total_ram_gib = snapshot.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let max_context = if total_ram_gib >= 32.0 { 8192 } else { 4096 };
    // Default name = the node's compact ID (`dca-xxxxxx`), derived from the
    // same identity the fabric already knows it by — operators never have to
    // invent a name; `setup --name` remains available for a semantic label.
    let node_name = args.name.unwrap_or_else(|| {
        let libp2p_keypair = Libp2pKeypair::ed25519_from_bytes(identity.signing_key_bytes())
            .expect("ed25519 key bytes are valid");
        let libp2p_peer = Libp2pPeerId::from(libp2p_keypair.public());
        decentraai_distributed::short_node_id(&libp2p_peer)
    });
    // A model's port is irrelevant here; the API is fixed and loopback-only.
    let api_port = 8080u16;
    let gpu_policy = if matches!(&gpu, decentraai_system_probe::GpuProbeStatus::Nvidia(_)) {
        "auto"
    } else {
        "off"
    };

    let yaml = setup_yaml(
        &node_name,
        &data_dir,
        max_context,
        api_port,
        gpu_policy,
        snapshot.logical_cpus,
    );

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, yaml)?;

    // 6. Validate the generated config actually parses (self-check).
    NodeConfig::load(&config_path).with_context(|| {
        format!(
            "generated config failed validation: {}",
            config_path.display()
        )
    })?;

    println!("\n=== DecentraAI node is READY ===");
    println!("  PeerId : {peer_id}");
    println!("  Node   : {node_name}");
    println!("  GPU    : {gpu_line}");
    println!("  Model  : {model_label}");
    println!("  Config : {}", config_path.display());
    println!();
    println!("Next steps (all auto-discover, no manual config):");
    println!(
        "  decentraai swarm start --config {conf}",
        conf = config_path.display()
    );
    println!(
        "  decentraai distributed start --config {conf}",
        conf = config_path.display()
    );
    Ok(())
}

/// Auto-detect the default model and produce a friendly label + the exact
/// `--model` argument the runtime expects, when one exists. Uses the real
/// filesystem scan; returns `(label, Option<model_path_arg>)`.
fn detect_model_label(models_dir: &std::path::Path, model_name: &str) -> (String, Option<String>) {
    if model_name.is_empty() {
        (
            "none detected — models are shared/downloaded via the verified transfer path"
                .to_string(),
            None,
        )
    } else {
        (
            model_name.to_string(),
            Some(models_dir.join(model_name).display().to_string()),
        )
    }
}

/// Scans `dir` for the first `*.gguf` model. Returns an empty string when
/// none is found (the node still comes up; models can be shared/downloaded
/// later via the verified transfer path). This is real detection against the
/// filesystem, not a placeholder.
fn auto_detect_model(dir: &std::path::Path) -> Result<String> {
    if !dir.exists() {
        return Ok(String::new());
    }
    let mut found: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.to_ascii_lowercase().ends_with(".gguf") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    found.sort();
    Ok(found.into_iter().next().unwrap_or_default())
}

/// Builds a valid `NodeConfig` YAML from detected state. The schema mirrors
/// `configs/node.example.yaml`; fields a fresh user should not tune are set
/// to safe defaults. Must round-trip through `NodeConfig::load`.
fn setup_yaml(
    node_name: &str,
    data_dir: &std::path::Path,
    max_context: u32,
    api_port: u16,
    gpu_policy: &str,
    logical_cpus: usize,
) -> String {
    let reserve_cores = (logical_cpus as u32 / 4).min(2);
    let max_parallel = 1;
    format!(
        r#"# Generated by `decentraai setup` — hardware auto-detected.
node:
  name: {node_name:?}
  mode: "balanced"
  data_dir: {data_dir_yaml}

network:
  private_swarm: true
  lan_discovery: true
  dht_enabled: false
  relay_enabled: false
  bootstrap_peers: []
  max_connections: 64
  max_message_bytes: 1048576

storage:
  chunk_size_mb: 4
  hash_algorithm: "blake3"
  max_cache_gb: 50
  min_free_disk_gb: 5
  verify_full_file_after_assembly: true
  allow_unsigned_models: false
  auto_seed_verified_models: false

resources:
  cpu_max_percent: 50
  memory_max_percent: 60
  reserve_cpu_cores: {reserve_cores}
  reserve_ram_mb: 1024
  gpu_enabled: "{gpu_policy}"
  gpu_max_vram_percent: 75
  reserve_vram_mb: 512
  stop_gpu_temperature_celsius: 83
  max_upload_mbps: 20
  max_download_mbps: 80

inference:
  enabled: "auto"
  runtime: "llama_server"
  bind_address: "127.0.0.1"
  api_auth_required: true
  allow_remote_inference: false
  max_concurrent_requests: {max_parallel}
  max_context_tokens: {max_context}
  max_generated_tokens: 1024
  request_timeout_seconds: 180
  queue_max_requests: 20
  idle_model_unload_minutes: 15
  api_port: {api_port}
  generation:
    temperature: 0.7
    top_p: 0.9
    top_k: 40
    repeat_penalty: 1.1
    system_prompt: "You are a helpful assistant. Answer in the same language as the user's message."

privacy:
  log_prompts: false
  log_outputs: false
  publish_exact_hardware: false
  telemetry_opt_in: false

security:
  trust_mode: "private"
  require_signed_announcements: true
  require_request_signatures: true
  ban_duration_minutes: 60
  max_invalid_chunks_per_peer: 2

sharing:
  mode: "auto"
  max_concurrent_downloads: 2
  provision_models_on_demand: true
"#,
        node_name = node_name,
        data_dir_yaml = data_dir.display().to_string().replace('"', "\\\""),
        reserve_cores = reserve_cores,
        gpu_policy = gpu_policy,
        max_parallel = max_parallel,
        max_context = max_context,
        api_port = api_port,
    )
}

/// Opens the running node's dashboard in the system default browser.
/// Cross-platform best-effort: xdg-open, open, start, in that order.
fn open_dashboard(args: OpenArgs) -> Result<()> {
    use std::process::Command as StdCommand;
    let url = format!("http://127.0.0.1:{}/", args.port);
    let tried: &[(&str, Vec<String>)] = &[
        // Most launchers (xdg-open) take the URL directly.
        ("xdg-open", vec![url.clone()]),
        ("gvfs-open", vec![url.clone()]),
        ("open", vec![url.clone()]), // macOS
        // Some xdg-open builds choke on a bare URL; keep -- as a fallback.
        ("xdg-open", vec!["--".to_string(), url.clone()]),
    ];
    for (bin, argv) in tried {
        if let Ok(mut child) = StdCommand::new(bin).args(argv).spawn() {
            let _ = child.wait();
            println!("Opened dashboard at {url}");
            return Ok(());
        }
    }
    anyhow::bail!("no browser launcher found; open {url} manually");
}

/// Runs the full node as a single background daemon (the desktop app / systemd
/// service target). It composes the pieces a normal user should never have to
/// wire by hand:
///
/// 1. ensures identity + validated config exist (auto-generating them),
/// 2. brings up LAN/P2P discovery + verified auto-share,
/// 3. serves the dashboard + OpenAI API, auto-loading a detected model,
/// 4. advertises real compute and exposes live node/worker/compute status.
///
/// Congestion and topology are hidden: the node just comes up and peers on the
/// same LAN discover each other automatically. Shuts down cleanly on
/// SIGINT/SIGTERM (which is what Ctrl+C and systemd send).
async fn node_start(args: NodeArgs) -> Result<()> {
    use PathBuf;
    use decentraai_distributed::{DistributedInference, InferenceConfig};
    use decentraai_inference_adapter::{BackendConfig, EngineKind, OpenAiCompatibleBackend};
    use decentraai_p2p::{
        ChainedHandler, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, P2PNode, RegistryServer,
    };
    use decentraai_runtime::api::{ApiState, DashboardInfo, ensure_api_token, serve_api};
    use decentraai_runtime::queue::InferenceQueue;
    use decentraai_runtime::{
        LlamaServer, RuntimeConfig, ensure_admitted, find_llama_server,
    };
    use libp2p::PeerId as Libp2pPeerId;
    use libp2p::identity::Keypair as Libp2pKeypair;
    use std::sync::Arc;
    use std::time::Duration;

    let config_path = expand_tilde(&args.config.to_string_lossy());

    // 1. Auto-provision identity + config if this is a truly first run.
    if !config_path.exists() {
        let data_dir = expand_tilde("~/.decentraai");
        setup(SetupArgs {
            data_dir: data_dir.to_string_lossy().into_owned(),
            config: config_path.clone(),
            models_dir: None,
            name: None,
        })?;
    }

    let config = NodeConfig::load(&config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    ensure_admitted(&config)?;

    let data_dir = expand_tilde(&config.node.data_dir);
    let node_name = config.node.name.clone();
    let api_port = config.inference.api_port;
    let bind_address = config.inference.bind_address.clone();

    let identity_path = data_dir.join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        let identity = Identity::generate();
        identity.save(&identity_path)?;
        identity
    };

    // The libp2p peer_id is derived from the identity signing key, exactly as
    // the standalone `distributed` node does, so the transport PeerId is
    // stable and matches `identity.peer_id()`.
    let libp2p_keypair = Libp2pKeypair::ed25519_from_bytes(identity.signing_key_bytes())
        .context("libp2p keypair from identity")?;
    let local_peer_id = Libp2pPeerId::from(libp2p_keypair.public());
    info!(peer_id = %identity.peer_id(), p2p_peer_id = %local_peer_id, "node identity ready");

    // 2. Detect a local model. If present, this node is BOTH a worker (serves
    // the model over P2P) and a coordinator (routes/streams to other workers);
    // if absent it is a coordinator-only discovery node.
    let models_dir = data_dir.join("models");
    let model_name = auto_detect_model(&models_dir).unwrap_or_default();
    let detected_model = if model_name.is_empty() {
        None
    } else {
        let model = models_dir.join(&model_name);
        if model.is_file() {
            Some((model, model_name.clone()))
        } else {
            None
        }
    };

    // Optional local llama-server handle: owned by the dashboard (ServeManager)
    // for idle-unload/status, while the worker backend is an HTTP client to the
    // same address. One runtime, two consumers.
    let mut maybe_server: Option<LlamaServer> = None;
    let mut provision_factory: Option<decentraai_distributed::ProvisioningFactory> = None;
    let mut worker_backend: Option<OpenAiCompatibleBackend> = None;
    // Single authoritative source of truth for the live engine base URL. The
    // engine supervisor (M24) writes the current port here after every start /
    // respawn; the worker backend resolves its base URL synchronously from this
    // cache each request, so routed inference can never hit a stale engine port
    // after a respawn (no frozen backend URL, no false worker).
    let live_engine_url: Arc<std::sync::Mutex<Option<String>>> =
        Arc::new(std::sync::Mutex::new(None));
    // M24 engine supervisor restart spec: captured so the ServeManager can
    // respawn llama-server if it crashes while the node stays up.
    let mut restart_binary: Option<std::path::PathBuf> = None;
    let mut restart_runtime: Option<RuntimeConfig> = None;
    let mut model_hash = String::new();
    let mut model_size_bytes: u64 = 0;
    let mut backend_url = String::new();

    if let Some((model, model_name)) = detected_model.as_ref() {
        model_size_bytes = metadata_size(model);
        model_hash = blake3::hash(std::fs::read(model).ok().as_deref().unwrap_or_default())
            .to_hex()
            .to_string();

        let binary = match find_llama_server(None) {
            Ok(b) => Some(b),
            Err(e) => {
                warn!(error = %e, "llama-server not found; node runs without serving (discovery/coordinator only)");
                None
            }
        };

        if let Some(binary) = binary {
            let mut runtime = RuntimeConfig::new(model.clone());
            runtime.ctx_size = config.inference.max_context_tokens;
            runtime.parallel = config.inference.max_concurrent_requests;
            runtime.threads = Some(
                SystemSnapshot::collect()
                    .logical_cpus
                    .saturating_sub(usize::from(config.resources.reserve_cpu_cores))
                    .max(1),
            );
            restart_binary = Some(binary.clone());
            restart_runtime = Some(runtime.clone());
            match LlamaServer::start(&binary, &runtime) {
                Ok(server) => {
                    backend_url = server.base_url();
                    *live_engine_url.lock().unwrap() = Some(backend_url.clone());
                    let resolver_state = live_engine_url.clone();
                    let backend = OpenAiCompatibleBackend::new(BackendConfig {
                        base_url: backend_url.clone(),
                        model: model_name.clone(),
                        api_key: None,
                        connect_timeout: Duration::from_secs(3),
                        request_timeout: Duration::from_secs(300),
                        max_prompt_bytes: 200_000,
                        max_output_tokens: 8192,
                        engine: EngineKind::LlamaServer,
                        // Follow the authoritative engine URL at request time so
                        // a respawn on a new port is never served on a stale one.
                        backend_url_resolver: Some(Arc::new(move || {
                            resolver_state.lock().ok().and_then(|g| g.clone())
                        })),
                    })
                    .ok();
                    worker_backend = backend;
                    // Provisioning factory (M14): a downloaded model gets its
                    // own llama-server instance, kept alive for the session.
                    let max_ctx = config.inference.max_context_tokens;
                    let parallel = config.inference.max_concurrent_requests;
                    let reserve_cores = config.resources.reserve_cpu_cores;
                    let binary_for_factory = binary.clone();
                    provision_factory = Some(Arc::new(move |model_path: PathBuf| {
                        let binary = binary_for_factory.clone();
                        Box::pin(async move {
                            let mut cfg = RuntimeConfig::new(model_path);
                            cfg.ctx_size = max_ctx;
                            cfg.parallel = parallel;
                            cfg.threads = Some(
                                SystemSnapshot::collect()
                                    .logical_cpus
                                    .saturating_sub(usize::from(reserve_cores))
                                    .max(1),
                            );
                            let server = LlamaServer::spawn(&binary, &cfg).await?;
                            let backend_cfg = BackendConfig {
                                base_url: server.base_url(),
                                model: "provisioned".to_string(),
                                api_key: None,
                                connect_timeout: Duration::from_secs(3),
                                request_timeout: Duration::from_secs(300),
                                max_prompt_bytes: 200_000,
                                max_output_tokens: 8192,
                                engine: EngineKind::LlamaServer,
                                // Provisioned engines are spawned fresh per
                                // model and die with the node; no respawn
                                // port drift, so a static base URL is correct.
                                backend_url_resolver: None,
                            };
                            let backend = OpenAiCompatibleBackend::new(backend_cfg)
                                .map_err(|e| anyhow::anyhow!("provisioned backend: {e}"))?;
                            Ok((Box::new(server) as Box<dyn std::any::Any + Send>, backend))
                        })
                    }));
                    maybe_server = Some(server);
                }
                Err(e) => {
                    warn!(error = %e, "failed to start llama-server; continuing as coordinator-only")
                }
            }
        }
    }

    // Multi-engine worker (Objective 7): when a remote OpenAI-compatible
    // backend (e.g. a vLLM/Ollama endpoint) is configured instead of a local
    // llama-server, register it as a FIRST-CLASS distributed worker — the same
    // register_worker_backend path a local engine uses — so P2P InferRequests
    // route to it just like any other worker. Opt-in (only when
    // inference.backend_url is set and no local engine was built). The model
    // identity is derived deterministically from the configured engine/model.
    if worker_backend.is_none() {
        if let Some(remote) = config.inference.backend_url.clone() {
            let engine = config
                .inference
                .engine
                .as_deref()
                .map(EngineKind::parse)
                .unwrap_or(EngineKind::LlamaServer);
            if let Ok(backend) = OpenAiCompatibleBackend::new(BackendConfig {
                base_url: remote.clone(),
                model: model_name.clone(),
                api_key: None,
                connect_timeout: Duration::from_secs(3),
                request_timeout: Duration::from_secs(300),
                max_prompt_bytes: 200_000,
                max_output_tokens: 8192,
                engine,
                // Remote URL is a fixed config value; no respawn, static is correct.
                backend_url_resolver: None,
            }) {
                // Deterministic model id/hash for a remote worker (no local GGUF).
                if model_hash.is_empty() {
                    model_hash =
                        blake3::hash(format!("{engine:?}:{model_name}").as_bytes())
                            .to_hex()
                            .to_string();
                }
                if model_size_bytes == 0 {
                    model_size_bytes = 1024;
                }
                backend_url = remote.clone();
                *live_engine_url.lock().unwrap() = Some(remote.clone());
                worker_backend = Some(backend);
                info!(
                    engine = %engine.as_str(),
                    base_url = %remote,
                    model = %model_name,
                    hash = %model_hash,
                    "registered remote OpenAI-compatible engine as a distributed worker"
                );
            }
        }
    }

    let is_worker = worker_backend.is_some();
    if is_worker {
        info!(model = %model_name, hash = %model_hash, "node will act as a remote worker");
    } else {
        info!("node running as a coordinator/discovery node (no servable model)");
    }

    // ---- Distributed stack (the exact wiring `decentraai distributed` uses) ----

    // Trust set from the pairing/trust store; empty until operators trust peers.
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
            Err(e) => {
                tracing::warn!(error = %e, "failed to open trust.db; compute trust set empty")
            }
        }
    }

    let worker_manager = Arc::new(decentraai_distributed::WorkerManager::new(
        local_peer_id,
        InferenceConfig::default(),
    ));
    let mut compute_manager = Arc::new(decentraai_distributed::ComputeManager::new(
        local_peer_id,
        node_name.clone(),
        compute_trusted,
    ));
    // P3: sign this node's advertisements so recipients authenticate them.
    if let Some(cm) = Arc::get_mut(&mut compute_manager) {
        cm.set_signing_key(identity.signing_key_bytes());
    }
    // M22: if the config selects an alternative engine, advertise it honestly
    // so coordinators' planners reason engine-aware instead of assuming
    // llama-server. llama-server stays the default when unset.
    if let Some(engine) = config.inference.engine.as_deref() {
        if let Some(cm) = Arc::get_mut(&mut compute_manager) {
            cm.set_engine(engine);
        }
    }
    // Remote-sharing opt-in: the advertisement carries whether this node
    // accepts inference routed from remote peers, so coordinators only ever
    // schedule remote workers that will actually serve the request.
    if let Some(cm) = Arc::get_mut(&mut compute_manager) {
        cm.set_accepts_remote_inference(config.inference.allow_remote_inference);
    }
    compute_manager
        .set_allow_provisioning(config.sharing.provision_models_on_demand)
        .await;

    let tracker = Arc::new(decentraai_distributed::RequestTracker::new());

    let mut distributed_handler =
        decentraai_distributed::DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    distributed_handler.set_tracker(tracker.clone());
    distributed_handler.set_compute_manager(compute_manager.clone());
    let mut chained_handler = ChainedHandler::new().add_handler(Arc::new(distributed_handler));

    // Serve manifests/chunks off the registry if one exists (model sharing).
    let registry_path = data_dir.join("db/registry.json");
    // Let the coordinator resolve persisted capability claims for a model, so
    // capability-requirement verdicts on decisions are real (not UNKNOWN) when
    // a model was pulled with Hub metadata.
    compute_manager.set_registry_path(registry_path.clone());
    if registry_path.exists() {
        if let Ok(reg) = ModelRegistry::load(&registry_path) {
            chained_handler = chained_handler.add_handler(Arc::new(RegistryServer::new(reg)));
        }
    }

    let p2p_node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_handler)),
    )?;
    let bound = p2p_node.listen("/ip4/0.0.0.0/tcp/0").await?;

    let mut distributed = DistributedInference::new(
        p2p_node,
        InferenceConfig::default(),
        Some(worker_manager.clone()),
        Some(tracker.clone()),
    )?;
    distributed.set_compute_manager(compute_manager.clone());
    // M10: per-request routing audit events (request/worker/model hash/status).
    distributed.set_logs_dir(Some(data_dir.join("logs")));
    // P1: sign outbound routed requests with the node identity so workers can
    // authenticate them and reject spoofed/unsigned traffic.
    distributed.set_signing_identity(identity.signing_key_bytes());

    // The dashboard owns the llama-server lifecycle; the worker advertises in
    // sync with its LIVE health (see spawn_compute_broadcaster). Create the
    // ServeManager early so the broadcaster can read the current engine port
    // every beat — after an M24 respawn on a new port the advertisement gate
    // must probe the new port, not the startup one.
    let instance_manager: Option<Arc<tokio::sync::Mutex<decentraai_runtime::ServeManager>>> =
        if is_worker && !backend_url.is_empty() {
            let idle_timeout =
                Duration::from_secs(u64::from(config.inference.idle_model_unload_minutes) * 60);
            // NO idle-unload for the universal node. This daemon is a distributed
            // worker whenever a model is present: it advertises capacity and answers
            // remote InferRequests at any time. Enabling idle-unload would stop the
            // shared llama-server (which ServeManager::unload_if_idle drops, with no
            // reload path), leaving the node advertising as a worker while its
            // engine is dead — a false-ready state (real bug found in the two-machine
            // trust/reservation test). Interactive single-user idle-unload only
            // belongs to the `decentraai serve` path.
            let manager = Arc::new(tokio::sync::Mutex::new(
                decentraai_runtime::ServeManager::new(
                    maybe_server.take().expect("server started"),
                    idle_timeout,
                ),
            ));
            // M24 engine supervisor: give the manager the restart spec and probe
            // the engine periodically, auto-restarting llama-server on crash.
            if let (Some(binary), Some(runtime)) = (restart_binary, restart_runtime) {
                manager.lock().await.set_restart_spec(binary, runtime);
                let supervisor = manager.clone();
                let live_for_supervisor = live_engine_url.clone();
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_secs(5));
                    loop {
                        tick.tick().await;
                        let mut guard = supervisor.lock().await;
                        if !guard.is_loaded() {
                            // The ServeManager has no live server (already stopped
                            // or shut down); stop supervising.
                            break;
                        }
                        let ok = guard.ensure_healthy().await.unwrap_or(false);
                        // Publish the authoritative engine URL after every pass
                        // (start or respawn), so the worker backend and the
                        // advertisement gate share the SAME live port — never a
                        // stale or frozen one.
                        if ok {
                            if let Some(url) = guard.base_url() {
                                *live_for_supervisor.lock().unwrap() = Some(url);
                            }
                        }
                    }
                });
            }
            Some(manager)
        } else {
            None
        };

    if is_worker {
        let model_name = model_name.clone();
        distributed.register_as_worker(model_name.clone(), vec![model_hash.clone()], 1.0)?;

        let can_provision =
            config.sharing.provision_models_on_demand && provision_factory.is_some();
        let provisioning = if can_provision {
            Some(decentraai_distributed::ProvisioningConfig {
                data_dir: data_dir.clone(),
                registry_path: registry_path.clone(),
                reputation_path: data_dir.join("db/reputation.json"),
                max_concurrent_downloads: config.sharing.max_concurrent_downloads as usize,
                max_invalid_chunks: config.security.max_invalid_chunks_per_peer,
                ban_duration: Duration::from_secs(
                    u64::from(config.security.ban_duration_minutes) * 60,
                ),
                backend_factory: provision_factory.take().expect("factory built above"),
            })
        } else {
            None
        };
        if let Some(backend) = worker_backend {
            distributed.register_worker_backend(
                backend,
                model_hash.clone(),
                provisioning,
                config.inference.allow_remote_inference,
            )?;
        }

        // Advertise real compute + hardware so the capability scheduler can
        // select this node and coordinators see it as a ready worker. GPU is
        // first-class: when a GPU is probed and the config policy allows, the
        // model advertises a real VRAM footprint (so GPU vs CPU workers are
        // distinguished and VRAM headroom is enforced), otherwise CPU-only.
        let snapshot = SystemSnapshot::collect();
        let gpu = decentraai_system_probe::probe_gpu();
        let gpu_offload = match &gpu {
            decentraai_system_probe::GpuProbeStatus::Nvidia(_) => {
                config.resources.gpu_enabled != decentraai_config::GpuPolicy::Off
            }
            _ => false,
        };
        let max_ctx = config.inference.max_context_tokens;
        let served_models = vec![decentraai_compute::ServedModel {
            model_hash: model_hash.clone(),
            file_name: model_name.clone(),
            size_mb: (model_size_bytes / (1024 * 1024)).max(1),
            est_ram_mb: model_size_bytes / (1024 * 1024) / 4 + 1024,
            est_vram_mb: decentraai_compute::ServedModel::estimate_vram_mb(
                model_size_bytes,
                gpu_offload,
                max_ctx,
            ),
            context_tokens: max_ctx,
        }];
        let available_models =
            build_available_models(&registry_path, config.inference.max_context_tokens)?;
        let adv = compute_manager
            .advertise_local(snapshot, gpu, served_models, available_models, can_provision)
            .await;
        info!(
            peer_id = %local_peer_id,
            node_name = %adv.node_name,
            model = %model_name,
            models = ?adv.capability.served_models.iter().map(|m| &m.model_hash).collect::<Vec<_>>(),
            on_disk = adv.capability.available_models.len(),
            can_provision,
            "registered as distributed compute worker"
        );
        spawn_compute_broadcaster(
            compute_manager.clone(),
            distributed.p2p_node().clone(),
            can_provision,
            instance_manager.clone(),
            Some(live_engine_url.clone()),
        )
        .await?;
    }

    // M19: RTT probing to known workers for network-aware planning.
    spawn_network_probe(compute_manager.clone(), distributed.p2p_node().clone()).await;
    // M24: reap stale reservations / evict dead workers with audit.
    spawn_worker_reaper(
        compute_manager.clone(),
        data_dir.join("logs"),
        default_reap_grace(),
    )
    .await;

    // mDNS / LAN worker discovery + heartbeats.
    distributed.start_worker_discovery().await?;

    // ---- Dashboard / OpenAI-compatible API (local serving + status) ----
    // The dashboard owns the llama-server lifecycle (idle-unload); the worker
    // backend points at the same address. When a node has a model it is fully
    // usable standalone (local inference + dashboard) even with no peers.
    if is_worker && !backend_url.is_empty() {
        // ServeManager + M24 supervisor are created above (before the compute
        // broadcaster) so the worker advertises in sync with the LIVE engine
        // port after any respawn. Reuse the same handle here so the dashboard
        // and the distributed worker share one engine lifecycle.
        let manager = instance_manager
            .clone()
            .expect("ServeManager created for worker above");
        let token = if config.inference.api_auth_required {
            ensure_api_token(&data_dir.join("runtime/api.token")).ok()
        } else {
            None
        };
        let info = DashboardInfo {
            repo_root: data_dir.clone(),
            reputation_path: Some(data_dir.join("db/reputation.json")),
            max_invalid_chunks: config.security.max_invalid_chunks_per_peer,
            ban_duration: Duration::from_secs(u64::from(config.security.ban_duration_minutes) * 60),
            api_port: config.inference.api_port,
            model_name: model_name.clone(),
            model_size_bytes,
            generation: config.inference.generation.clone(),
            resources: config.resources.clone(),
        };
        let token_store_path = config
            .tiers
            .as_ref()
            .map(|_| data_dir.join("db/tokens.json"));
        let queue = InferenceQueue::new(
            usize::from(config.inference.queue_max_requests),
            Duration::from_secs(u64::from(config.inference.request_timeout_seconds)),
        );
        let mut state = ApiState::new(
            backend_url.clone(),
            token.clone(),
            manager.clone(),
            info,
            token_store_path,
            config.tiers.clone(),
            queue,
            Some(compute_manager.clone()),
            Some(distributed.p2p_node().clone()),
        );
        // M18+: let the dashboard proxy route chat inference to trusted remote
        // workers that advertise the requested model (fabric chat routing).
        state.attach_distributed(distributed.clone().into());
        // Q2: enable consumer API keys (`dca_…`) sharing the authoritative
        // quota ledger with the compute manager, so worker credits and
        // consumer reserve/settle are one ledger.
        state.attach_consumer(
            Some(data_dir.join("db/consumer_keys.json")),
            Some(compute_manager.quota_ledger()),
        );
        match serve_api(state, &bind_address, api_port).await {
            Ok(addr) => info!(address = %addr, "dashboard serving"),
            Err(e) => warn!(error = %e, "dashboard failed to bind"),
        }
    } else {
        info!(
            "no local model/runtime; running as a coordinator/discovery node (dashboard skipped)"
        );
    }

    println!(
        "DecentraAI node running\n  Node      : {node_name}\n  PeerId    : {pid}\n  P2P PeerId: {p2p}\n  Listening : {bound}/p2p/{p2p}\n  Dashboard : http://{bind}:{port}/  (dashboard + OpenAI-compatible API)\n  Worker    : {worker}\n  Press Ctrl+C to stop",
        node_name = node_name,
        pid = identity.peer_id(),
        p2p = local_peer_id,
        bound = bound,
        bind = bind_address,
        port = api_port,
        worker = if is_worker {
            "yes (serving model)"
        } else {
            "no (coordinator-only)"
        }
    );

    // Product ingress: `decentraai node --prompt "…"` runs one routed request
    // through THIS node's existing fabric planner → reservation → P2P → worker
    // path, then shuts down. The planner decides local vs remote automatically.
    if let Some(prompt) = args.prompt {
        let result = node_ingress_ask(
            &distributed,
            &compute_manager,
            prompt,
            local_peer_id,
            args.session,
        )
        .await;
        // Stop the local llama-server we own before exiting (if any).
        if let Some(server) = maybe_server.take() {
            let _ = server.stop().await;
        }
        distributed.shutdown();
        return result;
    }

    tokio::signal::ctrl_c().await?;

    // Clean shutdown: stop the llama-server we own and the distributed node.
    if let Some(server) = maybe_server.take() {
        let _ = server.stop().await;
    }
    distributed.shutdown();
    Ok(())
}

/// Best-effort file size (MiB-in bytes); 0 when unknown.
fn metadata_size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
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
    if args.online {
        run_online_check(&config);
    }
    Ok(())
}
/// Maps the configured inference backend's bind address + API port to a
/// `host:port` probe target. `port == 0` means the node was configured for
/// an ephemeral API port, so there is no fixed port to probe and the check
/// must report that instead of guessing.
fn base_api_addr(bind_address: &str, api_port: u16) -> Option<String> {
    if api_port == 0 {
        return None;
    }
    let host = if bind_address.is_empty() {
        "127.0.0.1"
    } else {
        bind_address
    };
    Some(format!("{}:{}", host, api_port))
}
/// Minimal, non-destructive reachability probe. Resolves the configured
/// backend address and attempts a single short TCP connect to the API port.
/// No process is started and no descriptor is held beyond the probe; a
/// closed/unreachable port just prints a message and continues.
fn run_online_check(config: &NodeConfig) {
    println!("Online check:");
    let reachable = match base_api_addr(&config.inference.bind_address, config.inference.api_port) {
        Some(addr) => {
            let start = std::time::Instant::now();
            // 1.5s cap keeps the doctor command snappy and safe. The bind
            // address is normally a loopback literal, so parse directly.
            match addr.parse::<std::net::SocketAddr>() {
                Ok(socket) => match std::net::TcpStream::connect_timeout(
                    &socket,
                    Duration::from_millis(1500),
                ) {
                    Ok(_) => {
                        let latency_ms = start.elapsed().as_millis();
                        println!("  Backend {} reachable (yes, {} ms)", addr, latency_ms);
                        true
                    }
                    Err(e) => {
                        let latency_ms = start.elapsed().as_millis();
                        println!("  Backend {} reachable (no, {} ms): {}", addr, latency_ms, e);
                        println!(
                            "  Is the node serving? Run 'decentraai node' or 'decentraai serve start' first."
                        );
                        false
                    }
                },
                Err(e) => {
                    println!("  Backend {} address invalid ({}): {}", addr, e, addr);
                    false
                }
            }
        }
        None => {
            println!(
                "  Backend reachable (skipped): inference.api_port is 0 (ephemeral), so there is no fixed API port to probe."
            );
            false
        }
    };
    let status = if reachable { "ok" } else { "degraded" };
    println!("  online: {}", status);
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

/// Search the HuggingFace Hub for GGUF models (ModelCommand::Search).
///
/// The Hub search API accepts a `filter=gguf` so every hit is a GGUF repo
/// that DecentraAI can actually serve. `--category` narrows by pipeline tag
/// (the model's tool/use-case); `--categories` instead prints the distinct
/// categories found, so a user can discover "what kinds of tools are out
/// there" before picking one.
async fn model_search(
    query: String,
    category: Option<String>,
    categories: bool,
    limit: usize,
) -> Result<()> {
    let catalog = decentraai_hub::HubCatalog::new();
    let models = catalog
        .search(&query, limit)
        .await
        .with_context(|| format!("searching HuggingFace Hub for '{query}'"))?;

    if categories {
        // Distinct category → count, sorted by popularity of first sighting.
        let mut seen: Vec<(String, usize)> = Vec::new();
        for m in &models {
            let tag = m
                .pipeline_tag
                .as_ref()
                .map(|t| t.as_str().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            match seen.iter_mut().find(|(name, _)| *name == tag) {
                Some((_, n)) => *n += 1,
                None => seen.push((tag, 1)),
            }
        }
        seen.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        println!("Categories for '{query}' ({} models):", models.len());
        for (tag, count) in seen {
            println!("  {tag:<45} {count} model(s)");
        }
        println!(
            "\nFilter with: decentraai model search \"{query}\" --category <category>"
        );
        return Ok(());
    }

    println!("Models for '{query}' ({}, filter=gguf):", models.len());
    let mut shown = 0;
    for m in &models {
        let tag = m
            .pipeline_tag
            .as_ref()
            .map(|t| t.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        if let Some(cat) = &category {
            if !tag.eq_ignore_ascii_case(cat) {
                continue;
            }
        }
        shown += 1;
        println!(
            "  {:<60} {:<40} {} downloads",
            m.id, tag, m.downloads
        );
    }
    if shown == 0 && category.is_some() {
        println!(
            "  (no matches in category '{}'; use --categories to see what exists)",
            category.as_deref().unwrap_or_default()
        );
    }
    println!("\nDownload with: decentraai model pull hf:ORG/REPO");
    Ok(())
}

/// Download a model from the HuggingFace Hub into the local models/ dir and
/// refresh the registry (ModelCommand::Pull).
///
/// A Hub reference is `hf:org/repo` or `hf:org/repo:file.gguf`. The download
/// is verified against the Hub's SHA-256 before the file is atomically renamed
/// into place (no partial/ corrupted model ever enters the registry).
async fn model_pull(reference: String, models_dir: String, registry: String) -> Result<()> {
    let hf_ref = decentraai_hub::HfRef::parse(&reference)?;
    let models_dir = expand_tilde(&models_dir);
    fs::create_dir_all(&models_dir)
        .with_context(|| format!("creating models dir {}", models_dir.display()))?;

    println!(
        "Downloading {} ({} / {}) ...",
        reference,
        hf_ref.repo,
        hf_ref.file.as_deref().unwrap_or("auto (largest GGUF)")
    );
    let dl = decentraai_hub::download_model(&hf_ref, &models_dir).await?;
    println!(
        "Downloaded {} ({} bytes, sha256 {})",
        dl.path.display(),
        dl.bytes,
        dl.sha256
    );

    // Refresh the registry so the new model is immediately usable/servable.
    let registry_path = expand_tilde(&registry);
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
        ModelRegistry::new(models_dir.clone())
            .with_context(|| format!("creating new registry for {}", models_dir.display()))?
    };
    let count = registry
        .scan_directory(&models_dir)
        .with_context(|| format!("scanning directory {}", models_dir.display()))?;
    registry
        .save(&registry_path)
        .with_context(|| format!("saving registry to {}", registry_path.display()))?;
    println!(
        "Registry updated: {} models at {}",
        count,
        registry_path.display()
    );
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
    let ban_duration = Duration::from_secs(u64::from(config.security.ban_duration_minutes) * 60);
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
    mut ann_rx: tokio::sync::mpsc::UnboundedReceiver<(
        decentraai_p2p::PeerId,
        decentraai_manifest::Manifest,
    )>,
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
            download_multi(&node, &[peer], &manifest.model_id, &data_dir, &mut guard).await
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
    backend: Option<String>,
) -> Result<()> {
    use decentraai_runtime::{
        LlamaServer, RuntimeConfig, ServeManager, ensure_admitted, find_llama_server, resolve_model,
    };
    use decentraai_inference_adapter::{BackendConfig, EngineKind, OpenAiCompatibleBackend};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    let config = NodeConfig::load(&config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;

    let data_dir = expand_tilde(&config.node.data_dir);
    let idle_timeout =
        Duration::from_secs(u64::from(config.inference.idle_model_unload_minutes) * 60);

    // Q3 remote backend: the model runs on a remote OpenAI-compatible
    // server; this node keeps auth/tiers/queue/dashboard local. No local
    // llama-server is spawned or probed, and no local model/GPU is needed —
    // inference admission and the engine process are the remote machine's job.
    // The remote is selected by the `--backend` flag or, when absent, by
    // `inference.backend_url` (M22).
    let remote = backend.or_else(|| config.inference.backend_url.clone());
    if let Some(remote) = remote {
        if !remote.starts_with("http://") && !remote.starts_with("https://") {
            anyhow::bail!(
                "--backend must be an http(s) URL, e.g. http://192.168.1.50:8080 (got {remote})"
            );
        }
        let backend_url = remote;
        // M22: engine-kind selection + honest capability probe. When the
        // configured `inference.engine` selects a non-llama engine, build a
        // real probe backend and inspect the live endpoint once at startup,
        // logging the honest (possibly conservative) result. llama-server
        // stays the default and this probe only runs for an opt-in
        // alternative engine.
        let engine = config
            .inference
            .engine
            .as_deref()
            .map(EngineKind::parse)
            .unwrap_or(EngineKind::LlamaServer);
        if engine != EngineKind::LlamaServer {
            match OpenAiCompatibleBackend::new(BackendConfig {
                base_url: backend_url.clone(),
                model: "remote".to_string(),
                api_key: None,
                connect_timeout: Duration::from_secs(3),
                request_timeout: Duration::from_secs(300),
                max_prompt_bytes: 200_000,
                max_output_tokens: 8192,
                engine,
                // Remote backend is a fixed external URL (Q3); static is correct.
                backend_url_resolver: None,
            }) {
                Ok(probe) => {
                    let caps = probe.probe_capabilities().await;
                    tracing::info!(
                        engine = %engine.as_str(),
                        base_url = %backend_url,
                        streaming = caps.streaming,
                        kv_report = caps.kv_report,
                        prefill_decode_separation = caps.prefill_decode_separation,
                        expert_routing = caps.expert_routing,
                        tensor_parallel = caps.tensor_parallel,
                        "M22 probed non-llama backend capabilities (best-effort, not production-verified)"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        engine = %engine.as_str(),
                        "M22 failed to build probe backend; continuing with configured URL"
                    );
                }
            }
        }
        let manager = Arc::new(Mutex::new(ServeManager::unloaded(idle_timeout)));
        // No round-trip health probe here (a dead remote would block boot and
        // the proxy already surfaces 503 for an unreachable backend); the
        // remote is used as configured.
        let model_name = model
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or("remote")
            .to_string();
        let _api_addr = serve_common(
            &config,
            backend_url,
            manager.clone(),
            model_name,
            0,
            data_dir.clone(),
            config.inference.bind_address.clone(),
            config.inference.api_port,
            true,
        )
        .await?;
        tokio::signal::ctrl_c().await?;
        manager.lock().await.shutdown().await?;
        return Ok(());
    }

    ensure_admitted(&config)?;

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

    let api_addr = serve_common(
        &config,
        backend_url,
        manager.clone(),
        model_name,
        model_size_bytes,
        data_dir,
        config.inference.bind_address.clone(),
        config.inference.api_port,
        false,
    )
    .await?;

    println!(
        "  Model: {}\n  Threads: {} (logical CPUs minus reserve)",
        model_path.display(),
        runtime.threads.unwrap_or(0),
    );
    tokio::signal::ctrl_c().await?;
    manager.lock().await.shutdown().await?;
    let _ = api_addr;
    Ok(())
}

/// Shared tail for `serve start` (local engine and Q3 remote backend): builds
/// the dashboard/API state, serves it, and returns the bound API socket addr.
/// The caller is responsible for blocking until Ctrl+C and shutting down.
#[allow(clippy::too_many_arguments)]
async fn serve_common(
    config: &NodeConfig,
    backend_url: String,
    manager: Arc<tokio::sync::Mutex<decentraai_runtime::ServeManager>>,
    model_name: String,
    model_size_bytes: u64,
    data_dir: std::path::PathBuf,
    bind_address: String,
    api_port: u16,
    remote: bool,
) -> Result<std::net::SocketAddr> {
    use decentraai_runtime::api::{ApiState, DashboardInfo, ensure_api_token, serve_api};
    use decentraai_runtime::queue::InferenceQueue;
    use std::time::Duration;

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
        api_port,
        model_name,
        model_size_bytes,
        generation: config.inference.generation.clone(),
        resources: config.resources.clone(),
    };
    let token_store_path = config.tiers.as_ref().map(|_| data_dir.join("db/tokens.json"));
    // Q2: one request at a time reaches the backend with the machine's
    // full resources; the waiting room and wait limit come from config.
    let queue = InferenceQueue::new(
        usize::from(config.inference.queue_max_requests),
        Duration::from_secs(u64::from(config.inference.request_timeout_seconds)),
    );
    let state = ApiState::new(
        backend_url.clone(),
        token.clone(),
        manager.clone(),
        info,
        token_store_path,
        config.tiers.clone(),
        queue,
        None,
        None,
    );
    let api_addr = serve_api(state, &bind_address, api_port).await?;

    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        if remote { "remote_backend_started" } else { "inference_started" },
        serde_json::json!({
            "backend": backend_url,
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
    let mode_hint = if remote {
        format!("remote backend (no local engine): {backend_url}")
    } else {
        format!("local llama-server: {backend_url}  idle unload: {} min", config.inference.idle_model_unload_minutes)
    };
    println!(
        "DecentraAI inference running\n  Mode: {}\n  Queue: FIFO, {} waiting slots, {}s wait limit (dashboard shows it live)\n  Dashboard: http://{}/ (status, peers, share guide)\n  API: http://{}/v1 (OpenAI-compatible)\n  Auth: {}\n  Subscriptions: {}\n  Press Ctrl+C to stop",
        mode_hint,
        config.inference.queue_max_requests,
        config.inference.request_timeout_seconds,
        api_addr,
        api_addr,
        auth_hint,
        tiers_hint,
    );
    Ok(api_addr)
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

/// Manages the coordinator-side trust set (`trust.db`) that gates the
/// capability-aware compute scheduler. Trust is the answer to "which peer
/// may execute workloads on my behalf"; without a record here the scheduler
/// rejects every worker with `NotTrusted`.
fn trust_command(command: TrustCommand) -> Result<()> {
    use decentraai_discovery::{TrustRecordPersisted, TrustStore};

    let (config, op) = match &command {
        TrustCommand::Add { config, .. } => (config, Op::Add),
        TrustCommand::List { config } => (config, Op::List),
        TrustCommand::Remove { config, .. } => (config, Op::Remove),
    };
    enum Op {
        Add,
        List,
        Remove,
    }

    let node_config =
        NodeConfig::load(config).with_context(|| format!("loading {}", config.display()))?;
    let data_dir = expand_tilde(&node_config.node.data_dir);
    let trust_db_path = data_dir.join("trust.db");
    let store = TrustStore::new(&trust_db_path)
        .with_context(|| format!("opening trust store at {}", trust_db_path.display()))?;

    match op {
        Op::List => {
            let records = store.list_trusted()?;
            for record in records {
                println!(
                    "{} {} (trust={:.2} req={})",
                    record.worker_peer_id,
                    record.node_name,
                    record.trust_score,
                    record.total_requests
                );
            }
            println!("{} trusted worker(s)", store.list_trusted()?.len());
        }
        Op::Add => {
            let (peer, name) = match &command {
                TrustCommand::Add { peer, name, .. } => (peer, name),
                _ => unreachable!(),
            };
            // Reject malformed peer ids before writing anything.
            peer.parse::<libp2p::PeerId>()
                .map_err(|e| anyhow::anyhow!("invalid peer id {peer:?}: {e}"))?;
            let now = chrono::Utc::now();
            let record = TrustRecordPersisted {
                worker_peer_id: peer.clone(),
                controller_peer_id: String::new(),
                node_name: name.clone(),
                paired_at: now,
                last_seen: now,
                trust_score: 1.0,
                total_requests: 0,
                successful_requests: 0,
                pairing_token: String::new(),
            };
            store.add_trust(&record)?;
            println!("Trusted {peer} ({name}); capability-aware scheduling is now allowed");
        }
        Op::Remove => {
            let peer = match &command {
                TrustCommand::Remove { peer, .. } => peer,
                _ => unreachable!(),
            };
            store.remove_trust(peer)?;
            println!("Removed trust for {peer}");
        }
    }
    Ok(())
}

/// Splits an invite string (`<reachable-multiaddr> <token>`) into its two
/// parts. The multiaddr and token are separated by exactly one space, so a
/// path multiaddr (slashes, no spaces) parses unambiguously. The trailing
/// `/p2p/<peer-id>` from the invite is the dial target and is preserved.
fn parse_invite(invite: &str) -> Result<(String, String)> {
    let split_at = invite
        .find(' ')
        .context("invite must be '<reachable-multiaddr> <token>'")?;
    let multiaddr = invite[..split_at].trim();
    let token = invite[split_at..].trim_start();
    if multiaddr.is_empty() || token.is_empty() {
        anyhow::bail!("invite must be '<reachable-multiaddr> <token>'");
    }
    if !token.starts_with("dsk_") {
        anyhow::bail!("invite token must start with 'dsk_' — got an invalid invite string");
    }
    Ok((multiaddr.to_string(), token.to_string()))
}

/// Issues a join invite for a newcomer (P5). Creates a fresh Tier-1 Guest
/// token (least privilege) named `invite-<n>` so it is easy to revoke a single
/// seat, then prints a copy-pastable `<reachable-multiaddr>/p2p/<peer-id> <token>`
/// string. The multiaddr suffix is appended from the node identity's libp2p
/// peer id, so the printed value is dialable as-is. The plaintext token is
/// shown exactly once; only its BLAKE3 hash is stored.
fn invite(args: InviteArgs) -> Result<()> {
    use decentraai_identity::Identity;
    use decentraai_tokens::{Tier, TokenStore};
    use std::time::{SystemTime, UNIX_EPOCH};

    let config = NodeConfig::load(&args.config)
        .with_context(|| format!("loading {}", args.config.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    let identity = Identity::load(&identity_path).with_context(|| {
        format!(
            "no identity at {} — run 'decentraai init' or 'decentraai setup' first",
            identity_path.display()
        )
    })?;
    let peer_id = identity.peer_id().to_string();

    let addr = args.addr.trim();
    if addr.is_empty() {
        anyhow::bail!("--addr must be this node's reachable address (e.g. /ip4/192.168.1.5/tcp/4001)");
    }
    // Build the fully-qualified multiaddr so the printed invite dials directly.
    let multiaddr = format!("{addr}/p2p/{peer_id}");

    let mut store = TokenStore::load(&data_dir.join("db/tokens.json"))
        .with_context(|| "loading token registry".to_string())?;
    let name = format!(
        "invite-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_at = if args.ttl > 0 {
        Some(now_secs + args.ttl * 60)
    } else {
        None
    };
    let token = store.create(&name, Tier::GUEST, expires_at)?;
    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        "invite_created",
        serde_json::json!({
            "name": name,
            "tier": Tier::GUEST.0,
            "addr": addr,
            "expires_at": expires_at,
            "ttl_minutes": args.ttl,
        }),
    );

    println!("Join invite for '{name}' (Tier 1 — Guest, least privilege):");
    println!("  {multiaddr} {token}");
    if let Some(exp) = expires_at {
        println!("  Expires: {} ({} min left)", exp, args.ttl);
    }
    println!();
    println!(
        "Share this with a newcomer, who runs exactly:\n  decentraai join \"{multiaddr} {token}\""
    );
    println!("The token is shown once; notify 'decentraai token revoke --name {name}' to invalidate a seat.");
    Ok(())
}

/// Joins a private swarm from an invite produced by `decentraai invite` (P5).
/// Parses the `<reachable-multiaddr> <token>` string, auto-provisions an
/// identity + validated config for a fresh node (reusing the `setup` wizard so
/// nothing needs to be hand-tuned), stores the guest token as this node's
/// credential (`runtime/invite.token`, 0600), and verifies the multiaddr is
/// actually reachable before declaring success. Ongoing peer discovery is
/// handled by the node's normal mDNS/discovery path.
async fn join(args: JoinArgs) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = expand_tilde(&args.data_dir);
    let config_path = expand_tilde(&args.config.to_string_lossy());
    let (multiaddr, token) = parse_invite(&args.invite)?;

    // 1. Auto-provision identity + config for this fresh node if first run.
    if !config_path.exists() && !data_dir.join("identity/key.pem").exists() {
        setup(SetupArgs {
            data_dir: data_dir.to_string_lossy().into_owned(),
            config: config_path.clone(),
            models_dir: None,
            name: None,
        })?;
    }
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        anyhow::bail!(
            "no identity at {}; run 'decentraai init' first",
            identity_path.display()
        );
    }

    // 2. Store the guest token as this node's credential (0600). The joined node
    //    uses it to authenticate to the coordinator's API; the coordinator keeps
    //    only the hash, so this is the seat's only plaintext copy.
    let runtime_dir = data_dir.join("runtime");
    fs::create_dir_all(&runtime_dir)?;
    let credential_path = runtime_dir.join("invite.token");
    fs::write(&credential_path, format!("{token}\n"))?;
    let mut perms = fs::metadata(&credential_path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&credential_path, perms)?;

    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        "joined",
        serde_json::json!({"peer": multiaddr}),
    );

    // 3. Verify the coordinating peer is reachable over the verified P2P path.
    {
        use decentraai_p2p::DEFAULT_MAX_CHUNK_MESSAGE_BYTES;
        use decentraai_p2p::P2PNode;
        let identity = Identity::load(&identity_path)?;
        let node = P2PNode::new(&identity, 1_048_576, DEFAULT_MAX_CHUNK_MESSAGE_BYTES, None)?;
        node.dial(&multiaddr).await.with_context(|| {
            format!("could not reach the coordinating peer at {multiaddr}; is it online?")
        })?;
        node.shutdown();
    }

    println!("Joined the swarm — connected to the coordinating peer at {multiaddr}");
    println!("  Credential stored (0600): {}", credential_path.display());
    let conf = config_path.display();
    println!("  Start this node any time with:");
    println!("    decentraai node --config {conf}");
    println!("    decentraai open --port 8080");
    Ok(())
}

fn token_command(command: TokenCommand) -> Result<()> {
    use decentraai_tokens::{Tier, TokenStore};
    let config_path = match &command {
        TokenCommand::Create { config, .. }
        | TokenCommand::List { config, .. }
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
        TokenCommand::Create {
            name,
            tier,
            role,
            expires_at,
            ..
        } => {
            let tier = Tier::parse(tier)?;
            let role = decentraai_tokens::Role::parse(&role)?;
            let plaintext = store.create_with_role(&name, tier, expires_at, role)?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "token_created",
                serde_json::json!({"name": name, "tier": tier.0, "role": role.name(), "expires_at": expires_at}),
            );
            println!(
                "Subscription token for '{name}' (tier {} — {}, role {}):",
                tier.0,
                tier.name(),
                role.name()
            );
            println!("  {plaintext}");
            println!("Store it now: it is shown once and only its BLAKE3 hash is kept.");
            println!("Active at the next API request; no restart needed.");
            if let Some(ts) = expires_at {
                println!("Expires at unix time {ts} (then it stops authenticating).");
            }
        }
        TokenCommand::List { json, .. } => {
            let records = store.list();
            if json {
                println!("{}", serde_json::to_string(&records)?);
            } else {
                println!("Subscription tokens ({}):", records.len());
                for record in records {
                    let status = if record.revoked { "revoked" } else { "active" };
                    let expiry = record
                        .expires_at
                        .map(|ts| format!(", expires {ts}"))
                        .unwrap_or_default();
                    println!(
                        "  {} (tier {}, role {}, {}{}) — created {}",
                        record.name,
                        record.tier,
                        record.role.name(),
                        status,
                        expiry,
                        record.created_at
                    );
                }
                if store.list().is_empty() {
                    println!(
                        "  none yet — create one with: decentraai token create --name <n> --tier 1..3"
                    );
                }
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

/// Q2 — consumer API key management (`dca_…`): create/list/revoke keys for
/// the Compute Contribution & Quota consumer path. Reuses the `dsk_`-style
/// security model (hash-only storage, atomic persistence, revoke-by-id).
fn consumer_key_command(command: ConsumerKeyCommand) -> Result<()> {
    use decentraai_tokens::ConsumerKeyStore;
    let config_path = match &command {
        ConsumerKeyCommand::Create { config, .. }
        | ConsumerKeyCommand::List { config }
        | ConsumerKeyCommand::Revoke { config, .. } => config,
    };
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let registry_path = data_dir.join("db/consumer_keys.json");
    let mut store = ConsumerKeyStore::load(&registry_path)
        .with_context(|| format!("loading consumer key registry from {}", registry_path.display()))?;
    let logs_dir = data_dir.join("logs");

    match command {
        ConsumerKeyCommand::Create {
            account,
            quota_ceiling,
            rate_limit_per_minute,
            scopes,
            ..
        } => {
            let scopes: Vec<String> = scopes
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let plaintext = store.create(&account, quota_ceiling, rate_limit_per_minute, scopes.clone())?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "consumer_key_created",
                serde_json::json!({
                    "account": account,
                    "key_prefix": decentraai_tokens::key_prefix(&plaintext),
                    "quota_ceiling": quota_ceiling,
                    "rate_limit_per_minute": rate_limit_per_minute,
                    "scopes": scopes,
                }),
            );
            let key_id = store
                .lookup(&plaintext)
                .map(|r| r.key_id.clone())
                .unwrap_or_default();
            println!("Consumer API key for account '{account}' (quota ceiling {quota_ceiling} units/req, {rate_limit_per_minute} req/min):");
            println!("  {plaintext}");
            println!("  key_id: {key_id}");
            println!("Store it now: it is shown once and only its BLAKE3 hash is kept.");
            println!("Never share it; it is an inference credential, not an admin key.");
        }
        ConsumerKeyCommand::List { .. } => {
            let records = store.list();
            if records.is_empty() {
                println!("No consumer API keys yet — create one with: decentraai consumer-key create --account <n> --quota-ceiling <u> --rate-limit-per-minute <n>");
            } else {
                println!("Consumer API keys ({}):", records.len());
                for r in records {
                    let status = if r.revoked { "revoked" } else { "active" };
                    let scopes = r.scopes.join(",");
                    println!(
                        "  {} ({}): account={}, ceiling={}, rate={}/min, scopes=[{}], created {}",
                        r.key_id, status, r.owner_account, r.quota_ceiling, r.rate_limit_per_minute, scopes, r.created_at
                    );
                }
                println!("The plaintext secrets are never stored or shown here.");
            }
        }
        ConsumerKeyCommand::Revoke { key_id, .. } => {
            store.revoke(&key_id)?;
            decentraai_audit::record_best_effort(
                &logs_dir,
                "consumer_key_revoked",
                serde_json::json!({"key_id": key_id}),
            );
            println!("Consumer key '{key_id}' revoked; it stops authenticating at the next API request.");
        }
    }
    Ok(())
}

/// Reads the persisted contribution report and prints each worker's computed
/// `decentraai tier` — subscription tiers driven by measured contribution
/// (M17 suggest / P4 apply). Reads the report a coordinator wrote while
/// `distributed start` served requests; read-only in every mode.
fn tier_command(command: TierCommand) -> Result<()> {
    let config_path = match &command {
        TierCommand::Suggest { config } | TierCommand::Apply { config, .. } => config,
    };
    let node_config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&node_config.node.data_dir);

    let rows = load_contribution_report(&data_dir)?;

    match command {
        TierCommand::Suggest { .. } => print_tier_suggestions(&rows),
        TierCommand::Apply { yes, .. } => apply_tier_changes(&rows, &data_dir, yes),
    }
}

/// Reads the contribution report written by a running coordinator (M17).
/// A missing report is not an error: it prints guidance and returns `None`'s
/// empty list.
fn load_contribution_report(
    data_dir: &Path,
) -> Result<Vec<decentraai_distributed::ContributionRow>> {
    let path = data_dir.join("db/contributions.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("reading contribution report from {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "No contribution report at {}.\n\
                 Start a coordinator ('decentraai distributed start --metrics-port <P>')\n\
                 and let it serve a few requests, then re-run this command.",
                path.display()
            );
            Ok(Vec::new())
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Prints the raw contribution report (M17 `tier suggest`), read-only.
fn print_tier_suggestions(rows: &[decentraai_distributed::ContributionRow]) -> Result<()> {
    if rows.is_empty() {
        println!("No contributing workers recorded yet.");
        return Ok(());
    }
    println!(
        "{:<6} {:<24} {:>8} {:>8} {:>12}  {:>16}",
        "tier", "node", "reward", "score", "circ", "verified (hours, failed)"
    );
    for r in rows {
        println!(
            "{:<6} {:<24} {:>8} {:>8.2}  {} ({}h, {} failed)",
            r.suggested_tier,
            r.node_name,
            r.reward_tokens,
            r.score,
            r.verified_requests,
            r.online_seconds / 3600,
            r.failed_requests,
        );
    }
    println!(
        "Suggested tiers: 1=guest 2=contributor 3=core. Reward = M9-9 contribution\n\
         credits (hardware x availability x verified work, damped by failures).\n\
         Review with `decentraai tier apply` (dry-run), then `--yes` to write them."
    );
    Ok(())
}

/// Applies contribution-suggested tiers to the token registry (P4). Pairs each
/// worker's suggested tier to the active token of the same name. Dry-run by
/// default; with `yes` it reassigns tiers and records `tier_changed` audits.
fn apply_tier_changes(
    rows: &[decentraai_distributed::ContributionRow],
    data_dir: &Path,
    yes: bool,
) -> Result<()> {
    use decentraai_tokens::{Tier, TierChange, TokenStore, plan_tier_changes};

    let registry_path = data_dir.join("db/tokens.json");
    let store = TokenStore::load(&registry_path)
        .with_context(|| format!("loading token registry from {}", registry_path.display()))?;

    let suggestions: Vec<decentraai_tokens::SuggestedTier> = rows
        .iter()
        .map(|r| decentraai_tokens::SuggestedTier {
            name: r.node_name.clone(),
            suggested: r.suggested_tier,
        })
        .collect();
    let tokens = store.list();
    let changes = plan_tier_changes(&suggestions, &tokens);

    if changes.is_empty() {
        println!("No tier changes to apply (tokens already match their contribution).");
        return Ok(());
    }

    if !yes {
        println!("Planned tier changes (dry-run; add --yes to apply):");
        for c in &changes {
            println!(
                "  {}: tier {} → {} ({})",
                c.name,
                c.from,
                c.to,
                Tier(c.to).name()
            );
        }
        println!("Run again with --yes to write these and record tier_changed audit events.");
        return Ok(());
    }

    let logs_dir = data_dir.join("logs");
    let mut store = store;
    for TierChange { name, to, .. } in &changes {
        let _from = store
            .set_tier(name, Tier(*to))
            .with_context(|| format!("reassigning tier of token '{name}'"))?;
        decentraai_audit::record_best_effort(
            &logs_dir,
            "tier_changed",
            serde_json::json!({
                "name": name,
                "tier": to,
            }),
        );
    }
    println!("Applied {} tier change(s):", changes.len());
    for c in &changes {
        println!("  {}: tier {} → {}", c.name, c.from, c.to);
    }
    println!(
        "Tokens now reflect measured contribution; new tiers gate the proxy at the next request."
    );
    Ok(())
}

/// Persists the coordinator's contribution report (M17) atomically enough for
/// the CLI to read it back: write + sync + rename. Best-effort; the caller
/// treats failure as non-fatal.
fn persist_contributions(
    path: &std::path::Path,
    rows: &[decentraai_distributed::ContributionRow],
) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(rows)?;
    let tmp = path.with_extension("tmp");
    let mut out = std::fs::File::create(&tmp)?;
    out.write_all(content.as_bytes())?;
    out.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Runs a distributed inference node (M9)
///
/// This command starts a node that can act as both a worker (serving models)
/// and a client (routing requests to other workers).
async fn distributed_command(args: DistributedArgs) -> Result<()> {
    use decentraai_distributed::{DistributedInference, InferenceConfig};
    use decentraai_identity::Identity;
    use decentraai_inference_adapter::{BackendConfig, EngineKind, OpenAiCompatibleBackend};
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

    // Factory that loads a downloaded model into its own llama-server and
    // returns a serving backend (M14 on-demand provisioning). Set only when
    // worker mode finds a real llama-server binary.
    let mut provision_factory: Option<decentraai_distributed::ProvisioningFactory> = None;

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
        let binary = find_llama_server(args.binary.as_deref()).with_context(
            || "worker mode requires llama-server; pass --binary <path> or install llama.cpp",
        )?;

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

        // Create inference backend pointing at the chosen base URL.
        // M22: this node runs llama-server, so advertise that engine kind.
        let backend_config = BackendConfig {
            base_url: url,
            model: model_name.clone(),
            api_key: None,
            connect_timeout: std::time::Duration::from_secs(3),
            request_timeout: std::time::Duration::from_secs(300),
            max_prompt_bytes: 200_000,
            max_output_tokens: 8192,
            engine: EngineKind::LlamaServer,
            // Static URL for the low-level distributed worker backend (no
            // respawn supervisor on this path in current use); can be swapped
            // for a live resolver if it is ever supervised.
            backend_url_resolver: None,
        };

        let backend = OpenAiCompatibleBackend::new(backend_config)
            .map_err(|e| anyhow::anyhow!("Failed to create inference backend: {}", e))?;

        // Build the provisioning factory (M14): each downloaded model gets its
        // own llama-server instance, kept alive for the worker session.
        let max_ctx = config.inference.max_context_tokens;
        let parallel = config.inference.max_concurrent_requests;
        let reserve_cores = config.resources.reserve_cpu_cores;
        let binary_for_factory = binary.clone();
        provision_factory = Some(std::sync::Arc::new(move |model_path: PathBuf| {
            let binary = binary_for_factory.clone();
            Box::pin(async move {
                let mut cfg = RuntimeConfig::new(model_path);
                cfg.ctx_size = max_ctx;
                cfg.parallel = parallel;
                cfg.threads = Some(
                    SystemSnapshot::collect()
                        .logical_cpus
                        .saturating_sub(usize::from(reserve_cores))
                        .max(1),
                );
                let server = LlamaServer::spawn(&binary, &cfg).await?;
                let backend_cfg = BackendConfig {
                    base_url: server.base_url(),
                    model: "provisioned".to_string(),
                    api_key: None,
                    connect_timeout: std::time::Duration::from_secs(3),
                    request_timeout: std::time::Duration::from_secs(300),
                    max_prompt_bytes: 200_000,
                    max_output_tokens: 8192,
                    engine: EngineKind::LlamaServer,
                    // Provisioned engines are freshly spawned per model and
                    // live for the session; static URL is correct (see note at
                    // the node start provisioning factory).
                    backend_url_resolver: None,
                };
                let backend = OpenAiCompatibleBackend::new(backend_cfg)
                    .map_err(|e| anyhow::anyhow!("failed to create provisioned backend: {e}"))?;
                Ok((Box::new(server) as Box<dyn std::any::Any + Send>, backend))
            })
        }));

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
            Err(e) => {
                tracing::warn!(error = %e, "failed to open trust.db; compute trust set is empty")
            }
        }
    }
    let mut compute_manager = Arc::new(decentraai_distributed::ComputeManager::new(
        local_peer_id,
        args.name.clone(),
        compute_trusted,
    ));
    // P3: sign this node's advertisements so recipients authenticate them.
    if let Some(cm) = Arc::get_mut(&mut compute_manager) {
        cm.set_signing_key(identity.signing_key_bytes());
    }
// M22: advertise the configured engine kind honestly rather than assuming
    // llama-server (which remains the default when unset).
    if let Some(engine) = config.inference.engine.as_deref() {
        if let Some(cm) = Arc::get_mut(&mut compute_manager) {
            cm.set_engine(engine);
        }
    }
// Remote-sharing opt-in: the advertisement carries whether this node
    // accepts inference routed from remote peers, so coordinators only ever
    // schedule remote workers that will actually serve the request.
    if let Some(cm) = Arc::get_mut(&mut compute_manager) {
        cm.set_accepts_remote_inference(config.inference.allow_remote_inference);
    }
    // Part 17/22: persistent execution history (db/executions.jsonl) — the
    // coordinator replays past executions on restart instead of losing them.
    compute_manager.set_executions_path(Some(data_dir.join("db/executions.jsonl")));
    // Coordinator-side policy: when the node permits on-demand provisioning,
    // the scheduler may route workloads to workers that will fetch the model
    // instead of only to workers that already serve it (M14).
    compute_manager
        .set_allow_provisioning(config.sharing.provision_models_on_demand)
        .await;

    // Create a shared request tracker for streaming progress
    let tracker = Arc::new(decentraai_distributed::RequestTracker::new());

    // Create distributed P2P handler: worker announcements AND compute
    // advertisements are processed here on every node (worker or
    // coordinator). Inference serving is NOT handled through this chain
    // anymore — worker mode registers a streaming backend via
    // `register_worker_backend`, which installs the P2P-level on_infer
    // callback that enqueues + streams (queue → backend → progress).
    let mut distributed_handler =
        decentraai_distributed::DistributedP2PHandler::with_worker_manager(worker_manager.clone());
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
    // M10: per-request routing audit events (request/worker/model hash/status).
    distributed.set_logs_dir(Some(data_dir.join("logs")));
    // P1: sign outbound routed requests with the node identity so workers can
    // authenticate them and reject spoofed/unsigned traffic.
    distributed.set_signing_identity(identity.signing_key_bytes());
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

        // Register the local backend to handle inference and streaming. When
        // on-demand provisioning is enabled and a real llama-server binary is
        // available, the worker also answers workloads for models it does not
        // hold yet by fetching them through the verified-transfer pipeline.
        let can_provision =
            config.sharing.provision_models_on_demand && provision_factory.is_some();
        let provisioning = if can_provision {
            Some(decentraai_distributed::ProvisioningConfig {
                data_dir: data_dir.clone(),
                registry_path: registry_path.clone(),
                reputation_path: data_dir.join("db/reputation.json"),
                max_concurrent_downloads: config.sharing.max_concurrent_downloads as usize,
                max_invalid_chunks: config.security.max_invalid_chunks_per_peer,
                ban_duration: Duration::from_secs(
                    u64::from(config.security.ban_duration_minutes) * 60,
                ),
                backend_factory: provision_factory.take().expect("factory built above"),
            })
        } else {
            None
        };
        if let Some(backend) = &backend {
            distributed.register_worker_backend(
                backend.clone(),
                model_hash.clone(),
                provisioning,
                config.inference.allow_remote_inference,
            )?;
        }

        // Advertise compute capability from a real hardware probe so this
        // node can be selected through the capability-aware scheduler.
        let served_models = build_served_models(
            &registry_path,
            &model_hash,
            &model_name,
            config.inference.max_context_tokens,
        )?;
        let available_models =
            build_available_models(&registry_path, config.inference.max_context_tokens)?;
        let snapshot = SystemSnapshot::collect();
        let gpu = decentraai_system_probe::probe_gpu();
        let adv = compute_manager
            .advertise_local(snapshot, gpu, served_models, available_models, can_provision)
            .await;
        info!(
            peer_id = %local_peer_id,
            node_name = %adv.node_name,
            models = ?adv.capability.served_models.iter().map(|m| &m.model_hash).collect::<Vec<_>>(),
            on_disk = adv.capability.available_models.len(),
            can_provision,
            "registered as distributed compute worker"
        );
        spawn_compute_broadcaster(
            compute_manager.clone(),
            distributed.p2p_node().clone(),
            can_provision,
            None,
            None,
        )
        .await?;

        info!(peer_id = %local_peer_id, model = %model_name, "registered as distributed worker");
    }

    // M19: periodically measure RTT to each known remote worker so the
    // execution planner weights reach cost, not just nominal performance.
    spawn_network_probe(compute_manager.clone(), distributed.p2p_node().clone()).await;

    // M24: periodically prune stale reservations and evict dead workers, with
    // an audit trail for every removal (automatic removal of unhealthy peers).
    spawn_worker_reaper(
        compute_manager.clone(),
        data_dir.join("logs"),
        default_reap_grace(),
    )
    .await;

    // Start worker discovery
    distributed.start_worker_discovery().await?;

    let mode = if will_be_worker { "worker" } else { "client" };
    println!(
        "DecentraAI distributed node running\n  PeerId: {}\n  Listening: {}/p2p/{}\n  Mode: {}",
        local_peer_id, bound, local_peer_id, mode
    );

    // One-shot client mode: route the prompt to the best available worker,
    // stream the response, then exit. This exercises the REAL path end-to-end:
    // coordinator router → P2P InferRequest → worker queue → llama-server.
    if let Some(prompt) = args.prompt {
        run_distributed_ask(&distributed, prompt, &worker_manager, local_peer_id).await?;
    } else {
        // Persistent coordinator / worker: expose live compute metrics and
        // keep running until interrupted.
        if let Some(port) = args.metrics_port {
            let cm = compute_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = spawn_compute_metrics_server(cm, port).await {
                    tracing::warn!(error = %e, "compute metrics server exited");
                }
            });
        }
        // Persist the contribution report so `decentraai tier suggest` can
        // read it offline (M17). Best-effort; a write failure just logs.
        let contributions_path = data_dir.join("db/contributions.json");
        let cm = compute_manager.clone();
        let interval_ms = compute_manager.advertisement_interval_ms();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
            loop {
                interval.tick().await;
                let report = cm.contribution_report().await;
                if let Err(e) = persist_contributions(&contributions_path, &report) {
                    tracing::warn!(error = %e, "failed to persist contribution report");
                }
            }
        });
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
    context_tokens: u32,
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
        .or_else(|| {
            registry
                .models
                .values()
                .find(|r| r.relative_path.ends_with(model_name))
        });
    let size_mb = match record {
        Some(record) => (record.size_bytes / (1024 * 1024)).max(1),
        None => std::fs::metadata(std::path::Path::new(model_name))
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(0)
            .max(1),
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
        context_tokens,
    }])
}

/// All models this node has on disk, as `ServedModel` capability claims.
///
/// This feeds the advertisement's `available_models` channel: it tells the
/// fabric *what this worker could serve*, not just what is currently loaded.
/// The coordinator's model picker and planner can then route a workload to a
/// worker that holds the model on disk but has not loaded it yet (the worker
/// swaps its engine on demand), instead of pretending the model does not
/// exist outside the served set.
///
/// The BLAKE3 content hash is computed with a streaming hasher so a large
/// model (e.g. 4 GiB Mistral) never gets read into RAM wholesale; this runs
/// once at worker registration, not on every heartbeat.
fn build_available_models(
    registry_path: &std::path::Path,
    context_tokens: u32,
) -> Result<Vec<decentraai_compute::ServedModel>> {
    use decentraai_registry::ModelRegistry;
    use std::io::Read;

    if !registry_path.exists() {
        return Ok(vec![]);
    }
    let registry = ModelRegistry::load(registry_path)?;
    let gpu_present = matches!(
        decentraai_system_probe::probe_gpu(),
        decentraai_system_probe::GpuProbeStatus::Nvidia(_)
    );
    let mut models = Vec::new();
    for record in registry.models.values() {
        let file = std::path::Path::new(&record.canonical_path);
        // Hash streamingly (BLAKE3), never loading the whole file.
        let mut hasher = blake3::Hasher::new();
        let mut f = match std::fs::File::open(file) {
            Ok(f) => f,
            Err(_) => continue, // gone from disk; registry scan will prune it
        };
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let model_hash = hasher.finalize().to_hex().to_string();
        let size_mb = (record.size_bytes / (1024 * 1024)).max(1);
        models.push(decentraai_compute::ServedModel {
            model_hash,
            file_name: record.relative_path.clone(),
            size_mb,
            est_ram_mb: size_mb / 4 + 1024,
            est_vram_mb: if gpu_present { size_mb } else { 0 },
            context_tokens,
        });
    }
    Ok(models)
}

/// Re-probes this node's hardware and re-broadcasts the compute
/// advertisement on the heartbeat interval, so coordinators never see this
/// worker go stale. Fire-and-forget; a failing probe just skips a beat.
///
/// M24 false-ready gate: when `engine_health_url` is set (this node serves a
/// local inference engine), the advertisement is only broadcast while that
/// engine is actually alive. A worker must never advertise itself as ready
/// when its execution engine is unavailable — otherwise coordinators keep
/// routing to a node whose engine has crashed. The liveness probe is a real
/// TCP connect to the engine's host:port (dependency-free).
async fn spawn_compute_broadcaster(
    compute_manager: std::sync::Arc<decentraai_distributed::ComputeManager>,
    p2p_node: decentraai_p2p::P2PNode,
    can_provision: bool,
    manager: Option<Arc<tokio::sync::Mutex<decentraai_runtime::ServeManager>>>,
    live_engine_url: Option<Arc<std::sync::Mutex<Option<String>>>>,
) -> Result<()> {
    use decentraai_system_probe::{SystemSnapshot, probe_gpu};

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            compute_manager.advertisement_interval_ms(),
        ));
        loop {
            interval.tick().await;
            // M24: gate the advertisement on live engine health. A dead engine
            // means this node is not a usable worker this beat. The engine
            // address comes from the SINGLE authoritative source (the supervisor-
            // published live URL cache, falling back to the ServeManager's live
            // base_url), so a respawn on a new port is always probed and a frozen
            // startup URL can never suppress or wrongly enable advertisement.
            let health_sockaddr = match (&live_engine_url, &manager) {
                (Some(cache), _) => cache
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .and_then(|u| parse_http_addr(&u).map(|(h, p)| format!("{h}:{p}"))),
                (None, Some(m)) => m
                    .lock()
                    .await
                    .base_url()
                    .as_deref()
                    .and_then(parse_http_addr)
                    .map(|(h, p)| format!("{h}:{p}")),
                (None, None) => None,
            };
            if let Some(addr) = &health_sockaddr {
                let alive = tokio::net::TcpStream::connect(addr).await.is_ok();
                if !alive {
                    tracing::warn!(
                        "skipping worker advertisement: local inference engine not reachable at {addr}"
                    );
                    continue;
                }
            }
            let snapshot = SystemSnapshot::collect();
            let gpu = probe_gpu();
            // Advertise the latest probe; served_models and available_models
            // come from the last full advertisement stored in the manager (the
            // on-disk model set is recomputed at registration, not re-hashed on
            // every heartbeat).
            let workers = compute_manager.workers().await;
            let (served_models, available_models) = workers
                .iter()
                .find(|w| w.peer_id == compute_manager.local_peer())
                .map(|w| {
                    (
                        w.capability.served_models.clone(),
                        w.capability.available_models.clone(),
                    )
                })
                .unwrap_or_default();
            let adv = compute_manager
                .advertise_local(
                    snapshot,
                    gpu,
                    served_models,
                    available_models,
                    can_provision,
                )
                .await;
            // P3: sign the advertisement when the node has a signing key set,
            // so recipients authenticate it (anti-spoof).
            if let Ok(bytes) = compute_manager.advertisement_wire_bytes(&adv) {
                p2p_node.announce(bytes);
            }
        }
    });
    Ok(())
}

/// Parses `http://host:port/...` into `(host, port)`. Returns `None` on an
/// unrecognised address so the liveness gate degrades to "always advertise"
/// (never falsely blocks a healthy worker on a malformed URL).
fn parse_http_addr(url: &str) -> Option<(String, u16)> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()?),
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

/// Periodically measures round-trip latency to each known *remote* worker by
/// sending an `InferPing` over the P2P request/response channel and timing the
/// reply (M19). The measured RTT is recorded on the compute manager's network
/// graph so the execution planner weights reach cost. The local node's own
/// advertisement is never pinged (self-dial is refused by libp2p).
async fn spawn_network_probe(
    compute_manager: std::sync::Arc<decentraai_distributed::ComputeManager>,
    p2p_node: decentraai_p2p::P2PNode,
) {
    use decentraai_protocol::{InferMessage, serialize_message};
    use std::time::{Duration, Instant};

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let local = compute_manager.local_peer();
            let peers = compute_manager.workers().await;
            for adv in peers.iter().filter(|w| w.peer_id != local) {
                let peer = adv.peer_id;
                let pong = InferMessage::InferPing {
                    request_id: uuid::Uuid::new_v4(),
                };
                let Ok(bytes) = serialize_message(&pong) else {
                    continue;
                };
                let start = Instant::now();
                // Best-effort: a busy worker may drop the ping; we just skip it.
                if p2p_node.request(peer, bytes).await.is_ok() {
                    let rtt_us = start.elapsed().as_micros() as u64;
                    compute_manager.record_rtt(&peer, rtt_us, 0);
                    let link = compute_manager.network_graph().get(&peer.to_string());
                    info!(
                        peer = %peer,
                        measured_rtt_us = rtt_us,
                        graph_rtt_us = link.rtt_us,
                        graph_locality = ?link.locality,
                        graph_peers = compute_manager.network_graph().measured_len(),
                        "M19 network probe: measured RTT recorded, planner reads via NetworkGraph"
                    );
                }
            }
        }
    });
}

/// How long a worker may stay silently offline before the coordinator evicts
/// it. Stale detection already trips at 30s; the grace gives a missing worker
/// a chance to heartbeated back before its advertisement is dropped.
fn default_reap_grace() -> std::time::Duration {
    std::time::Duration::from_secs(60)
}

/// Periodically runs the coordinator's resilient-fabric maintenance (M24):
/// expire stale reservations, flip no-heartbeat peers offline, and evict
/// peers that stay gone past the grace window. Every eviction is an audit
/// event (`worker_evicted`), so the removal of unhealthy workers is
/// attributable.
async fn spawn_worker_reaper(
    compute_manager: std::sync::Arc<decentraai_distributed::ComputeManager>,
    logs_dir: std::path::PathBuf,
    grace: std::time::Duration,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            let (expired, evicted) = compute_manager.reap_unhealthy(grace).await;
            if expired > 0 {
                tracing::warn!(expired, "released expired reservations");
            }
            for (peer, name) in evicted {
                tracing::warn!(%peer, node = %name, "evicting unhealthy worker");
                decentraai_audit::record_best_effort(
                    &logs_dir,
                    "worker_evicted",
                    serde_json::json!({ "peer_id": peer.to_string(), "node_name": name }),
                );
            }
        }
    });
}

/// Serves live compute metrics (M16) as JSON on a loopback-only HTTP port.
/// `/v1/compute` returns the coordinator's view of the mesh: each worker's
/// load, queue, tokens/sec, latency, capacity and current reservations, plus
/// the local node's perf and lifetime totals. Bound to 127.0.0.1 so it never
/// leaks capacity/paths over the LAN.
async fn spawn_compute_metrics_server(
    compute_manager: std::sync::Arc<decentraai_distributed::ComputeManager>,
    port: u16,
) -> anyhow::Result<()> {
    use axum::Json;
    use axum::Router;
    use axum::extract::State;
    use axum::routing::get;

    async fn compute_metrics_handler(
        State(cm): State<std::sync::Arc<decentraai_distributed::ComputeManager>>,
    ) -> Json<decentraai_distributed::ComputeMetricsReport> {
        Json(cm.metrics_report().await)
    }

    let app = Router::new()
        .route("/v1/compute", get(compute_metrics_handler))
        .with_state(compute_manager);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, endpoint = "/v1/compute", "serving live compute metrics");
    axum::serve(listener, app).await?;
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
        distributed
            .route_request_streamed(request, progress_tx)
            .await
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
            let _ = distributed.p2p_node().request(worker.peer_id, bytes).await;
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
                resp.tokens_used, resp.processing_time_ms, resp.worker_peer_id
            );
            Ok(())
        }
        Err(e) => {
            println!("--- failed: {e} ---");
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// Product ingress for `decentraai node --prompt`: runs ONE routed request
/// through the node's own fabric planner (existing `route_request_streamed`),
/// letting the planner decide the target worker from the live compute registry
/// (trusted, eligible remote worker) — never forcing self-dial or the legacy
/// announcement pre-selection. Streams the response and reports completion.
async fn node_ingress_ask(
    distributed: &decentraai_distributed::DistributedInference,
    compute_manager: &std::sync::Arc<decentraai_distributed::ComputeManager>,
    prompt: String,
    local_peer_id: libp2p::PeerId,
    session_id: Option<String>,
) -> Result<()> {
    use decentraai_protocol::InferRequest;
    use std::time::Instant;
    use tokio::sync::mpsc;

    // Wait (up to 15s) for a trusted REMOTE worker to advertise a served model,
    // so routing has a candidate and the fabric planner can weigh eligibility.
    let deadline = Instant::now() + Duration::from_secs(15);
    let model_hash = loop {
        let candidate: Option<String> = {
            let mut found = None;
            for adv in compute_manager.workers().await {
                if adv.peer_id == local_peer_id || !compute_manager.is_trusted(&adv.peer_id).await {
                    continue;
                }
                if let Some(m) = adv.capability.served_models.first() {
                    found = Some(m.model_hash.clone());
                    break;
                }
            }
            found
        };
        if let Some(h) = candidate {
            break h;
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "no trusted remote worker advertising a served model discovered within 15s"
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    info!(model_hash = %model_hash, "node ingress: routing via fabric planner");

    let mut request = InferRequest::new(model_hash, prompt, 512);
    request = request.with_sender(local_peer_id);
    request = request.with_streaming(true);
    if let Some(sid) = &session_id {
        request = request.with_session(sid.clone());
    }
    request.timeout_ms = 120_000;

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let ask = distributed.route_request_streamed(request, progress_tx);
    let printing = tokio::spawn(async move {
        while let Some(chunk) = progress_rx.recv().await {
            print!("{chunk}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    });

    let result = ask.await;
    let _ = printing.await;
    println!();
    match result {
        Ok(resp) => {
            println!(
                "--- done (tokens={} elapsed_ms={} worker={}) ---",
                resp.tokens_used, resp.processing_time_ms, resp.worker_peer_id
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
    fn parses_engine_health_addresses() {
        assert_eq!(
            parse_http_addr("http://127.0.0.1:45249"),
            Some(("127.0.0.1".into(), 45249))
        );
        assert_eq!(
            parse_http_addr("http://127.0.0.1:45249/v1"),
            Some(("127.0.0.1".into(), 45249))
        );
        assert_eq!(
            parse_http_addr("localhost:8080"),
            Some(("localhost".into(), 8080))
        );
        // Unparseable -> None (gate degrades to always-advertise).
        assert_eq!(parse_http_addr(""), None);
        assert_eq!(parse_http_addr("http://host:notaport"), None);
    }

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
    fn parses_model_search_command() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "model",
            "search",
            "qwen",
            "--category",
            "text-generation",
            "--limit",
            "5",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Model {
                command: ModelCommand::Search { .. }
            }
        ));
    }

    #[test]
    fn parses_model_search_categories_flag() {
        let cli =
            Cli::try_parse_from(["decentraai", "model", "search", "mistral", "--categories"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Command::Model {
                command: ModelCommand::Search { categories: true, .. }
            }
        ));
    }

    #[test]
    fn parses_model_pull_command() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "model",
            "pull",
            "hf:Qwen/Qwen2.5-0.5B-Instruct-GGUF",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Model {
                command: ModelCommand::Pull { .. }
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
    fn parses_doctor_online_flag() {
        let cli = Cli::try_parse_from(["decentraai", "doctor", "--online"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor(DoctorArgs { online: true, .. })));
    }

    #[test]
    fn parses_doctor_without_online_flag() {
        let cli = Cli::try_parse_from(["decentraai", "doctor"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor(DoctorArgs { online: false, .. })));
    }

    #[test]
    fn base_api_addr_maps_bind_and_port() {
        assert_eq!(base_api_addr("127.0.0.1", 8080), Some("127.0.0.1:8080".into()));
        assert_eq!(base_api_addr("::1", 8080), Some("::1:8080".into()));
        // Empty bind falls back to loopback host.
        assert_eq!(base_api_addr("", 8080), Some("127.0.0.1:8080".into()));
        // Port 0 (ephemeral) has no fixed probe target.
        assert_eq!(base_api_addr("127.0.0.1", 0), None);
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
    fn parse_serves_remote_backend_flag() {
        let cli = Cli::try_parse_from([
            "decentraai",
            "serve",
            "start",
            "--backend",
            "http://192.168.1.50:8080",
        ])
        .unwrap();
        match cli.command {
            Command::Serve {
                command:
                    ServeCommand::Start {
                        backend: Some(url),
                        ..
                    },
            } => assert_eq!(url, "http://192.168.1.50:8080"),
            other => panic!("expected remote-backend Start, got {other:?}"),
        }
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
    fn parses_token_list_json_flag() {
        let cli = Cli::try_parse_from(["decentraai", "token", "list", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Token {
                command: TokenCommand::List { json: true, .. }
            }
        ));
    }

    #[test]
    fn parses_token_list_default_no_json() {
        let cli = Cli::try_parse_from(["decentraai", "token", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Token {
                command: TokenCommand::List { json: false, .. }
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

    #[test]
    fn auto_detects_a_gguf_model_and_ignores_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tiny.gguf"), b"gguf").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not a model").unwrap();
        assert_eq!(auto_detect_model(dir.path()).unwrap(), "tiny.gguf");
        assert!(
            auto_detect_model(&dir.path().join("missing"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn generated_setup_config_round_trips_through_validation() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("node");
        let yaml = setup_yaml("test-node", &data_dir, 4096, 0, "off", 4);
        let cfg_path = dir.path().join("node.yaml");
        std::fs::write(&cfg_path, &yaml).unwrap();
        let cfg = NodeConfig::load(&cfg_path).expect("wizard YAML must be a valid NodeConfig");
        assert_eq!(cfg.node.name, "test-node");
        assert_eq!(cfg.inference.max_context_tokens, 4096);
    }

    // ---- P5: invites & join ----

    #[test]
    fn invite_builds_a_fully_qualified_dialable_multiaddr() {
        let peer = Identity::generate().peer_id().to_string();
        let multiaddr = format!("/ip4/10.0.0.5/tcp/4001/p2p/{peer}");
        let (addr, token) =
            parse_invite(&format!("{multiaddr} dsk_abc123def456")).expect("valid invite");
        assert_eq!(addr, multiaddr, "path multiaddr must round-trip (no spaces in it)");
        assert_eq!(token, "dsk_abc123def456");
    }

    #[test]
    fn invite_parsing_rejects_malformed_strings() {
        assert!(parse_invite("/ip4/10.0.0.5/tcp/4001").is_err(), "missing token");
        assert!(
            parse_invite("/ip4/10.0.0.5/tcp/4001 xyz_token").is_err(),
            "bad token prefix"
        );
        assert!(parse_invite("   dsk_xyz").is_err(), "empty multiaddr");
    }

    #[test]
    fn invite_token_is_a_least_privilege_guest_seat() {
        // Mirrors the token call the `invite` command performs: a fresh seat is
        // always Tier 1 (Guest) and stored only as a hash, so an invite leak is
        // never more than a guest — the least privilege roadmap (P5) guarantee.
        let dir = tempfile::tempdir().unwrap();
        let mut store = decentraai_tokens::TokenStore::load(&dir.path().join("tokens.json")).unwrap();
        let plaintext = store
            .create("invite-0", decentraai_tokens::Tier::GUEST, None)
            .unwrap();
        assert_eq!(store.lookup(&plaintext).unwrap().tier, 1);
        let on_disk = std::fs::read_to_string(dir.path().join("tokens.json")).unwrap();
        assert!(!on_disk.contains(&plaintext), "plaintext must never be persisted");
    }
}
