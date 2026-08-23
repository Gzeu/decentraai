//! Memory-sync E2E (M19): two real libp2p nodes on loopback exchanging a
//! bounded collective-memory batch through the existing request/response
//! channel — no second network protocol.
//!
//! Verifies the full path: outbound encode → transport → inbound decode in
//! the swarm cascade → deterministic additive merge into the receiver's
//! store → encoded response back to the sender.

use anyhow::Result;
use decentraai_agents::memory::{
    MemoryEntry, MemoryLevel, MemoryPolicy, MemoryStatus, WriteOutcome,
};
use decentraai_distributed::agent_memory::{MemoryStore, sync_entry_to_memory};
use decentraai_identity::Identity;
use decentraai_p2p::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode};
use decentraai_protocol::memory_sync::{
    MemorySyncRequest, MemorySyncResponse, SyncEntryMeta, SyncMemoryEntry,
};
use std::sync::Arc;

fn public_remote_scope(name: &str, owner: &str) -> decentraai_agents::memory::MemoryScope {
    let policy = MemoryPolicy::default().public().with_remote_write();
    decentraai_agents::memory::MemoryScope::new(name, owner, MemoryLevel::Fabric)
        .with_policy(policy)
}

fn wire_entry(id: &str, content: &str, subject: &str, status: &str) -> SyncMemoryEntry {
    SyncMemoryEntry {
        entry_id: id.to_string(),
        author_agent: "remote-researcher".to_string(),
        author_node: "peer-far".to_string(),
        content: content.to_string(),
        created_at_ms: 1724428800000,
        meta: SyncEntryMeta {
            kind: "learning".to_string(),
            status: status.to_string(),
            version: 7,
            subject_key: subject.to_string(),
            source: "execution".to_string(),
            confidence: 95,
            evidence_ref: String::new(),
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_sync_survives_the_wire_and_respects_trust_boundaries() -> Result<()> {
    // Receiver node: owns the store + handler.
    let store = Arc::new(MemoryStore::open(std::path::Path::new(":memory:")).unwrap());
    store
        .register_scope(&public_remote_scope("team.knowledge", "governor"))
        .unwrap();
    // Pre-existing claim on subject q:x so the batch produces a conflict link
    // and an exact duplicate.
    let mut local = MemoryEntry::new(
        "local1",
        "team.knowledge",
        "governor",
        "node-b",
        "shared lesson",
    );
    local.created_at_ms = 100;
    local.meta.subject_key = "q:x".to_string();
    assert_eq!(
        store
            .write_checked("team.knowledge", &local, "governor", true, false, false)
            .unwrap(),
        WriteOutcome::Stored
    );

    let receiver = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    let addr = receiver.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    {
        let store_for_handler = store.clone();
        let mut receiver_mut = receiver.clone();
        receiver_mut.set_on_memory_sync(move |_peer, req| {
            use decentraai_protocol::memory_sync::MemorySyncResponse;
            if !req.is_shape_valid() {
                return serde_json::to_vec(&MemorySyncResponse {
                    protocol_version: 1,
                    declined: false,
                    accepted: 0,
                    duplicates: 0,
                    conflicts_linked: 0,
                    expired: 0,
                    rejected: req.entries.len() as u32,
                })
                .unwrap_or_default();
            }
            let mut accepted = 0u32;
            let mut duplicates = 0u32;
            let mut conflicts_linked = 0u32;
            let mut rejected = 0u32;
            for se in req.entries {
                let entry = sync_entry_to_memory(se, &req.scope);
                match store_for_handler.write_checked(
                    &req.scope,
                    &entry,
                    "memory-sync",
                    false,
                    false,
                    false,
                ) {
                    Ok(WriteOutcome::Stored) => accepted += 1,
                    Ok(WriteOutcome::Duplicate { .. }) => duplicates += 1,
                    Ok(WriteOutcome::CompetingClaim { .. }) => {
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
        });
        // The handler slot is Arc-shared with the swarm task; dropping this
        // handle is safe, but keep it explicit that registration happened.
        drop(receiver_mut);
    }

    // Sender node dials the receiver.
    let sender = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    sender
        .dial(&format!("{addr}/p2p/{}", receiver.local_peer_id()))
        .await
        .unwrap();

    // Batch: fresh entry, exact duplicate of the pre-seeded local one, and a
    // competing claim on the same subject. The remote claims present
    // themselves as "trusted" — the receiver MUST downgrade them.
    let request = MemorySyncRequest {
        protocol_version: MemorySyncRequest::VERSION,
        sender_node: "peer-far".to_string(),
        scope: "team.knowledge".to_string(),
        entries: vec![
            wire_entry("r1", "fresh remote lesson", "q:fresh", "candidate"),
            wire_entry("dup", "shared lesson", "q:same", "trusted"),
            wire_entry("comp", "competing view on x", "q:x", "trusted"),
        ],
    };
    let bytes = decentraai_protocol::serialize_message(&request).unwrap();
    assert!(bytes.len() <= decentraai_protocol::memory_sync::MAX_MEMORY_SYNC_BYTES);

    // Connection settling retry (same discipline as e2e_transfer).
    let mut response: Option<MemorySyncResponse> = None;
    for _ in 0..20 {
        match sender
            .request(receiver.local_peer_id(), bytes.clone())
            .await
        {
            Ok(reply) => {
                response = serde_json::from_slice(&reply).ok();
                if response.is_some() {
                    break;
                }
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    }
    let resp = response.expect("receiver answered the memory-sync batch");
    assert!(!resp.declined);
    assert_eq!(resp.accepted, 2, "fresh + competing stored");
    assert_eq!(resp.duplicates, 1, "exact duplicate collapsed");
    assert_eq!(resp.conflicts_linked, 1, "competitor linked to local1");

    // Trust boundary: imported claims are Candidate locally, provenance kept.
    let seen = store.read("team.knowledge", "governor", false).unwrap();
    let comp = seen.iter().find(|e| e.entry_id == "comp").unwrap();
    assert_eq!(
        comp.meta.status,
        MemoryStatus::Candidate,
        "remote 'trusted' downgraded"
    );
    assert_eq!(comp.author_agent, "remote-researcher");
    let detail = comp.meta.detail.as_ref().expect("provenance preserved");
    assert_eq!(detail.source, "execution");
    assert_eq!(detail.confidence, 95);
    // Conflict links are bidirectional across the wire boundary.
    let l1 = seen.iter().find(|e| e.entry_id == "local1").unwrap();
    assert!(l1.meta.competes_with.contains(&"comp".to_string()));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn node_without_handler_declines_explicitly() -> Result<()> {
    // A node that never registered a memory-sync handler answers `declined`
    // instead of silence or an empty body.
    let receiver = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    let addr = receiver.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let sender = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    sender
        .dial(&format!("{addr}/p2p/{}", receiver.local_peer_id()))
        .await
        .unwrap();

    let request = MemorySyncRequest {
        protocol_version: 1,
        sender_node: "peer-x".to_string(),
        scope: "anything".to_string(),
        entries: vec![wire_entry("e1", "x", "q:x", "candidate")],
    };
    let bytes = decentraai_protocol::serialize_message(&request).unwrap();
    let mut resp: Option<MemorySyncResponse> = None;
    for _ in 0..20 {
        match sender
            .request(receiver.local_peer_id(), bytes.clone())
            .await
        {
            Ok(reply) => {
                resp = serde_json::from_slice(&reply).ok();
                if resp.is_some() {
                    break;
                }
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    }
    let resp = resp.expect("declined response received");
    assert!(resp.declined);
    assert_eq!(resp.accepted + resp.rejected + resp.duplicates, 0);
    Ok(())
}
