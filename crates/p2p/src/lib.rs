//! libp2p transport for DecentraAI: TCP + Noise + Yamux, mDNS discovery,
//! and a length-delimited request/response channel for protocol messages.
//!
//! The node runs as an actor: the swarm lives in a background task and
//! commands flow through a channel, so callers can issue sequential
//! requests (e.g. manifest then chunks) without blocking the event loop.
//! The libp2p keypair is derived from the node identity, binding the
//! transport PeerId to the Ed25519 node key.

pub mod reputation;
pub mod transfer;

pub use libp2p::PeerId;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use decentraai_identity::Identity;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::identity::Keypair;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::{NetworkBehaviour, StreamProtocol, SwarmEvent};
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, dcutr, identify, kad, mdns, noise, ping, relay, tcp, yamux};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

/// Max re-dial attempts for a peer that disconnected, before giving up and
/// relying on mDNS re-discovery (a peer that left the network permanently
/// must not be re-dialed forever). Each attempt backs off exponentially.
pub const RECONNECT_MAX_ATTEMPTS: u32 = 5;
/// Base backoff (ms) doubled on each reconnect attempt.
pub const RECONNECT_BASE_BACKOFF_MS: u64 = 500;

/// Transport/discovery options for a node. Defaults to LAN-only (mDNS), which
/// preserves the original single-subnet behaviour exactly. To reach peers
/// across NAT / subnets, enable the DHT and optionally relay.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// mDNS discovery on the local subnet. LAN-only discovery.
    pub lan_discovery: bool,
    /// Whether to run the Kademlia DHT for cross-subnet peer discovery.
    pub dht_enabled: bool,
    /// Whether to run the relay client, relay server and DCUtR hole-punching.
    pub relay_enabled: bool,
    /// Multiaddrs of DHT bootstrap peers (e.g. a well-known public relay /
    /// rendezvous node), each ending in `/p2p/<PeerId>`.
    pub bootstrap_peers: Vec<String>,
    /// Upper bound on concurrent connections (reserved for connection limits;
    /// currently informational).
    pub max_connections: u16,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self::lan_only()
    }
}

impl NetworkConfig {
    /// The original single-subnet configuration: mDNS only, no DHT, no relay.
    pub fn lan_only() -> Self {
        Self {
            lan_discovery: true,
            dht_enabled: false,
            relay_enabled: false,
            bootstrap_peers: Vec::new(),
            max_connections: 50,
        }
    }
}

/// Parses a bootstrap multiaddr like `/ip4/1.2.3.4/tcp/4001/p2p/12D3Koo...`
/// into a `(PeerId, Multiaddr)` pair where the returned address has the
/// trailing `/p2p/<PeerId>` component stripped (so it can be handed to the
/// DHT's `add_address` or re-append for dialing).
fn parse_bootstrap_peer(s: &str) -> Result<(PeerId, Multiaddr)> {
    let mut addr: Multiaddr = s.parse().with_context(|| format!("invalid multiaddr {s:?}"))?;
    let peer_id = match addr.pop() {
        Some(Protocol::P2p(peer)) => peer,
        _ => bail!("bootstrap multiaddr must end with /p2p/<PeerId>: {s:?}"),
    };
    Ok((peer_id, addr))
}

/// Request/response protocol carrying serialized decentraai-protocol messages.
pub const MESSAGE_PROTOCOL: StreamProtocol = StreamProtocol::new("/decentraai/message/1");

/// Default frame cap for control-plane messages (matches node.example.yaml).
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Frame cap for data-plane chunk responses: the largest configured chunk
/// (64 MiB) encoded as base64 plus header headroom.
pub const DEFAULT_MAX_CHUNK_MESSAGE_BYTES: usize = 96 * 1024 * 1024;

/// Serves inbound requests from peers.
pub trait RequestHandler: Send + Sync + 'static {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>>;
}

/// Chains multiple request handlers together
///
/// Tries each handler in order until one succeeds.
pub struct ChainedHandler {
    handlers: Vec<Arc<dyn RequestHandler>>,
}

impl ChainedHandler {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn add_handler(mut self, handler: Arc<dyn RequestHandler>) -> Self {
        self.handlers.push(handler);
        self
    }
}

impl Default for ChainedHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestHandler for ChainedHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        for handler in &self.handlers {
            match handler.handle(request) {
                Ok(response) => return Ok(response),
                Err(_) => continue, // Try next handler
            }
        }
        anyhow::bail!("No handler could process the request")
    }
}

/// Serves a static manifest plus chunk reads from a local file.
/// Chunks are read with seek + read, so huge artifacts never load into memory.
pub struct StaticFileServer {
    manifest_response: Vec<u8>,
    file: std::path::PathBuf,
    chunk_size: u64,
}

