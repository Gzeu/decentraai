//! Fabric graph model — Capability Graph, Compute Graph, and Network Graph.
//!
//! DecentraAI's fabric is modeled as three overlapping graphs derived from real
//! evidence:
//!
//! - **Capability Graph**: what each node can do (models, engine, hardware
//!   class).
//! - **Network Graph**: how well nodes can communicate (RTT, bandwidth,
//!   stability, locality).
//! - **Compute Graph**: how node resources can be combined to satisfy a
//!   workload that no single node can run alone.
//!
//! The new types are pure and I/O-free. The coordinator maintains the live
//! graphs from advertisements and probes, and the planner consumes them to build
//! explainable placement plans.
//!
//! # Crate boundary
//!
//! `decentraai-compute` must stay dependency-free (it is the pure decision
//! core; `decentraai-fabric` already depends on it). The full network graph
//! lives in `decentraai-fabric::network`; here we define the minimal
//! [`LinkFacts`] value type and let the coordinator map the fabric's measured
//! links into it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::capability::ComputeCapability;
use crate::placement::ModelRequirements;

/// Minimal serializable link facts from the coordinator to a peer.
///
/// Mirrors the measured fields the fabric's network graph carries (RTT,
/// bandwidth, jitter, packet loss, locality) so the pure planner can reason
/// about reach cost without importing the fabric crate.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinkFacts {
    /// Round-trip time to the peer in microseconds. `0`/None semantics: a
    /// missing measurement is UNKNOWN and the caller applies a locality prior.
    pub rtt_us: u32,
    /// Measured throughput in megabits/sec. `0` = unmeasured.
    pub bandwidth_mbps: u32,
    /// RTT jitter (mean absolute deviation) in microseconds. `None` = unknown.
    pub jitter_us: Option<u32>,
    /// Packet loss percent (0.0..=100.0). `None` = unknown.
    pub packet_loss_percent: Option<f64>,
    /// Coarse locality label: local / same_host / lan / remote.
    pub locality: String,
}

impl LinkFacts {
    /// A conservative reach-cost estimate (ms) for moving `data_mib` to this
    /// peer: one RTT plus transfer time at the advertised bandwidth.
    pub fn reach_cost_ms(&self, data_mib: u64) -> u32 {
        let rtt = self.rtt_us / 1000;
        let bw = self.bandwidth_mbps.max(1);
        let transfer = (67_108.864_f64 / bw as f64) as u32;
        rtt.saturating_add(data_mib.saturating_mul(u64::from(transfer)) as u32)
    }
}

/// A node as seen by the fabric graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabricNode {
    pub peer_id: String,
    pub node_name: String,
    pub node_version: String,
    pub trusted: bool,
    pub healthy: bool,
    pub accepts_remote: bool,
    pub capability: ComputeCapability,
    pub availability: crate::availability::ComputeAvailability,
    /// Network link from the coordinator to this node (if measured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkFacts>,
}

impl FabricNode {
    /// Total VRAM across all GPUs advertised by the node (MiB). Multi-GPU
    /// advertisements sum every GPU's VRAM.
    pub fn total_vram_mb(&self) -> u64 {
        self.capability
            .gpu
            .as_ref()
            .map(|g| g.total_vram_mb())
            .unwrap_or(0)
    }

    /// Number of GPUs advertised by the node (`0` when GPU-less).
    pub fn gpu_count(&self) -> u32 {
        self.capability.gpu.as_ref().map(|g| g.count.max(1)).unwrap_or(0)
    }

    /// Total RAM advertised by the node (MiB).
    pub fn total_ram_mb(&self) -> u64 {
        self.capability.ram_mb
    }

    /// Whether this node can run a model by itself (single-node whole model).
    pub fn can_run_whole_model(&self, req: &ModelRequirements) -> bool {
        self.total_vram_mb() >= req.min_vram_mb && self.total_ram_mb() >= req.min_ram_mb
    }

    /// Whether this node serves or can provision the requested model.
    pub fn has_model(&self, model_id: &str) -> bool {
        self.capability.serves_or_provisions(model_id)
    }
}

/// Capability graph: index nodes by what they can do.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityGraph {
    nodes: BTreeMap<String, FabricNode>,
    /// capability → peers that advertise it.
    by_capability: BTreeMap<String, BTreeSet<String>>,
    /// model_hash → peers that serve/provision it.
    by_model: BTreeMap<String, BTreeSet<String>>,
    /// engine string → peers.
    by_engine: BTreeMap<String, BTreeSet<String>>,
}

