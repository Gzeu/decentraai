use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use decentraai_config::NodeConfig;
use decentraai_identity::Identity;
use decentraai_registry::ModelRegistry;
use decentraai_runtime::tools::{
    HfSkillsManager, HfSkillsServer, OcrManager, OcrServer, SttManager, SttServer,
    TransformersManager, TransformersServer,
};
use decentraai_system_probe::{AdmissionDecision, GpuProbeStatus, SystemSnapshot, probe_gpu};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod upgrade;

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
    /// Inspect this node's logical agents (Collective Intelligence P1).
    /// Agents are logical execution contexts hosted by the node — not extra
    /// processes — and are advertised to the fabric with signed capability
    /// claims.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Fabric Intelligence: status, provider list and a live planning test
    /// against the configured local/external intelligence source.
    Intel {
        #[command(subcommand)]
        command: IntelCommand,
    },
    /// P13 — signed verified compute receipts: cryptographically sign a
    /// verified compute receipt with this node's identity, or verify a signed
    /// receipt independently. Read-only / non-mutating; the raw signing primitives
    /// are those any fabric node uses (same Ed25519 identity as libp2p peer id).
    Receipt {
        #[command(subcommand)]
        command: ReceiptCommand,
    },
    /// Run the full node as a background daemon — LAN/P2P discovery, model
    /// serving and the dashboard all at once, the way the desktop app / systemd
    /// service drives it. Detects and provisions a model automatically; the
    /// node is usable without any manual topology or port configuration.
    Node(NodeArgs),
    /// RAG: index documents into / query the local semantic retrieval index
    /// (needs an embeddings backend configured).
    Rag {
        #[command(subcommand)]
        command: RagCommand,
    },
    /// Collective memory: inspect the persistent memory scopes/entries.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
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
    /// Self-upgrade the node software from its git remote (origin/main).
    Upgrade(UpgradeArgs),
    /// DecentraAI Benchmark Lab: run single vs RAG vs collective tasks through
    /// a running node's `/v1/bench` API. `run` fires one task; `dataset`
    /// loads a decrypted benchmark JSONL (see scripts/bench-browsecomp-plus.py)
    /// and runs a batch, printing the honest comparison.
    Bench {
        #[command(subcommand)]
        command: BenchCommand,
    },
    /// P14 — Compute Contribution / Credits: inspect node-local verified
    /// contribution state, credit balances/events, and placement plans.
    Contribution {
        #[command(subcommand)]
        command: ContributionCommand,
    },
}
#[derive(Debug, Subcommand)]
enum UpgradeCommand {
    /// Read-only: fetch origin and report whether a newer main exists.
    Check,
    /// Pull main, rebuild, swap the binary and restart the service. Fails
    /// (with rollback) unless the working tree is clean.
    Apply,
    /// Loop forever: check every interval; apply when behind.
    Auto(AutoUpgradeArgs),
}
#[derive(Debug, Args)]
struct UpgradeArgs {
    #[command(subcommand)]
    command: UpgradeCommand,
    /// Repository root (defaults to the current directory).
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
}
#[derive(Debug, Args)]
struct AutoUpgradeArgs {
    /// Seconds between update checks.
    #[arg(long, default_value_t = 21_600)] // 6h
    interval_secs: u64,
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
    /// Self-upgrade watcher: check the git remote every interval and apply a
    /// newer main automatically (build + binary swap + service restart).
    #[arg(long)]
    auto_upgrade: bool,
    /// Seconds between auto-upgrade checks (default 6h).
    #[arg(long, default_value_t = 21_600)]
    auto_upgrade_interval_secs: u64,
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
/// Manage consumer API keys (`dca_…`) for the Compute Contribution &
/// Quota consumer path (Q2): an access credential + quota ceiling, never
/// an admin credential.
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

/// `decentraai agent` subcommands (Collective Intelligence P1).
#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// List the logical agents this node advertises (the defaults derived
    /// from its model + config). Read-only, never mutates state.
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show the full record of one local agent (by agent_id): role,
    /// description, state, semantic capabilities with provenance, allowed
    /// models, tools, policies and memory scopes. Read-only.
    Show {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The agent_id to inspect (e.g. `<short-id>:generalist`).
        #[arg(long)]
        agent: String,
    },
    /// Add a custom logical agent to this node and persist it (db/agents.json)
    /// so it survives restarts. The agent advertises itself on the fabric
    /// with its declared capabilities after the node restarts/reloads.
    Add {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Agent id suffix (e.g. `writer` → `<short-id>:writer`).
        #[arg(long)]
        id: String,
        /// Display name.
        #[arg(long, default_value = "")]
        name: String,
        /// Role label (e.g. `generalist`, `researcher`, `writer`).
        #[arg(long, default_value = "generalist")]
        role: String,
        /// Short description.
        #[arg(long, default_value = "")]
        description: String,
        /// Comma-separated capabilities (snake_case, e.g. `writing,summarization`).
        #[arg(long, default_value = "")]
        capabilities: String,
    },
    /// Remove a logical agent from this node and persist the change
    /// (db/agents.json). Cannot remove the last agent.
    Remove {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The full agent_id (e.g. `<short-id>:writer`).
        #[arg(long)]
        agent: String,
    },
    /// List the steps of a named workflow template (a local, pure fabric
    /// primitive — no network, no engine). Supports `research_report`.
    /// Read-only.
    Workflow {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The template id to inspect.
        #[arg(long, default_value = "research_report")]
        template: String,
    },
    /// Run a collective workflow on the local node via its API
    /// (POST /v1/agents/orchestrate). Useful for scripting.
    WorkflowRun {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The workflow prompt.
        #[arg(long)]
        prompt: String,
        /// The workflow template id.
        #[arg(long, default_value = "research_report")]
        template: String,
        /// Optional semantic retrieval query (RAG context per stage).
        #[arg(long)]
        retrieve: Option<String>,
    },
    /// Show a reputation profile for one local agent, built from synthetic
    /// sample data (this demonstrates the P6 reputation model locally — the
    /// numbers are NOT real measurements). Read-only.
    Reputation {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The agent_id the profile belongs to.
        #[arg(long)]
        agent: String,
        /// Minimum sample count before a factor counts as meaningful.
        #[arg(long, default_value = "1")]
        min_samples: u64,
    },
    /// Inspect the talent tree (P8 capability graph): list every capability,
    /// which ones are unlockable given what you have and a memory budget, and
    /// the cheapest path to a target. Read-only, local.
    TalentTree {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Comma-separated capability names you already hold (snake_case).
        #[arg(long)]
        have: String,
        /// Memory budget (MiB) for the unlockable-capability filter.
        #[arg(long, default_value = "2048")]
        budget_mb: u64,
        /// A target capability to resolve the cheapest unlock path to.
        #[arg(long)]
        target: Option<String>,
    },
    /// Manage the dataset/skill layer (P8): show the demonstration, list the
    /// persistent registry, or register a real dataset + skill that drives the
    /// agent's capabilities.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

/// P13 — signed compute receipt subcommands.
#[derive(Debug, Subcommand)]
enum ReceiptCommand {
    /// Build + cryptographically sign a peeked VerifiedComputeReceipt with this
    /// node's Ed25519 identity. Prints the canonical receipt bytes, the signer
    /// public key and the signature as JSON — the artifact a peer can verify.
    /// Non-mutating; never touches the ledger.
    Sign {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Execution id (idempotency key).
        #[arg(long)]
        execution_id: String,
        /// Worker node id (peer id / node id) that executed.
        #[arg(long)]
        worker_node: String,
        /// Capability exercised (e.g. `inference`).
        #[arg(long, default_value = "inference")]
        capability: String,
        /// Duration in ms.
        #[arg(long, default_value = "0")]
        duration_ms: u64,
        /// BLAKE3 output hash (hex) that this receipt attests.
        #[arg(long)]
        output_hash: Option<String>,
    },
    /// Verify a signed receipt independently: loads the SignedComputeReceipt
    /// JSON, checks the Ed25519 signature against the embedded public key and
    /// the canonical bytes. Exit 0 = valid, non-zero + message = invalid (tampered
    /// / wrong key / missing fields). Non-mutating.
    Verify {
        /// Path to a signed-receipt JSON file produced by `receipt sign`.
        #[arg(long)]
        file: PathBuf,
    },
}

/// P13 — signed receipt marshalled across nodes.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CliSignedReceipt {
    version: u16,
    receipt_bytes: Vec<u8>,
    signer_public_key: Option<Vec<u8>>,
    signature: Option<Vec<u8>>,
}

/// `decentraai agent skill` subcommands.
#[derive(Debug, Subcommand)]
enum SkillCommand {
    /// Show the P8 demonstration (Qwen-Coder + code-finetune + code-agent →
    /// tool calling). Read-only, from the real demo registry.
    Demo {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// The served model file name to demonstrate against (defaults to the
        /// node's current model).
        #[arg(long)]
        model: Option<String>,
    },
    /// List the persistent dataset/skill registry (db/skills.json).
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Register a real dataset + a skill that unlocks capabilities (evidence).
    /// The dataset's provenance is a claim; it only feeds the agent when the
    /// node restarts and applies the persistent registry.
    Add {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Dataset id (e.g. "code_finetune_2024").
        #[arg(long)]
        dataset_id: String,
        /// Dataset name.
        #[arg(long)]
        name: String,
        /// Source reference (e.g. hf:org/repo@rev).
        #[arg(long)]
        source: String,
        /// Dataset kind (training|fine_tune|knowledge_base|benchmarks).
        #[arg(long, default_value = "fine_tune")]
        kind: String,
        /// Capabilities the dataset develops (comma-separated snake_case).
        #[arg(long)]
        develops: String,
        /// Provenance of the claims (verified|inferred).
        #[arg(long, default_value = "inferred")]
        provenance: String,
        /// Skill id.
        #[arg(long)]
        skill_id: String,
        /// Model base capability the skill requires (snake_case).
        #[arg(long)]
        requires_model: String,
        /// Capabilities the skill unlocks (comma-separated; must be ⊆ dataset
        /// develops).
        #[arg(long)]
        unlock: String,
    },
}

/// `decentraai rag` subcommands — index documents and query the semantic
/// retrieval index backed by the node's embeddings store.
#[derive(Debug, Subcommand)]
enum RagCommand {
    /// Index a document into the semantic retrieval index.
    /// The text is embedded and stored under `doc_id`.
    Index {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Unique document identifier.
        #[arg(long)]
        doc_id: String,
        /// Document text to embed and index.
        #[arg(long)]
        text: String,
        /// Optional capability tag for the document (filters lookup later).
        #[arg(long)]
        capability: Option<String>,
    },
    /// Query the semantic retrieval index. Returns the top-k most relevant
    /// chunks for the given query text.
    Query {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        /// Free-text query against the index.
        #[arg(long)]
        text: String,
        /// Number of results to return (default 5).
        #[arg(long, default_value = "5")]
        k: usize,
    },
}