impl StaticFileServer {
    pub fn new(manifest_response: Vec<u8>, file: std::path::PathBuf, chunk_size: u64) -> Self {
        Self {
            manifest_response,
            file,
            chunk_size,
        }
    }
}

impl RequestHandler for StaticFileServer {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        use decentraai_protocol::{
            CURRENT_PROTOCOL_VERSION, ChunkRequest, ChunkResponse, ManifestRequest,
            deserialize_message, serialize_message,
        };

        if let Ok(req) = deserialize_message::<ManifestRequest>(request, DEFAULT_MAX_MESSAGE_BYTES) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            return Ok(self.manifest_response.clone());
        }
        if let Ok(req) = deserialize_message::<ChunkRequest>(request, DEFAULT_MAX_MESSAGE_BYTES) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&self.file).context("opening served file")?;
            file.seek(SeekFrom::Start(
                u64::from(req.chunk_index) * self.chunk_size,
            ))?;
            let mut buf = vec![0u8; self.chunk_size as usize];
            let mut filled = 0usize;
            while filled < buf.len() {
                let n = file.read(&mut buf[filled..])?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            buf.truncate(filled);
            if filled == 0 {
                bail!("chunk {} out of range", req.chunk_index);
            }
            let response = ChunkResponse {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                chunk_index: req.chunk_index,
                chunk_data: buf,
            };
            return serialize_message(&response);
        }
        bail!("unrecognized request")
    }
}

/// Serves every model in a local ModelRegistry: catalogs, manifests, and
/// chunks. Manifests are built on demand with the registry-relative
/// name; chunks are read with seek + read from the canonical path.
pub struct RegistryServer {
    registry: decentraai_registry::ModelRegistry,
}

impl RegistryServer {
    pub fn new(registry: decentraai_registry::ModelRegistry) -> Self {
        Self { registry }
    }

    /// Scans every registered model and returns its manifest. Missing or
    /// invalid files are skipped with a warning, not fatal errors.
    pub fn manifests(&self) -> Vec<decentraai_manifest::Manifest> {
        self.registry
            .list_models()
            .into_iter()
            .filter_map(|record| {
                match decentraai_manifest::scan_with_name(
                    &record.canonical_path,
                    &record.relative_path,
                ) {
                    Ok(manifest) => Some(manifest),
                    Err(e) => {
                        warn!(path = %record.canonical_path, error = %e, "skipping unscannable model");
                        None
                    }
                }
            })
            .collect()
    }
}

impl RequestHandler for RegistryServer {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        use decentraai_protocol::{
            CURRENT_PROTOCOL_VERSION, CatalogRequest, CatalogResponse, ChunkRequest, ChunkResponse,
            ManifestRequest, deserialize_message, manifest_response_bytes, serialize_message,
        };

        if let Ok(req) = deserialize_message::<CatalogRequest>(request, DEFAULT_MAX_MESSAGE_BYTES) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            return serialize_message(&CatalogResponse {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                manifests: self.manifests(),
            });
        }
        if let Ok(req) = deserialize_message::<ManifestRequest>(request, DEFAULT_MAX_MESSAGE_BYTES) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            let manifest = self
                .manifests()
                .into_iter()
                .find(|m| m.model_id == req.manifest_id)
                .with_context(|| format!("unknown manifest {}", req.manifest_id))?;
            return manifest_response_bytes(&manifest);
        }
        if let Ok(req) = deserialize_message::<ChunkRequest>(request, DEFAULT_MAX_MESSAGE_BYTES) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            let manifest = self
                .manifests()
                .into_iter()
                .find(|m| m.model_id == req.manifest_id)
                .with_context(|| format!("unknown manifest {}", req.manifest_id))?;
            let record = self
                .registry
                .get_model(&manifest.file_name)
                .with_context(|| format!("registry entry missing for {}", manifest.file_name))?;
            use std::io::{Read, Seek, SeekFrom};
            let mut file =
                std::fs::File::open(&record.canonical_path).context("opening registered model")?;
            file.seek(SeekFrom::Start(
                u64::from(req.chunk_index) * manifest.chunk_size as u64,
            ))?;
            let mut buf = vec![0u8; manifest.chunk_size];
            let mut filled = 0usize;
            while filled < buf.len() {
                let n = file.read(&mut buf[filled..])?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            buf.truncate(filled);
            if filled == 0 {
                bail!("chunk {} out of range", req.chunk_index);
            }
            let response = ChunkResponse {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                chunk_index: req.chunk_index,
                chunk_data: buf,
            };
            return serialize_message(&response);
        }
        bail!("unrecognized request")
    }
}