impl CapabilityGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a node, keeping every index consistent. A replaced
    /// node's old indices are removed first so stale capability claims never
    /// survive an update.
    pub fn upsert(&mut self, node: FabricNode) {
        let peer = node.peer_id.clone();

        if let Some(old) = self.nodes.get(&peer) {
            for cap in old.advertised_capabilities() {
                if let Some(set) = self.by_capability.get_mut(&cap) {
                    set.remove(&peer);
                }
            }
            for m in &old.capability.served_models {
                if let Some(set) = self.by_model.get_mut(&m.model_hash) {
                    set.remove(&peer);
                }
            }
            for m in &old.capability.available_models {
                if let Some(set) = self.by_model.get_mut(&m.model_hash) {
                    set.remove(&peer);
                }
            }
            if let Some(set) = self.by_engine.get_mut(&old.capability.engine) {
                set.remove(&peer);
            }
        }

        for cap in node.advertised_capabilities() {
            self.by_capability
                .entry(cap)
                .or_default()
                .insert(peer.clone());
        }
        for m in &node.capability.served_models {
            self.by_model
                .entry(m.model_hash.clone())
                .or_default()
                .insert(peer.clone());
        }
        for m in &node.capability.available_models {
            self.by_model
                .entry(m.model_hash.clone())
                .or_default()
                .insert(peer.clone());
        }
        self.by_engine
            .entry(node.capability.engine.clone())
            .or_default()
            .insert(peer.clone());

        self.nodes.insert(peer, node);
    }

    pub fn nodes(&self) -> &BTreeMap<String, FabricNode> {
        &self.nodes
    }

    pub fn get(&self, peer: &str) -> Option<&FabricNode> {
        self.nodes.get(peer)
    }

    pub fn peers_for_capability(&self, cap: &str) -> Vec<&FabricNode> {
        self.by_capability
            .get(cap)
            .map(|s| s.iter().filter_map(|p| self.nodes.get(p)).collect())
            .unwrap_or_default()
    }

    pub fn peers_for_model(&self, model_hash: &str) -> Vec<&FabricNode> {
        self.by_model
            .get(model_hash)
            .map(|s| s.iter().filter_map(|p| self.nodes.get(p)).collect())
            .unwrap_or_default()
    }

    pub fn peers_for_engine(&self, engine: &str) -> Vec<&FabricNode> {
        self.by_engine
            .get(engine)
            .map(|s| s.iter().filter_map(|p| self.nodes.get(p)).collect())
            .unwrap_or_default()
    }
}

/// Compute graph: aggregate resources across nodes and reason about combined
/// capacity for workloads that no single node can run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputeGraph {
    nodes: BTreeMap<String, FabricNode>,
}

impl ComputeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, node: FabricNode) {
        self.nodes.insert(node.peer_id.clone(), node);
    }

    pub fn nodes(&self) -> &BTreeMap<String, FabricNode> {
        &self.nodes
    }

    /// Total combined VRAM of the fabric (sum of all nodes).
    pub fn total_vram_mb(&self) -> u64 {
        self.nodes.values().map(|n| n.total_vram_mb()).sum()
    }

    /// Total combined RAM of the fabric.
    pub fn total_ram_mb(&self) -> u64 {
        self.nodes.values().map(|n| n.total_ram_mb()).sum()
    }

    /// Find candidate groups of exactly `size` nodes whose combined resources
    /// meet `req`, sorted descending by total VRAM (a coarse fit heuristic).
    ///
    /// The search is exponential in group size but the fabric is small and
    /// trusted (LAN-first); a future pass can prune by prefix sums.
    pub fn candidate_groups(
        &self,
        req: &ModelRequirements,
        size: usize,
    ) -> Vec<(Vec<String>, u64)> {
        let peers: Vec<_> = self.nodes.keys().cloned().collect();
        if size == 0 || size > peers.len() {
            return Vec::new();
        }
        let mut combinations = Vec::new();
        let mut stack = Vec::new();
        fn combine(
            peers: &[String],
            start: usize,
            size: usize,
            stack: &mut Vec<String>,
            out: &mut Vec<Vec<String>>,
        ) {
            if size == 0 {
                out.push(stack.clone());
                return;
            }
            for i in start..=peers.len().saturating_sub(size) {
                stack.push(peers[i].clone());
                combine(peers, i + 1, size - 1, stack, out);
                stack.pop();
            }
        }
        combine(&peers, 0, size, &mut stack, &mut combinations);
        let mut groups = Vec::new();
        for group in combinations {
            let total_vram: u64 = group.iter().map(|p| self.nodes[p].total_vram_mb()).sum();
            let total_ram: u64 = group.iter().map(|p| self.nodes[p].total_ram_mb()).sum();
            if total_vram >= req.min_vram_mb && total_ram >= req.min_ram_mb {
                groups.push((group, total_vram));
            }
        }
        groups.sort_by_key(|b| std::cmp::Reverse(b.1));
        groups
    }
}

/// Aggregate fabric graph: capability + compute + network (link facts).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FabricGraph {
    pub capability: CapabilityGraph,
    pub compute: ComputeGraph,
    /// peer → measured link facts from the coordinator (if measured).
    pub links: BTreeMap<String, LinkFacts>,
}

