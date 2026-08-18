//! AgentManager — the runtime half of the collective-intelligence agent
//! model (P1). The pure shapes live in `decentraai-agents`; this manager
//! holds the node's local agents plus the last advertisement seen from every
//! remote peer, builds the signed wire bytes and exposes a flattened view
//! for the dashboard.
//!
//! Mirrors `ComputeManager`'s role for compute advertisements: the *stateful*
//! bookkeeping that turns the pure [`AgentAdvertisement`] into a live
//! registry. Broadcasting is done by the caller (a periodic loop in
//! `node-cli`, exactly like the compute broadcaster).

use decentraai_agents::{AgentAdvertisement, AgentRecord};
use decentraai_protocol::{
    SignedAgentAdvertisement, sign_agent_advertisement, verify_signed_agent_advertisement,
};
use libp2p::PeerId;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A flattened agent row for the dashboard: which node hosts it and whether
/// it is local or remote.
#[derive(Debug, Clone)]
pub struct AgentView {
    pub peer_id: PeerId,
    pub node_name: String,
    pub remote: bool,
    pub record: AgentRecord,
}

/// Runtime registry of local + remote logical agents.
#[derive(Debug)]
pub struct AgentManager {
    local_peer: PeerId,
    node_name: String,
    /// The node's own logical agents (in registration order).
    local: Mutex<Vec<AgentRecord>>,
    /// Last advertisement per remote peer.
    remote: Mutex<BTreeMap<PeerId, AgentAdvertisement>>,
    /// When the last advertisement per peer was received (stale detection).
    last_seen: Mutex<HashMap<PeerId, Instant>>,
    signing_key: Option<[u8; 32]>,
}

impl AgentManager {
    /// A manager with no local agents and an empty remote view.
    pub fn new(local_peer: PeerId, node_name: String) -> Self {
        Self {
            local_peer,
            node_name,
            local: Mutex::new(Vec::new()),
            remote: Mutex::new(BTreeMap::new()),
            last_seen: Mutex::new(HashMap::new()),
            signing_key: None,
        }
    }

    /// Enables signed advertisements (P3 discipline: without a signing key the
    /// manager still works, but peers reject unsigned agent advertisements —
    /// mirrors the compute advertisement policy).
    pub fn set_signing_key(&mut self, key: [u8; 32]) {
        self.signing_key = Some(key);
    }

    /// Replaces the node's full set of logical agents.
    pub fn set_local_agents(&self, records: Vec<AgentRecord>) {
        let mut local = self.local.lock().unwrap();
        local.clear();
        local.extend(records);
    }

    /// Adds (or replaces) one local agent by id.
    pub fn register_local(&self, record: AgentRecord) {
        let mut local = self.local.lock().unwrap();
        if let Some(existing) = local.iter_mut().find(|a| a.agent_id == record.agent_id) {
            *existing = record;
        } else {
            local.push(record);
        }
    }

    /// Removes a local agent by id; returns whether it existed.
    pub fn unregister_local(&self, agent_id: &str) -> bool {
        let mut local = self.local.lock().unwrap();
        let before = local.len();
        local.retain(|a| a.agent_id != agent_id);
        local.len() != before
    }

    /// The node's own agents (cloned, registration order).
    pub fn local_agents(&self) -> Vec<AgentRecord> {
        self.local.lock().unwrap().clone()
    }

    /// The current advertisement this node would broadcast.
    pub fn advertisement(&self) -> AgentAdvertisement {
        AgentAdvertisement::new(self.local_peer, self.node_name.clone(), self.local_agents())
            .announced_at(now_ms())
    }

    /// Serializes the advertisement; signs it when a signing key is set.
    pub fn advertisement_wire_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let adv = self.advertisement();
        let bytes = serde_json::to_vec(&adv)?;
        if let Some(key) = self.signing_key {
            let signed = sign_agent_advertisement(&key, &bytes);
            Ok(serde_json::to_vec(&signed)?)
        } else {
            Ok(bytes)
        }
    }

    /// Records a remote peer's advertisement. The caller MUST verify the
    /// signature (for signed frames) before calling this — the manager does
    /// not re-verify.
    pub fn process_advertisement(&self, adv: AgentAdvertisement) {
        let peer = adv.peer_id;
        self.remote.lock().unwrap().insert(peer, adv);
        self.last_seen.lock().unwrap().insert(peer, Instant::now());
    }

    /// Removes a peer's agent view entirely.
    pub fn drop_peer(&self, peer: &PeerId) {
        self.remote.lock().unwrap().remove(peer);
        self.last_seen.lock().unwrap().remove(peer);
    }

    /// Flattened view of every agent the node knows about (local + remote),
    /// deterministic: remote peers sorted by PeerId, each peer's agents in
    /// advertisement order, local agents first.
    pub fn view(&self) -> Vec<AgentView> {
        let mut out = Vec::new();
        for record in self.local_agents() {
            out.push(AgentView {
                peer_id: self.local_peer,
                node_name: self.node_name.clone(),
                remote: false,
                record,
            });
        }
        let remote = self.remote.lock().unwrap();
        for (peer, adv) in remote.iter() {
            for record in &adv.agents {
                out.push(AgentView {
                    peer_id: *peer,
                    node_name: adv.node_name.clone(),
                    remote: true,
                    record: record.clone(),
                });
            }
        }
        out
    }

    /// Total number of known agents (local + remote).
    pub fn total_count(&self) -> usize {
        let local = self.local.lock().unwrap().len();
        let remote: usize = self
            .remote
            .lock()
            .unwrap()
            .values()
            .map(|a| a.agents.len())
            .sum();
        local + remote
    }

    /// Local agent count.
    pub fn local_count(&self) -> usize {
        self.local.lock().unwrap().len()
    }

    /// Remote peers that currently advertise agents.
    pub fn remote_peer_count(&self) -> usize {
        self.remote.lock().unwrap().len()
    }

    /// How long ago a peer was last seen (if known).
    pub fn last_seen(&self, peer: &PeerId) -> Option<Instant> {
        self.last_seen.lock().unwrap().get(peer).copied()
    }

    /// Drops remote agent views that have not refreshed within `stale_after`;
    /// returns the number of peers evicted. Network hiccups never punish
    /// anything — this is pure bookkeeping.
    pub fn prune_stale(&self, stale_after: Duration) -> usize {
        let now = Instant::now();
        let mut evicted = 0usize;
        let mut last_seen = self.last_seen.lock().unwrap();
        let stale_peers: Vec<PeerId> = last_seen
            .iter()
            .filter_map(|(p, at)| {
                if now.duration_since(*at) > stale_after {
                    Some(*p)
                } else {
                    None
                }
            })
            .collect();
        for peer in stale_peers {
            last_seen.remove(&peer);
            self.remote.lock().unwrap().remove(&peer);
            evicted += 1;
        }
        evicted
    }
}

