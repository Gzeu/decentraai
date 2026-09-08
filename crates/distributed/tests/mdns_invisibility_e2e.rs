//! mDNS invisibility E2E: a node with `lan_discovery: false` must stay
//! invisible on the LAN segment — no multicast announcements, no reaction
//! to neighbours — while a neighbouring `lan_discovery: true` node keeps
//! working but never learns the silent peer.
//!
//! This exercises the `Toggle`-off mDNS wiring in `P2PNode::new_with_network`:
//! with the old code (behaviour always built, only the reaction side gated)
//! the silent node still answered mDNS queries, so the loud node discovered
//! and dialed it and this test failed. No manual dial happens here; any
//! connection or learned address is proof of a leak.

use std::time::Duration;

use decentraai_identity::Identity;
use decentraai_p2p::{
    DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, NetworkConfig, P2PNode,
};
use libp2p::PeerId;
use libp2p::identity::Keypair;

fn peer_id(identity: &Identity) -> PeerId {
    let keypair = Keypair::ed25519_from_bytes(identity.signing_key_bytes()).unwrap();
    PeerId::from(keypair.public())
}

fn net(lan_discovery: bool) -> NetworkConfig {
    NetworkConfig {
        lan_discovery,
        dht_enabled: false,
        relay_enabled: false,
        bootstrap_peers: vec![],
        max_connections: 8,
        data_dir: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lan_discovery_false_stays_invisible_to_lan_neighbours() {
    let silent_identity = Identity::generate();
    let loud_identity = Identity::generate();
    let silent_peer = peer_id(&silent_identity);
    let loud_peer = peer_id(&loud_identity);

    let silent = P2PNode::new_with_network(
        &silent_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
        net(false),
    )
    .unwrap();
    let loud = P2PNode::new_with_network(
        &loud_identity,
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
        net(true),
    )
    .unwrap();
    silent.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    loud.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    // mDNS exchanges queries/responses at startup on the multicast group;
    // 8s is ample for a leak to surface (a full extra query cycle would be
    // 5 minutes and is unnecessary: any announcement/response happens here).
    tokio::time::sleep(Duration::from_secs(8)).await;

    // NOTE: assertions are pair-specific, never global. The machine running
    // this test may host a real live node (systemd `decentraai-node`) that
    // legitimately answers the loud node's multicast queries — that is mDNS
    // working as designed, not a leak. The leak this test guards is the
    // SILENT peer becoming visible.
    assert!(
        silent.connected_peers().await.is_empty(),
        "lan_discovery=false node must not auto-connect to anyone"
    );
    let loud_connected = loud.connected_peers().await;
    assert!(
        !loud_connected.contains(&silent_peer),
        "loud node must not discover+connect the silent peer via mDNS"
    );
    let loud_snapshot = loud.peers_snapshot().await;
    assert!(
        !loud_snapshot.addresses.contains_key(&silent_peer),
        "loud node must not learn the silent peer's address"
    );
    let silent_snapshot = silent.peers_snapshot().await;
    assert!(
        !silent_snapshot.addresses.contains_key(&loud_peer),
        "silent node must not learn the loud peer's address"
    );
}