impl FabricGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, node: FabricNode) {
        let node_clone = node.clone();
        self.capability.upsert(node);
        self.compute.upsert(node_clone);
    }

    /// Records (or replaces) the link facts to a peer.
    pub fn set_link(&mut self, peer: &str, link: LinkFacts) {
        self.links.insert(peer.to_string(), link);
    }

    pub fn get(&self, peer: &str) -> Option<&FabricNode> {
        self.compute.nodes().get(peer)
    }

    /// Computes the combined resource fit score for a candidate group.
    pub fn group_score(&self, group: &[String], req: &ModelRequirements) -> f64 {
        let total_vram: u64 = group
            .iter()
            .filter_map(|p| self.get(p))
            .map(|n| n.total_vram_mb())
            .sum();
        let total_ram: u64 = group
            .iter()
            .filter_map(|p| self.get(p))
            .map(|n| n.total_ram_mb())
            .sum();
        let vram_fit = (total_vram as f64 / req.min_vram_mb.max(1) as f64).min(10.0);
        let ram_fit = (total_ram as f64 / req.min_ram_mb.max(1) as f64).min(10.0);
        // Network cost: sum of reach costs across the group.
        let net_cost: f64 = group
            .iter()
            .map(|p| f64::from(self.link_reach_cost_ms(p, req.context_tokens as u64 / 1024)))
            .sum();
        let net_score = 1.0 / (1.0 + net_cost / 1000.0);
        vram_fit * 0.5 + ram_fit * 0.3 + net_score * 0.2
    }

    fn link_reach_cost_ms(&self, peer: &str, data_mib: u64) -> u32 {
        self.links
            .get(peer)
            .map(|l| l.reach_cost_ms(data_mib))
            .unwrap_or(0)
    }
}

/// Advertised capabilities of a node, used by the capability graph index.
pub trait AdvertisedCapabilities {
    fn advertised_capabilities(&self) -> Vec<String>;
}

impl AdvertisedCapabilities for ComputeCapability {
    fn advertised_capabilities(&self) -> Vec<String> {
        let mut caps = vec!["compute".to_string(), self.engine.clone()];
        if self.gpu.is_some() {
            caps.push("gpu".to_string());
        }
        for m in &self.served_models {
            caps.push(format!("model:{}", m.model_hash));
        }
        for m in &self.available_models {
            caps.push(format!("model:{}", m.model_hash));
        }
        caps
    }
}

impl AdvertisedCapabilities for FabricNode {
    fn advertised_capabilities(&self) -> Vec<String> {
        self.capability.advertised_capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{ComputeCapability, GpuSpec};

    fn node(peer: &str, vram_mb: u64, ram_mb: u64) -> FabricNode {
        FabricNode {
            peer_id: peer.to_string(),
            node_name: peer.to_string(),
            node_version: "1.0.0".to_string(),
            trusted: true,
            healthy: true,
            accepts_remote: true,
            capability: ComputeCapability {
                cpu_cores: 8,
                ram_mb,
                gpu: Some(GpuSpec::simple("test", vram_mb, "test")),
                engine: "llama_server".to_string(),
                served_models: vec![],
                can_provision: false,
                available_models: vec![],
            },
            availability: crate::availability::ComputeAvailability::ready(),
            link: None,
        }
    }

    #[test]
    fn capability_graph_indexes_nodes() {
        let mut g = CapabilityGraph::new();
        g.upsert(node("peer-a", 16_000, 64_000));
        g.upsert(node("peer-b", 8_000, 32_000));
        assert_eq!(g.nodes().len(), 2);
        assert!(g.peers_for_engine("llama_server").len() >= 2);
        assert_eq!(g.peers_for_capability("gpu").len(), 2);
    }

    #[test]
    fn capability_graph_replaces_indices_on_update() {
        let mut g = CapabilityGraph::new();
        g.upsert(node("peer-a", 16_000, 64_000));
        // Update the same peer to drop its GPU: the "gpu" index must shrink.
        let mut updated = node("peer-a", 0, 64_000);
        updated.capability.gpu = None;
        g.upsert(updated);
        assert_eq!(g.peers_for_capability("gpu").len(), 0);
    }

    #[test]
    fn compute_graph_finds_candidate_groups() {
        let mut g = ComputeGraph::new();
        g.upsert(node("peer-a", 40_000, 64_000));
        g.upsert(node("peer-b", 40_000, 64_000));
        g.upsert(node("peer-c", 40_000, 64_000));
        let req = ModelRequirements {
            model_id: "big.gguf".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 70_000,
            min_ram_mb: 60_000,
            ..Default::default()
        };
        let groups = g.candidate_groups(&req, 2);
        assert!(
            !groups.is_empty(),
            "two 40 GiB nodes should combine to satisfy a 70 GiB workload"
        );
    }

    #[test]
    fn compute_graph_returns_empty_when_insufficient() {
        let mut g = ComputeGraph::new();
        g.upsert(node("peer-a", 8_000, 16_000));
        let req = ModelRequirements {
            model_id: "big.gguf".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 70_000,
            min_ram_mb: 60_000,
            ..Default::default()
        };
        assert!(g.candidate_groups(&req, 2).is_empty());
    }

    #[test]
    fn link_facts_reach_cost_is_deterministic() {
        let link = LinkFacts {
            rtt_us: 2_000,
            bandwidth_mbps: 1_000,
            jitter_us: None,
            packet_loss_percent: None,
            locality: "lan".to_string(),
        };
        let cost = link.reach_cost_ms(1024);
        assert!(cost > 2, "one RTT plus transfer time");
    }
}