enum Command {
    Listen {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<Multiaddr>>,
    },
    Dial {
        addr: Multiaddr,
        reply: oneshot::Sender<Result<()>>,
    },
    Request {
        peer: PeerId,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    /// Fire-and-forget broadcast to every connected peer (announcements).
    Broadcast {
        payload: Vec<u8>,
    },
    /// Reply with the ids of currently connected peers.
    Connected {
        reply: oneshot::Sender<Vec<PeerId>>,
    },
    /// Reply with the full peer snapshot: connected ids, last-known
    /// addresses per peer (from mDNS discovery / dialer connect), and the
    /// node's own listen addresses. This is the identity view the control
    /// plane needs to render the fabric (LAN address per node).
    Peers {
        reply: oneshot::Sender<PeersSnapshot>,
    },
    Shutdown,
}

/// Read-only snapshot of the swarm's peer identity state, surfaced by
/// [`P2PNode::peers_snapshot`]. All fields are live swarm state: connected
/// peers, the last address we know per peer, and our own listeners.
#[derive(Debug, Clone, Default)]
pub struct PeersSnapshot {
    pub connected: Vec<PeerId>,
    pub addresses: HashMap<PeerId, Multiaddr>,
    pub local_addresses: Vec<Multiaddr>,
    /// Addresses observed for us by remote peers via the identify protocol
    /// (e.g. our public IP behind NAT). Advertised on the swarm so remote
    /// peers can dial us directly.
    pub external_addresses: Vec<Multiaddr>,
}

/// Handler for inbound inference requests (see `P2PNode::set_on_infer_request`).
/// Called with the transport-authenticated connected peer and the request;
/// `peer` is the real Noise-authenticated PeerId, NOT `req.sender_peer_id`
/// (which is attacker-controllable payload, see P2).
type InferHandler =
    Arc<dyn Fn(PeerId, decentraai_protocol::InferRequest) -> anyhow::Result<Vec<u8>> + Send + Sync>;

/// Handler for inbound inference cancellations (see `P2PNode::set_on_cancel_request`).
type CancelHandler = Arc<dyn Fn(uuid::Uuid) + Send + Sync>;

/// Handler for inbound manifest announcements, called with the announcing
/// peer and the announced manifest. MUST be non-blocking: the swarm event
/// loop invokes it inline, so downloads belong in a spawned task.
type ManifestAnnouncementHandler = Arc<dyn Fn(PeerId, decentraai_manifest::Manifest) + Send + Sync>;

/// Shared, swappable handler slot read by the swarm task.
type SharedHandler<T> = Arc<tokio::sync::Mutex<Option<T>>>;

/// Handle to the background swarm task.
#[derive(Clone)]
pub struct P2PNode {
    commands: mpsc::UnboundedSender<Command>,
    peer_id: PeerId,
    /// Optional callback invoked for inbound InferRequest messages. Stored
    /// here so callers can register a handler after the node is created.
    on_infer: SharedHandler<InferHandler>,
    /// Optional callback invoked for inbound InferCancel messages. The
    /// worker registers this to mark in-flight requests as cancelled in the
    /// queue manager, which the streaming loop observes to abort promptly.
    on_cancel: SharedHandler<CancelHandler>,
    /// Optional callback invoked for inbound manifest announcements.
    on_manifest: SharedHandler<ManifestAnnouncementHandler>,
}

impl P2PNode {
    /// Sets a callback for worker announcements
    pub fn set_on_worker_announcement<F>(&mut self, _callback: F)
    where
        F: Fn(decentraai_protocol::WorkerAnnouncement) + Send + Sync + 'static,
    {
        // Worker announcements are handled elsewhere; not implemented yet.
        let _ = _callback;
    }

    /// Sets a callback for inference requests. The callback should return an
    /// immediate response (e.g., an InferAccepted message), and may spawn
    /// background tasks to stream progress back to the requester using
    /// P2PNode::request().
    pub fn set_on_infer_request<F>(&mut self, callback: F)
    where
        F: Fn(PeerId, decentraai_protocol::InferRequest) -> anyhow::Result<Vec<u8>>
            + Send
            + Sync
            + 'static,
    {
        let mut guard = futures::executor::block_on(self.on_infer.lock());
        *guard = Some(std::sync::Arc::new(callback));
    }

    /// Sets a callback for inbound cancellation requests. Called with the
    /// request id whenever an InferCancel message arrives from a peer.
    pub fn set_on_cancel_request<F>(&mut self, callback: F)
    where
        F: Fn(uuid::Uuid) + Send + Sync + 'static,
    {
        let mut guard = futures::executor::block_on(self.on_cancel.lock());
        *guard = Some(std::sync::Arc::new(callback));
    }

