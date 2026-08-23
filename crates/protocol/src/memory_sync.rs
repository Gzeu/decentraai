//! Memory sync wire schema (M18) — collective-memory propagation over the
//! EXISTING fabric transport.
//!
//! # Design (per the M18 contract)
//!
//! Cross-node memory propagation reuses the established request/response
//! channel (`crates/p2p`) and the DFCP discipline — this module defines ONLY
//! the payload schema plus its bounds, not a new network protocol. A node
//! that opts in (scope policy `allow_remote_write` + operator config) sends a
//! [`MemorySyncRequest`] to a trusted peer; the peer runs the deterministic,
//! additive-only merge ([`decentraai_agents::memory::MemoryRegistry::
//! merge_batch`] / `MemoryStore::write_checked`) and answers with a bounded
//! [`MemorySyncResponse`]. Nothing is ever overwritten on either side:
//! duplicates collapse by content hash, competing claims are linked.
//!
//! Security posture (same as every wire type here):
//! - `deny_unknown_fields` everywhere — hostile/unknown fields are rejects.
//! - Hard byte bound ([`MAX_MEMORY_SYNC_BYTES`]) enforced by callers BEFORE
//!   decoding, matching DFCP practice.
//! - Entries carry their own provenance/status; trust facts are resolved by
//!   the receiving store's policy gates, never from payload claims.

use serde::{Deserialize, Serialize};

/// Maximum serialized size of one memory sync request (64 KiB). Batches are
/// small by design: knowledge syncs in chunks, never in bulk dumps.
pub const MAX_MEMORY_SYNC_BYTES: usize = 64 * 1024;

/// One memory entry as it travels between nodes for synchronization.
///
/// Field-minimal projection of [`decentraai_agents::memory::MemoryEntry`] —
/// the receiving side reconstructs the full entry with conservative
/// defaults for anything absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncMemoryEntry {
    /// Stable entry id within the scope.
    pub entry_id: String,
    /// Authoring agent.
    pub author_agent: String,
    /// Node the author ran on.
    pub author_node: String,
    /// The knowledge content itself (bounded by the message cap).
    pub content: String,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
    /// Full metadata (kind/status/version/subject/provenance/history).
    #[serde(default)]
    pub meta: SyncEntryMeta,
}

/// Metadata projection for [`SyncMemoryEntry`]. Kept structurally separate
/// from the domain type so wire evolution cannot silently change domain
/// semantics; conversion happens at the runtime boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncEntryMeta {
    /// Knowledge kind tag (`observation`, `learning`, …).
    #[serde(default)]
    pub kind: String,
    /// Lifecycle status tag (`candidate`, `verified`, `trusted`, `obsolete`).
    #[serde(default)]
    pub status: String,
    /// Monotonic version.
    #[serde(default)]
    pub version: u32,
    /// Conflict-grouping subject key (may be empty).
    #[serde(default)]
    pub subject_key: String,
    /// Provenance source tag (e.g. `execution`), when claimed.
    #[serde(default)]
    pub source: String,
    /// Confidence percent 0..=100.
    #[serde(default)]
    pub confidence: u8,
    /// Evidence reference, when the claim is evidence-backed.
    #[serde(default)]
    pub evidence_ref: String,
}

/// A request to merge a batch of memory entries into one named scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySyncRequest {
    /// Protocol version (currently 1) — reject before decode surprises.
    pub protocol_version: u32,
    /// The sending node's peer id (informational; identity comes from the
    /// transport, never from this field).
    pub sender_node: String,
    /// Target scope name on the receiving node.
    pub scope: String,
    /// Entries to merge (bounded batch).
    pub entries: Vec<SyncMemoryEntry>,
}

impl MemorySyncRequest {
    /// The current protocol version.
    pub const VERSION: u32 = 1;

    /// Whether this request is well-formed enough to process further
    /// (version match + non-empty scope + bounded entry count).
    pub fn is_shape_valid(&self) -> bool {
        self.protocol_version == Self::VERSION
            && !self.scope.is_empty()
            && self.entries.len() <= MAX_SYNC_BATCH_ENTRIES
    }
}

