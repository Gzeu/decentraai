//! libp2p transport for DecentraAI: TCP + Noise + Yamux, mDNS discovery,
//! and a length-delimited request/response channel for protocol messages.
//!
//! The node runs as an actor: the swarm lives in a background task and
//! commands flow through a channel, so callers can issue sequential
//! requests (e.g. manifest then chunks) without blocking the event loop.
//! The libp2p keypair is derived from the node identity, binding the
//! transport PeerId to the Ed25519 node key.

pub mod transfer;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use decentraai_identity::Identity;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::identity::Keypair;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::{NetworkBehaviour, StreamProtocol, SwarmEvent};
use libp2p::{Multiaddr, PeerId, mdns, noise, tcp, yamux};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

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

        if let Ok(req) = deserialize_message::<ManifestRequest>(request, request.len()) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            return Ok(self.manifest_response.clone());
        }
        if let Ok(req) = deserialize_message::<ChunkRequest>(request, request.len()) {
            if req.protocol_version != CURRENT_PROTOCOL_VERSION {
                bail!("unsupported protocol version {}", req.protocol_version);
            }
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&self.file).context("opening served file")?;
            file.seek(SeekFrom::Start(u64::from(req.chunk_index) * self.chunk_size))?;
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
    Shutdown,
}

/// Handle to the background swarm task.
pub struct P2PNode {
    commands: mpsc::UnboundedSender<Command>,
    peer_id: PeerId,
}

impl P2PNode {
    /// Creates a node and spawns its swarm task. Must be called from within
    /// a Tokio runtime: the mDNS behaviour registers with the reactor.
    pub fn new(
        identity: &Identity,
        max_message_bytes: usize,
        max_chunk_message_bytes: usize,
        handler: Option<Arc<dyn RequestHandler>>,
    ) -> Result<Self> {
        let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes())
            .context("deriving libp2p keypair from node identity")?;
        let peer_id = PeerId::from(&keypair.public());
        let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
            .context("creating mDNS behaviour")?;
        let codec = FrameCodec {
            max_frame_bytes: max_chunk_message_bytes.max(max_message_bytes),
        };
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .context("building TCP transport")?
            .with_behaviour(|_| NodeBehaviour {
                mdns: mdns_behaviour,
                messages: request_response::Behaviour::with_codec(
                    codec,
                    [(MESSAGE_PROTOCOL, ProtocolSupport::Full)],
                    request_response::Config::default(),
                ),
            })
            .context("attaching network behaviour")?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        let (commands, mut inbox) = mpsc::unbounded_channel::<Command>();
        tokio::spawn(async move {
            let mut pending: HashMap<
                request_response::OutboundRequestId,
                oneshot::Sender<Result<Vec<u8>>>,
            > = HashMap::new();
            let mut pending_listens: VecDeque<oneshot::Sender<Result<Multiaddr>>> =
                VecDeque::new();

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
                            Command::Shutdown => break,
                        }
                    }
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!(%address, "listening");
                                if let Some(reply) = pending_listens.pop_front() {
                                    let _ = reply.send(Ok(address));
                                }
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                info!(%peer_id, "peer connected")
                            }
                            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                                info!(%peer_id, "peer disconnected")
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(
                                mdns::Event::Discovered(list),
                            )) => {
                                for (peer, addr) in list {
                                    info!(%peer, %addr, "mDNS discovered peer");
                                    swarm.add_peer_address(peer, addr);
                                }
                            }
                            SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                                request_response::Event::Message { peer, message },
                            )) => match message {
                                request_response::Message::Request {
                                    request, channel, ..
                                } => {
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
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(Self { commands, peer_id })
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

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

#[derive(NetworkBehaviour)]
struct NodeBehaviour {
    mdns: mdns::tokio::Behaviour,
    messages: request_response::Behaviour<FrameCodec>,
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

    async fn read_response<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
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
}