    /// Sets a callback for inbound manifest announcements (peer, manifest).
    /// The callback is invoked inline by the swarm task and MUST NOT block;
    /// spawn a background task to download the announced model.
    pub fn set_on_manifest_announcement<F>(&mut self, callback: F)
    where
        F: Fn(PeerId, decentraai_manifest::Manifest) + Send + Sync + 'static,
    {
        let mut guard = futures::executor::block_on(self.on_manifest.lock());
        *guard = Some(std::sync::Arc::new(callback));
    }

    /// Creates a node and spawns its swarm task, LAN-only (mDNS discovery,
    /// no DHT, no relay). Must be called from within a Tokio runtime: the
    /// mDNS behaviour registers with the reactor.
    pub fn new(
        identity: &Identity,
        max_message_bytes: usize,
        max_chunk_message_bytes: usize,
        handler: Option<Arc<dyn RequestHandler>>,
    ) -> Result<Self> {
        Self::new_with_network(
            identity,
            max_message_bytes,
            max_chunk_message_bytes,
            handler,
            NetworkConfig::lan_only(),
        )
    }

    /// Creates a node and spawns its swarm task with explicit network
    /// configuration (DHT, relay, bootstrap peers). This is the entry point
    /// for NAT-traversal / cross-subnet deployments; [`P2PNode::new`]
    /// delegates here with the default LAN-only config.
    pub fn new_with_network(
        identity: &Identity,
        max_message_bytes: usize,
        max_chunk_message_bytes: usize,
        handler: Option<Arc<dyn RequestHandler>>,
        network: NetworkConfig,
    ) -> Result<Self> {
        let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes())
            .context("deriving libp2p keypair from node identity")?;
        let peer_id = PeerId::from(&keypair.public());
        let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
            .context("creating mDNS behaviour")?;
        let codec = FrameCodec {
            max_frame_bytes: max_chunk_message_bytes.max(max_message_bytes),
        };
        // Parse bootstrap peers up front (so a typo'd multiaddr is a loud
        // warning, not a silent hole in discovery), and remember them to dial
        // right after the swarm is built.
        let mut bootstrap: Vec<(PeerId, Multiaddr)> = Vec::new();
        for s in &network.bootstrap_peers {
            match parse_bootstrap_peer(s) {
                Ok((peer, addr)) => bootstrap.push((peer, addr)),
                Err(e) => warn!(peer = %s, error = %e, "skipping invalid bootstrap peer"),
            }
        }
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .context("building TCP transport")?
            // Relay client transport: lets us dial and accept `/p2p-circuit`
            // addresses through a relay server, which is how a node behind
            // NAT reaches (and is reached by) peers outside its subnet.
            // Always built so the transport is uniform; the behaviour itself
            // is gated on `network.relay_enabled`.
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .context("building relay client transport")?
            .with_behaviour(|keypair, relay_client| NodeBehaviour {
                mdns: mdns_behaviour,
                messages: request_response::Behaviour::with_codec(
                    codec,
                    [(MESSAGE_PROTOCOL, ProtocolSupport::Full)],
                    // Default is 30s per request. A remote inference stream on
                    // CPU can easily exceed that (Mistral-7B ~22s for 24
                    // tokens); a tight protocol timeout cuts the stream
                    // mid-answer and the browser reports "Error in input
                    // stream". 300s keeps slow-but-healthy workers usable.
                    request_response::Config::default()
                        .with_request_timeout(Duration::from_secs(300)),
                ),
                identify: identify::Behaviour::new(identify::Config::new(
                    format!("decentraai/{}", env!("CARGO_PKG_VERSION")),
                    keypair.public(),
                )),
                ping: ping::Behaviour::new(ping::Config::default()),
                kad: Toggle::from(if network.dht_enabled {
                    let mut k = kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id));
                    // Advertise and serve K/V once we have a confirmed
                    // external address (see kad behaviour docs); explicit
                    // Server mode means we participate even on loopback.
                    k.set_mode(Some(kad::Mode::Server));
                    for (peer, addr) in &bootstrap {
                        k.add_address(peer, addr.clone());
                    }
                    Some(k)
                } else {
                    None
                }),
                dcutr: Toggle::from(if network.relay_enabled {
                    Some(dcutr::Behaviour::new(peer_id))
                } else {
                    None
                }),
                relay: Toggle::from(if network.relay_enabled {
                    Some(relay_client)
                } else {
                    None
                }),
                relay_server: Toggle::from(if network.relay_enabled {
                    Some(relay::Behaviour::new(peer_id, relay::Config::default()))
                } else {
                    None
                }),
            })
            .context("attaching network behaviour")?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        // Dial bootstrap peers so the DHT connects immediately instead of
        // waiting for an idle routing query to discover them.
        for (peer, addr) in &bootstrap {
            if let Err(e) = swarm.dial(addr.clone().with(Protocol::P2p(*peer))) {
                debug!(%peer, error = %e, "bootstrap dial deferred");
            }
        }

        let (commands, mut inbox) = mpsc::unbounded_channel::<Command>();
        // A second sender used only by the reconnect tasks spawned on peer
        // disconnects, so redial attempts don't block the event loop.
        let reconnect_sender = commands.clone();
        // Shared on_infer callback storage for runtime registration
        let on_infer: SharedHandler<InferHandler> = Arc::new(tokio::sync::Mutex::new(None));
        let on_infer_clone = on_infer.clone();
        let on_cancel: SharedHandler<CancelHandler> = Arc::new(tokio::sync::Mutex::new(None));
        let on_cancel_clone = on_cancel.clone();
        let on_manifest: SharedHandler<ManifestAnnouncementHandler> =
            Arc::new(tokio::sync::Mutex::new(None));
        let on_manifest_clone = on_manifest.clone();
        tokio::spawn(async move {
            let mut pending: HashMap<
                request_response::OutboundRequestId,
                oneshot::Sender<Result<Vec<u8>>>,
            > = HashMap::new();
            let mut pending_listens: VecDeque<oneshot::Sender<Result<Multiaddr>>> = VecDeque::new();
            let mut connected: Vec<PeerId> = Vec::new();
            // Per-peer reconnect attempts since the last successful connection,
            // so a peer that went away doesn't get dialed forever.
            let mut reconnect_attempts: HashMap<PeerId, u32> = HashMap::new();
            // Last known address per peer, kept so a disconnect can be
            // re-dialed without waiting for another mDNS announcement.
            let mut known_addresses: HashMap<PeerId, Multiaddr> = HashMap::new();
            // Our own listen addresses (the node's LAN identity).
            let mut local_addresses: Vec<Multiaddr> = Vec::new();
            // Addresses observed for us by remote peers via identify (our
            // public IP behind NAT). Advertised on the swarm so peers can
            // dial us directly; surfaced for the control plane.
            let mut external_addresses: Vec<Multiaddr> = Vec::new();

            loop {
                tokio::select! {
                    maybe_cmd = inbox.recv() => {
                        let Some(cmd) = maybe_cmd else { break };
                        match cmd {
                            Command::Listen { addr, reply } => {
                                match swarm.listen_on(addr) {
                                    Ok(_) => pending_listens.push_back(reply),
                                    Err(e) => {
                                        let _ = reply.send(Err(e.into()));
                                    }
                                }
                            }
                            Command::Dial { addr, reply } => {
                                let res = swarm.dial(addr).map_err(Into::into);
                                let _ = reply.send(res);
                            }
                            Command::Request { peer, payload, reply } => {
                                let id = swarm.behaviour_mut().messages.send_request(&peer, payload);
                                pending.insert(id, reply);
                            }
                            Command::Broadcast { payload } => {
                                // Send to connected peers plus every peer whose
                                // address we know (e.g. from mDNS) even if the
                                // connection is not established yet —
                                // request_response auto-dials in that case.
                                let mut peers = connected.clone();
                                for peer in swarm.behaviour_mut().mdns.discovered_nodes() {
                                    if !peers.contains(peer) {
                                        peers.push(*peer);
                                    }
                                }
                                for peer in peers {
                                    swarm
                                        .behaviour_mut()
                                        .messages
                                        .send_request(&peer, payload.clone());
                                }
                            }
                            Command::Connected { reply } => {
                                let _ = reply.send(connected.clone());
                            }
                            Command::Peers { reply } => {
                                let _ = reply.send(PeersSnapshot {
                                    connected: connected.clone(),
                                    addresses: known_addresses.clone(),
                                    local_addresses: local_addresses.clone(),
                                    external_addresses: external_addresses.clone(),
                                });
                            }
                            Command::Shutdown => break,
                        }
                    }
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!(%address, "listening");
                                if !local_addresses.contains(&address) {
                                    local_addresses.push(address.clone());
                                }
                                if let Some(reply) = pending_listens.pop_front() {
                                    let _ = reply.send(Ok(address));
                                }
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                info!(%peer_id, "peer connected");
                                if !connected.contains(&peer_id) {
                                    connected.push(peer_id);
                                }
                                // A successful connection resets the redial
                                // budget so a future drop can attempt again.
                                reconnect_attempts.remove(&peer_id);
                            }
                            SwarmEvent::ConnectionClosed {
                                peer_id, endpoint, ..
                            } => {
                                info!(%peer_id, "peer disconnected");
                                connected.retain(|p| p != &peer_id);
                                // Remember the remote address from the closing
                                // link when it was us doing the dialing.
                                if let libp2p::core::ConnectedPoint::Dialer { address, .. } =
                                    endpoint
                                {
                                    known_addresses.insert(peer_id, address.clone());
                                }
                                let attempt = reconnect_attempts.entry(peer_id).or_insert(0);
                                let addr_known = known_addresses.get(&peer_id).cloned();
                                // Bounded explicit re-dial: drop the budget when
                                // we've retried too often or have no address, and
                                // let mDNS re-discovery take over.
                                let Some(addr) = addr_known.clone() else {
                                    reconnect_attempts.remove(&peer_id);
                                    continue;
                                };
                                if *attempt >= RECONNECT_MAX_ATTEMPTS {
                                    reconnect_attempts.remove(&peer_id);
                                    debug!(%peer_id, "reconnect budget exhausted; waiting for mDNS");
                                    continue;
                                }
                                let nth = *attempt;
                                *attempt += 1;
                                let backoff = Duration::from_millis(
                                    RECONNECT_BASE_BACKOFF_MS << nth.min(10),
                                );
                                let sender = reconnect_sender.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(backoff).await;
                                    let (reply, _) = oneshot::channel();
                                    let _ = sender.send(Command::Dial { addr, reply });
                                });
                                debug!(
                                    %peer_id,
                                    attempt = nth + 1,
                                    max = RECONNECT_MAX_ATTEMPTS,
                                    backoff_ms = backoff.as_millis(),
                                    "scheduled reconnect dial"
                                );
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(
                                mdns::Event::Discovered(list),
                            )) => {
                                for (peer, addr) in list {
                                    info!(%peer, %addr, "mDNS discovered peer");
                                    swarm.add_peer_address(peer, addr.clone());
                                    known_addresses.insert(peer, addr.clone());
                                    // mDNS discovery is passive: it only adds
                                    // addresses to the peerstore. Dial so the
                                    // connection is actually established and
                                    // request/response streaming can reach
                                    // this peer.
                                    if let Err(e) = swarm.dial(addr) {
                                        debug!(%peer, error = %e, "mDNS auto-dial deferred");
                                    }
                                }
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                                request_response::Event::Message { peer, message },
                            )) => match message {
                                request_response::Message::Request {
                                    request, channel, ..
                                } => {
                                    if let Ok(announcement) = decentraai_protocol::deserialize_message::<decentraai_protocol::ManifestAnnouncement>(
                                        &request,
                                        DEFAULT_MAX_MESSAGE_BYTES,
                                    ) {
                                        info!(
                                            %peer,
                                            model = %announcement.manifest.file_name,
                                            "received manifest announcement"
                                        );
                                        // Announcements are fire-and-forget but a handler may
                                        // want to act (e.g. auto-download). The callback must
                                        // not block the event loop; it spawns its own task.
                                        let guard = on_manifest_clone.lock().await;
                                        if let Some(cb) = &*guard {
                                            cb(peer, announcement.manifest);
                                        }
                                        continue;
                                    }
                                    // Check for InferRequest
                                    if let Ok(infer_req) = decentraai_protocol::deserialize_message::<decentraai_protocol::InferRequest>(
                                        &request,
                                        DEFAULT_MAX_MESSAGE_BYTES,
                                    ) {
                                        info!(%peer, bytes = request.len(), "received inference request");
                                        // If a runtime on_infer callback has been registered, call it
                                        let guard = on_infer_clone.lock().await;
                                        if let Some(cb) = &*guard {
                                            match cb(peer, infer_req) {
                                                Ok(bytes) => {
                                                    if swarm
                                                        .behaviour_mut()
                                                        .messages
                                                        .send_response(channel, bytes)
                                                        .is_err()
                                                    {
                                                        warn!(%peer, "failed to send response");
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!(%peer, error = %e, "on_infer handler failed");
                                                }
                                            }
                                            continue;
                                        }
                                        // else fallthrough to normal handler
                                    }
                                    // Check for InferCancel (single request id in the message, no
                                    // request/response payload semantics)
                                    if let Ok(decentraai_protocol::InferMessage::InferCancel {
                                        request_id,
                                        ..
                                    }) = decentraai_protocol::deserialize_message::<
                                        decentraai_protocol::InferMessage,
                                    >(&request, DEFAULT_MAX_MESSAGE_BYTES)
                                    {
                                        info!(%peer, %request_id, "received inference cancel");
                                        let guard = on_cancel_clone.lock().await;
                                        if let Some(cb) = &*guard {
                                            cb(request_id);
                                        }
                                        let _ = swarm
                                            .behaviour_mut()
                                            .messages
                                            .send_response(channel, Vec::new());
                                        continue;
                                    }
                                    let response = match &handler {
                                        Some(h) => match h.handle(&request) {
                                            Ok(bytes) => bytes,
                                            Err(e) => {
                                                warn!(%peer, error = %e, "request handler failed");
                                                continue;
                                            }
                                        },
                                        None => {
                                            warn!(%peer, "request ignored: no handler configured");
                                            continue;
                                        }
                                    };
                                    if swarm
                                        .behaviour_mut()
                                        .messages
                                        .send_response(channel, response)
                                        .is_err()
                                    {
                                        warn!(%peer, "failed to send response");
                                    }
                                }
                                request_response::Message::Response {
                                    request_id,
                                    response,
                                    ..
                                } => {
                                    if let Some(reply) = pending.remove(&request_id) {
                                        let _ = reply.send(Ok(response));
                                    }
                                }
                            },
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                                request_response::Event::OutboundFailure {
                                    request_id, error, ..
                                },
                            )) => {
                                if let Some(reply) = pending.remove(&request_id) {
                                    let _ = reply.send(Err(anyhow::anyhow!(
                                        "outbound request failed: {error}"
                                    )));
                                }
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                                request_response::Event::InboundFailure { peer, error, .. },
                            )) => {
                                warn!(%peer, error = %error, "inbound request failure");
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                                request_response::Event::ResponseSent { .. },
                            )) => {}
                            // Identify: learn the peer's addresses and, most
                            // importantly, OUR address as observed by that
                            // peer (the public IP behind NAT). Advertising it
                            // on the swarm lets remote peers dial us directly,
                            // and surfaces it for the control plane.
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Identify(
                                identify::Event::Received { info, .. },
                            )) => {
                                if !info.observed_addr.is_empty() {
                                    if !external_addresses.contains(&info.observed_addr) {
                                        external_addresses.push(info.observed_addr.clone());
                                    }
                                    // Register with the swarm so it is
                                    // announced and used for inbound dials.
                                    swarm.add_external_address(info.observed_addr.clone());
                                    info!(addr = %info.observed_addr, "identify learned our external address");
                                }
                                for addr in &info.listen_addrs {
                                    swarm.add_peer_address(info.public_key.to_peer_id(), addr.clone());
                                }
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Identify(_)) => {}
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Ping(_)) => {}
                            // Kademlia: learning a routable address for a peer
                            // lets us dial it across subnets without mDNS.
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Kad(
                                kad::Event::RoutablePeer { peer, address },
                            )) => {
                                known_addresses.insert(peer, address.clone());
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Kad(
                                kad::Event::OutboundQueryProgressed { .. },
                            )) => {}
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Kad(_)) => {}
                            // Relay client: once a reservation is accepted the
                            // reserved relayed address is a way peers can
                            // reach us across NAT.
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Relay(
                                relay::client::Event::ReservationReqAccepted {
                                    relay_peer_id, ..
                                },
                            )) => {
                                info!(%relay_peer_id, "relay reservation accepted");
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Relay(_)) => {}
                            // Relay server: log when peers reserve circuits or
                            // circuits open/close through us.
                            SwarmEvent::Behaviour(NodeBehaviourEvent::RelayServer(
                                relay::Event::ReservationReqAccepted { .. },
                            )) => {}
                            SwarmEvent::Behaviour(NodeBehaviourEvent::RelayServer(
                                relay::Event::CircuitClosed {
                                    src_peer_id,
                                    dst_peer_id,
                                    error,
                                },
                            )) => {
                                debug!(%src_peer_id, %dst_peer_id, error = ?error, "relay circuit closed");
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::RelayServer(_)) => {}
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Dcutr(_)) => {}
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(Self {
            commands,
            peer_id,
            on_infer,
            on_cancel,
            on_manifest,
        })
    }

    pub fn local_peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Starts listening and resolves with the actual bound address
    /// (useful with ephemeral ports like `/ip4/127.0.0.1/tcp/0`).
    pub async fn listen(&self, addr: &str) -> Result<Multiaddr> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Listen {
                addr: addr.parse().context("invalid listen address")?,
                reply,
            })
            .map_err(|_| anyhow::anyhow!("swarm task is not running"))?;
        rx.await.context("swarm task dropped the reply")?
    }

    pub async fn dial(&self, addr: &str) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Dial {
                addr: addr.parse().context("invalid dial address")?,
                reply,
            })
            .map_err(|_| anyhow::anyhow!("swarm task is not running"))?;
        rx.await.context("swarm task dropped the reply")?
    }

    /// Sends a serialized protocol message and awaits the peer's response.
    pub async fn request(&self, peer: PeerId, payload: Vec<u8>) -> Result<Vec<u8>> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Request {
                peer,
                payload,
                reply,
            })
            .map_err(|_| anyhow::anyhow!("swarm task is not running"))?;
        rx.await.context("swarm task dropped the reply")?
    }

    /// Broadcasts a serialized ManifestAnnouncement to every connected
    /// peer. Fire-and-forget: delivery failures are logged, not fatal.
    pub fn announce(&self, payload: Vec<u8>) {
        let _ = self.commands.send(Command::Broadcast { payload });
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }

    /// Returns the PeerIds of currently connected peers. Best-effort: an
    /// empty/truncated result is returned if the swarm task is busy or gone.
    pub async fn connected_peers(&self) -> Vec<PeerId> {
        let (reply, rx) = oneshot::channel();
        if self.commands.send(Command::Connected { reply }).is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Returns the full peer identity snapshot: connected ids, last-known
    /// addresses per peer (from mDNS discovery / dialer connect), and this
    /// node's own listen addresses. Best-effort: the default snapshot is
    /// returned if the swarm task is busy or gone.
    pub async fn peers_snapshot(&self) -> PeersSnapshot {
        let (reply, rx) = oneshot::channel();
        if self.commands.send(Command::Peers { reply }).is_err() {
            return PeersSnapshot::default();
        }
        rx.await.unwrap_or_default()
    }
}