/// Verifies a signed agent advertisement against the embedded claiming peer
/// (convenience wrapper for the P2P handler).
pub fn verify_agent_advertisement_signed(
    signed: &SignedAgentAdvertisement,
) -> anyhow::Result<AgentAdvertisement> {
    let adv: AgentAdvertisement = serde_json::from_slice(&signed.advertisement)?;
    let claiming_peer = adv.peer_id;
    verify_signed_agent_advertisement(signed, &claiming_peer)?;
    Ok(adv)
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::{
        AgentRecord, ROLE_GENERALIST, ROLE_SPECIALIST, TOOL_KIND_HTTP, ToolDescriptor,
    };
    use decentraai_hub::capability::{CapabilityKind, Provenance};
    use decentraai_identity::Identity;
    use libp2p::identity::Keypair;

    fn peer() -> PeerId {
        PeerId::from(Keypair::generate_ed25519().public())
    }

    fn generalist(id: &str) -> AgentRecord {
        AgentRecord::new(id, "Generalist", ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_capability(CapabilityKind::Reasoning, Provenance::Inferred)
    }

    fn ocr_agent(id: &str) -> AgentRecord {
        AgentRecord::new(id, "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
            .with_tool(ToolDescriptor::new("ocr.api", TOOL_KIND_HTTP))
    }

    #[test]
    fn local_registry_and_view() {
        let manager = AgentManager::new(peer(), "node".into());
        assert_eq!(manager.total_count(), 0);
        manager.register_local(generalist("a:generalist"));
        manager.register_local(ocr_agent("a:ocr"));
        manager.register_local(generalist("a:generalist")); // replace
        assert_eq!(manager.local_count(), 2);
        assert_eq!(manager.total_count(), 2);
        let view = manager.view();
        assert_eq!(view.len(), 2);
        assert!(view.iter().all(|v| !v.remote));
        assert!(manager.unregister_local("a:ocr"));
        assert!(!manager.unregister_local("a:ocr"));
        assert_eq!(manager.local_count(), 1);
    }

    #[test]
    fn remote_advertisements_are_tracked_and_evicted() {
        let manager = AgentManager::new(peer(), "coordinator".into());
        let remote_peer = peer();
        let adv = AgentAdvertisement::new(remote_peer, "worker-1", vec![ocr_agent("w:ocr")]);
        manager.process_advertisement(adv);
        assert_eq!(manager.remote_peer_count(), 1);
        assert_eq!(manager.total_count(), 1);
        let view = manager.view();
        assert_eq!(view.len(), 1);
        assert!(view[0].remote);
        assert_eq!(view[0].node_name, "worker-1");

        // Stale eviction.
        manager
            .last_seen
            .lock()
            .unwrap()
            .insert(remote_peer, Instant::now() - Duration::from_secs(100));
        let evicted = manager.prune_stale(Duration::from_secs(30));
        assert_eq!(evicted, 1);
        assert_eq!(manager.remote_peer_count(), 0);
        assert_eq!(manager.total_count(), 0);
    }

    #[test]
    fn signed_wire_bytes_round_trip_verify() {
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        let mut manager = AgentManager::new(peer, "node".into());
        manager.register_local(generalist("n:generalist"));
        manager.set_signing_key(identity.signing_key_bytes());
        let bytes = manager.advertisement_wire_bytes().unwrap();
        let signed: SignedAgentAdvertisement = serde_json::from_slice(&bytes).unwrap();
        let adv = verify_agent_advertisement_signed(&signed).unwrap();
        assert_eq!(adv.agent_count(), 1);
        assert_eq!(adv.peer_id, peer);
    }

    #[test]
    fn unsigned_wire_bytes_are_plain_advertisement() {
        let manager = AgentManager::new(peer(), "node".into());
        manager.register_local(generalist("n:generalist"));
        let bytes = manager.advertisement_wire_bytes().unwrap();
        // Without a signing key the payload is the plain advertisement.
        let adv: AgentAdvertisement = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(adv.agent_count(), 1);
    }

    fn peer_of(identity: &Identity) -> PeerId {
        let pubkey = identity.public_key();
        let kp = libp2p::identity::ed25519::PublicKey::try_from_bytes(&pubkey.to_bytes()).unwrap();
        PeerId::from_public_key(&libp2p::identity::PublicKey::from(kp))
    }
}