/// `decentraai memory` subcommands — inspect collective memory entries and
/// scopes managed by the agent orchestrator.
#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// List all persistent memory entries with their scope, level and access
    /// policy. Each entry shows the memory key, scope (agent|team|global),
    /// confidence and timestamp.
    List {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BenchCommand {
    /// Run one benchmark task through the node's live executor and print the
    /// graded verdict. `--gold` is optional: ungradable tasks are Abstained.
    Run {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        gold: Option<String>,
        #[arg(long, default_value = "single")]
        mode: String,
        #[arg(long)]
        evidence: Option<String>,
    },
    /// Load a decrypted benchmark JSONL (BrowseComp-Plus format) and run up
    /// to `--limit` tasks through the node's `/v1/bench` API in the given
    /// mode. With `--mode both` (default) each task runs in single AND
    /// collective — the only honest comparison (paired over shared tasks);
    /// the headline verdict needs MIN_SAMPLES shared tasks + MIN_MARGIN.
    Dataset {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "5")]
        limit: usize,
        #[arg(long, default_value = "both")]
        mode: String,
        #[arg(long, default_value = "3")]
        agents: usize,
    },
    /// CPU pool evaluation: load the Model Intelligence corpus (24 tasks) and
    /// partition it across the requesting node's CPU + connected worker peers
    /// through `/v1/pool/bench`, then print the aggregated accuracy, wall
    /// times and speedup vs the serial single-node baseline. Use
    /// `--max-workers 1` to measure the serial baseline itself.
    Pool {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        #[arg(long, default_value = "3")]
        max_workers: usize,
        #[arg(long, default_value = "chat")]
        capability: String,
        #[arg(long, default_value = "Qwen3-1.7B-Q4_K_M.gguf")]
        model: String,
        #[arg(long, default_value = "90")]
        lease_seconds: u64,
        #[arg(long, default_value = "64")]
        max_tokens: u64,
        #[arg(long)]
        tasks_limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
enum ContributionCommand {
    /// Show the node-local contribution state: verified/failed executions,
    /// credits earned/consumed, and breakdowns by resource/model/worker.
    State {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show credit balances from the receipt-backed credit ledger.
    Credits {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show recent credit events (provenance).
    Events {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show verified compute history (recent executions).
    History {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Explain a placement plan for a model (read-only).
    Plan {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "0")]
        min_vram_mb: u64,
        #[arg(long, default_value = "0")]
        min_ram_mb: u64,
        #[arg(long, default_value = "1")]
        min_gpu_count: u32,
        #[arg(long, default_value = "4096")]
        context_tokens: u32,
        #[arg(long)]
        distributed: bool,
    },
    /// Show the live fabric graphs (capability, compute, network) as a read-only
    /// projection.
    Graph {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
    },
    /// Show the evidence chain for one execution (P14 Phase P): execution
    /// record → credit event → worker balance, each hop id-linked.
    EvidenceChain {
        #[arg(long, default_value = "configs/node.example.yaml")]
        config: PathBuf,
        #[arg(long)]
        execution_id: String,
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
            command:
                ModelCommand::Search {
                    query,
                    category,
                    categories,
                    limit,
                },
        } => model_search(query, category, categories, limit).await,
        Command::Model {
            command:
                ModelCommand::Pull {
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
        Command::Agent { command } => agent_command(command).await,
        Command::Intel { command } => intel_command(command).await,
        Command::Receipt { command } => Ok(receipt_command(command)?),
        Command::Rag { command } => rag_command(command).await,
        Command::Memory { command } => memory_command(command).await,
        Command::Node(args) => node_start(args).await,
        Command::Open(args) => open_dashboard(args),
        Command::Invite(args) => invite(args),
        Command::Join(args) => join(args).await,
        Command::Upgrade(args) => upgrade_command(args).await,
        Command::Bench { command } => bench_command(command).await,
        Command::Contribution { command } => contribution_command(command).await,
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

/// Resolves which model file to serve: an explicit `node.model` name wins
/// (and must exist on disk — a wrong name is a hard error, not a silent
/// fallback to auto-detect, so the operator notices the typo); otherwise the
/// first `.gguf` in the models dir is auto-detected.
fn resolve_model_name(models_dir: &std::path::Path, explicit: Option<&str>) -> Result<String> {
    match explicit {
        Some(name) => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Ok(String::new());
            }
            let path = models_dir.join(trimmed);
            if !path.is_file() {
                anyhow::bail!(
                    "node.model '{}' not found in {} — check the model file name",
                    trimmed,
                    models_dir.display()
                );
            }
            Ok(trimmed.to_string())
        }
        None => auto_detect_model(models_dir),
    }
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
///
/// Spawns the opt-in Tool Runtime subprocesses (OCR/STT/HF skills). Returns
/// the managers (empty = disabled/missing setup) so callers can both attach
/// them to the API state and build real tool bindings for the agent executor.
/// Missing venv/model files never fail startup — the node serves without the
/// tool and logs a warning.
async fn spawn_tool_runtimes(
    config: &decentraai_config::NodeConfig,
    data_dir: &std::path::Path,
) -> (
    Option<OcrManager>,
    Option<SttManager>,
    Option<HfSkillsManager>,
    Option<TransformersManager>,
) {
    let mut ocr = None;
    if let Some(cfg) = config.ocr.clone() {
        if cfg.enabled {
            match OcrServer::spawn(data_dir).await {
                Ok(server) => {
                    info!("OCR online (RapidOCR subprocess)");
                    ocr = Some(OcrManager::new(Some(server)));
                }
                Err(e) => warn!(error = %e, "OCR unavailable (run scripts/setup-ocr.sh)"),
            }
        }
    }
    let mut stt = None;
    if let Some(cfg) = config.stt.clone() {
        if cfg.enabled {
            match SttServer::spawn(data_dir, &cfg.model).await {
                Ok(server) => {
                    info!(model = %cfg.model, "STT online (faster-whisper subprocess)");
                    stt = Some(SttManager::new(Some(server), cfg.model.clone()));
                }
                Err(e) => warn!(error = %e, "STT unavailable (run scripts/setup-stt.sh)"),
            }
        }
    }
    let mut skills = None;
    if let Some(cfg) = config.skills.clone() {
        if cfg.enabled {
            match HfSkillsServer::spawn(data_dir, &cfg.list).await {
                Ok(server) => {
                    info!(skills = ?cfg.list, "HF skills online (transformers subprocess)");
                    skills = Some(HfSkillsManager::new(Some(server)));
                }
                Err(e) => warn!(error = %e, "HF skills unavailable (run scripts/setup-skills.sh)"),
            }
        }
    }
    // Transformers for embeddings: when `transformers.enabled = true` and the
    // main engine is NOT Transformers (that case is handled earlier), spawn
    // the Python server for embeddings only. The backend URL is returned so
    // the caller can wire `embeddings_backend_url` if it wasn't set.
    let mut tx = None;
    let engine_is_transformers = config
        .inference
        .engine
        .as_deref()
        .map(decentraai_inference_adapter::EngineKind::parse)
        .map(|e| e == decentraai_inference_adapter::EngineKind::Transformers)
        .unwrap_or(false);
    if !engine_is_transformers {
        if let Some(tx_cfg) = config.transformers.as_ref() {
            if tx_cfg.enabled {
                match TransformersServer::spawn(data_dir, &tx_cfg.model, &tx_cfg.device).await {
                    Ok(server) => {
                        info!(
                            model = %tx_cfg.model,
                            device = %tx_cfg.device,
                            base_url = %server.base_url(),
                            "Transformers embeddings backend online (Python subprocess)"
                        );
                        tx = Some(TransformersManager::new(Some(server)));
                    }
                    Err(e) => {
                        warn!(error = %e, "Transformers embeddings unavailable (run scripts/setup-transformers.sh)")
                    }
                }
            }
        }
    }
    (ocr, stt, skills, tx)
}

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
        LlamaServer, RuntimeConfig, TtsManager, TtsServer, ensure_admitted, find_llama_server,
    };
    use libp2p::PeerId as Libp2pPeerId;
    use libp2p::identity::Keypair as Libp2pKeypair;
    use std::sync::Arc;
    use std::time::Duration;

    let config_path = expand_tilde(&args.config.to_string_lossy());

    // 0. Self-upgrade watcher (before identity/config are loaded, so an
    //    upgrade that restarts the service does not race the node's own
    //    startup). Runs in the background; logs, never blocks the node.
    if args.auto_upgrade {
        let repo = PathBuf::from(".");
        let interval = args.auto_upgrade_interval_secs;
        tokio::spawn(async move {
            loop {
                match upgrade::check_for_update(&repo) {
                    upgrade::UpdateStatus::Behind { behind, .. } if behind > 0 => {
                        info!(behind, "auto-upgrade: update available, applying");
                        match upgrade::apply_update(&repo) {
                            Ok(report) => info!(
                                from = %report.from,
                                to = %report.to,
                                "auto-upgrade: upgraded — node service restarted"
                            ),
                            Err(e) => {
                                warn!(error = %e, "auto-upgrade: apply failed, retrying next interval")
                            }
                        }
                    }
                    upgrade::UpdateStatus::UpToDate => {}
                    upgrade::UpdateStatus::NoRepo => {
                        warn!(
                            "auto-upgrade: not a git checkout ({}); disabling watcher",
                            repo.display()
                        );
                        break;
                    }
                    upgrade::UpdateStatus::Error(e) => {
                        warn!(error = %e, "auto-upgrade: check failed, retrying next interval");
                    }
                    upgrade::UpdateStatus::Behind { .. } => {}
                }
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
        });
    }

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
    // An explicit `node.model` wins over auto-detection (and errors on a
    // typo); otherwise the first .gguf in the models dir is chosen.
    let model_name =
        resolve_model_name(&models_dir, config.node.model.as_deref()).unwrap_or_default();
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
    let mut transformers_manager: Option<TransformersManager> = None;
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
                    model_hash = blake3::hash(format!("{engine:?}:{model_name}").as_bytes())
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

    // Transformers backend: when `inference.engine` is "transformers" (or
    // aliases "hf"/"huggingface"), spawn the Python inference server and
    // register it as the distributed worker backend. This is the same
    // pattern as the multi-engine remote backend, but the backend URL
    // comes from a locally-spawned Python subprocess.
    if worker_backend.is_none() {
        let engine = config
            .inference
            .engine
            .as_deref()
            .map(EngineKind::parse)
            .unwrap_or(EngineKind::LlamaServer);
        if engine == EngineKind::Transformers {
            if let Some(tx_cfg) = &config.transformers {
                if tx_cfg.enabled {
                    match TransformersServer::spawn(&data_dir, &tx_cfg.model, &tx_cfg.device).await
                    {
                        Ok(server) => {
                            let base = server.base_url();
                            let tx_model = tx_cfg.model.clone();
                            info!(
                                model = %tx_model,
                                device = %tx_cfg.device,
                                base_url = %base,
                                "Transformers inference backend online (Python subprocess)"
                            );
                            transformers_manager = Some(TransformersManager::new(Some(server)));
                            if let Ok(backend) = OpenAiCompatibleBackend::new(BackendConfig {
                                base_url: base.clone(),
                                model: tx_model.clone(),
                                api_key: None,
                                connect_timeout: Duration::from_secs(3),
                                request_timeout: Duration::from_secs(300),
                                max_prompt_bytes: 200_000,
                                max_output_tokens: 8192,
                                engine: EngineKind::Transformers,
                                backend_url_resolver: None,
                            }) {
                                if model_hash.is_empty() {
                                    model_hash =
                                        blake3::hash(format!("transformers:{tx_model}").as_bytes())
                                            .to_hex()
                                            .to_string();
                                }
                                if model_size_bytes == 0 {
                                    model_size_bytes = 1024;
                                }
                                backend_url = base.clone();
                                *live_engine_url.lock().unwrap() = Some(base.clone());
                                worker_backend = Some(backend);
                                info!(
                                    engine = "transformers",
                                    base_url = %base,
                                    model = %tx_model,
                                    hash = %model_hash,
                                    "registered local Transformers backend as a distributed worker"
                                );
                            }
                        }
                        Err(e) => warn!(
                            error = %e,
                            "Transformers backend unavailable (run scripts/setup-transformers.sh)"
                        ),
                    }
                }
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
        InferenceConfig::from_section(&config.inference),
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

    // ---- Quota ledger persistence (db/quota_ledger.json) ----
    // The ledger is authoritative for consumer credit + worker earnings; a
    // purely in-memory copy meant every node restart silently zeroed
    // operator-granted balances. Restore any previous snapshot, then keep an
    // atomic snapshot on disk shortly after every mutation.
    let quota_snapshot_path = data_dir.join("db/quota_ledger.json");
    {
        let ledger = compute_manager.quota_ledger();
        if let Some(saved) = decentraai_compute::QuotaLedger::load_snapshot(&quota_snapshot_path) {
            let mut guard = ledger.lock().expect("quota ledger mutex poisoned");
            guard.restore(saved);
            tracing::info!(
                "restored quota ledger from {}",
                quota_snapshot_path.display()
            );
        }
    }
    {
        let ledger = compute_manager.quota_ledger();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                tick.tick().await;
                // Brief sync lock, no await while held (Q2 rule).
                if let Ok(l) = ledger.lock() {
                    if l.take_dirty() {
                        if let Err(e) = l.save_atomic(&quota_snapshot_path) {
                            tracing::warn!(error = %e, "failed to persist quota ledger");
                        }
                    }
                }
            }
        });
    }

    // ---- Collective Intelligence P1: this node's logical agents ----
    // Agents are logical execution contexts hosted by the node (NOT extra
    // processes): identity + capabilities + policies, advertised to the
    // fabric with signed capability claims. The default agent set mirrors
    // what the node can honestly claim — see `default_local_agents`.
    let mut agent_manager = Arc::new(decentraai_distributed::agents::AgentManager::new(
        local_peer_id,
        node_name.clone(),
    ));
    // P1 signing: peers reject forged agent advertisements (anti-spoof).
    if let Some(am) = Arc::get_mut(&mut agent_manager) {
        am.set_signing_key(identity.signing_key_bytes());
    }
    let short_id = decentraai_distributed::short_node_id(&local_peer_id);
    // Persistent dataset/skill registry (db/skills.json) — real skills that
    // drive the agent's capabilities. Loaded once; empty until the operator
    // registers datasets/skills via `decentraai agent skill`.
    let skills_registry = load_skill_registry(&data_dir.join("db/skills.json"));
    // Persistent agent records (db/agents.json): operator-editable, survives
    // restarts. Missing/corrupt file falls back to the deterministic default
    // set, which is then persisted so the file appears and stays stable.
    let agents_path = data_dir.join("db/agents.json");
    let local_agents: Vec<decentraai_agents::AgentRecord> = match load_agent_records(&agents_path) {
        Some(records) if !records.is_empty() => {
            info!(count = records.len(), "loaded persistent agent records");
            records
        }
        _ => {
            let defaults = default_local_agents(
                &short_id,
                &node_name,
                &model_hash,
                config.inference.allow_remote_inference,
                &model_name,
                &skills_registry,
                config.inference.embeddings_backend_url.is_some(),
            );
            if let Err(e) = save_agent_records(&agents_path, &defaults) {
                tracing::warn!(error = %e, "failed to persist default agent records");
            }
            defaults
        }
    };
    agent_manager.set_local_agents(local_agents);
    info!(
        agents = agent_manager.local_count(),
        "node advertises logical agents",
    );

    let tracker = Arc::new(decentraai_distributed::RequestTracker::new());

    let mut distributed_handler =
        decentraai_distributed::DistributedP2PHandler::with_worker_manager(worker_manager.clone());
    distributed_handler.set_tracker(tracker.clone());
    distributed_handler.set_compute_manager(compute_manager.clone());
    distributed_handler.set_agent_manager(agent_manager.clone());
    // Agent messenger: routes inbound agent messages to the right recipient's
    // inbox. Starts WITHOUT a transport (a placeholder P2PNode would spawn a
    // live swarm whose handler-less node answers none of the peers' probes);
    // re-pointed at the real P2P node below (circular wiring).
    let agent_messenger =
        Arc::new(decentraai_distributed::agent_messenger::AgentMessenger::uninitialized());
    distributed_handler.set_messenger(agent_messenger.clone());
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

    let p2p_node = P2PNode::new_with_network(
        &identity,
        config.network.max_message_bytes as usize,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(chained_handler)),
        decentraai_p2p::NetworkConfig {
            lan_discovery: config.network.lan_discovery,
            dht_enabled: config.network.dht_enabled,
            relay_enabled: config.network.relay_enabled,
            bootstrap_peers: config.network.bootstrap_peers.clone(),
            max_connections: config.network.max_connections,
        },
    )?;
    let bound = p2p_node.listen("/ip4/0.0.0.0/tcp/32937").await?;
    // Re-point the agent messenger at the real, handler-bearing P2P node so
    // both outbound Delegates and inbound Replies ride the connected peer.
    agent_messenger.set_transport(p2p_node.clone());

    let mut distributed = DistributedInference::new(
        p2p_node,
        InferenceConfig::from_section(&config.inference),
        Some(worker_manager.clone()),
        Some(tracker.clone()),
    )?;
    distributed.set_compute_manager(compute_manager.clone());
    // M10: per-request routing audit events (request/worker/model hash/status).
    distributed.set_logs_dir(Some(data_dir.join("logs")));
    // P1: sign outbound routed requests with the node identity so workers can
    // authenticate them and reject spoofed/unsigned traffic.
    distributed.set_signing_identity(identity.signing_key_bytes());

    // ---- Collective Intelligence: make this node a live agent host ----
    // A production AgentRuntime per local agent answers delegated LLM tasks
    // through the fabric (inference executor); a SQLite MemoryStore persists
    // collective memory. Both are best-effort and never disturb the flow.
    // RAG embeddings/retrieval are created once and shared by the inference
    // executor (retrieval tool at runtime) and the API (/v1/embeddings, /v1/rag).
    let (mut embedding_client, mut retrieval_manager): (
        Option<Arc<decentraai_distributed::embedding::EmbeddingClient>>,
        Option<Arc<decentraai_distributed::retrieval_manager::RetrievalManager>>,
    ) = match config.inference.embeddings_backend_url.as_deref() {
        Some(url) if !url.is_empty() => {
            let client = Arc::new(decentraai_distributed::embedding::EmbeddingClient::new(
                url.to_string(),
            ));
            let rm = Arc::new(
                decentraai_distributed::retrieval_manager::RetrievalManager::new(client.clone()),
            );
            (Some(client), Some(rm))
        }
        _ => (None, None),
    };
    // Evidence RAG (experimental memory) is created once here because the
    // Benchmark Lab shares it: benchmark runs feed `EvidenceFamily::Benchmark`
    // entries so the fabric's lessons include lab results. The index itself
    // syncs lazily from the live sources at request time.
    let evidence_rag = Arc::new(
        decentraai_distributed::evidence_manager::EvidenceManager::new(embedding_client.clone()),
    );
    // Tool Runtime managers are created at most once and shared by the agent
    // executors (as real tool bindings) and the API (as /v1/<tool> proxies).
    let mut ocr_manager: Option<OcrManager> = None;
    let mut stt_manager: Option<SttManager> = None;
    let mut skills_manager: Option<HfSkillsManager> = None;
    // DecentraAI Benchmark Lab: populated when an inference executor exists
    // (worker with a servable model). The lab runs single/RAG/collective
    // tasks through the real executor and feeds evidence.
    let mut benchmark_manager: Option<
        Arc<decentraai_distributed::benchmark_manager::BenchmarkManager>,
    > = None;
    if is_worker && !model_hash.is_empty() {
        // Tool Runtime: spawn OCR/STT/HF-skills subprocesses BEFORE the agent
        // executors so the real tool bindings (name + description + loopback
        // URL) can be attached to the executor. Missing setups fail graceful —
        // the node runs without the tool and logs a warning.
        let (ocr_new, stt_new, skills_new, tx_new) = spawn_tool_runtimes(&config, &data_dir).await;
        ocr_manager = ocr_new;
        stt_manager = stt_new;
        skills_manager = skills_new;
        // Don't overwrite transformers_manager if the early path (engine=transformers) already set it.
        if tx_new.is_some() && transformers_manager.is_none() {
            transformers_manager = tx_new;
        }
        // If Transformers spawned for embeddings but no embeddings_backend_url was
        // configured, wire the auto-started server as the embeddings backend.
        if embedding_client.is_none() {
            if let Some(tx) = &transformers_manager {
                if let Some(url) = tx.base_url() {
                    let client = Arc::new(decentraai_distributed::embedding::EmbeddingClient::new(
                        url.clone(),
                    ));
                    let rm = Arc::new(
                        decentraai_distributed::retrieval_manager::RetrievalManager::new(
                            client.clone(),
                        ),
                    );
                    embedding_client = Some(client);
                    retrieval_manager = Some(rm);
                    info!(url = %url, "embeddings wired to auto-started Transformers server");
                }
            }
        }
        // Real tool bindings for the agent executor — only for tools that are
        // actually online (spawn succeeded). The model is told about them and
        // may emit a [TOOL_CALL] block; the executor runs the tool and re-asks.
        let mut tool_bindings: Vec<decentraai_distributed::tool_calling::ToolBinding> = Vec::new();
        if let Some(m) = &ocr_manager {
            if let Some(base) = m.base_url() {
                tool_bindings.push(decentraai_distributed::tool_calling::ToolBinding::new(
                    "ocr",
                    "extracts text from an image (input: image_b64, the base64 of an image)",
                    format!("{base}/v1/ocr"),
                ));
            }
        }
        if let Some(m) = &stt_manager {
            if let Some(base) = m.base_url() {
                tool_bindings.push(decentraai_distributed::tool_calling::ToolBinding::new(
                    "stt",
                    "transcribes speech to text (input: audio_b64, the base64 of a WAV/MP3/OGG)",
                    format!("{base}/v1/stt"),
                ));
            }
        }
        if let Some(m) = &skills_manager {
            if let Some(base) = m.base_url() {
                for skill in m.skills() {
                    let desc = match skill.as_str() {
                        "sentiment" => "classifies text sentiment (input: text)",
                        "ner" => "extracts named entities (input: text)",
                        "summarize" => "summarizes text (input: text)",
                        "translate_ro_en" => "translates Romanian to English (input: text)",
                        "translate_en_ro" => "translates English to Romanian (input: text)",
                        other => return Err(anyhow::anyhow!("unknown skill '{other}'")),
                    };
                    tool_bindings.push(decentraai_distributed::tool_calling::ToolBinding::new(
                        skill.clone(),
                        desc,
                        format!("{base}/v1/skills/{skill}"),
                    ));
                }
            }
        }
        let mut inference_executor =
            decentraai_distributed::agent_runtime::InferenceAgentExecutor::new(
                Arc::new(distributed.clone()),
                model_hash.clone(),
            );
        // Single-node path: execute delegated tasks directly on this node's
        // live local llama-server backend over HTTP (distributed
        // route_request cannot self-route over libp2p). The live URL cache is
        // re-read per call, so an engine respawn (new port) is always hit.
        if !backend_url.is_empty() {
            inference_executor.with_live_backend(live_engine_url.clone());
        }
        // RAG retrieval tool at runtime: a delegated task with a `retrieve`
        // input gets its prompt augmented with semantic search results.
        if let Some(rm) = &retrieval_manager {
            inference_executor.with_retrieval(rm.clone());
        }
        // Real tool calling: attach the spawned OCR/STT/HF-skills bindings.
        inference_executor.with_tools(tool_bindings);
        // One runtime per local logical agent (the orchestrator selects these
        // as executors for delegated stages).
        let local_agents = agent_manager.local_agents();
        for agent in &local_agents {
            let mut agent_runtime = decentraai_distributed::agent_runtime::AgentRuntime::new(
                agent.agent_id.clone(),
                agent_messenger.clone(),
            );
            let ex = inference_executor.clone();
            agent_runtime.with_executor(move |task, inputs| {
                let ex = ex.clone();
                async move { ex.execute(&task, &inputs).await }
            });
            // P7 policy gate wiring (review): the agent's declared policy is
            // enforced before any delegated task executes — model allowlist
            // (check_model) + working state. An agent with an empty allowlist
            // may use any node-served model; a denied task is answered with a
            // policy error, never executed.
            let policy_record = agent.clone();
            agent_runtime.with_policy_gate(move |task| {
                let policy = decentraai_agents::policy_engine();
                if let Some(wl) = &task.required_workload {
                    match policy.check_model(&policy_record, &wl.model_hash) {
                        decentraai_agents::PolicyDecision::Allow => {}
                        decentraai_agents::PolicyDecision::Deny { reason } => {
                            return Err(reason);
                        }
                    }
                }
                Ok(())
            });
            tokio::spawn(async move { agent_runtime.run_forever().await });
        }
        info!(
            agents = local_agents.len(),
            "node is a live agent host (inference executor wired)"
        );

        // ═══ Sprint 0.1: LocalAgentRuntime lifecycle integration ═══
        // Wire the tested foundation (spawn → observe → decide → act → learn)
        // into the live daemon. Each AgentRecord becomes a managed agent in the
        // LocalAgentRuntime. A periodic task drives the lifecycle loop.
        {
            use decentraai_agent_runtime::local::{
                LocalAgentRuntime, ObservationBuilder, StaticObservationBuilder,
            };
            use decentraai_agent_runtime::saes::persistence::{
                SqliteBehaviorStore, SqliteGoalStore,
            };
            use decentraai_agent_runtime::{AgentConfig, AgentRuntime as _, ResourceLimits};

            let event_store = Arc::new(decentraai_event_bus::InMemoryEventStore::new(10000));
            let event_bus = Arc::new(decentraai_event_bus::EventBus::new(event_store));

            // Build the observation from the node's available capabilities.
            let all_caps: Vec<String> = local_agents
                .iter()
                .flat_map(|a| {
                    a.semantic_capabilities
                        .iter()
                        .map(|c| format!("{:?}", c.capability))
                })
                .collect();
            let obs_builder: Arc<dyn ObservationBuilder> = Arc::new(StaticObservationBuilder {
                hub_state: serde_json::json!({
                    "available_capabilities": all_caps,
                    "node_id": short_id,
                }),
                society_state: serde_json::json!({}),
                arena_state: None,
                personal_memory: serde_json::json!({}),
            });

            // SAES 0.3: open file-backed SQLite stores for persistence.
            // Agent goals, progress, and behavior profiles survive restarts.
            let goals_db_path = data_dir.join("db/agent_goals.sqlite");
            let behavior_db_path = data_dir.join("db/agent_behavior.sqlite");
            let goal_store = match SqliteGoalStore::open(&goals_db_path) {
                Ok(s) => {
                    info!(path = %goals_db_path.display(), "saes: goal store opened");
                    Arc::new(s)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %goals_db_path.display(),
                        "saes: failed to open goal store, falling back to in-memory"
                    );
                    Arc::new(
                        SqliteGoalStore::new_in_memory()
                            .expect("in-memory goal store must not fail"),
                    )
                }
            };
            let behavior_store = match SqliteBehaviorStore::open(&behavior_db_path) {
                Ok(s) => {
                    info!(path = %behavior_db_path.display(), "saes: behavior store opened");
                    Arc::new(s)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %behavior_db_path.display(),
                        "saes: failed to open behavior store, falling back to in-memory"
                    );
                    Arc::new(
                        SqliteBehaviorStore::new_in_memory()
                            .expect("in-memory behavior store must not fail"),
                    )
                }
            };

            let local_runtime = LocalAgentRuntime::new(event_bus.clone(), obs_builder)
                .with_goal_store(goal_store)
                .with_behavior_store(behavior_store);

            for record in &local_agents {
                let caps: Vec<String> = record
                    .semantic_capabilities
                    .iter()
                    .map(|c| format!("{:?}", c.capability))
                    .collect();
                let goals: Vec<String> = if caps.is_empty() {
                    vec!["serve requests".to_string()]
                } else {
                    caps.iter()
                        .map(|c| format!("fulfill {c} requests"))
                        .collect()
                };
                let agent_config = AgentConfig {
                    agent_id: record.agent_id.clone(),
                    name: record.name.clone(),
                    capabilities: caps,
                    initial_goals: goals,
                    initial_memory: None,
                    policy_overrides: None,
                    resource_limits: ResourceLimits::default(),
                };
                match local_runtime.spawn(agent_config).await {
                    Ok(_handle) => {
                        info!(agent = %record.agent_id, "sprint-0.1 agent spawned");
                    }
                    Err(e) => {
                        tracing::warn!(
                            agent = %record.agent_id,
                            error = %e,
                            "sprint-0.1: failed to spawn agent"
                        );
                    }
                }
            }

            // Periodic lifecycle loop: observe → decide → act → learn for each
            // agent. Runs every 30 seconds. Builds observations from the node's
            // available capabilities; the decision policy selects an action.
            let runtime_for_loop = Arc::new(tokio::sync::RwLock::new(local_runtime));
            let agents_for_loop: Vec<String> =
                local_agents.iter().map(|a| a.agent_id.clone()).collect();
            let agent_count = agents_for_loop.len();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    let rt = runtime_for_loop.read().await;
                    for agent_id in &agents_for_loop {
                        // Observe
                        let observation = match rt.observe(agent_id).await {
                            Ok(obs) => obs,
                            Err(e) => {
                                tracing::debug!(
                                    agent = agent_id,
                                    error = %e,
                                    "sprint-0.1: observe failed"
                                );
                                continue;
                            }
                        };
                        // Decide
                        let decision = match rt.decide(agent_id, &observation).await {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::debug!(
                                    agent = agent_id,
                                    error = %e,
                                    "sprint-0.1: decide failed"
                                );
                                continue;
                            }
                        };
                        // Act
                        let action = match rt.act(agent_id, &decision).await {
                            Ok(a) => a,
                            Err(e) => {
                                tracing::debug!(
                                    agent = agent_id,
                                    error = %e,
                                    "sprint-0.1: act failed"
                                );
                                continue;
                            }
                        };
                        // Learn (best-effort)
                        let _ = rt
                            .learn(
                                agent_id,
                                &action,
                                &action.result.clone().unwrap_or(
                                    decentraai_agent_runtime::ActionResult {
                                        success: true,
                                        output: None,
                                        error: None,
                                        evidence_id: None,
                                        reward: None,
                                        reputation_delta: None,
                                    },
                                ),
                            )
                            .await;
                        tracing::debug!(
                            agent = agent_id,
                            decision = ?decision.decision_type,
                            "sprint-0.1: lifecycle cycle complete"
                        );
                    }
                }
            });
            info!(
                agents = agent_count,
                "sprint-0.1: agent lifecycle loop started (30s interval)"
            );
        }
        // ═══ End Sprint 0.1 lifecycle integration ═══

        // Benchmark Lab: run single vs RAG vs collective tasks through the
        // live executor, graded deterministically, feeding evidence. The
        // executor is consumed here — the agent runtimes above each hold
        // their own clone.
        benchmark_manager = Some(Arc::new(
            decentraai_distributed::benchmark_manager::BenchmarkManager::new(
                Arc::new(
                    decentraai_distributed::benchmark_manager::InferenceBenchmarkExecutor::new(
                        Arc::new(inference_executor),
                    ),
                ),
                Some(evidence_rag.clone()),
            ),
        ));
    } else {
        info!("node agent host: no servable model — agent runtime idle");
    }
    // The coordinator-side orchestrator runs collective workflows by delegating
    // stages to the local/remote agents. Shared with the API so a user can
    // trigger a workflow from the dashboard/CLI.
    // Persistent collective memory (best-effort; the node keeps working if it
    // cannot open or create the store). Held and shared with the API so the
    // dashboard can show it and workflows can write verified results to it.
    let agent_memory_path = data_dir.join("db/agent_memory.sqlite");
    let agent_memory_store: Option<Arc<decentraai_distributed::agent_memory::MemoryStore>> =
        match decentraai_distributed::agent_memory::MemoryStore::open(&agent_memory_path) {
            Ok(store) => {
                info!(path = %agent_memory_path.display(), "collective memory store ready");
                Some(Arc::new(store))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to open collective memory store");
                None
            }
        };
    let mut agent_orchestrator = decentraai_distributed::agent_orchestrator::AgentOrchestrator::new(
        agent_messenger.clone(),
        agent_manager.clone(),
        local_peer_id,
    );
    // Verified workflow outcomes are written into collective memory
    // (scope `workflow_results`) so completed work becomes reusable knowledge.
    if let Some(store) = &agent_memory_store {
        agent_orchestrator.with_memory_store(store.clone());
    }
    // Review wiring: executor selection now applies the UNIFIED matcher
    // (semantic + agent model allowlist + compute physical gate) for LOCAL
    // agents, where the node's real advertisement is available synchronously.
    // Remote agents keep the semantic match (their physical gate is UNKNOWN
    // without a sync advertisement handle — honest, never a fabricated block).
    {
        let gate_compute = compute_manager.clone();
        let gate_peer = local_peer_id;
        let gate: Arc<decentraai_distributed::agent_orchestrator::ExecutionGate> = Arc::new(
            move |view: &decentraai_distributed::agents::AgentView,
                  req: &decentraai_agents::AgentRequirement| {
                if view.remote {
                    // Physical gate unknown for remote agents: the semantic
                    // gate already passed; do not block on UNKNOWN.
                    return true;
                }
                let Some(adv) = gate_compute.last_local_advertisement_sync() else {
                    return true; // no advertisement yet — UNKNOWN, not a block
                };
                let matcher = decentraai_compute::CapabilityMatcher::default();
                // A fresh reservation ledger for the gate: the capability
                // check reads headroom; the real scheduler books reservations
                // at execution time.
                let ledger = decentraai_compute::ReservationLedger::new(
                    std::time::Duration::from_secs(60),
                    4,
                );
                matches!(
                    decentraai_agents::match_agent(
                        &view.record,
                        &adv,
                        req,
                        &matcher,
                        &ledger,
                        true,
                        Some(&gate_peer),
                    ),
                    decentraai_agents::AgentMatchOutcome::Eligible
                )
            },
        );
        agent_orchestrator.with_execution_gate(gate);
    }
    // Larger models (e.g. 7B on CPU) generate slower than the 60s default; a
    // per-stage reply can take minutes. Keep the timeout generous so a real
    // collective workflow completes instead of timing out mid-stage.
    agent_orchestrator.with_delegate_timeout(Duration::from_secs(600));
    let agent_orchestrator = Arc::new(agent_orchestrator);

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
            .advertise_local(
                snapshot,
                gpu,
                served_models,
                available_models,
                can_provision,
            )
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
    // P1: keep the fabric's agent view fresh (agent advertisements change
    // rarely, but the periodic refresh also expires stale remote views).
    spawn_agent_broadcaster(agent_manager.clone(), distributed.p2p_node().clone());

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
    // World v1 fix: serve API even if backend_url is still empty at this
    // point (engine may restart on next health probe); is_worker gate keeps
    // coordinator-only nodes from exposing the API unintentionally, and
    // api_auth_required remains enforced.
    if is_worker {
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
            dht_enabled: config.network.dht_enabled,
            relay_enabled: config.network.relay_enabled,
            lan_discovery: config.network.lan_discovery,
            bootstrap_peer_count: config.network.bootstrap_peers.len(),
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
        state.set_dashboard(config.node.dashboard);
        // Personal Memory store (per-agent Markdown workspaces)
        let pm_store = std::sync::Arc::new(
            decentraai_agent_personal_memory::PersonalMemoryStore::new(&data_dir),
        );
        state.attach_personal_memory(pm_store);
        // Fabric Intelligence: reasoning layer between a task and the
        // deterministic planner. Opt-in via config; disabled/absent = no-op.
        if let Some(intel_cfg) = config.fabric_intelligence.as_ref() {
            if intel_cfg.enabled {
                state.attach_intel(std::sync::Arc::new(
                    decentraai_fabric_intelligence::FabricIntelligence::from_config(intel_cfg),
                ));
            }
        }
        // Fabric Intelligence: reasoning layer between a task and the
        // deterministic planner. Opt-in via config; disabled/absent = no-op.
        if let Some(intel_cfg) = config.fabric_intelligence.as_ref() {
            if intel_cfg.enabled {
                state.attach_intel(std::sync::Arc::new(
                    decentraai_fabric_intelligence::FabricIntelligence::from_config(intel_cfg),
                ));
            }
        }

        // M18+: let the dashboard proxy route chat inference to trusted remote
        // workers that advertise the requested model (fabric chat routing).
        let dist_handle = std::sync::Arc::new(distributed.clone());
        state.attach_distributed(dist_handle.clone());
        // Personal Memory store (per-agent Markdown workspaces)
        let pm_store = std::sync::Arc::new(
            decentraai_agent_personal_memory::PersonalMemoryStore::new(&data_dir),
        );
        state.attach_personal_memory(pm_store);
        // M15 — Autonomous Compute Pressure: observe own signals; when the
        // pressure engine fires, request assist through the EXISTING DFCP
        // flow. The pressure layer only PROPOSES; the planner still routes.
        // Opt-in via config; disabled by default.
        if let Some(auto_cfg) = config.autonomous_assist.as_ref() {
            if auto_cfg.enabled && auto_cfg.profile.is_some() {
                let auto = std::sync::Arc::new(auto_cfg.clone());
                let api_port = config.inference.api_port;
                let data_dir = expand_tilde(&config.node.data_dir);
                let p2p_auto = distributed.p2p_node().clone();
                let state_auto = state.clone();
                tokio::spawn(async move {
                    let thresholds: decentraai_compute::pressure::PressureThresholds =
                        decentraai_compute::pressure::PressureThresholds {
                            queue_depth_high: auto.thresholds.queue_depth_high,
                            latency_ms_high: auto.thresholds.latency_ms_high,
                            cpu_percent_high: auto.thresholds.cpu_percent_high,
                            ram_percent_high: auto.thresholds.ram_percent_high,
                        };
                    let mut state_machine = decentraai_compute::pressure::AssistState::Normal;
                    let mut last_fired: Option<std::time::Instant> = None;
                    let mut was_distributed = false;
                    loop {
                        tokio::time::sleep(Duration::from_secs(auto.tick_seconds)).await;
                        // Honest signals only, measured by the API state itself.
                        let signals = state_auto.pressure_signals().await;
                        let (new_state, decision) = decentraai_compute::pressure::evaluate(
                            &signals,
                            &thresholds,
                            state_machine,
                        );
                        state_machine = new_state;
                        tracing::info!(
                            score = format_args!("{:.2}", decision.score),
                            should_assist = decision.should_assist,
                            reasons = ?decision.reasons,
                            "autonomous pressure evaluated"
                        );
                        if !decision.should_assist {
                            // Pressure released: if we were executing
                            // distributed, record the release and return to
                            // LOCAL operation.
                            if was_distributed {
                                was_distributed = false;
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0);
                                if let Some(evidence) = &state_auto.evidence {
                                    evidence
                                        .index()
                                        .lock()
                                        .expect("evidence lock")
                                        .add(
                                            decentraai_agents::evidence::EvidenceEntry::new(
                                                format!("m15:release:{now}"),
                                                decentraai_agents::evidence::EvidenceFamily::Execution,
                                                "M15 PRESSURE_RELEASED: fabric pressure fell below release threshold; released borrowed capacity, returned to LOCAL".to_string(),
                                                now,
                                            )
                                            .tagged("m15")
                                            .tagged("release"),
                                        );
                                }
                                tracing::info!("M15 pressure released; returned to LOCAL");
                            }
                            continue;
                        }
                        if let Some(t) = last_fired {
                            if t.elapsed() < Duration::from_secs(auto.cooldown_seconds) {
                                continue; // hysteresis cooldown
                            }
                        }
                        last_fired = Some(std::time::Instant::now());
                        was_distributed = true;
                        // M15: Governor autonomously fires. We drive it through
                        // the node's OWN governor_execute endpoint, so the whole
                        // decision (Model Colony, resource verdict, distributed
                        // map-reduce, EvidenceChain, Economy credit) runs — no
                        // operator POST required.
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        if let Some(evidence) = &state_auto.evidence {
                            evidence
                                .index()
                                .lock()
                                .expect("evidence lock")
                                .add(
                                    decentraai_agents::evidence::EvidenceEntry::new(
                                        format!("m15:pressure:{now}"),
                                        decentraai_agents::evidence::EvidenceFamily::Execution,
                                        format!(
                                            "M15 PRESSURE_FIRED: should_assist=true reasons={:?} score={:.2}",
                                            decision.reasons, decision.score
                                        ),
                                        now,
                                    )
                                    .tagged("m15")
                                    .tagged("pressure-fired"),
                                );
                        }
                        // Autonomous workload: a deterministic fabric-analysis
                        // task, large enough that the Governor's resource
                        // verdict can legitimately choose DISTRIBUTED.
                        let content = format!(
                            "DecentraAI autonomous pressure probe. The fabric reports cpu={:.0}% ram={:.0}% queue={} workers={}. \
                             Section 1 covers capability routing and DFCP negotiation between verified peers, including how offers are scored \
                             deterministically and how fairness is a bias rather than a dictator. Section 2 covers reservation, lease expiry \
                             and release semantics under owner limits, and why every lease must expire. Section 3 covers evidence verification \
                             before any contribution credit is recorded for a remote worker, and how only cryptographic failures punish a peer. \
                             Section 4 covers the reward policy and how verified work maps to credit through the economy ledger. \
                             Section 5 covers Model Colony selection where the best model for a task is chosen by capability, RAM fit and verified \
                             benchmark evidence, with non-reasoners preferred for reduction. Section 6 covers the governor resource-aware verdict \
                             choosing LOCAL, DISTRIBUTED, QUEUE or REJECT from real pressure signals. Section 7 covers distributed map-reduce \
                             inference where a single logical workload is split into shards, mapped across workers and reduced into one final \
                             answer, with the honest boundary that llama-server cannot split a forward pass across nodes. Section 8 covers \
                             deterministic ordering, stable ids for batched tasks, retries, and how worker failure must never corrupt results. \
                             Section 9 covers the economic loop where a remote worker earns verified contribution credit only after the evidence \
                             chain records a successful execution, and how the reward ledger is separate from any real token registry.",
                            signals.cpu_percent,
                            signals.ram_percent,
                            signals.queue_depth,
                            1 + p2p_auto.connected_peers().await.len()
                        );
                        let gov_body = serde_json::json!({
                            "task_id": format!("m15-auto-{now}"),
                            "task_kind": "summarize",
                            "instruction": "Summarize the fabric state in ONE short paragraph.",
                            "content": content,
                        });
                        let client = reqwest::Client::builder()
                            .timeout(Duration::from_secs(240))
                            .build()
                            .unwrap_or_else(|_| reqwest::Client::new());
                        let gov_url = format!("http://127.0.0.1:{api_port}/v1/governor/execute");
                        let gov_resp = client
                            .post(&gov_url)
                            .bearer_auth(
                                std::fs::read_to_string(data_dir.join("runtime/api.token"))
                                    .map(|s| s.trim().to_string())
                                    .unwrap_or_default(),
                            )
                            .json(&gov_body)
                            .send()
                            .await;
                        match gov_resp {
                            Ok(r) => {
                                let status = r.status();
                                let text = r.text().await.unwrap_or_default();
                                tracing::info!(
                                    status = %status,
                                    body_len = text.len(),
                                    "M15 governor executed autonomously"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "M15 governor self-request failed");
                            }
                        }
                    }
                });
            }
        }
        // Sharing is Caring (M14/M15 M1): answer DFCP assist requests under
        // owner limits when configured; disabled by default.
        if let Some(assist_cfg) = config.sharing.assist.as_ref() {
            if assist_cfg.enabled {
                let worker_state = std::sync::Arc::new(
                    decentraai_runtime::intel_assist::AssistWorkerState::with_embeddings(
                        std::sync::Arc::new(assist_cfg.clone()),
                        backend_url.clone(),
                        config.inference.embeddings_backend_url.clone(),
                        vec![], // empty = any TRUSTED peer may request
                    ),
                );
                let p2p_for_worker = distributed.p2p_node().clone();
                let sender_p2p = p2p_for_worker.clone();
                let send_to_peer: decentraai_runtime::intel_assist::PeerSender =
                    std::sync::Arc::new(move |peer, bytes| {
                        let p2p = sender_p2p.clone();
                        tokio::spawn(async move {
                            if let Err(e) = p2p.request(peer, bytes).await {
                                tracing::warn!(%peer, error = %e, "assist result delivery failed");
                            }
                        });
                    });
                let mut p2p_mut = p2p_for_worker.clone();
                p2p_mut.set_on_dfcp(decentraai_runtime::intel_assist::attach_dfcp_worker(
                    std::sync::Arc::clone(&worker_state),
                    send_to_peer,
                ));
                // Keep the live engine URL fresh for assist execution.
                {
                    let ws = worker_state.clone();
                    let manager = manager.clone();
                    tokio::spawn(async move {
                        loop {
                            let url = manager.lock().await.base_url().unwrap_or_default();
                            ws.update_backend_url(&url);
                            tokio::time::sleep(Duration::from_secs(10)).await;
                        }
                    });
                }
                let _ = p2p_mut;
            }
        }
        // M19 auto-propagation (OPT-IN via env DECENTRAAI_MEMORY_PROPAGATE=1):
        // verified/trusted entries in eligible scopes (public + remote-write +
        // network/fabric/system level) are offered to connected peers every
        // cycle. Deterministic: id-ascending peers, newest-first batches,
        // bounded counts; receivers keep their own gates and downgrade
        // imports to candidate. Off by default — sharing is always a choice.
        if std::env::var("DECENTRAAI_MEMORY_PROPAGATE").as_deref() == Ok("1") {
            if let Some(store) = agent_memory_store.clone() {
                let interval_secs = std::env::var("DECENTRAAI_MEMORY_PROPAGATE_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60)
                    .max(10);
                let p2p_for_prop = distributed.p2p_node().clone();
                let local = local_peer_id.to_string();
                tokio::spawn(async move {
                    let cfg =
                        decentraai_distributed::memory_propagator::PropagationConfig::default();
                    loop {
                        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                        match decentraai_distributed::memory_propagator::propagate_once(
                            &store,
                            &p2p_for_prop,
                            &local,
                            &cfg,
                        )
                        .await
                        {
                            report if report.entries_offered > 0 => {
                                tracing::info!(
                                    scopes = report.scopes_propagated,
                                    offered = report.entries_offered,
                                    peers = report.peers_targeted,
                                    accepted = report.accepted,
                                    duplicates = report.duplicates,
                                    declined_peers = report.declined_peers,
                                    errors = report.errors,
                                    "memory propagation cycle"
                                );
                            }
                            _ => {} // nothing travel-worthy: stay silent
                        }
                    }
                });
            }
        }
        // TTS: Kokoro subprocess for the chat speak button. Enabled only when
        // `tts.enabled` is set AND the venv/model files exist; a missing setup
        // logs a warning and serves without voice rather than failing startup.
        if let Some(tts_cfg) = config.tts.clone() {
            if tts_cfg.enabled {
                match TtsServer::spawn(&data_dir, &tts_cfg.voice, tts_cfg.speed).await {
                    Ok(server) => {
                        info!(
                            voice = %tts_cfg.voice,
                            speed = tts_cfg.speed,
                            "TTS online (Kokoro subprocess)"
                        );
                        state.attach_tts(Arc::new(TtsManager::new(
                            Some(server),
                            tts_cfg.voice.clone(),
                            tts_cfg.speed,
                        )));
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            "TTS unavailable; chat speak disabled (run scripts/setup-tts.sh)"
                        );
                    }
                }
            }
        }
        // OCR: attach the manager spawned earlier (subprocess for `/v1/ocr`).
        if let Some(manager) = ocr_manager {
            state.attach_ocr(Arc::new(manager));
        }
        // STT: attach the manager spawned earlier (subprocess for `/v1/stt`).
        if let Some(manager) = stt_manager {
            state.attach_stt(Arc::new(manager));
        }
        // HF skills: attach the manager spawned earlier (`/v1/skills/<id>`).
        if let Some(manager) = skills_manager {
            state.attach_skills_tool(Arc::new(manager));
        }
        // Transformers: attach the manager spawned earlier (Python subprocess
        // for OpenAI-compatible inference backend).
        if let Some(manager) = transformers_manager {
            state.attach_transformers(Arc::new(manager));
        }
        // P1: the AGENTS dashboard view reads the node's agent manager.
        state.attach_agents(agent_manager.clone());
        // P3.5/P9: the API can trigger collective workflows by delegating to
        // the node's local agents.
        state.attach_orchestrator(agent_orchestrator.clone());
        // P8: expose the dataset/skill registry to the dashboard (read-only).
        // The persistent registry drives the agent; the demo is shown only as a
        // labelled demonstration (the handler adds it).
        state.attach_skills(Arc::new(skills_registry.clone()));
        // P8: expose the talent tree (capability graph) to the dashboard.
        state.attach_talent_tree(Arc::new(decentraai_agents::seed_talent_tree()));
        // RAG: expose /v1/embeddings + /v1/rag when an embeddings backend is
        // configured (created once above, shared with the inference executor).
        if let (Some(client), Some(rm)) = (&embedding_client, &retrieval_manager) {
            state.attach_embedding(client.clone());
            state.attach_retrieval(rm.clone());
        }
        // Collective memory (SQLite) for the dashboard + workflow results.
        if let Some(store) = agent_memory_store.clone() {
            state.attach_memory(store);
        }
        // Model Colony registry (M-I): governance stages persist across
        // restarts; seeds the initial three candidates on first boot.
        state.attach_model_intel(data_dir.join("db/model_intel.json"));
        // Memory-sync inbound (M19): accept collective-memory batches from
        // peers into scopes that EXPLICITLY opted in (access public +
        // allow_remote_write). Remote claims always land as Candidate —
        // verification is a local act, never imported from the wire.
        if let Some(store) = agent_memory_store.clone() {
            let mut p2p_mut_sync = distributed.p2p_node().clone();
            p2p_mut_sync.set_on_memory_sync(move |_peer, req| {
                use decentraai_distributed::agent_memory::sync_entry_to_memory;
                use decentraai_protocol::memory_sync::MemorySyncResponse;
                let reject_all = |n: usize| {
                    serde_json::to_vec(&MemorySyncResponse {
                        protocol_version: 1,
                        declined: false,
                        accepted: 0,
                        duplicates: 0,
                        conflicts_linked: 0,
                        expired: 0,
                        rejected: n.min(u32::MAX as usize) as u32,
                    })
                    .unwrap_or_default()
                };
                if !req.is_shape_valid() || req.scope.is_empty() {
                    return reject_all(req.entries.len());
                }
                let mut accepted = 0u32;
                let mut duplicates = 0u32;
                let mut conflicts_linked = 0u32;
                let mut rejected = 0u32;
                for se in req.entries {
                    let entry = sync_entry_to_memory(se, &req.scope);
                    match store.write_checked(
                        &req.scope,
                        &entry,
                        "memory-sync",
                        false,
                        false,
                        false,
                    ) {
                        Ok(decentraai_agents::memory::WriteOutcome::Stored) => accepted += 1,
                        Ok(decentraai_agents::memory::WriteOutcome::Duplicate { .. }) => {
                            duplicates += 1
                        }
                        Ok(decentraai_agents::memory::WriteOutcome::CompetingClaim { .. }) => {
                            accepted += 1;
                            conflicts_linked += 1;
                        }
                        Err(_) => rejected += 1,
                    }
                }
                tracing::info!(
                    scope = %req.scope,
                    accepted,
                    duplicates,
                    conflicts_linked,
                    rejected,
                    "memory-sync batch processed"
                );
                serde_json::to_vec(&MemorySyncResponse {
                    protocol_version: 1,
                    declined: false,
                    accepted,
                    duplicates,
                    conflicts_linked,
                    expired: 0,
                    rejected,
                })
                .unwrap_or_default()
            });
        }
        // P12: collective knowledge & decisions runtime. It shares the
        // authoritative compensation ledger with the compute manager, so a
        // verified compute receipt credits the SAME earnings bookkeeping the
        // Workers view shows. A receipt's credit always uses the worker's
        // *measured* contribution profile: the receipt handler reads the live
        // ComputeManager M17 tracker first (auto-seed — no manual wiring
        // needed), falls back to an explicitly wired profile, then to zero
        // (unknown workers earn 0 — honest by default). A client can never
        // supply its own profile through the API.
        if let Some(store) = agent_memory_store.clone() {
            match decentraai_distributed::knowledge_runtime::KnowledgeRuntime::new(
                compute_manager.compensation_ledger(),
                identity.peer_id().to_string(),
                Some(store),
            ) {
                Ok(knowledge_runtime) => {
                    // M19: when an embeddings backend exists, new feedback
                    // entries are auto-indexed for semantic search.
                    let knowledge_runtime = match &embedding_client {
                        Some(client) => knowledge_runtime.with_embedder(client.clone()),
                        None => knowledge_runtime,
                    };
                    state.attach_knowledge(Arc::new(knowledge_runtime));
                }
                Err(e) => warn!(error = %e, "P12 knowledge runtime failed to attach"),
            }
        }
        // Evidence RAG (experimental memory): indexes real executions, receipts,
        // decisions and collective memory; `/v1/evidence` answers "what have we
        // learned?" with derived lessons (never invented numbers). The index
        // syncs lazily from the live sources at request time, so it needs no
        // background task and never falls out of step with the sources. The
        // same runtime is shared with the Benchmark Lab (bench runs feed it).
        state.attach_evidence(evidence_rag);
        // M16/M17 security: sign evidence entries backing economic attribution
        // with the node identity (fail-closed credit verification).
        state.attach_identity_signer(identity.signing_key_bytes());
        // Benchmark Lab: expose `/v1/bench` when an inference executor was
        // wired (the lab needs a real model to run tasks).
        if let Some(benchmark) = benchmark_manager {
            state.attach_benchmark(benchmark);
        }
        // Q2: enable consumer API keys (`dca_…`) sharing the authoritative
        // quota ledger with the compute manager, so worker credits and
        // consumer reserve/settle are one ledger.
        state.attach_consumer(
            Some(data_dir.join("db/consumer_keys.json")),
            Some(compute_manager.quota_ledger()),
        );
        // Model Fabric: the provider control plane persists its catalog to
        // `db/providers.json`; credentials stay in memory only (re-entered
        // after restart by design — see ProviderManager docs).
        let provider_manager = Arc::new(tokio::sync::Mutex::new(
            decentraai_providers::ProviderManager::new(&data_dir),
        ));
        state.attach_providers(provider_manager);
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
                Ok(socket) => {
                    match std::net::TcpStream::connect_timeout(&socket, Duration::from_millis(1500))
                    {
                        Ok(_) => {
                            let latency_ms = start.elapsed().as_millis();
                            println!("  Backend {} reachable (yes, {} ms)", addr, latency_ms);
                            true
                        }
                        Err(e) => {
                            let latency_ms = start.elapsed().as_millis();
                            println!(
                                "  Backend {} reachable (no, {} ms): {}",
                                addr, latency_ms, e
                            );
                            println!(
                                "  Is the node serving? Run 'decentraai node' or 'decentraai serve start' first."
                            );
                            false
                        }
                    }
                }
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
        println!("\nFilter with: decentraai model search \"{query}\" --category <category>");
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
        println!("  {:<60} {:<40} {} downloads", m.id, tag, m.downloads);
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
    let bound = node.listen("/ip4/0.0.0.0/tcp/32937").await?;

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
                Some(identity.public_key().to_bytes()),
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
        node.set_on_manifest_announcement(move |peer, manifest, signed_ok| {
            let _ = tx.send((peer, manifest, signed_ok));
        });
    }
    let share_mode = config.sharing.mode;
    let require_signed = config.security.require_signed_announcements;
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
            require_signed_announcements: require_signed,
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
    /// Trust gate (review, security): when true, an announced model is
    /// auto-downloaded ONLY if the announcement carried a valid signature
    /// from the announcing peer. This closes the hole where a LAN peer could
    /// push arbitrary bytes into models/ via ShareMode::Auto + mDNS.
    require_signed_announcements: bool,
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
        bool,
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
        require_signed_announcements,
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

    while let Some((peer, manifest, signed_ok)) = ann_rx.recv().await {
        if in_flight.contains(&manifest.model_id) {
            continue;
        }
        if models_dir.join(&manifest.file_name).exists() {
            info!(model = %manifest.file_name, "already present; skipping auto-download");
            continue;
        }
        // Trust gate: when require_signed_announcements is set (default in
        // node.example.yaml), an unsigned or invalid announcement is never
        // auto-downloaded. The swarm layer already dropped forged signatures;
        // this covers unsigned peers (legacy / mDNS strangers).
        if require_signed_announcements && !signed_ok {
            warn!(
                peer = %peer,
                model = %manifest.file_name,
                "skipping unsigned manifest announcement (require_signed_announcements=true)"
            );
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
                        Some(identity.public_key().to_bytes()),
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
    use decentraai_inference_adapter::{BackendConfig, EngineKind, OpenAiCompatibleBackend};
    use decentraai_runtime::{
        LlamaServer, RuntimeConfig, ServeManager, ensure_admitted, find_llama_server, resolve_model,
    };
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
        dht_enabled: config.network.dht_enabled,
        relay_enabled: config.network.relay_enabled,
        lan_discovery: config.network.lan_discovery,
        bootstrap_peer_count: config.network.bootstrap_peers.len(),
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
    let mut state = ApiState::new(
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
    state.set_dashboard(config.node.dashboard);
    let api_addr = serve_api(state, &bind_address, api_port).await?;

    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        if remote {
            "remote_backend_started"
        } else {
            "inference_started"
        },
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
        format!(
            "local llama-server: {backend_url}  idle unload: {} min",
            config.inference.idle_model_unload_minutes
        )
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
    let addr = args.addr.trim();
    if addr.is_empty() {
        anyhow::bail!(
            "--addr must be this node's reachable address (e.g. /ip4/192.168.1.5/tcp/4001)"
        );
    }
    // The reachable dial target uses the libp2p peer id (base58, e.g.
    // 12D3KooW...), NOT the identity hex id — libp2p cannot parse the raw
    // identity id in a multiaddr. Derive it the same way the swarm does.
    use libp2p::PeerId as Libp2pPeerId;
    use libp2p::identity::Keypair as Libp2pKeypair;
    let libp2p_keypair = Libp2pKeypair::ed25519_from_bytes(identity.signing_key_bytes())
        .map_err(|e| anyhow::anyhow!("deriving libp2p keypair from node key: {e}"))?;
    let libp2p_peer = Libp2pPeerId::from(libp2p_keypair.public());
    let multiaddr = format!("{addr}/p2p/{libp2p_peer}");

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
    println!(
        "The token is shown once; notify 'decentraai token revoke --name {name}' to invalidate a seat."
    );
    Ok(())
}

/// Joins a private swarm from an invite produced by `decentraai invite` (P5).
/// Parses the `<reachable-multiaddr> <token>` string, auto-provisions an
/// identity + validated config for a fresh node (reusing the `setup` wizard so
/// nothing needs to be hand-tuned), stores the guest token as this node's
/// credential (`runtime/invite.token`, 0600), and verifies the multiaddr is
/// actually reachable before declaring success. Ongoing peer discovery is
/// handled by the node's normal mDNS/discovery path.
/// P13 — build + sign, or independently verify, a verified-compute receipt.
fn receipt_command(command: ReceiptCommand) -> Result<()> {
    use decentraai_agents::receipt::{ReceiptVerdict, VerifiedComputeReceipt};
    use decentraai_agents::signed_receipt::{
        SignedComputeReceipt, canonicalize_receipt, sign_receipt, verify_receipt_signature,
    };
    use decentraai_config::NodeConfig;
    use decentraai_identity::Identity;

    match command {
        ReceiptCommand::Sign {
            config,
            execution_id,
            worker_node,
            capability,
            duration_ms,
            output_hash,
        } => {
            let cfg = NodeConfig::load(&config)
                .with_context(|| format!("loading {}", config.display()))?;
            let data_dir = expand_tilde(&cfg.node.data_dir);
            let identity_path = data_dir.join("identity/key.pem");
            let identity = Identity::load(&identity_path).with_context(|| {
                format!(
                    "no identity at {} — run 'decentraai init' or 'decentraai setup' first",
                    identity_path.display()
                )
            })?;

            let receipt = VerifiedComputeReceipt::new(
                execution_id,
                worker_node,
                "agent",
                capability,
                duration_ms,
                ReceiptVerdict::Verified,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            );
            let receipt = match output_hash {
                Some(h) => receipt.with_output_hash(h),
                None => receipt,
            };

            let canonical = canonicalize_receipt(&receipt);
            let signed = sign_receipt(&identity.signing_key_bytes(), &canonical);
            // Exit 0 also prints a machine-readable signed envelope so the
            // operator can pipe it straight into `receipt verify`.
            println!(
                "{}",
                serde_json::to_string_pretty(&CliSignedReceipt {
                    version: signed.version,
                    receipt_bytes: signed.receipt_bytes,
                    signer_public_key: signed.signer_public_key.map(|k| k.to_vec()),
                    signature: signed.signature.clone(),
                })?
            );
            eprintln!(
                "signed receipt for execution {} — signer pub key {}",
                {
                    // We do NOT print secrets. The derivation is public.
                    let signing = identity.signing_key_bytes();
                    let _ = signing;
                    "0x…".to_string()
                },
                hex(signed.signer_public_key.unwrap_or([0u8; 32]))
            );
            Ok(())
        }
        ReceiptCommand::Verify { file } => {
            let raw = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let cli: CliSignedReceipt = serde_json::from_str(&raw)
                .with_context(|| format!("parsing signed receipt from {}", file.display()))?;
            let signed = SignedComputeReceipt {
                version: cli.version,
                receipt_bytes: cli.receipt_bytes,
                signer_public_key: cli.signer_public_key.map(|k| {
                    let mut arr = [0u8; 32];
                    if k.len() == 32 {
                        arr.copy_from_slice(&k);
                    }
                    arr
                }),
                signature: cli.signature,
            };
            verify_receipt_signature(&signed).map_err(|e| anyhow::anyhow!(e))?;
            println!("VERIFICATION = SUCCESS");
            println!(
                "receipt_bytes = {}",
                String::from_utf8_lossy(&signed.receipt_bytes)
            );
            Ok(())
        }
    }
}

/// Lowercase-hex helper for a byte array (display only, public key).
fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Handler for `decentraai upgrade check|apply|auto`.
async fn upgrade_command(args: UpgradeArgs) -> Result<()> {
    use std::time::Duration;
    use upgrade::{ApplyReport, UpdateStatus, apply_update, check_for_update, installed_bin_path};

    let repo = expand_tilde(&args.repo.to_string_lossy());
    match args.command {
        UpgradeCommand::Check => match check_for_update(Path::new(&repo)) {
            UpdateStatus::UpToDate => {
                println!("up to date (HEAD == origin/main)");
            }
            UpdateStatus::Behind {
                behind,
                local_head,
                remote_head,
            } => {
                println!(
                    "update available: {behind} commit(s) behind — {local_head} -> {remote_head}"
                );
                println!("  run `decentraai upgrade apply` to update");
            }
            UpdateStatus::NoRepo => bail!("not a git checkout: {}", repo.display()),
            UpdateStatus::Error(e) => bail!("update check failed: {e}"),
        },
        UpgradeCommand::Apply => {
            let report: ApplyReport = apply_update(Path::new(&repo))?;
            println!("upgraded {} -> {}", report.from, report.to);
            println!("binary backed up at {}", report.binary_backup.display());
            println!(
                "installed at {} — node service restarted (if installed)",
                installed_bin_path().display()
            );
        }
        UpgradeCommand::Auto(args) => {
            println!(
                "auto-upgrade watcher: checking every {}s against {}",
                args.interval_secs,
                repo.display()
            );
            loop {
                match check_for_update(Path::new(&repo)) {
                    UpdateStatus::Behind { behind, .. } if behind > 0 => {
                        println!("==> update found ({behind} commits behind); applying");
                        match apply_update(Path::new(&repo)) {
                            Ok(report) => {
                                println!("==> upgraded {} -> {}", report.from, report.to)
                            }
                            Err(e) => eprintln!("==> upgrade failed (will retry): {e:#}"),
                        }
                    }
                    UpdateStatus::UpToDate => println!("==> up to date"),
                    UpdateStatus::NoRepo => bail!("not a git checkout: {}", repo.display()),
                    UpdateStatus::Error(e) => eprintln!("==> update check failed: {e}"),
                    UpdateStatus::Behind { .. } => {}
                }
                tokio::time::sleep(Duration::from_secs(args.interval_secs)).await;
            }
        }
    }
    Ok(())
}

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
    let mut store = ConsumerKeyStore::load(&registry_path).with_context(|| {
        format!(
            "loading consumer key registry from {}",
            registry_path.display()
        )
    })?;
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
            let plaintext = store.create(
                &account,
                quota_ceiling,
                rate_limit_per_minute,
                scopes.clone(),
            )?;
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
            println!(
                "Consumer API key for account '{account}' (quota ceiling {quota_ceiling} units/req, {rate_limit_per_minute} req/min):"
            );
            println!("  {plaintext}");
            println!("  key_id: {key_id}");
            println!("Store it now: it is shown once and only its BLAKE3 hash is kept.");
            println!("Never share it; it is an inference credential, not an admin key.");
        }
        ConsumerKeyCommand::List { .. } => {
            let records = store.list();
            if records.is_empty() {
                println!(
                    "No consumer API keys yet — create one with: decentraai consumer-key create --account <n> --quota-ceiling <u> --rate-limit-per-minute <n>"
                );
            } else {
                println!("Consumer API keys ({}):", records.len());
                for r in records {
                    let status = if r.revoked { "revoked" } else { "active" };
                    let scopes = r.scopes.join(",");
                    println!(
                        "  {} ({}): account={}, ceiling={}, rate={}/min, scopes=[{}], created {}",
                        r.key_id,
                        status,
                        r.owner_account,
                        r.quota_ceiling,
                        r.rate_limit_per_minute,
                        scopes,
                        r.created_at
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
            println!(
                "Consumer key '{key_id}' revoked; it stops authenticating at the next API request."
            );
        }
    }
    Ok(())
}

/// `decentraai intel` subcommands (Fabric Intelligence). Status and provider
/// listing are config-driven (offline); `test` calls the LIVE node API so it
/// exercises the exact path a request would take.
#[derive(Debug, Subcommand)]
enum IntelCommand {
    /// Show the Fabric Intelligence section of the given config (offline,
    /// no node required). Secrets are NEVER printed — only the env NAME.
    Status {
        #[arg(long, default_value = "~/.decentraai/node.yaml")]
        config: PathBuf,
    },
    /// List the configured intelligence providers with availability.
    Providers {
        #[arg(long, default_value = "~/.decentraai/node.yaml")]
        config: PathBuf,
    },
    /// Send a test task to the live node's /v1/intel/plan endpoint.
    Test {
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api: String,
        #[arg(long)]
        token: Option<String>,
        /// The task to analyze (e.g. "compare these three PDFs").
        #[arg(short = 'm', long)]
        task: String,
    },
}

async fn intel_command(command: IntelCommand) -> Result<()> {
    match command {
        IntelCommand::Status { config } => {
            let path = expand_tilde(config.to_string_lossy().as_ref());
            let cfg = decentraai_config::NodeConfig::load(&path)
                .map_err(|e| anyhow::anyhow!("config {}: {e}", path.display()))?;
            match &cfg.fabric_intelligence {
                None => println!("fabric intelligence: disabled (no config section)"),
                Some(fi) => {
                    println!("fabric intelligence:");
                    println!("  enabled:         {}", fi.enabled);
                    println!("  policy:          {:?}", fi.policy);
                    println!("  min_confidence:  {}", fi.min_confidence);
                    println!(
                        "  local_model:     {}",
                        fi.local_model.as_deref().unwrap_or("(node default)")
                    );
                    match &fi.external {
                        Some(ext) => {
                            println!("  external:        {} @ {}", ext.model, ext.base_url);
                            println!("  api key env:     {}", ext.api_key_env);
                        }
                        None => println!("  external:        not configured"),
                    }
                    println!(
                        "  max_artifact_bytes: {} (hard cap {})",
                        fi.max_artifact_bytes,
                        decentraai_config::MAX_FABRIC_ARTIFACT_BYTES
                    );
                }
            }
            Ok(())
        }
        IntelCommand::Providers { config } => {
            let path = expand_tilde(config.to_string_lossy().as_ref());
            let cfg = decentraai_config::NodeConfig::load(&path)
                .map_err(|e| anyhow::anyhow!("config {}: {e}", path.display()))?;
            println!("intelligence providers:");
            println!("  local llama.cpp backend — available when the node is serving");
            if let Some(ext) = cfg
                .fabric_intelligence
                .as_ref()
                .and_then(|f| f.external.as_ref())
            {
                let keyed = std::env::var(&ext.api_key_env)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                println!(
                    "  external {} @ {} — key env {}: {}",
                    ext.model,
                    ext.base_url,
                    ext.api_key_env,
                    if keyed { "SET" } else { "UNSET" }
                );
            } else {
                println!("  external — not configured");
            }
            Ok(())
        }
        IntelCommand::Test { api, token, task } => {
            let url = format!("{api}/v1/intel/plan");
            let client = reqwest::Client::new();
            let mut req = client.post(&url).json(&serde_json::json!({ "task": task }));
            if let Some(t) = token.as_deref().filter(|t| !t.is_empty()) {
                req = req.bearer_auth(t);
            }
            let res = req
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
            let status = res.status();
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            println!("POST {url} → HTTP {status}");
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            if status.is_success() {
                Ok(())
            } else {
                anyhow::bail!("intel plan failed with HTTP {status}")
            }
        }
    }
}

/// `decentraai agent` subcommands (Collective Intelligence P1).
async fn agent_command(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::List { config } => agent_list(&config),
        AgentCommand::Show { config, agent } => agent_show(&config, &agent),
        AgentCommand::Add {
            config,
            id,
            name,
            role,
            description,
            capabilities,
        } => agent_add(&config, &id, &name, &role, &description, &capabilities),
        AgentCommand::Remove { config, agent } => agent_remove(&config, &agent),
        AgentCommand::Workflow { template, .. } => agent_workflow(&template),
        AgentCommand::WorkflowRun {
            config,
            prompt,
            template,
            retrieve,
        } => agent_workflow_run(&config, &prompt, &template, retrieve.as_deref()).await,
        AgentCommand::Reputation {
            agent, min_samples, ..
        } => agent_reputation(&agent, min_samples),
        AgentCommand::TalentTree {
            have,
            budget_mb,
            target,
            ..
        } => agent_talent_tree(&have, budget_mb, target.as_deref()),
        AgentCommand::Skill { command } => match command {
            SkillCommand::Demo { config, model } => agent_skill(&config, model.as_deref()),
            SkillCommand::List { config } => skill_list(&config),
            SkillCommand::Add {
                config,
                dataset_id,
                name,
                source,
                kind,
                develops,
                provenance,
                skill_id,
                requires_model,
                unlock,
            } => skill_add(
                &config,
                &dataset_id,
                &name,
                &source,
                &kind,
                &develops,
                &provenance,
                &skill_id,
                &requires_model,
                &unlock,
            ),
        },
    }
}

/// Dispatches `decentraai rag` subcommands — index/query the semantic retrieval index.
async fn rag_command(command: RagCommand) -> Result<()> {
    match command {
        RagCommand::Index {
            config,
            doc_id,
            text,
            capability,
        } => rag_index(&config, &doc_id, &text, capability.as_deref()).await,
        RagCommand::Query { config, text, k } => rag_query(&config, &text, k).await,
    }
}

/// Dispatches `decentraai memory` subcommands — inspect collective memory entries.
async fn memory_command(command: MemoryCommand) -> Result<()> {
    match command {
        MemoryCommand::List { config } => memory_list(&config).await,
    }
}

/// DecentraAI Benchmark Lab CLI: run tasks through the node's live executor.
async fn bench_command(command: BenchCommand) -> Result<()> {
    match command {
        BenchCommand::Run {
            config,
            prompt,
            gold,
            mode,
            evidence,
        } => {
            let mut body = serde_json::json!({ "prompt": prompt, "mode": mode, "task_id": "cli" });
            if let Some(gold) = gold {
                if !gold.trim().is_empty() {
                    body["gold"] = serde_json::Value::String(gold);
                }
            }
            if let Some(evidence) = evidence {
                if !evidence.trim().is_empty() {
                    body["evidence"] = serde_json::Value::Array(
                        evidence
                            .split(',')
                            .map(|s| serde_json::Value::String(s.trim().to_string()))
                            .collect(),
                    );
                }
            }
            if mode == "collective" {
                body["agents"] = serde_json::json!(3);
            }
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.post(format!("{base_url}/bench/run")).json(&body);
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "bench run failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            let run = &j["run"];
            let verdict = run.get("verdict").and_then(|v| v.as_str()).unwrap_or("?");
            let metrics = &run["metrics"];
            let latency = metrics
                .get("latency_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let tokens = metrics.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("verdict: {verdict} ({latency}ms, {tokens} tokens)");
            let output = run.get("output").and_then(|v| v.as_str()).unwrap_or("");
            if !output.is_empty() {
                println!("output: {}", &output[..output.len().min(300)]);
            }
        }
        BenchCommand::Dataset {
            config,
            file,
            limit,
            mode,
            agents,
        } => {
            let tasks =
                decentraai_distributed::benchmark_datasets::load_browsecomp_plus(&file, limit)?;
            if tasks.is_empty() {
                anyhow::bail!("no tasks loaded from {}", file.display());
            }
            let modes: &[&str] = if mode == "both" {
                &["single", "collective"]
            } else {
                &[mode.as_str()]
            };
            println!(
                "running {} task(s) in mode(s) {:?} (agents={agents}) through {}",
                tasks.len(),
                modes,
                config.display()
            );
            let (client, base_url, token) = build_local_client(&config)?;
            let mut total = 0usize;
            for m in modes {
                println!("  === mode {m} ===");
                for (i, task) in tasks.iter().enumerate() {
                    let mut body = serde_json::json!({
                        "prompt": task.prompt,
                        "mode": m,
                        "agents": agents,
                        "task_id": task.task_id,
                    });
                    if let Some(gold) = &task.gold {
                        body["gold"] = serde_json::Value::String(gold.clone());
                    }
                    if !task.evidence.is_empty() {
                        body["evidence"] = serde_json::Value::Array(
                            task.evidence
                                .iter()
                                .map(|s| serde_json::Value::String(s.clone()))
                                .collect(),
                        );
                    }
                    let mut req = client.post(format!("{base_url}/bench/run")).json(&body);
                    if let Some(t) = &token {
                        req = req.bearer_auth(t);
                    }
                    let resp = req.send().await?;
                    let status = resp.status();
                    let j: serde_json::Value = resp.json().await?;
                    if !status.is_success() {
                        eprintln!(
                            "  [{}/{}] {} failed (HTTP {}): {}",
                            i + 1,
                            tasks.len(),
                            task.task_id,
                            status,
                            j.get("error").map(|e| e.to_string()).unwrap_or_default()
                        );
                        continue;
                    }
                    total += 1;
                    let verdict = j["run"]
                        .get("verdict")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let latency = j["run"]["metrics"]
                        .get("latency_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    println!(
                        "  [{}/{}] {} -> {verdict} ({latency}ms)",
                        i + 1,
                        tasks.len(),
                        task.task_id
                    );
                }
            }
            println!("completed {total} graded run(s)");
            // After the batch, show the node's honest comparison (paired).
            let mut req = client.get(format!("{base_url}/bench"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let j: serde_json::Value = resp.json().await?;
            print_bench_comparison(&j);
        }
        BenchCommand::Pool {
            config,
            max_workers,
            capability,
            model,
            lease_seconds,
            max_tokens,
            tasks_limit,
        } => {
            let tasks: Vec<decentraai_distributed::pool::PoolTask> =
                decentraai_distributed::benchmark_datasets::model_intelligence_tasks()
                    .iter()
                    .take(tasks_limit.unwrap_or(usize::MAX))
                    .map(decentraai_distributed::pool::PoolTask::from)
                    .collect();
            if tasks.is_empty() {
                anyhow::bail!("no Model Intelligence tasks to run");
            }
            println!(
                "pool-bench: {} tasks, max_workers={max_workers}, capability={capability}, model={model}",
                tasks.len()
            );
            let (client, base_url, token) = build_local_client(&config)?;
            let body = serde_json::json!({
                "tasks": tasks,
                "capability": capability,
                "model": model,
                "lease_seconds": lease_seconds,
                "max_tokens": max_tokens,
                "max_workers": max_workers,
            });
            let mut req = client.post(format!("{base_url}/v1/pool/bench")).json(&body);
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "pool-bench failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
    }
    Ok(())
}

/// P14 — Compute Contribution / Credits CLI: read-only inspection of the
/// node-local verified contribution state, credit balances/events, and
/// placement plans.
async fn contribution_command(command: ContributionCommand) -> Result<()> {
    match command {
        ContributionCommand::State { config } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.get(format!("{base_url}/v1/contribution"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "contribution state failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::Credits { config } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.get(format!("{base_url}/v1/credits/balance"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "credit balance failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::Events { config } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.get(format!("{base_url}/v1/credits/events"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "credit events failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::History { config } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.get(format!("{base_url}/v1/verified-compute/history"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "verified compute history failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::Plan {
            config,
            model,
            min_vram_mb,
            min_ram_mb,
            min_gpu_count,
            context_tokens,
            distributed,
        } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let url = format!(
                "{base_url}/v1/placement/plan?model_id={model}&min_vram_mb={min_vram_mb}&\
                 min_ram_mb={min_ram_mb}&min_gpu_count={min_gpu_count}&context_tokens={context_tokens}&\
                 distributed={distributed}"
            );
            let mut req = client.get(url);
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "placement plan failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::Graph { config } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let mut req = client.get(format!("{base_url}/v1/fabric/graphs"));
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "fabric graphs failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
        ContributionCommand::EvidenceChain {
            config,
            execution_id,
        } => {
            let (client, base_url, token) = build_local_client(&config)?;
            let url = format!("{base_url}/v1/evidence-chain?execution_id={execution_id}");
            let mut req = client.get(url);
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let j: serde_json::Value = resp.json().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "evidence chain failed (HTTP {}): {}",
                    status,
                    j.get("error").map(|e| e.to_string()).unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&j)?);
        }
    }
    Ok(())
}

/// Prints the `/v1/bench` payload as a compact comparison table. The
/// headline verdict is the PAIRED comparison (tasks run in both modes);
/// the global per-mode aggregate is shown as secondary data.
fn print_bench_comparison(j: &serde_json::Value) {
    let runs = j.get("runs").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("\nregistry runs: {runs}");
    if !j.get("attached").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("benchmark runtime not attached on this node");
        return;
    }
    let cmp = &j["comparison"];
    let global = &j["global"];
    let pct = |m: &serde_json::Value| -> String {
        let graded = m.get("graded").and_then(|v| v.as_u64()).unwrap_or(0);
        if graded == 0 {
            "—".to_string()
        } else {
            let acc = m.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            format!("{:.0}%", acc * 100.0)
        }
    };
    println!(
        "  PAIRED (shared tasks)  : single {}   collective: {}",
        pct(&cmp["single"]),
        pct(&cmp["collective"])
    );
    println!(
        "  global (all runs)      : single {}   rag: {}   collective: {}",
        pct(&global["single"]),
        pct(&global["rag"]),
        pct(&global["collective"])
    );
    let verdict = cmp
        .get("collective_beats_single")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reasoning = cmp.get("reasoning").and_then(|v| v.as_str()).unwrap_or("");
    if verdict {
        println!("  verdict: COLLECTIVE BEATS SINGLE — {reasoning}");
    } else {
        println!("  verdict: no conclusion yet — {reasoning}");
    }
}

/// Shared helper: build a reqwest client pointed at the local node's API.
/// Reads the api_port from NodeConfig and loads the master token from the
/// data dir if present. Returns `(Client, base_url, Option<bearer_token>)`.
fn build_local_client(config_path: &Path) -> Result<(reqwest::Client, String, Option<String>)> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let api_port = config.inference.api_port;
    let token = std::fs::read_to_string(data_dir.join("runtime/api.token"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{api_port}/v1");
    Ok((client, base_url, token))
}

/// Index a document into the semantic retrieval index.
async fn rag_index(
    config_path: &Path,
    doc_id: &str,
    text: &str,
    capability: Option<&str>,
) -> Result<()> {
    if doc_id.trim().is_empty() || text.trim().is_empty() {
        anyhow::bail!("--doc-id and --text must not be empty");
    }
    let (client, base_url, token) = build_local_client(config_path)?;
    let url = format!("{base_url}/rag/index");
    let mut body = serde_json::json!({
        "doc_id": doc_id,
        "text": text,
    });
    if let Some(cap) = capability {
        if !cap.is_empty() {
            body["capability"] = serde_json::Value::String(cap.to_string());
        }
    }
    let mut req = client.post(&url).json(&body);
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let j: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!(
            "index failed (HTTP {}): {}",
            status,
            j.get("error").map(|e| e.to_string()).unwrap_or_default()
        );
    }
    println!("indexed document '{doc_id}' successfully");
    if let Some(meta) = j.get("metadata") {
        println!("metadata: {meta}");
    }
    Ok(())
}

/// Query the semantic retrieval index and return top-k results.
async fn rag_query(config_path: &Path, text: &str, k: usize) -> Result<()> {
    if text.trim().is_empty() {
        anyhow::bail!("--text must not be empty");
    }
    let (client, base_url, token) = build_local_client(config_path)?;
    let url = format!("{base_url}/rag/query");
    let body = serde_json::json!({
        "query": text,
        "k": k,
    });
    let mut req = client.post(&url).json(&body);
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let j: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!(
            "query failed (HTTP {}): {}",
            status,
            j.get("error").map(|e| e.to_string()).unwrap_or_default()
        );
    }
    let results = j
        .get("results")
        .or_else(|| j.get("chunks"))
        .cloned()
        .unwrap_or_default();
    let results_array: Vec<_> = if let Some(arr) = results.as_array() {
        arr.clone()
    } else {
        vec![results]
    };
    println!("retrieval results ({count}):", count = results_array.len());
    for (i, chunk) in results_array.iter().enumerate() {
        let score = chunk
            .get("score")
            .map(|s| format!(" (score: {s})"))
            .unwrap_or_default();
        let content = if let Some(t) = chunk.get("text").and_then(|t| t.as_str()) {
            t.to_string()
        } else {
            chunk
                .to_string()
                .trim_matches('"')
                .lines()
                .next()
                .unwrap_or("")
                .to_string()
        };
        let truncated: String = content.chars().take(200).collect();
        println!("\n[{}]{}{}", i + 1, score, truncated);
    }
    if results_array.is_empty() {
        println!("no matching chunks found");
    }
    Ok(())
}

/// List all persistent memory entries with scope, level and access policy.
async fn memory_list(config_path: &Path) -> Result<()> {
    let (client, base_url, token) = build_local_client(config_path)?;
    let url = format!("{base_url}/memory");
    let mut req = client.get(&url);
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let j: serde_json::Value = resp.json().await?;
    if !status.is_success() {
        anyhow::bail!(
            "memory query failed (HTTP {}): {}",
            status,
            j.get("error").map(|e| e.to_string()).unwrap_or_default()
        );
    }
    let entries = j
        .get("entries")
        .or_else(|| j.get("memory"))
        .cloned()
        .unwrap_or_default();
    let entries_array = if let Some(arr) = entries.as_array() {
        arr.clone()
    } else {
        vec![entries]
    };
    println!(
        "persistent memory entries ({count}):",
        count = entries_array.len()
    );
    for entry in &entries_array {
        let key = entry.get("key").and_then(|k| k.as_str()).unwrap_or("?");
        let scope = entry.get("scope").and_then(|s| s.as_str()).unwrap_or("?");
        let level = entry.get("level").and_then(|l| l.as_str()).unwrap_or("?");
        let confidence = entry
            .get("confidence")
            .map(|c| c.to_string())
            .unwrap_or("?".to_string());
        let timestamp = entry
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or("?");
        let preview = entry.get("text").and_then(|t| t.as_str()).unwrap_or("");
        println!("  [{key}] scope={scope} level={level} conf={confidence} ts={timestamp}");
        if !preview.is_empty() {
            println!("    \"{preview}\"");
        }
    }
    if entries_array.is_empty() {
        println!("no persistent memory entries found");
    }
    Ok(())
}

/// Loads the config + identity and returns this node's default local agents
/// (shared by `agent list` and `agent show` so the two never disagree).
fn load_local_agents(config_path: &Path) -> Result<Vec<decentraai_agents::AgentRecord>> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);

    let identity_path = data_dir.join("identity/key.pem");
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        Identity::generate()
    };
    // The libp2p peer id is derived from the identity signing key, exactly as
    // the node daemon does, so the short id matches what the node advertises.
    let libp2p_keypair =
        libp2p::identity::Keypair::ed25519_from_bytes(identity.signing_key_bytes())
            .context("libp2p keypair from identity")?;
    let libp2p_peer = libp2p::PeerId::from(libp2p_keypair.public());
    let short_id = decentraai_distributed::short_node_id(&libp2p_peer);
    Ok(default_local_agents(
        &short_id,
        &config.node.name,
        "",
        config.inference.allow_remote_inference,
        &config.node.name,
        &decentraai_agents::SkillRegistry::new(),
        false,
    ))
}

/// Finds an agent by id within a default agent set, if present.
fn find_agent_by_id<'a>(
    agents: &'a [decentraai_agents::AgentRecord],
    agent_id: &str,
) -> Option<&'a decentraai_agents::AgentRecord> {
    agents.iter().find(|a| a.agent_id == agent_id)
}