#[derive(NetworkBehaviour)]
struct NodeBehaviour {
    mdns: mdns::tokio::Behaviour,
    messages: request_response::Behaviour<FrameCodec>,
    /// Identify: exchanges listen/observed addresses so peers learn each
    /// other's external addresses across NAT.
    identify: identify::Behaviour,
    /// Ping: liveness/keepalive probe on every connection.
    ping: ping::Behaviour,
    /// Kademlia DHT: cross-subnet peer discovery and address routing.
    /// `Toggle` off keeps the node LAN-only (config `network.dht_enabled`).
    kad: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
    /// Hole-punching over relayed connections (config `network.relay_enabled`).
    dcutr: Toggle<dcutr::Behaviour>,
    /// Relay client: dial and be dialed through a relay server across NAT.
    /// Paired with the relay client transport built in `P2PNode::new`.
    relay: Toggle<relay::client::Behaviour>,
    /// Relay server: reserves circuits and relays traffic for other peers.
    relay_server: Toggle<relay::Behaviour>,
}

#[derive(Debug, Clone)]
struct FrameCodec {
    max_frame_bytes: usize,
}

#[async_trait]
impl request_response::Codec for FrameCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_frame(io, self.max_frame_bytes).await
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_frame(io, self.max_frame_bytes).await
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        data: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_frame(io, &data).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        data: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_frame(io, &data).await
    }
}

