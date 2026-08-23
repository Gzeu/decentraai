//! Memory auto-propagation tests (M19): pure policy filters plus a
//! two-real-node loopback cycle proving verified knowledge travels while
//! candidates stay home and imports land as `candidate` locally.

use anyhow::Result;
use decentraai_agents::memory::{
    MemoryAccess, MemoryEntry, MemoryLevel, MemoryPolicy, MemoryScope, MemoryStatus,
};
use decentraai_distributed::agent_memory::MemoryStore;
use decentraai_distributed::memory_propagator::{
    eligible_scopes, propagate_once, travel_worthy, PropagationConfig, PropagationReport,
};
use decentraai_identity::Identity;
use decentraai_p2p::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode};
use std::path::Path;
use std::sync::Arc;

fn scope_with(level: MemoryLevel, access: MemoryAccess, remote_write: bool) -> MemoryScope {
    let mut policy = MemoryPolicy {
        access,
        ..MemoryPolicy::default()
    };
    policy.level = level;
    if remote_write {
        policy.allow_remote_write = true;
    }
    MemoryScope::new("s", "governor", level).with_policy(policy)
}

#[test]
fn only_public_remote_writable_network_plus_scopes_are_eligible() {
    let scopes = [
        scope_with(MemoryLevel::Agent, MemoryAccess::Private, false),
        scope_with(MemoryLevel::Team, MemoryAccess::TeamOnly, true),
        // Node-level public+remote still stays LOCAL (never crosses nodes).
        scope_with(MemoryLevel::Node, MemoryAccess::Public, true),
        scope_with(MemoryLevel::Network, MemoryAccess::Public, true),
        scope_with(MemoryLevel::Fabric, MemoryAccess::Public, true),
        // Public WITHOUT remote-write opt-in never travels.
        scope_with(MemoryLevel::Fabric, MemoryAccess::Public, false),
    ];
    let named: Vec<MemoryScope> = scopes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut c = s.clone();
            c.name = format!("scope-{i}");
            c
        })
        .collect();
    let got = eligible_scopes(&named);
    let names: Vec<&str> = got.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["scope-3", "scope-4"],
        "network+fabric only; node-level and non-opt-in excluded"
    );
}

#[test]
fn only_verified_and_trusted_entries_travel() {
    let mk = |id: &str, status: MemoryStatus, content: &str| {
        let mut e = MemoryEntry::new(id, "s", "a", "n", content);
        e.meta.status = status;
        e
    };
    let entries = vec![
        mk("v", MemoryStatus::Verified, "goes"),
        mk("t", MemoryStatus::Trusted, "goes too"),
        mk("c", MemoryStatus::Candidate, "stays"),
        mk("o", MemoryStatus::Obsolete, "stays"),
        mk("blank", MemoryStatus::Verified, "   "),
    ];
    let ids: Vec<String> = travel_worthy(&entries)
        .into_iter()
        .map(|e| e.entry_id)
        .collect();
    assert_eq!(ids, vec!["v", "t"], "candidate/obsolete/blank stay local");
}

fn fabric_public_scope(name: &str) -> MemoryScope {
    let policy = MemoryPolicy::default().public().with_remote_write();
    MemoryScope::new(name, "governor", MemoryLevel::Fabric).with_policy(policy)
}

/// Unique-per-run scope name: ambient LAN nodes (e.g. a live node on the
/// same segment) must never be able to pollute this test's assertions.
fn unique_scope() -> String {
    format!(
        "fabric.lessons.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    )
}

fn seeded_store(scope_name: &str) -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::open(Path::new(":memory:")).unwrap());
    store.register_scope(&fabric_public_scope(scope_name)).unwrap();
    let mut verified = MemoryEntry::new("e-verified", scope_name, "researcher", "node-a", "verified lesson");
    verified.created_at_ms = 100;
    verified.meta.status = MemoryStatus::Verified;
    let mut candidate = MemoryEntry::new("e-candidate", scope_name, "researcher", "node-a", "unverified hunch");
    candidate.created_at_ms = 100;
    // meta.status defaults to Candidate.
    let mut trusted = MemoryEntry::new("e-trusted", scope_name, "researcher", "node-a", "trusted lesson");
    trusted.created_at_ms = 100;
    trusted.meta.status = MemoryStatus::Trusted;

    for e in [&verified, &candidate, &trusted] {
        store
            .write_checked(scope_name, e, "governor", true, false, false)
            .unwrap();
    }
    store
}