fn agent_list(config_path: &Path) -> Result<()> {
    let agents = load_local_agents(config_path)?;
    if agents.is_empty() {
        println!("No logical agents advertised by this node.");
        return Ok(());
    }
    for agent in &agents {
        let caps: Vec<String> = agent
            .semantic_capabilities
            .iter()
            .map(|c| format!("{}:{:?}", c.capability.label(), c.provenance))
            .collect();
        let models = if agent.allowed_models.is_empty() {
            "(none)".to_string()
        } else {
            format!("{} model(s)", agent.allowed_models.len())
        };
        let tools = if agent.tools.is_empty() {
            "(none)".to_string()
        } else {
            format!("{} tool(s)", agent.tools.len())
        };
        println!(
            "{}  role={}  state={:?}\n  capabilities: {}\n  models: {}   tools: {}\n",
            agent.agent_id,
            agent.role,
            agent.state,
            if caps.is_empty() {
                "(none)".to_string()
            } else {
                caps.join(", ")
            },
            models,
            tools,
        );
    }
    println!(
        "Total: {} logical agent(s) on node {}",
        agents.len(),
        agent_node_name(config_path).unwrap_or_else(|_| "?".to_string()),
    );
    Ok(())
}

/// Adds a custom logical agent to the node's persistent records
/// (db/agents.json). Loads the current records (or the defaults), appends the
/// new agent with the given capabilities, writes back atomically, and shows
/// the resulting record. A running node picks the change up on next restart.
fn agent_add(
    config_path: &Path,
    id: &str,
    name: &str,
    role: &str,
    description: &str,
    capabilities: &str,
) -> Result<()> {
    if id.trim().is_empty() {
        anyhow::bail!("--id must not be empty");
    }
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let agents_path = data_dir.join("db/agents.json");
    let short_id = node_short_id(config_path)?;
    let agent_id = format!("{short_id}:{}", id.trim());

    // Load current records (persisted or defaults). The default set needs the
    // skill registry + a model hash; reuse the exact helpers the daemon uses.
    let models_dir = data_dir.join("models");
    let mut records = match load_agent_records(&agents_path) {
        Some(r) if !r.is_empty() => r,
        _ => {
            let skills = load_skill_registry(&data_dir.join("db/skills.json"));
            let model_name =
                resolve_model_name(&models_dir, config.node.model.as_deref()).unwrap_or_default();
            let model_hash = if model_name.is_empty() {
                String::new()
            } else {
                blake3::hash(
                    std::fs::read(models_dir.join(&model_name))
                        .ok()
                        .as_deref()
                        .unwrap_or_default(),
                )
                .to_hex()
                .to_string()
            };
            default_local_agents(
                &short_id,
                &config.node.name,
                &model_hash,
                config.inference.allow_remote_inference,
                &model_name,
                &skills,
                config.inference.embeddings_backend_url.is_some(),
            )
        }
    };
    if records.iter().any(|a| a.agent_id == agent_id) {
        anyhow::bail!("agent '{agent_id}' already exists");
    }

    // Build the new record: name defaults to the id suffix, capabilities are
    // parsed from the comma-separated snake_case list (unknown names are a
    // hard error — no silent typos).
    let display_name = if name.is_empty() {
        id.to_string()
    } else {
        name.to_string()
    };
    let mut record = decentraai_agents::AgentRecord::new(&agent_id, &display_name, role);
    record.description = description.to_string();
    for part in capabilities.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        use std::str::FromStr;
        let kind = decentraai_hub::capability::CapabilityKind::from_str(part).map_err(|_| {
            anyhow::anyhow!("unknown capability '{part}' (see --capabilities help)")
        })?;
        record = record.with_capability(kind, decentraai_hub::capability::Provenance::Verified);
    }
    records.push(record.clone());
    save_agent_records(&agents_path, &records)?;
    println!("added agent {agent_id}");
    println!(
        "  role={}  capabilities={}",
        record.role,
        record
            .semantic_capabilities
            .iter()
            .map(|c| c.capability.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("persisted to {}", agents_path.display());
    println!("restart the node to advertise it on the fabric");
    Ok(())
}

/// Removes a logical agent from the node's persistent records (db/agents.json)
/// and writes back atomically. Refuses to remove the last remaining agent.
fn agent_remove(config_path: &Path, agent_id: &str) -> Result<()> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let agents_path = data_dir.join("db/agents.json");
    let mut records = match load_agent_records(&agents_path) {
        Some(r) if !r.is_empty() => r,
        _ => anyhow::bail!("no persisted agent records — nothing to remove"),
    };
    if records.len() <= 1 {
        anyhow::bail!("cannot remove the last agent");
    }
    let before = records.len();
    records.retain(|a| a.agent_id != agent_id);
    if records.len() == before {
        anyhow::bail!("agent '{agent_id}' not found");
    }
    save_agent_records(&agents_path, &records)?;
    println!(
        "removed agent {agent_id}; {} agent(s) remain",
        records.len()
    );
    println!("persisted to {}", agents_path.display());
    Ok(())
}