/// Upper bound on entries per sync batch (keeps worst-case decode work
/// bounded even under the byte cap).
pub const MAX_SYNC_BATCH_ENTRIES: usize = 128;

/// The receiver's deterministic verdict on a sync batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySyncResponse {
    /// Protocol version (mirrors the request).
    pub protocol_version: u32,
    /// True when the receiver does not run a memory-sync handler at all
    /// (feature off). All counters are zero; the sender must not retry.
    #[serde(default)]
    pub declined: bool,
    /// Fresh entries stored (including linked competing claims).
    pub accepted: u32,
    /// Exact duplicates skipped.
    pub duplicates: u32,
    /// Accepted entries that linked into existing conflicts.
    pub conflicts_linked: u32,
    /// Entries dropped as already expired.
    pub expired: u32,
    /// Entries rejected by policy gates.
    pub rejected: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str) -> SyncMemoryEntry {
        SyncMemoryEntry {
            entry_id: id.to_string(),
            author_agent: "researcher".to_string(),
            author_node: "node-1".to_string(),
            content: "verified lesson".to_string(),
            created_at_ms: 1724428800000,
            meta: SyncEntryMeta {
                kind: "learning".to_string(),
                status: "verified".to_string(),
                version: 2,
                subject_key: "q:backoff".to_string(),
                source: "execution".to_string(),
                confidence: 90,
                evidence_ref: "aud-77".to_string(),
            },
        }
    }

    #[test]
    fn round_trips_and_rejects_unknown_fields() {
        let req = MemorySyncRequest {
            protocol_version: 1,
            sender_node: "node-1".to_string(),
            scope: "team.knowledge".to_string(),
            entries: vec![sample_entry("e1")],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.len() < MAX_MEMORY_SYNC_BYTES);
        let back: MemorySyncRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);

        // Unknown field → hard reject (closed schema).
        let hostile = r#"{"protocol_version":1,"sender_node":"x","scope":"s",
            "entries":[],"exec":"rm -rf"}"#;
        assert!(serde_json::from_str::<MemorySyncRequest>(hostile).is_err());
    }

    #[test]
    fn shape_validation_bounds_the_batch() {
        let ok = MemorySyncRequest {
            protocol_version: 1,
            sender_node: "n".to_string(),
            scope: "s".to_string(),
            entries: vec![sample_entry("e1"); MAX_SYNC_BATCH_ENTRIES],
        };
        assert!(ok.is_shape_valid());
        // Wrong version.
        let bad_version = MemorySyncRequest {
            protocol_version: 99,
            ..ok.clone()
        };
        assert!(!bad_version.is_shape_valid());
        // Empty scope.
        let empty_scope = MemorySyncRequest {
            scope: String::new(),
            ..ok.clone()
        };
        assert!(!empty_scope.is_shape_valid());
        // Oversized batch.
        let too_many = MemorySyncRequest {
            entries: vec![sample_entry("e"); MAX_SYNC_BATCH_ENTRIES + 1],
            ..ok
        };
        assert!(!too_many.is_shape_valid());
    }

    #[test]
    fn response_mirrors_merge_report_semantics() {
        let resp = MemorySyncResponse {
            protocol_version: 1,
            declined: false,
            accepted: 3,
            duplicates: 2,
            conflicts_linked: 1,
            expired: 0,
            rejected: 1,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: MemorySyncResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
        // Declined responses parse with the default flag and carry no work.
        let declined: MemorySyncResponse =
            serde_json::from_str(r#"{"protocol_version":1,"declined":true,"accepted":0,"duplicates":0,"conflicts_linked":0,"expired":0,"rejected":0}"#).unwrap();
        assert!(declined.declined);
        // Conservation: every reported entry is accounted for exactly once.
        let total_in_batch: u32 = 3 + 2 + 1; // accepted + duplicates + rejected
        assert_eq!(
            back.accepted + back.duplicates + back.expired + back.rejected,
            total_in_batch
        );
    }
}