/// The receiver-side handler: identical contract the runtime registers in
/// node-cli (merge into store, trust boundary downgrades to candidate).
fn receiver_handler(
    store: Arc<MemoryStore>,
) -> impl Fn(
    libp2p::PeerId,
    decentraai_protocol::memory_sync::MemorySyncRequest,
) -> Vec<u8> {
    move |_peer, req| {
        use decentraai_protocol::memory_sync::MemorySyncResponse;
        let mut accepted = 0u32;
        let mut duplicates = 0u32;
        let mut conflicts_linked = 0u32;
        let mut rejected = 0u32;
        for se in req.entries {
            let entry =
                decentraai_distributed::agent_memory::sync_entry_to_memory(se, &req.scope);
            match store.write_checked(&req.scope, &entry, "memory-sync", false, false, false) {
                Ok(decentraai_agents::memory::WriteOutcome::Stored) => accepted += 1,
                Ok(decentraai_agents::memory::WriteOutcome::Duplicate { .. }) => duplicates += 1,
                Ok(decentraai_agents::memory::WriteOutcome::CompetingClaim { .. }) => {
                    accepted += 1;
                    conflicts_linked += 1;
                }
                Err(_) => rejected += 1,
            }
        }
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
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn propagation_cycle_moves_only_travel_worthy_knowledge() -> Result<()> {
    let scope_name = unique_scope();
    // Receiver: owns an accepting store + handler.
    let receiver_store = Arc::new(MemoryStore::open(Path::new(":memory:")).unwrap());
    receiver_store.register_scope(&fabric_public_scope(&scope_name)).unwrap();
    let receiver = P2PNode::new_with_network(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
        decentraai_p2p::NetworkConfig {
            lan_discovery: false,
            ..Default::default()
        },
    )
    .unwrap();
    let addr = receiver.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    {
        let mut rx = receiver.clone();
        rx.set_on_memory_sync(receiver_handler(receiver_store.clone()));
        drop(rx);
    }

    // Sender: owns the source store; dials the receiver.
    let sender_store = seeded_store(&scope_name);
    let sender = P2PNode::new_with_network(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
        decentraai_p2p::NetworkConfig {
            lan_discovery: false,
            ..Default::default()
        },
    )
    .unwrap();
    sender
        .dial(&format!("{addr}/p2p/{}", receiver.local_peer_id()))
        .await
        .unwrap();

    let cfg = PropagationConfig::default();

    // Connection-settling retries (same discipline as e2e_transfer): keep
    // cycling until the receiver holds both travel-worthy entries. Cycles
    // are idempotent by content-hash dedup on both ends.
    let mut report = PropagationReport::default();
    for _round in 0..25 {
        report = propagate_once(&sender_store, &sender, &sender.local_peer_id().to_string(), &cfg).await;
        let have = receiver_store.read(&scope_name, "governor", false).unwrap().len();
        if have >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    assert_eq!(report.entries_offered, 2, "verified + trusted only");
    assert!(
        report.peers_targeted >= 1,
        "the explicitly dialed receiver is always targeted"
    );
    assert_eq!(report.rejected, 0);
    assert_eq!(report.declined_peers, 0);

    // Receiver state: exactly the two imports, BOTH downgraded to candidate.
    let seen = receiver_store.read(&scope_name, "governor", false).unwrap();
    assert_eq!(seen.len(), 2, "candidate/obsolete never traveled");
    for e in &seen {
        assert_eq!(
            e.meta.status,
            MemoryStatus::Candidate,
            "imports always land as candidate locally"
        );
    }
    let ids: Vec<&str> = seen.iter().map(|e| e.entry_id.as_str()).collect();
    assert!(ids.contains(&"e-verified") && ids.contains(&"e-trusted"));

    // Idempotency on a settled link: another full cycle adds nothing new and
    // everything collapses as duplicates.
    let quiet = propagate_once(&sender_store, &sender, &sender.local_peer_id().to_string(), &cfg).await;
    let after = receiver_store.read(&scope_name, "governor", false).unwrap().len();
    assert_eq!(after, 2, "content-hash dedup keeps cycles idempotent");
    if quiet.errors == 0 {
        assert_eq!(quiet.accepted, 0);
        assert_eq!(quiet.duplicates, 2);
    }
    // else: transport still settling in rare cases; the store-level assert
    // above is the durable guarantee.

    Ok(())
}