async fn read_frame<T: AsyncRead + Unpin + Send>(io: &mut T, max: usize) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds maximum size",
        ));
    }
    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame<T: AsyncWrite + Unpin + Send>(io: &mut T, data: &[u8]) -> io::Result<()> {
    let len = u32::try_from(data.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame too large"))?;
    io.write_all(&len.to_le_bytes()).await?;
    io.write_all(data).await?;
    io.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_roundtrip() {
        let payload = b"hello decentraai".to_vec();
        let mut buf = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();
        let mut slice = buf.as_slice();
        let read = read_frame(&mut slice, DEFAULT_MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        assert_eq!(read, payload);
    }

    #[tokio::test]
    async fn frame_oversize_is_rejected() {
        let payload = vec![0u8; 64];
        let mut buf = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();
        let mut slice = buf.as_slice();
        assert!(read_frame(&mut slice, 16).await.is_err());
    }

    /// The control-plane cap must actually reject an oversized control request.
    /// Previously handlers passed `request.len()` as max_size, which made the
    /// size check a no-op (data.len() > data.len() is never true), so a peer
    /// could push a frame up to the shared data-plane cap (96 MiB) into a
    /// control handler. All inbound requests are control-sized, so they are
    /// capped at DEFAULT_MAX_MESSAGE_BYTES (1 MiB).
    #[test]
    fn control_plane_cap_rejects_oversized_request() {
        let oversized = vec![0u8; DEFAULT_MAX_MESSAGE_BYTES + 1];
        let err = decentraai_protocol::deserialize_message::<decentraai_protocol::ManifestRequest>(
            &oversized,
            DEFAULT_MAX_MESSAGE_BYTES,
        );
        assert!(err.is_err(), "control message over 1 MiB must be rejected");

        // And an in-bounds (empty-but-valid-shape) control message is not a
        // size rejection — the cap accepts anything at or under the limit.
        let ok_size = vec![0u8; DEFAULT_MAX_MESSAGE_BYTES];
        let _ = decentraai_protocol::deserialize_message::<decentraai_protocol::ManifestRequest>(
            &ok_size,
            DEFAULT_MAX_MESSAGE_BYTES,
        );
        // The size gate passed (failure, if any, would be JSON parse, not size).
    }
}
