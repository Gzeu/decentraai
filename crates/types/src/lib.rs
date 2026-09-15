//! Shared leaf types for the DecentraAI fabric.
//!
//! This crate exists to break circular/heavy dependency edges. Types here
//! must be plain data + traits — no async, no I/O, no tokio, no libp2p
//! transport/protocol stack.
//!
//! The key re-export is [`PeerId`], which comes from `libp2p-identity`
//! (lightweight, no tokio/kad/relay). This means `decentraai_types::PeerId`
//! IS `libp2p::PeerId` — the same type — so downstream crates can use
//! either import path with zero conversion overhead.

// Re-export PeerId from libp2p-identity (the only dep that matters).
pub use libp2p_identity::PeerId;

/// Version of this types crate (bump when the PeerId wire format changes).
pub const TYPES_VERSION: &str = "1.0.0";

/// Helper: create a valid SHA2-256 multihash PeerId from a 32-byte digest.
///
/// The resulting bytes are: `[0x12, 0x20, <digest>]` — a valid multihash
/// that `PeerId::from_bytes` will accept.
#[cfg(test)]
fn test_peer_id_from_digest(digest: [u8; 32]) -> PeerId {
    let mut bytes = Vec::with_capacity(34);
    bytes.push(0x12); // SHA2-256 multihash code
    bytes.push(0x20); // 32 bytes
    bytes.extend_from_slice(&digest);
    PeerId::from_bytes(&bytes).expect("valid SHA2-256 multihash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_from_bytes_roundtrip() {
        let pid = test_peer_id_from_digest([0xAA; 32]);
        let bytes = pid.to_bytes();
        let back = PeerId::from_bytes(&bytes).expect("roundtrip");
        assert_eq!(pid, back);
    }

    #[test]
    fn peer_id_display_and_parse() {
        let pid = test_peer_id_from_digest([0xBB; 32]);
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("roundtrip");
        assert_eq!(pid, back);
    }

    #[test]
    fn peer_id_json_roundtrip() {
        let pid = test_peer_id_from_digest([0x00; 32]);
        let json = serde_json::to_string(&pid).unwrap();
        let back: PeerId = serde_json::from_str(&json).unwrap();
        assert_eq!(pid, back);
    }

    #[test]
    fn peer_id_ord_deterministic() {
        let a = test_peer_id_from_digest([0x01; 32]);
        let b = test_peer_id_from_digest([0x02; 32]);
        assert!(a < b);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Less);
    }

    #[test]
    fn peer_id_hash_in_hashmap() {
        use std::collections::HashMap;
        let pid = test_peer_id_from_digest([0x42; 32]);
        let mut map = HashMap::new();
        map.insert(pid, "value");
        assert_eq!(map.get(&pid), Some(&"value"));
    }
}