/// The node name from config, best-effort (only used for display).
fn agent_node_name(config_path: &Path) -> Result<String> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    Ok(config.node.name)
}

/// The node's short id (derived from its identity, exactly as the daemon
/// derives it), used to build stable agent ids like `<short-id>:role`.
fn node_short_id(config_path: &Path) -> Result<String> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let identity =
        Identity::load(&data_dir.join("identity/key.pem")).context("loading node identity")?;
    let libp2p_keypair =
        libp2p::identity::Keypair::ed25519_from_bytes(identity.signing_key_bytes())
            .context("libp2p keypair from identity")?;
    let libp2p_peer = libp2p::PeerId::from(libp2p_keypair.public());
    Ok(decentraai_distributed::short_node_id(&libp2p_peer))
}

fn agent_show(config_path: &Path, agent_id: &str) -> Result<()> {
    let agents = load_local_agents(config_path)?;
    let agent = find_agent_by_id(&agents, agent_id).ok_or_else(|| {
        let ids: Vec<&str> = agents.iter().map(|a| a.agent_id.as_str()).collect();
        anyhow::anyhow!(
            "no local agent with id '{agent_id}'. Local agents: {}",
            if ids.is_empty() {
                "(none)".to_string()
            } else {
                ids.join(", ")
            }
        )
    })?;

    println!("Agent: {} ({})", agent.name, agent.agent_id);
    println!("  role        : {}", agent.role);
    println!("  description : {}", agent.description);
    println!("  state       : {:?}", agent.state);

    println!("  capabilities:");
    if agent.semantic_capabilities.is_empty() {
        println!("    (none)");
    }
    for c in &agent.semantic_capabilities {
        println!(
            "    - {}  [provenance: {:?}]",
            c.capability.label(),
            c.provenance
        );
    }

    println!("  allowed models:");
    if agent.allowed_models.is_empty() {
        println!("    (none)");
    }
    for m in &agent.allowed_models {
        println!("    - {m}");
    }

    println!("  tools:");
    if agent.tools.is_empty() {
        println!("    (none)");
    }
    for t in &agent.tools {
        println!("    - {} (kind: {})", t.name, t.kind);
    }

    println!("  policies:");
    println!("    sandbox            : {:?}", agent.policies.sandbox);
    println!(
        "    max_concurrent_tasks: {}",
        agent.policies.max_concurrent_tasks
    );
    println!("    allow_remote       : {}", agent.policies.allow_remote);

    println!("  memory scopes:");
    if agent.memory_scopes.is_empty() {
        println!("    (none)");
    }
    for s in &agent.memory_scopes {
        println!("    - {s}");
    }
    Ok(())
}

