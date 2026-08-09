//! libp2p transport for DecentraAI: TCP + Noise + Yamux, mDNS discovery,
//! and a length-delimited request/response channel for protocol messages.
//!
//! The libp2p keypair is derived from the node identity, so the transport
//! PeerId is cryptographically bound to the Ed25519 node key. The transfer
//! engine and authenticated handshake land in M3 part 2.

use anyhow::{Context, Result};
use async_trait::async_trait;
use decentraai_identity::Identity;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::identity::Keypair;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::{NetworkBehaviour, StreamProtocol, SwarmEvent};
use libp2p::{PeerId, Swarm, mdns, noise, tcp, yamux};
use std::io;
use std::time::Duration;
use tracing::info;

/// Request/response protocol carrying serialized decentraai-protocol messages.
pub const MESSAGE_PROTOCOL: StreamProtocol = StreamProtocol::new("/decentraai/message/1");

/// Default frame cap matching the control-plane limit in node.example.yaml.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(NetworkBehaviour)]
struct NodeBehaviour {
    mdns: mdns::tokio::Behaviour,
    messages: request_response::Behaviour<FrameCodec>,
}

pub struct P2PNode {
    swarm: Swarm<NodeBehaviour>,
    peer_id: PeerId,
}

impl P2PNode {
    pub fn new(identity: &Identity, max_message_bytes: usize) -> Result<Self> {
        let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes())
            .context("deriving libp2p keypair from node identity")?;
        let peer_id = PeerId::from(&keypair.public());
        let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .context("building TCP transport")?
            .with_behaviour(|key| {
                Ok(NodeBehaviour {
                    mdns: mdns::tokio::Behaviour::new(
                        mdns::Config::default(),
                        key.public().to_peer_id(),
                    )?,
                    messages: request_response::Behaviour::with_codec(
                        FrameCodec { max_frame_bytes: max_message_bytes },
                        [(MESSAGE_PROTOCOL, ProtocolSupport::Full)],
                        request_response::Config::default(),
                    ),
                })
            })
            .context("attaching network behaviour")?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();
        Ok(Self { swarm, peer_id })
    }

    pub fn local_peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn listen(&mut self, addr: &str) -> Result<()> {
        self.swarm
            .listen_on(addr.parse().context("invalid listen address")?)
            .context("failed to listen")?;
        Ok(())
    }

    pub fn dial(&mut self, addr: &str) -> Result<()> {
        self.swarm
            .dial(addr.parse().context("invalid dial address")?)
            .context("failed to dial")?;
        Ok(())
    }

    pub fn send_request(
        &mut self,
        peer: &PeerId,
        payload: Vec<u8>,
    ) -> request_response::OutboundRequestId {
        self.swarm.behaviour_mut().messages.send_request(peer, payload)
    }

    /// Drive the swarm event loop. Discovered mDNS peers are registered with
    /// the request/response behaviour; inbound requests are only logged until
    /// the transfer engine lands in M3 part 2.
    pub async fn run(mut self) -> Result<()> {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!(%address, "listening")
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    info!(%peer_id, "peer connected")
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    info!(%peer_id, "peer disconnected")
                }
                SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(mdns::Event::Discovered(
                    list,
                ))) => {
                    for (peer, addr) in list {
                        info!(%peer, %addr, "mDNS discovered peer");
                        self.swarm.behaviour_mut().messages.add_address(&peer, addr);
                    }
                }
                SwarmEvent::Behaviour(NodeBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer, addr) in list {
                        self.swarm.behaviour_mut().messages.remove_address(&peer, &addr);
                    }
                }
                SwarmEvent::Behaviour(NodeBehaviourEvent::Messages(
                    request_response::Event::Message { peer, message },
                )) => match message {
                    request_response::Message::Request { request, .. } => {
                        info!(%peer, bytes = request.len(), "received request")
                    }
                    request_response::Message::Response { response, .. } => {
                        info!(%peer, bytes = response.len(), "received response")
                    }
                },
                _ => {}
            }
        }
    }
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

    #[test]
    fn libp2p_peer_id_is_derived_from_node_identity() {
        let identity = Identity::generate();
        let node = P2PNode::new(&identity, DEFAULT_MAX_MESSAGE_BYTES).unwrap();
        let again = P2PNode::new(&identity, DEFAULT_MAX_MESSAGE_BYTES).unwrap();
        assert_eq!(node.local_peer_id(), again.local_peer_id());
    }

    #[test]
    fn distinct_identities_have_distinct_peer_ids() {
        let a = P2PNode::new(&Identity::generate(), DEFAULT_MAX_MESSAGE_BYTES).unwrap();
        let b = P2PNode::new(&Identity::generate(), DEFAULT_MAX_MESSAGE_BYTES).unwrap();
        assert_ne!(a.local_peer_id(), b.local_peer_id());
    }

    #[test]
    fn listen_on_localhost() {
        let identity = Identity::generate();
        let mut node = P2PNode::new(&identity, DEFAULT_MAX_MESSAGE_BYTES).unwrap();
        node.listen("/ip4/127.0.0.1/tcp/0").unwrap();
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let payload = b"hello decentraai".to_vec();
        let mut cursor = io::Cursor::new(Vec::new());
        write_frame(&mut cursor, &payload).await.unwrap();
        cursor.set_position(0);
        let read = read_frame(&mut cursor, DEFAULT_MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        assert_eq!(read, payload);
    }

    #[tokio::test]
    async fn frame_oversize_is_rejected() {
        let payload = vec![0u8; 64];
        let mut cursor = io::Cursor::new(Vec::new());
        write_frame(&mut cursor, &payload).await.unwrap();
        cursor.set_position(0);
        assert!(read_frame(&mut cursor, 16).await.is_err());
    }
}
