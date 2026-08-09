use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use decentraai_config::NodeConfig;
use decentraai_identity::Identity;
use decentraai_registry::ModelRegistry;
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::fs;
use std::path::PathBuf;
use tracing::info;
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
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Explicit llama-server binary path (overrides env and PATH search).
        #[arg(long)]
        binary: Option<PathBuf>,
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
            command: ServeCommand::Start {
                model,
                config,
                binary,
            },
        } => serve_start(config, model, binary).await,
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

/// Runs the swarm: loads identity and config, listens on an ephemeral TCP
/// port, and drives the event loop until interrupted.
async fn swarm_start(config_path: PathBuf) -> Result<()> {
    use decentraai_p2p::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, P2PNode};

    let config = NodeConfig::load(&config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity_path = data_dir.join("identity/key.pem");
    if !identity_path.exists() {
        anyhow::bail!("identity not found at {}; run 'decentraai init' first", identity_path.display());
    }
    let identity = Identity::load(&identity_path)?;

    let node = P2PNode::new(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )?;
    let bound = node.listen("/ip4/0.0.0.0/tcp/0").await?;
    println!(
        "DecentraAI swarm running\n  PeerId (identity): {}\n  PeerId (libp2p): {}\n  Listening: {}/p2p/{}\n  Press Ctrl+C to stop",
        identity.peer_id(),
        node.local_peer_id(),
        bound,
        node.local_peer_id()
    );
    tokio::signal::ctrl_c().await?;
    node.shutdown();
    Ok(())
}

/// Runs gated inference with the OpenAI-compatible API: admission check,
/// registry resolution, llama-server spawn, Bearer token, thin proxy on
/// inference.bind_address:api_port, idle unload, Ctrl+C to stop.
async fn serve_start(config_path: PathBuf, model: String, binary: Option<PathBuf>) -> Result<()> {
    use decentraai_runtime::api::{ApiState, ensure_api_token, serve_api};
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
    let model_path = resolve_model(&registry, &model)?;
    let binary = find_llama_server(binary.as_deref())?;

    let mut runtime = RuntimeConfig::new(model_path.clone());
    runtime.ctx_size = config.inference.max_context_tokens;
    runtime.parallel = config.inference.max_concurrent_requests;

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
    let state = ApiState::new(backend_url, token.clone(), manager.clone());
    let api_addr =
        serve_api(state, &config.inference.bind_address, config.inference.api_port).await?;

    let auth_hint = match &token {
        Some(_) => format!("Bearer token: {}", data_dir.join("runtime/api.token").display()),
        None => "no auth required by config".to_string(),
    };
    println!(
        "DecentraAI inference running\n  Model: {}\n  API: http://{}/v1 (OpenAI-compatible)\n  Auth: {}\n  Idle unload: {} min\n  Press Ctrl+C to stop",
        model_path.display(),
        api_addr,
        auth_hint,
        config.inference.idle_model_unload_minutes
    );
    tokio::signal::ctrl_c().await?;
    manager.lock().await.shutdown().await?;
    Ok(())
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
    fn serve_start_requires_a_model() {
        assert!(Cli::try_parse_from(["decentraai", "serve", "start"]).is_err());
    }
}