/// The known workflow templates this CLI can inspect.
fn workflow_templates() -> Vec<(&'static str, decentraai_agents::WorkflowTemplate)> {
    vec![(
        "research_report",
        decentraai_agents::research_report_template(),
    )]
}

fn agent_workflow(template: &str) -> Result<()> {
    let templates = workflow_templates();
    let Some((_, t)) = templates.iter().find(|(id, _)| *id == template) else {
        let available: Vec<&str> = templates.iter().map(|(id, _)| *id).collect();
        anyhow::bail!(
            "unknown workflow template '{template}'. Available templates: {}",
            available.join(", ")
        );
    };
    println!("Template: {} ({})", t.name, t.template_id);
    println!("  {}", t.description.trim());
    println!(
        "Synthesis: {}",
        if t.synthesis { "enabled" } else { "disabled" }
    );
    for step in &t.steps {
        let deps = if step.depends_on.is_empty() {
            "(none)".to_string()
        } else {
            step.depends_on.join(", ")
        };
        println!(
            "  step {}: capability={} evidence={:?} depends_on=[{}]",
            step.step_id,
            step.capability.label(),
            step.evidence,
            deps
        );
    }
    Ok(())
}

/// Runs a collective workflow on the local node via its API
/// (POST /v1/agents/orchestrate). Reads the node's API port + token from the
/// config/data dir. Prints the verdict + generated output for scripting.
async fn agent_workflow_run(
    config_path: &Path,
    prompt: &str,
    template: &str,
    retrieve: Option<&str>,
) -> Result<()> {
    if prompt.trim().is_empty() {
        anyhow::bail!("--prompt must not be empty");
    }
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let api_port = config.inference.api_port;
    // Read the master token if present (optional; the API may be open).
    let token = std::fs::read_to_string(data_dir.join("runtime/api.token"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{api_port}/v1/agents/orchestrate");
    let mut body = serde_json::json!({ "prompt": prompt, "template": template });
    if let Some(r) = retrieve {
        if !r.is_empty() {
            body["retrieve"] = serde_json::Value::String(r.to_string());
        }
    }
    let mut req = client.post(&url).json(&body);
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }

    let (status, j): (reqwest::StatusCode, serde_json::Value) = {
        let resp = req.send().await?;
        let status = resp.status();
        let j = resp.json().await?;
        (status, j)
    };
    if !status.is_success() {
        anyhow::bail!(
            "workflow failed (HTTP {}): {}",
            status,
            j.get("error").map(|e| e.to_string()).unwrap_or_default()
        );
    }
    let verdict = j.get("verdict").cloned().unwrap_or_default();
    println!("verdict: {verdict}");
    let fo = j
        .get("final_output")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if let Some(text) = fo.get("text").and_then(|v| v.as_str()) {
        println!("output: {text}");
    } else {
        println!("output: {fo}");
    }
    Ok(())
}

/// Builds an in-memory reputation store seeded with deterministic synthetic
/// samples for one (agent, capability) — a local demonstration of the P6
/// reputation model. These are NOT real measurements.
fn sample_reputation_store(
    agent: &str,
    capability: &str,
    samples: u64,
) -> decentraai_agents::ReputationStore {
    use decentraai_agents::{ReputationFactor, ReputationStore, ReputationUpdate};
    let factors: &[(ReputationFactor, f32)] = &[
        (ReputationFactor::Reliability, 0.9),
        (ReputationFactor::Quality, 0.8),
        (ReputationFactor::Latency, 0.6),
        (ReputationFactor::Uptime, 0.95),
        (ReputationFactor::Safety, 1.0),
    ];
    let mut store = ReputationStore::new();
    let mut at = 1_000_000u64;
    for (factor, value) in factors {
        for _ in 0..samples {
            store.observe(ReputationUpdate::new(
                agent, capability, *factor, *value, at,
            ));
            at += 1;
        }
    }
    store
}

/// Aggregate score over the meaningful factors, honoring `min_samples`.
/// Mirrors `AgentReputation::score` but lets the CLI respect a custom floor.
fn aggregate_score(rep: &decentraai_agents::AgentReputation, min_samples: u64) -> f32 {
    use decentraai_agents::default_weights;
    let weights = default_weights();
    let mut weighted_sum = 0.0f64;
    let mut weight_total = 0.0f64;
    for (factor, score) in &rep.factors {
        if !score.is_meaningful(min_samples) {
            continue;
        }
        let w = weights.get(factor).copied().unwrap_or(0.0) as f64;
        weighted_sum += w * score.value as f64;
        weight_total += w;
    }
    if weight_total == 0.0 {
        return 0.0;
    }
    (weighted_sum / weight_total).clamp(0.0, 1.0) as f32
}

fn agent_reputation(agent: &str, min_samples: u64) -> Result<()> {
    // The generalist agent's primary claimed capability; synthetic sample data.
    let capability = "chat";
    let store = sample_reputation_store(agent, capability, 3);
    let rep = store
        .get(agent, capability)
        .ok_or_else(|| anyhow::anyhow!("no reputation generated for agent '{agent}'"))?;

    println!("Reputation profile for agent '{agent}' (capability '{capability}')");
    println!(
        "  NOTE: built from synthetic sample data for demonstration — these are\n\
         \x20       NOT real measurements."
    );
    println!(
        "  aggregate score (min_samples={min_samples}): {:.3}",
        aggregate_score(rep, min_samples)
    );
    println!("  per-factor:");
    for reason in rep.reasons() {
        println!("    - {reason}");
    }
    Ok(())
}

/// Parses a comma-separated list of capability names, skipping (and warning
/// on) names that do not map to a known [`CapabilityKind`].
fn parse_have_capabilities(have: &str) -> Vec<decentraai_hub::capability::CapabilityKind> {
    use std::str::FromStr;
    let mut out = Vec::new();
    for part in have.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match decentraai_hub::capability::CapabilityKind::from_str(part) {
            Ok(kind) => out.push(kind),
            Err(()) => warn!(capability = %part, "skipping unknown capability in --have"),
        }
    }
    out
}

