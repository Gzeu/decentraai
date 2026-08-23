//! Memory propagation policy (M19): verified collective knowledge travels
//! to connected peers automatically — OPT-IN, bounded, deterministic.
//!
//! # Policy (all deterministic, no randomness anywhere)
//!
//! - **Eligible scopes** only: access `public` AND `allow_remote_write` AND
//!   level `network`/`fabric`/`system`. Private/team/node scopes never
//!   travel, no matter what their entries claim. Sharing stays opt-in by
//!   scope policy (the receiver's gates apply independently).
//! - **Travel-worthy entries**: lifecycle status `verified` or `trusted`
//!   with non-empty content. Candidates and obsolete entries stay local.
//! - **Peers**: currently connected peers, ordered ascending by peer id,
//!   capped at `max_peers`. Same tie-break discipline as the rest of the
//!   fabric (score desc / id asc; here there is no score, so id asc only).
//! - **Receiver side unchanged**: its own scope gates accept/reject and
//!   every imported claim lands as `candidate` locally — verification is a
//!   local act. Re-sending is idempotent (content-hash dedup), so the
//!   propagator keeps no watermark state and self-heals lost receivers.
//!
//! Pure decision functions are unit-testable; [`propagate_once`] performs
//! one bounded cycle over the real transport.

use crate::agent_memory::{MemoryStore, memory_entry_to_sync};
use decentraai_agents::memory::{MemoryEntry, MemoryLevel, MemoryScope, MemoryStatus};
use decentraai_p2p::P2PNode;
use decentraai_protocol::memory_sync::{
    MemorySyncRequest, MemorySyncResponse, MAX_SYNC_BATCH_ENTRIES,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Propagation bounds — small by design; knowledge syncs in chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropagationConfig {
    /// Connected peers targeted per cycle (id-ascending order).
    pub max_peers: usize,
    /// Newest-first entries offered per eligible scope per cycle.
    pub max_entries: usize,
}

impl Default for PropagationConfig {
    fn default() -> Self {
        Self {
            max_peers: 4,
            max_entries: MAX_SYNC_BATCH_ENTRIES,
        }
    }
}

/// Whether a scope's declared policy allows its knowledge to travel.
pub fn scope_is_eligible(scope: &MemoryScope) -> bool {
    matches!(
        scope.policy.access,
        decentraai_agents::memory::MemoryAccess::Public
    ) && scope.policy.allow_remote_write
        && matches!(
            scope.level,
            MemoryLevel::Network | MemoryLevel::Fabric | MemoryLevel::System
        )
}

/// Eligible scopes, sorted by name (deterministic).
pub fn eligible_scopes(scopes: &[MemoryScope]) -> Vec<MemoryScope> {
    let mut out: Vec<MemoryScope> = scopes
        .iter()
        .filter(|s| scope_is_eligible(s))
        .cloned()
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Travel-worthy subset: verified/trusted, non-empty content. Input order
/// preserved (the store's read() already orders newest-first).
pub fn travel_worthy(entries: &[MemoryEntry]) -> Vec<MemoryEntry> {
    entries
        .iter()
        .filter(|e| {
            !e.content.trim().is_empty()
                && matches!(e.meta.status, MemoryStatus::Verified | MemoryStatus::Trusted)
        })
        .cloned()
        .collect()
}

/// Deterministic outcome of one propagation cycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationReport {
    /// Eligible scopes that had at least one travel-worthy entry.
    pub scopes_propagated: u32,
    /// Entries offered in total (before receiver verdicts).
    pub entries_offered: u32,
    /// Peers actually contacted this cycle.
    pub peers_targeted: u32,
    /// Receiver accepted fresh entries (sum across peers).
    pub accepted: u32,
    /// Receiver collapsed duplicates (sum across peers).
    pub duplicates: u32,
    /// Competing claims linked on receivers.
    pub conflicts_linked: u32,
    /// Entries rejected by receiver policy gates.
    pub rejected: u32,
    /// Peers that answered `declined` (feature off on them).
    pub declined_peers: u32,
    /// Transport/decode failures (peer unreachable, bad response).
    pub errors: u32,
}

/// One bounded propagation cycle against the live transport.
pub async fn propagate_once(
    store: &MemoryStore,
    p2p: &P2PNode,
    local_peer: &str,
    cfg: &PropagationConfig,
) -> PropagationReport {
    let mut report = PropagationReport::default();

    // Eligible scopes + their travel-worthy batches (deterministic order).
    let mut batches: BTreeMap<String, Vec<MemoryEntry>> = BTreeMap::new();
    for scope in eligible_scopes(&store.list_scopes().unwrap_or_default()) {
        if let Ok(entries) = store.read(&scope.name, "governor", true) {
            let batch: Vec<MemoryEntry> =
                travel_worthy(&entries).into_iter().take(cfg.max_entries).collect();
            if !batch.is_empty() {
                report.entries_offered += batch.len() as u32;
                report.scopes_propagated += 1;
                batches.insert(scope.name.clone(), batch);
            }
        }
    }
    if batches.is_empty() {
        return report;
    }

    // Connected peers, id-ascending, capped. A peer with multiple parallel
    // connections must be targeted ONCE per cycle.
    let mut peers = p2p.connected_peers().await;
    peers.sort_by_key(|p| p.to_string());
    peers.dedup();
    peers.truncate(cfg.max_peers);
    report.peers_targeted = peers.len() as u32;

    for peer in peers {
        for (scope_name, entries) in &batches {
            let wire: Vec<decentraai_protocol::memory_sync::SyncMemoryEntry> =
                entries.iter().map(memory_entry_to_sync).collect();
            let request = MemorySyncRequest {
                protocol_version: MemorySyncRequest::VERSION,
                sender_node: local_peer.to_string(),
                scope: scope_name.clone(),
                entries: wire,
            };
            let Ok(bytes) = decentraai_protocol::serialize_message(&request) else {
                report.errors += 1;
                continue;
            };
            if bytes.len() > decentraai_protocol::memory_sync::MAX_MEMORY_SYNC_BYTES {
                // Batch too big for the wire: skip this scope this cycle
                // (bounded transport always wins over partial pushes).
                continue;
            }
            match p2p.request(peer, bytes).await {
                Ok(reply) => match serde_json::from_slice::<MemorySyncResponse>(&reply) {
                    Ok(resp) => {
                        if resp.declined {
                            report.declined_peers += 1;
                        } else {
                            report.accepted += resp.accepted;
                            report.duplicates += resp.duplicates;
                            report.conflicts_linked += resp.conflicts_linked;
                            report.rejected += resp.rejected;
                        }
                    }
                    Err(_) => report.errors += 1,
                },
                Err(_) => report.errors += 1,
            }
        }
    }
    report
}
