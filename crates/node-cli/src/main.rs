use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use decentraai_config::NodeConfig;
use std::fs;
use std::path::PathBuf;
use tracing::{info, Level};
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
    Doctor,
    Config { #[command(subcommand)] command: ConfigCommand },
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, default_value = "~/.decentraai")]
    data_dir: String,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Validate { #[arg(long, default_value = "configs/node.example.yaml")] file: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt().with_env_filter(EnvFilter::new(cli.log_level)).with_target(false).init();
    match cli.command {
        Command::Init(args) => init(args),
        Command::Doctor => doctor(),
        Command::Config { command: ConfigCommand::Validate { file } } => validate_config(file),
    }
}

fn init(args: InitArgs) -> Result<()> {
    let data_dir = expand_tilde(&args.data_dir);
    for directory in ["config", "identity", "cache/chunks", "cache/partial", "models", "quarantine", "db", "logs", "runtime"] {
        fs::create_dir_all(data_dir.join(directory)).with_context(|| format!("creating {}", directory))?;
    }
    info!(path = %data_dir.display(), "node data directories initialized");
    Ok(())
}

fn doctor() -> Result<()> {
    let logical_cpus = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1);
    info!(logical_cpus, os = std::env::consts::OS, arch = std::env::consts::ARCH, "basic system probe completed");
    println!("DecentraAI doctor\n  OS: {}\n  Architecture: {}\n  Logical CPUs: {}\n  GPU/VRAM probe: planned for M1\n  Network probe: planned for M1", std::env::consts::OS, std::env::consts::ARCH, logical_cpus);
    Ok(())
}

fn validate_config(file: PathBuf) -> Result<()> {
    let config = NodeConfig::load(&file).with_context(|| format!("validating {}", file.display()))?;
    info!(node = %config.node.name, "configuration validated");
    println!("Configuration is valid: {}", file.display());
    Ok(())
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" { return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())); }
    if let Some(rest) = value.strip_prefix("~/") { return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(rest); }
    PathBuf::from(value)
}