fn agent_talent_tree(have: &str, budget_mb: u64, target: Option<&str>) -> Result<()> {
    use std::str::FromStr;
    let tree = decentraai_agents::seed_talent_tree();
    let have_kinds = parse_have_capabilities(have);

    println!("Talent tree (P8 capability graph):");
    println!("  all capabilities ({}):", tree.capabilities().len());
    for kind in &tree.capabilities() {
        let node = tree.get(*kind).expect("capability present");
        let marker = if node.experimental {
            " (experimental)"
        } else {
            ""
        };
        println!("    - {}{marker}", kind.label());
    }

    println!(
        "  unlockable with [{}] and budget {budget_mb} MiB:",
        have_kinds
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let available = tree.available_capabilities(&have_kinds, budget_mb);
    if available.is_empty() {
        println!("    (none)");
    }
    for kind in &available {
        println!("    - {}", kind.label());
    }

    if let Some(target) = target {
        match decentraai_hub::capability::CapabilityKind::from_str(target) {
            Ok(kind) => {
                let path = tree.resolve_path(kind, &have_kinds);
                if path.is_empty() {
                    println!(
                        "  resolve_path to '{target}': not reachable (already held, or a prerequisite chain is missing)"
                    );
                } else {
                    let steps: Vec<&str> = path.iter().map(|k| k.label()).collect();
                    println!("  resolve_path to '{target}': {}", steps.join(" -> "));
                }
            }
            Err(()) => anyhow::bail!("unknown target capability '{target}'"),
        }
    }
    Ok(())
}

/// Loads the persistent dataset/skill registry from disk (best-effort: a
/// missing/corrupt file yields an empty registry so the node still runs).
fn load_skill_registry(path: &std::path::Path) -> decentraai_agents::SkillRegistry {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => decentraai_agents::SkillRegistry::new(),
    }
}

/// Loads the persistent agent records (`db/agents.json`) if present. The
/// operator can edit this file to add/rename/remove the node's logical
/// agents; an empty or missing file yields `None` and the caller falls back
/// to the deterministic defaults.
fn load_agent_records(path: &std::path::Path) -> Option<Vec<decentraai_agents::AgentRecord>> {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).ok(),
        Err(_) => None,
    }
}

/// Persists the node's logical agent records atomically (tmp + rename) so
/// operator edits survive restarts. Best-effort; never fails the boot path.
fn save_agent_records(
    path: &std::path::Path,
    records: &[decentraai_agents::AgentRecord],
) -> Result<()> {
    use std::io::Write;
    let json = serde_json::to_string_pretty(records)?;
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(json.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Persists the dataset/skill registry atomically (tmp + rename).
fn save_skill_registry(
    path: &std::path::Path,
    registry: &decentraai_agents::SkillRegistry,
) -> Result<()> {
    use std::io::Write;
    let json = serde_json::to_string_pretty(registry)?;
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(json.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Shows the dataset/skill layer (P8 dataset): how a dataset + a skill
/// applied to a model unlock capabilities that feed the Talent Tree.
/// Demonstrates the mechanism with a seeded code-finetune dataset/skill and
/// the local model's inferred base capabilities.
fn agent_skill(config_path: &std::path::Path, model_override: Option<&str>) -> Result<()> {
    use decentraai_agents::{build_agent_capabilities, demo_skill_registry};
    use decentraai_hub::capability::{CapabilityClaim, CapabilityKind, Provenance};

    // Single source of truth for the demo data (shared with the runtime
    // /v1/skills view) — never duplicated.
    let registry = demo_skill_registry();

    // Resolve the served model (override or default local agents' model).
    let model_name = match model_override {
        Some(m) => m.to_string(),
        None => load_local_agents(config_path)?
            .into_iter()
            .find_map(|a| a.allowed_models.first().cloned())
            .unwrap_or_else(|| "unknown".to_string()),
    };

    // Infer the model's base capabilities from its file name (honest,
    // INFERRED): "coder" → Coding; otherwise a general chat model.
    let lower = model_name.to_ascii_lowercase();
    let mut base = vec![
        CapabilityClaim {
            capability: CapabilityKind::Chat,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::TextGeneration,
            provenance: Provenance::Inferred,
        },
    ];
    if lower.contains("coder") {
        base.push(CapabilityClaim {
            capability: CapabilityKind::Coding,
            provenance: Provenance::Inferred,
        });
        base.push(CapabilityClaim {
            capability: CapabilityKind::Reasoning,
            provenance: Provenance::Inferred,
        });
    }

    println!("Dataset/skill layer (P8 dataset -> capabilities -> talents):");
    println!("  datasets:");
    for d in registry.datasets() {
        println!(
            "    - {} [{}] develops: {} (provenance {:?}, {:.1} GiB)",
            d.id,
            d.name,
            d.develops
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(", "),
            d.provenance,
            d.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        );
    }
    println!("  skills:");
    for s in registry.skills() {
        println!(
            "    - {} [{}] dataset={} requires_model={:?} develops: {}",
            s.id,
            s.name,
            s.dataset_id,
            s.requires_model.map(|k| k.label()),
            s.develops
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    println!("  model: {model_name}");
    let build = build_agent_capabilities(base.clone(), &registry);
    println!(
        "  base capabilities: {}",
        base.iter()
            .map(|c| c.capability.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if build.unlocked.is_empty() {
        println!("  skills unlocked: (none — model does not satisfy skill prerequisites)");
    } else {
        println!(
            "  skills unlocked: {}",
            build
                .unlocked
                .iter()
                .map(|c| c.capability.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("  (demonstration data — register real datasets/skills to drive agent evolution)");
    Ok(())
}

/// Lists the persistent dataset/skill registry (db/skills.json).
fn skill_list(config_path: &Path) -> Result<()> {
    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let path = data_dir.join("db/skills.json");
    let registry = load_skill_registry(&path);
    println!("Dataset/skill registry ({}):", path.display());
    if registry.datasets().is_empty() && registry.skills().is_empty() {
        println!("  (empty — register a dataset+skill with `decentraai agent skill add`)");
        return Ok(());
    }
    for d in registry.datasets() {
        println!(
            "  dataset: {} [{}] develops: {} · provenance {:?} · source {}",
            d.id,
            d.name,
            d.develops
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(", "),
            d.provenance,
            d.source,
        );
    }
    for s in registry.skills() {
        println!(
            "  skill:   {} [{}] dataset={} requires_model={:?} unlocks: {}",
            s.id,
            s.name,
            s.dataset_id,
            s.requires_model.map(|k| k.label()),
            s.develops
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(())
}

/// Registers a real dataset + a skill into the persistent registry. The skill
/// must only unlock capabilities its dataset develops (integrity invariant).
/// The node applies the persistent registry to its agent on restart.
#[allow(clippy::too_many_arguments)] // clap CLI flags, not a domain signature
fn skill_add(
    config_path: &Path,
    dataset_id: &str,
    name: &str,
    source: &str,
    kind: &str,
    develops: &str,
    provenance: &str,
    skill_id: &str,
    requires_model: &str,
    unlock: &str,
) -> Result<()> {
    use decentraai_agents::{DatasetDescriptor, DatasetKind, SkillDescriptor};
    use decentraai_hub::capability::{CapabilityKind, Provenance};
    use std::str::FromStr;

    let config = NodeConfig::load(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    let data_dir = expand_tilde(&config.node.data_dir);
    let path = data_dir.join("db/skills.json");
    let mut registry = load_skill_registry(&path);

    // Parse capability lists (comma-separated snake_case).
    let parse_caps = |s: &str| -> Result<Vec<CapabilityKind>> {
        s.split(',')
            .map(|c| {
                CapabilityKind::from_str(c.trim())
                    .map_err(|_| anyhow::anyhow!("unknown capability '{c}'"))
            })
            .collect()
    };
    let develops_kinds = parse_caps(develops)?;
    let unlock_kinds = parse_caps(unlock)?;
    let requires = if requires_model.trim().is_empty() {
        None
    } else {
        Some(
            CapabilityKind::from_str(requires_model.trim())
                .map_err(|_| anyhow::anyhow!("unknown requires_model '{requires_model}'"))?,
        )
    };
    let prov = match provenance.trim().to_ascii_lowercase().as_str() {
        "verified" => Provenance::Verified,
        "inferred" => Provenance::Inferred,
        other => anyhow::bail!("unknown provenance '{other}' (use verified|inferred)"),
    };
    let kind_enum = match kind.trim().to_ascii_lowercase().as_str() {
        "training" => DatasetKind::Training,
        "fine_tune" => DatasetKind::FineTune,
        "knowledge_base" => DatasetKind::KnowledgeBase,
        "benchmarks" => DatasetKind::Benchmarks,
        other => anyhow::bail!("unknown dataset kind '{other}'"),
    };

    registry
        .add_dataset(
            DatasetDescriptor::new(dataset_id, name, develops_kinds, kind_enum)
                .from(source)
                .with_provenance(prov),
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    registry
        .add_skill(SkillDescriptor::new(
            skill_id,
            format!("{name} skill"),
            dataset_id,
            requires,
            unlock_kinds,
        ))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    save_skill_registry(&path, &registry)?;
    println!(
        "Registered dataset '{dataset_id}' + skill '{skill_id}' in {}",
        path.display()
    );
    println!(
        "Restart the node (systemctl --user restart decentraai-node) for the agent to apply these capabilities."
    );
    Ok(())
}
/// node's identity short id and the model it serves — this is the node's
/// own "generalist" agent plus, when a model is present, a model-tied
/// executor. Provenance stays honest: without Hub metadata the LLM claims
/// are INFERRED, never VERIFIED (a claim the node cannot back is not
/// claimed as verified).
///
/// This is the shared builder used by both the running node and the
/// `agent list` CLI so the two never disagree about what is advertised.
fn default_local_agents(
    short_id: &str,
    node_name: &str,
    model_hash: &str,
    allow_remote: bool,
    model_name: &str,
    skills: &decentraai_agents::SkillRegistry,
    has_retrieval: bool,
) -> Vec<decentraai_agents::AgentRecord> {
    use decentraai_agents::{
        AgentPolicies, AgentRecord, AgentState, ROLE_GENERALIST, build_agent_capabilities,
    };
    use decentraai_hub::capability::{CapabilityKind, Provenance};

    let mut agents = Vec::new();
    // Base capabilities are derived from the actually-served model (runtime
    // wiring); all INFERRED. Then real skills from the persistent registry are
    // applied: a skill only unlocks capabilities its dataset develops (with
    // the dataset's provenance). The demo registry is never applied here.
    let base = model_base_capabilities(model_name);
    let build = build_agent_capabilities(base, skills);
    let mut generalist = AgentRecord::new(
        format!("{short_id}:generalist"),
        format!("{node_name} Generalist"),
        ROLE_GENERALIST,
    )
    .described("chat, reasoning and text generation on this node");
    for c in build.all() {
        generalist = generalist.with_capability(c.capability, c.provenance);
    }
    // RAG: a node with a configured embeddings backend can perform semantic
    // retrieval, so its agent honestly claims the Retrieval capability.
    if has_retrieval {
        generalist = generalist.with_capability(CapabilityKind::Retrieval, Provenance::Inferred);
    }
    if !model_hash.is_empty() {
        generalist = generalist.with_model(model_hash);
    }
    if allow_remote {
        generalist = generalist.with_policies(AgentPolicies {
            allow_remote: true,
            ..AgentPolicies::default()
        });
    }
    generalist.set_state(AgentState::Ready);
    agents.push(generalist);
    agents
}

/// Honest base capabilities of a served model, derived from its file name
/// (all INFERRED — a name cannot back a VERIFIED claim). "coder" models claim
/// Coding (plus the general chat/reasoning set); anything else claims the
/// general set only.
fn model_base_capabilities(model_name: &str) -> Vec<decentraai_hub::capability::CapabilityClaim> {
    use decentraai_hub::capability::{CapabilityClaim, CapabilityKind, Provenance};
    let lower = model_name.to_ascii_lowercase();
    let mut caps = vec![
        CapabilityClaim {
            capability: CapabilityKind::Chat,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::TextGeneration,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::Reasoning,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::DocumentUnderstanding,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::Summarization,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::Classification,
            provenance: Provenance::Inferred,
        },
        CapabilityClaim {
            capability: CapabilityKind::StructuredOutput,
            provenance: Provenance::Inferred,
        },
    ];
    if lower.contains("coder") {
        caps.push(CapabilityClaim {
            capability: CapabilityKind::Coding,
            provenance: Provenance::Inferred,
        });
    }
    caps
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
        decentraai_distributed::InferenceConfig::from_section(&config.inference),
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
    // P1 (Collective Intelligence): this node's logical agents, advertised
    // with signed capability claims (same anti-spoof discipline as compute).
    let mut agent_manager = Arc::new(decentraai_distributed::agents::AgentManager::new(
        local_peer_id,
        args.name.clone(),
    ));
    if let Some(am) = Arc::get_mut(&mut agent_manager) {
        am.set_signing_key(identity.signing_key_bytes());
    }
    let short_id = decentraai_distributed::short_node_id(&local_peer_id);
    agent_manager.set_local_agents(default_local_agents(
        &short_id,
        &args.name,
        model_hash.as_deref().unwrap_or(""),
        config.inference.allow_remote_inference,
        model_name.as_deref().unwrap_or(""),
        &decentraai_agents::SkillRegistry::new(),
        false,
    ));
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
    // P14 Phase Q: persistent credit ledger + node-local contribution state
    // (db/credits.json, db/contribution.json) — balances, idempotency and
    // lifetime projections survive node restarts (storage separation:
    // HOT state in memory, HISTORY on disk).
    compute_manager.set_credits_path(Some(data_dir.join("db/credits.json")));
    compute_manager.set_contribution_path(Some(data_dir.join("db/contribution.json")));
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
    distributed_handler.set_agent_manager(agent_manager.clone());
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

    let bound = p2p_node.listen("/ip4/0.0.0.0/tcp/32937").await?;

    // Create distributed inference coordinator with the shared worker manager
    let distributed_config = InferenceConfig::from_section(&config.inference);
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
            .advertise_local(
                snapshot,
                gpu,
                served_models,
                available_models,
                can_provision,
            )
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
    // P1: keep the fabric's agent view fresh (agent advertisements change
    // rarely, but the periodic beat enforces staleness of remote views).
    spawn_agent_broadcaster(agent_manager.clone(), distributed.p2p_node().clone());

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

/// P1 (Collective Intelligence): periodically broadcast this node's logical
/// agents and prune stale remote agent views. Agent advertisements change
/// rarely, but the periodic beat keeps the fabric's agent view fresh and
/// enforces staleness — mirroring the compute advertisement heartbeat.
fn spawn_agent_broadcaster(
    agent_manager: Arc<decentraai_distributed::agents::AgentManager>,
    p2p_node: decentraai_p2p::P2PNode,
) {
    use decentraai_compute::DEFAULT_ADVERTISEMENT_INTERVAL_MS;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            DEFAULT_ADVERTISEMENT_INTERVAL_MS,
        ));
        loop {
            interval.tick().await;
            match agent_manager.advertisement_wire_bytes() {
                Ok(bytes) => p2p_node.announce(bytes),
                Err(e) => tracing::warn!(error = %e, "failed to build agent advertisement"),
            }
            // Expire remote agent views that have not refreshed (pure
            // bookkeeping — never touches trust or reputation).
            let stale =
                std::time::Duration::from_millis(decentraai_compute::DEFAULT_STALE_AFTER_MS);
            let evicted = agent_manager.prune_stale(stale);
            if evicted > 0 {
                tracing::debug!(evicted, "pruned stale remote agent views");
            }
        }
    });
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
                // A request error/timeout counts as a *lost* probe (M9 P2): it
                // contributes to the packet-loss derivation but yields no RTT.
                // Best-effort: a busy worker may drop the ping; we just record
                // the lost sample and keep probing.
                if p2p_node.request(peer, bytes).await.is_ok() {
                    let rtt_us = start.elapsed().as_micros() as u64;
                    compute_manager.record_rtt_sample(&peer, rtt_us, 0, false);
                    let link = compute_manager.network_graph().get(&peer.to_string());
                    info!(
                        peer = %peer,
                        measured_rtt_us = rtt_us,
                        jitter_us = ?link.jitter_us,
                        packet_loss_percent = link.packet_loss_percent,
                        graph_rtt_us = link.rtt_us,
                        graph_locality = ?link.locality,
                        graph_peers = compute_manager.network_graph().measured_len(),
                        "M19 network probe: measured RTT recorded, planner reads via NetworkGraph"
                    );
                } else {
                    compute_manager.record_rtt_sample(&peer, 0, 0, true);
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
    use decentraai_hub::capability::{CapabilityKind, Provenance};

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
        let cli = Cli::try_parse_from(["decentraai", "model", "search", "mistral", "--categories"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Command::Model {
                command: ModelCommand::Search {
                    categories: true,
                    ..
                }
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
        assert!(matches!(
            cli.command,
            Command::Doctor(DoctorArgs { online: true, .. })
        ));
    }

    #[test]
    fn parses_doctor_without_online_flag() {
        let cli = Cli::try_parse_from(["decentraai", "doctor"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Doctor(DoctorArgs { online: false, .. })
        ));
    }

    #[test]
    fn base_api_addr_maps_bind_and_port() {
        assert_eq!(
            base_api_addr("127.0.0.1", 8080),
            Some("127.0.0.1:8080".into())
        );
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
                        backend: Some(url), ..
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
        assert_eq!(
            addr, multiaddr,
            "path multiaddr must round-trip (no spaces in it)"
        );
        assert_eq!(token, "dsk_abc123def456");
    }

    #[test]
    fn invite_parsing_rejects_malformed_strings() {
        assert!(
            parse_invite("/ip4/10.0.0.5/tcp/4001").is_err(),
            "missing token"
        );
        assert!(
            parse_invite("/ip4/10.0.0.5/tcp/4001 xyz_token").is_err(),
            "bad token prefix"
        );
        assert!(parse_invite("   dsk_xyz").is_err(), "empty multiaddr");
    }

    #[test]
    fn invite_peer_id_is_libp2p_not_identity_hex() {
        // Regression: `decentraai invite` once printed the identity's raw hex
        // id (64 chars) in the multiaddr, which libp2p cannot parse — the
        // fresh node's reachability check failed with "invalid dial address /
        // Invalid base string". The invite must derive the libp2p peer id
        // (base58, 12D3KooW...) from the node key, exactly like the swarm.
        use libp2p::PeerId as Libp2pPeerId;
        use libp2p::identity::Keypair as Libp2pKeypair;
        let identity = Identity::generate();
        let keypair = Libp2pKeypair::ed25519_from_bytes(identity.signing_key_bytes())
            .expect("node key must derive an ed25519 libp2p keypair");
        let peer_id = Libp2pPeerId::from(keypair.public());
        let s = peer_id.to_string();
        assert!(
            s.starts_with("12D3KooW"),
            "expected a base58 libp2p peer id, got: {s}"
        );
        assert!(
            !s.chars().all(|c| c.is_ascii_hexdigit()),
            "raw identity hex must never be used in an invite multiaddr"
        );
        // The fully-qualified multiaddr must parse as a libp2p dial target.
        let multiaddr: libp2p::Multiaddr = format!("/ip4/10.0.0.5/tcp/4001/p2p/{peer_id}")
            .parse()
            .expect("invite multiaddr must be a valid libp2p multiaddr");
        assert!(multiaddr.to_string().contains("12D3KooW"));
    }

    #[test]
    fn invite_token_is_a_least_privilege_guest_seat() {
        // Mirrors the token call the `invite` command performs: a fresh seat is
        // always Tier 1 (Guest) and stored only as a hash, so an invite leak is
        // never more than a guest — the least privilege roadmap (P5) guarantee.
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            decentraai_tokens::TokenStore::load(&dir.path().join("tokens.json")).unwrap();
        let plaintext = store
            .create("invite-0", decentraai_tokens::Tier::GUEST, None)
            .unwrap();
        assert_eq!(store.lookup(&plaintext).unwrap().tier, 1);
        let on_disk = std::fs::read_to_string(dir.path().join("tokens.json")).unwrap();
        assert!(
            !on_disk.contains(&plaintext),
            "plaintext must never be persisted"
        );
    }

    // ---- `decentraai agent` inspection subcommands (Show/Workflow/Reputation/TalentTree) ----

    #[test]
    fn agent_show_resolves_an_agent_by_id_from_the_default_set() {
        let agents = default_local_agents(
            "dca-test",
            "TestNode",
            "",
            false,
            "",
            &decentraai_agents::SkillRegistry::new(),
            false,
        );
        let generalist = find_agent_by_id(&agents, "dca-test:generalist")
            .expect("generalist agent must be present in the default set");
        assert_eq!(generalist.role, decentraai_agents::ROLE_GENERALIST);
        assert_eq!(generalist.name, "TestNode Generalist");
        // A missing id resolves to None (caller turns that into an error).
        assert!(find_agent_by_id(&agents, "dca-test:nope").is_none());
    }

    #[test]
    fn agent_workflow_returns_the_research_report_template_steps() {
        let (id, template) = workflow_templates()
            .into_iter()
            .find(|(id, _)| *id == "research_report")
            .expect("research_report template is registered");
        assert_eq!(id, "research_report");
        assert!(template.synthesis, "research_report synthesizes");
        let ids: Vec<&str> = template.steps.iter().map(|s| s.step_id.as_str()).collect();
        assert_eq!(ids, vec!["research", "finance", "documents"]);
        // finance and documents both depend on research.
        let finance = template
            .steps
            .iter()
            .find(|s| s.step_id == "finance")
            .unwrap();
        assert_eq!(finance.depends_on, vec!["research".to_string()]);
        assert_eq!(template.validate(), Ok(()));
    }

    #[test]
    fn agent_reputation_produces_a_meaningful_score() {
        let agent = "dca-test:generalist";
        let store = sample_reputation_store(agent, "chat", 3);
        let rep = store.get(agent, "chat").expect("reputation seeded");
        // Every factor has 3 samples, so at min_samples = 3 all are meaningful.
        assert!(rep.is_meaningful(3));
        assert_eq!(rep.reasons().len(), 5);
        let score = aggregate_score(rep, 3);
        assert!(
            score > 0.0,
            "aggregate of positive factors must be positive, got {score}"
        );
        assert!(score <= 1.0);
        // At a higher floor than any sample count, nothing is meaningful -> 0.
        assert_eq!(aggregate_score(rep, 4), 0.0);
    }

    #[test]
    fn agent_talent_tree_availability_filter_works() {
        let tree = decentraai_agents::seed_talent_tree();
        let have = parse_have_capabilities("embeddings,coding,not_a_cap");
        assert_eq!(
            have,
            vec![
                decentraai_hub::capability::CapabilityKind::Embeddings,
                decentraai_hub::capability::CapabilityKind::Coding,
            ],
            "unknown capabilities are skipped with a warning"
        );
        // With Embeddings + Coding held, the one-hop frontier under a 2048 MiB
        // budget is the leaves plus Retrieval and Summarization. FunctionCalling
        // needs ToolCalling (not held), so it stays locked.
        let available = tree.available_capabilities(&have, 2048);
        for kind in [
            decentraai_hub::capability::CapabilityKind::Embeddings,
            decentraai_hub::capability::CapabilityKind::Coding,
            decentraai_hub::capability::CapabilityKind::Retrieval,
            decentraai_hub::capability::CapabilityKind::Summarization,
        ] {
            assert!(
                available.contains(&kind),
                "missing {kind:?} in {available:?}"
            );
        }
        assert!(!available.contains(&decentraai_hub::capability::CapabilityKind::FunctionCalling));
        // DocumentUnderstanding (4096 MiB) is out of budget.
        assert!(
            !available.contains(&decentraai_hub::capability::CapabilityKind::DocumentUnderstanding)
        );
        // Target resolution: reach Retrieval starting from Embeddings. The path
        // may pick up other unlocked leaves first, but must end at Retrieval.
        let path = tree.resolve_path(
            decentraai_hub::capability::CapabilityKind::Retrieval,
            &[decentraai_hub::capability::CapabilityKind::Embeddings],
        );
        assert_eq!(
            path.last(),
            Some(&decentraai_hub::capability::CapabilityKind::Retrieval)
        );

        // CLI parsing: the new inspection subcommands accept their args.
        let cli = Cli::try_parse_from(["decentraai", "agent", "show", "--agent", "x:generalist"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Show { .. }
            }
        ));
        let cli = Cli::try_parse_from(["decentraai", "agent", "workflow"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Workflow { .. }
            }
        ));
        let cli = Cli::try_parse_from([
            "decentraai",
            "agent",
            "reputation",
            "--agent",
            "x:generalist",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::Reputation { .. }
            }
        ));
        let cli =
            Cli::try_parse_from(["decentraai", "agent", "talent-tree", "--have", "embeddings"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Command::Agent {
                command: AgentCommand::TalentTree { .. }
            }
        ));
    }

    // ---- node.model resolution (pivot on local GGUF models) ----

    #[test]
    fn explicit_model_wins_over_auto_detect() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.gguf"), b"a").unwrap();
        std::fs::write(dir.path().join("B.gguf"), b"b").unwrap();
        // Explicit name is served even though "A.gguf" would auto-detect first.
        assert_eq!(
            resolve_model_name(dir.path(), Some("B.gguf")).unwrap(),
            "B.gguf"
        );
    }

    #[test]
    fn explicit_missing_model_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_model_name(dir.path(), Some("Nope.gguf")).unwrap_err();
        assert!(err.to_string().contains("Nope.gguf"));
    }

    #[test]
    fn explicit_blank_model_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_model_name(dir.path(), Some("   ")).unwrap(), "");
    }

    #[test]
    fn no_explicit_model_auto_detects_first_sorted_gguf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("B.gguf"), b"b").unwrap();
        std::fs::write(dir.path().join("A.gguf"), b"a").unwrap();
        std::fs::write(dir.path().join("junk.txt"), b"j").unwrap();
        assert_eq!(resolve_model_name(dir.path(), None).unwrap(), "A.gguf");
    }

    #[test]
    fn empty_dir_yields_no_model() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_model_name(dir.path(), None).unwrap(), "");
    }

    #[test]
    fn model_base_capabilities_are_honest_and_model_aware() {
        // A coding model claims Coding (INFERRED — a name cannot back VERIFIED).
        let coding = model_base_capabilities("qwen2.5-coder-7b-instruct-q4_k_m.gguf");
        assert!(coding.iter().any(
            |c| c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
        ));
        // A general model claims only the general set (no Coding invented).
        let general = model_base_capabilities("Llama-3.2-1B-Instruct-Q4_K_M.gguf");
        assert!(
            !general
                .iter()
                .any(|c| c.capability == CapabilityKind::Coding)
        );
        assert!(general.iter().any(|c| c.capability == CapabilityKind::Chat));
        // No capability is ever claimed VERIFIED from a model name.
        assert!(coding.iter().all(|c| c.provenance == Provenance::Inferred));
    }

    #[test]
    fn default_local_agents_reflect_the_served_model() {
        // Runtime wiring: the generalist agent advertises the served model's
        // base capabilities. A coding model -> Coding claim.
        let agents = default_local_agents(
            "dca-test",
            "Node",
            "hash123",
            false,
            "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
            &decentraai_agents::SkillRegistry::new(),
            false,
        );
        let g = agents
            .iter()
            .find(|a| a.agent_id == "dca-test:generalist")
            .unwrap();
        assert!(g.has_capability(CapabilityKind::Coding));
        assert!(g.has_model("hash123"));
        // A general model -> no Coding.
        let gens = default_local_agents(
            "dca-test",
            "Node",
            "",
            false,
            "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
            &decentraai_agents::SkillRegistry::new(),
            false,
        );
        let gg = gens
            .iter()
            .find(|a| a.agent_id == "dca-test:generalist")
            .unwrap();
        assert!(!gg.has_capability(CapabilityKind::Coding));
    }
}
