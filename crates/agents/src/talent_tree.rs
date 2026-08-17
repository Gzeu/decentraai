//! P8 talent tree / capability graph — the dynamic prerequisites model.
//!
//! # What this is (and is not)
//!
//! This is NOT gamification ("levels", "XP", "rank badges") and NOT a fixed,
//! hard-coded list of milestones. It is a *lookup table of prerequisites* so
//! the planner can answer a single, concrete question:
//!
//! > "Given the capabilities I already have and the memory budget I can spend,
//! >   which further capability can I unlock, and what is the cheapest chain of
//! >   prerequisites to get there?"
//!
//! Each capability is a [`TalentNode`] in a graph. An edge from capability A
//! to capability B means "you must hold A before B is available". The graph is
//! **registry-driven**: a new capability is added with [`TalentTree::add`] —
//! no code changes at the graph level. [`seed_talent_tree`] only provides a
//! sensible default topology matching the architecture doc.
//!
//! # The honesty boundary
//!
//! [`CapabilityKind`] is a *fixed* enum from the hub (no `Other` variant), so
//! composite capabilities with no native variant ("SemanticSearch", "RAG",
//! "MCP", "KnowledgeAgent", …) are represented by the **closest existing
//! kind** (e.g. RAG→`Retrieval`, MCP→`ToolCalling`, CodeReview→`Coding`).
//! The meaning of such a node lives in the *graph topology* (its position,
//! its prerequisites), not in the enum name — and this module documents that
//! mapping openly rather than pretending a fixed enum can name every composite.
//!
//! Nodes carry `experimental: true` and low `confidence` when the composite is
//! **not production-verified**. An experimental node is never claimed as
//! verified by a planner consumer: it is offered as "reachable in principle,
//! but treat its output cautiously".
//!
//! The module is pure (no I/O, no async): every type derives `serde` so a tree
//! can be shipped to another node or persisted, and every decision is a pure
//! function unit tests drive with synthetic inputs.

use decentraai_hub::capability::{CapabilityKind, Provenance};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A single node in the capability graph.
///
/// `prerequisites` are the capabilities that must already be held before this
/// node becomes available. `resource_estimate_mb` is the planner's memory cost
/// to hold this capability. `provenance_required` is the minimum
/// [`Provenance`] a consumer must accept to treat the capability as *claimed*;
/// `None` means no provenance gate. `confidence` is the node's claimed
/// reliability, clamped to `0.0..=1.0` by [`TalentNode::new`]. `experimental`
/// flags nodes that are not production-verified (honesty: an experimental node
/// is never claimed as verified by consumers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TalentNode {
    /// The capability this node provides.
    pub capability: CapabilityKind,
    /// Capabilities that must be held before this node is available.
    pub prerequisites: Vec<CapabilityKind>,
    /// Memory needed to hold this capability, in MiB (planner budget input).
    pub resource_estimate_mb: u64,
    /// Minimum provenance accepted before this capability counts as claimed.
    pub provenance_required: Option<Provenance>,
    /// Reliability of this capability, clamped to `0.0..=1.0`.
    pub confidence: f32,
    /// True when this capability is not production-verified (never claimed as
    /// verified).
    pub experimental: bool,
}

impl TalentNode {
    /// Builds a node, clamping `confidence` into `0.0..=1.0`.
    ///
    /// The clamp happens at the construction boundary so a node that reaches
    /// the tree always carries a defensible fraction; wire data is trusted
    /// only after passing through this constructor.
    pub fn new(
        capability: CapabilityKind,
        prerequisites: Vec<CapabilityKind>,
        resource_estimate_mb: u64,
        provenance_required: Option<Provenance>,
        confidence: f32,
        experimental: bool,
    ) -> Self {
        Self {
            capability,
            prerequisites,
            resource_estimate_mb,
            provenance_required,
            confidence: confidence.clamp(0.0, 1.0),
            experimental,
        }
    }

    /// The direct prerequisites of this node (an implicit edge from each
    /// prerequisite to this node).
    pub fn direct_prerequisites(&self) -> &[CapabilityKind] {
        &self.prerequisites
    }
}

/// Errors from building/querying a [`TalentTree`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TalentError {
    /// A node for this capability is already registered; the tree is a
    /// `BTreeMap<CapabilityKind, _>`, so a capability is a unique key.
    #[error("talent node for capability '{capability:?}' already exists")]
    DuplicateNode { capability: CapabilityKind },
    /// A capability was queried that has no node in the tree.
    #[error("unknown capability '{capability:?}' in talent tree")]
    UnknownCapability { capability: CapabilityKind },
}

/// The dynamic capability graph: a map from a capability to its [`TalentNode`].
///
/// Backed by a `BTreeMap` (not a `HashMap`) so iteration — and therefore
/// every returned capability list — is deterministic (ordered by
/// `CapabilityKind`, which is `Ord`). This determinism is a hard requirement:
/// the planner ranks and diffs capability sets, so equal inputs must always
/// yield equal outputs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TalentTree {
    nodes: BTreeMap<CapabilityKind, TalentNode>,
}

impl TalentTree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a node. Fails with [`TalentError::DuplicateNode`] when the
    /// capability is already present — a capability is a unique key.
    pub fn add(&mut self, node: TalentNode) -> Result<(), TalentError> {
        let capability = node.capability;
        if self.nodes.contains_key(&capability) {
            return Err(TalentError::DuplicateNode { capability });
        }
        self.nodes.insert(capability, node);
        Ok(())
    }

    /// Looks up a node by capability.
    pub fn get(&self, kind: CapabilityKind) -> Option<&TalentNode> {
        self.nodes.get(&kind)
    }

    /// All capabilities in the tree, sorted (deterministic).
    pub fn capabilities(&self) -> Vec<CapabilityKind> {
        self.nodes.keys().copied().collect()
    }

    /// Whether a capability has a node in the tree.
    pub fn has(&self, kind: CapabilityKind) -> bool {
        self.nodes.contains_key(&kind)
    }

    /// Direct prerequisites of a capability, sorted.
    ///
    /// Empty for an unknown capability or a leaf (no prerequisites).
    pub fn direct_prerequisites(&self, kind: CapabilityKind) -> Vec<CapabilityKind> {
        let mut prereqs = self
            .nodes
            .get(&kind)
            .map(|n| n.prerequisites.clone())
            .unwrap_or_default();
        prereqs.sort_unstable();
        prereqs
    }

    /// Whether `target` can be unlocked now: all its direct prerequisites are
    /// in `have`. Unknown targets have no node and therefore cannot unlock.
    pub fn can_unlock(&self, target: CapabilityKind, have: &[CapabilityKind]) -> bool {
        match self.nodes.get(&target) {
            None => false,
            Some(node) => node
                .prerequisites
                .iter()
                .all(|p| have.contains(p)),
        }
    }

    /// Whether every prerequisite chain beneath `target` leads only to
    /// capabilities that are already held or are known nodes in the tree.
    ///
    /// Unknown capabilities are **not** assumed: if any transitive
    /// prerequisite is not present in the tree, the path is unreachable even
    /// if a consumer happens to hold the name — we only reason about
    /// capabilities the tree actually knows. Cycle-safe via a visited set (a
    /// self-referential node is treated as unreachable rather than looping).
    pub fn reachable(&self, target: CapabilityKind, have: &[CapabilityKind]) -> bool {
        let mut visited = BTreeSet::new();
        self.reachable_from(target, have, &mut visited)
    }

    fn reachable_from(
        &self,
        kind: CapabilityKind,
        have: &[CapabilityKind],
        visited: &mut BTreeSet<CapabilityKind>,
    ) -> bool {
        if have.contains(&kind) {
            return true;
        }
        // Cycle guard: revisiting a capability on this path means the graph
        // loops, which can never actually unlock anything — treat as
        // unreachable instead of recursing forever.
        if !visited.insert(kind) {
            return false;
        }
        let Some(node) = self.nodes.get(&kind) else {
            // Unknown prerequisite: not assumed, therefore not reachable.
            return false;
        };
        node.prerequisites
            .iter()
            .all(|p| self.reachable_from(*p, have, visited))
    }

    /// A deterministic dependency-ordered list of the NEW capabilities to
    /// acquire to reach `target`, starting from `have`.
    ///
    /// Returns an empty vec when `target` is already in `have`, or when it is
    /// not [`reachable`](Self::reachable) from `have` (some prerequisite chain
    /// leads to an unknown node). The order is topological: every capability
    /// appears only after all of its own prerequisites. The search is a
    /// breadth-first expansion of the "all prerequisites held" frontier, and
    /// because the frontier is iterated in `CapabilityKind` order the result is
    /// deterministic.
    pub fn resolve_path(&self, target: CapabilityKind, have: &[CapabilityKind]) -> Vec<CapabilityKind> {
        if have.contains(&target) {
            return Vec::new();
        }
        if !self.reachable(target, have) {
            return Vec::new();
        }

        let mut acquired: Vec<CapabilityKind> = have.to_vec();
        let mut path: Vec<CapabilityKind> = Vec::new();

        loop {
            // Capabilities whose prerequisites are all held, not yet acquired
            // and not already on the path. BTreeMap iteration is sorted, so
            // the frontier is deterministic.
            let frontier: Vec<CapabilityKind> = self
                .nodes
                .keys()
                .filter(|k| !acquired.contains(k) && !path.contains(k))
                .filter(|k| {
                    self.nodes[k]
                        .prerequisites
                        .iter()
                        .all(|p| acquired.contains(p))
                })
                .copied()
                .collect();

            if frontier.is_empty() {
                break;
            }

            let mut progressed = false;
            for kind in frontier {
                if kind == target {
                    path.push(kind);
                    return path;
                }
                path.push(kind);
                acquired.push(kind);
                progressed = true;
            }
            if !progressed {
                break;
            }
        }

        // Unreachable in practice (guarded above), but keep the invariant: an
        // unfinished path is not a valid unlock, so report nothing.
        Vec::new()
    }

    /// Every capability whose prerequisites are all in `have` AND whose
    /// `resource_estimate_mb` fits `resource_budget_mb`, sorted.
    ///
    /// This is the planner's input: "what can I unlock with what I have and
    /// what I can spend?" Note this is a *one-hop* frontier (directly
    /// unlockable now), not a transitive closure — chaining is the planner's
    /// job via [`resolve_path`](Self::resolve_path).
    pub fn available_capabilities(
        &self,
        have: &[CapabilityKind],
        resource_budget_mb: u64,
    ) -> Vec<CapabilityKind> {
        self.nodes
            .values()
            .filter(|n| n.resource_estimate_mb <= resource_budget_mb)
            .filter(|n| n.prerequisites.iter().all(|p| have.contains(p)))
            .map(|n| n.capability)
            .collect()
    }
}

/// The default talent tree matching the architecture doc §8.
///
/// Three chains are seeded:
///
/// - **Knowledge**: `Embeddings → SemanticSearch → RAG → KnowledgeAgent`
/// - **Agentic tools**: `ToolCalling → MCP → MultiToolAgent → AutonomousAgent`
/// - **Coding**: `CodingModel → CodeReview → RepoAgent`
///
/// Because [`CapabilityKind`] is a fixed enum, each composite concept is
/// mapped to its **closest existing kind** (SemanticSearch→`Retrieval`,
/// RAG→`DocumentUnderstanding`, KnowledgeAgent→`Agents`, MCP→`FunctionCalling`,
/// MultiToolAgent→`Classification`, AutonomousAgent→`Reasoning`,
/// CodeReview→`Summarization`, RepoAgent→`Reranking`). The enum name is only a
/// label; the graph topology carries the real meaning. Distinct kinds are
/// mixed so the tree exercises a meaningful ordering, base capabilities are
/// explicit leaf nodes with no prerequisites, and the deeper/composite nodes
/// are marked `experimental` with lower `confidence` (honesty: they are
/// reachable in principle but not production-verified).
///
/// The graph is acyclic by construction: every node's prerequisites are either
/// leaf nodes or earlier nodes in the same chain, and no chain references
/// itself or a later node.
pub fn seed_talent_tree() -> TalentTree {
    let mut tree = TalentTree::new();

    // Base capabilities: explicit leaves with no prerequisites.
    let leaves: &[(CapabilityKind, u64)] = &[
        (CapabilityKind::Embeddings, 512),
        (CapabilityKind::ToolCalling, 1024),
        (CapabilityKind::Coding, 2048),
    ];
    for (kind, mb) in leaves {
        tree.add(TalentNode::new(*kind, Vec::new(), *mb, Some(Provenance::Verified), 1.0, false))
            .expect("leaf capabilities are unique keys");
    }

    // Chain 1: knowledge (Embeddings → SemanticSearch → RAG → KnowledgeAgent).
    tree.add(TalentNode::new(
        CapabilityKind::Retrieval, // SemanticSearch
        vec![CapabilityKind::Embeddings],
        1024,
        Some(Provenance::Verified),
        0.9,
        false,
    ))
    .expect("semantic search node");
    tree.add(TalentNode::new(
        CapabilityKind::DocumentUnderstanding, // RAG
        vec![CapabilityKind::Retrieval],
        4096,
        None,
        0.7,
        true,
    ))
    .expect("RAG node");
    tree.add(TalentNode::new(
        CapabilityKind::Agents, // KnowledgeAgent
        vec![CapabilityKind::DocumentUnderstanding],
        8192,
        None,
        0.5,
        true,
    ))
    .expect("knowledge agent node");

    // Chain 2: agentic tools (ToolCalling → MCP → MultiToolAgent → AutonomousAgent).
    tree.add(TalentNode::new(
        CapabilityKind::FunctionCalling, // MCP
        vec![CapabilityKind::ToolCalling],
        2048,
        Some(Provenance::Verified),
        0.85,
        false,
    ))
    .expect("MCP node");
    tree.add(TalentNode::new(
        CapabilityKind::Classification, // MultiToolAgent
        vec![CapabilityKind::FunctionCalling],
        4096,
        None,
        0.6,
        true,
    ))
    .expect("multi-tool agent node");
    tree.add(TalentNode::new(
        CapabilityKind::Reasoning, // AutonomousAgent
        vec![CapabilityKind::Classification],
        8192,
        None,
        0.4,
        true,
    ))
    .expect("autonomous agent node");

    // Chain 3: coding (Coding → CodeReview → RepoAgent).
    tree.add(TalentNode::new(
        CapabilityKind::Summarization, // CodeReview
        vec![CapabilityKind::Coding],
        2048,
        Some(Provenance::Verified),
        0.8,
        false,
    ))
    .expect("code review node");
    tree.add(TalentNode::new(
        CapabilityKind::Reranking, // RepoAgent
        vec![CapabilityKind::Summarization],
        4096,
        None,
        0.5,
        true,
    ))
    .expect("repo agent node");

    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(capability: CapabilityKind) -> TalentNode {
        TalentNode::new(capability, Vec::new(), 256, None, 1.0, false)
    }

    fn chain_tree() -> TalentTree {
        let mut tree = TalentTree::new();
        tree.add(node(CapabilityKind::Embeddings)).unwrap();
        tree.add(TalentNode::new(
            CapabilityKind::Retrieval,
            vec![CapabilityKind::Embeddings],
            512,
            None,
            0.9,
            false,
        ))
        .unwrap();
        tree.add(TalentNode::new(
            CapabilityKind::Agents,
            vec![CapabilityKind::Retrieval],
            1024,
            None,
            0.5,
            true,
        ))
        .unwrap();
        tree
    }

    #[test]
    fn add_and_get_round_trip_with_sorted_capabilities() {
        let mut tree = TalentTree::new();
        tree.add(node(CapabilityKind::Embeddings)).unwrap();
        tree.add(node(CapabilityKind::Coding)).unwrap();
        tree.add(node(CapabilityKind::Agents)).unwrap();

        assert!(tree.has(CapabilityKind::Embeddings));
        assert!(tree.has(CapabilityKind::Agents));
        assert!(!tree.has(CapabilityKind::Vision));
        assert!(tree.get(CapabilityKind::Coding).is_some());
        assert_eq!(tree.get(CapabilityKind::Vision), None);

        // Declaration order (derive Ord): Coding < Agents < Embeddings.
        assert_eq!(
            tree.capabilities(),
            vec![
                CapabilityKind::Coding,
                CapabilityKind::Agents,
                CapabilityKind::Embeddings,
            ]
        );
    }

    #[test]
    fn duplicate_add_is_rejected() {
        let mut tree = TalentTree::new();
        tree.add(node(CapabilityKind::Embeddings)).unwrap();
        let err = tree.add(node(CapabilityKind::Embeddings)).unwrap_err();
        assert_eq!(
            err,
            TalentError::DuplicateNode {
                capability: CapabilityKind::Embeddings
            }
        );
        // Original entry is untouched.
        assert_eq!(tree.capabilities(), vec![CapabilityKind::Embeddings]);
    }

    #[test]
    fn can_unlock_requires_all_prerequisites() {
        let tree = chain_tree();
        // Agents needs Retrieval, which needs Embeddings.
        assert!(tree.can_unlock(CapabilityKind::Retrieval, &[CapabilityKind::Embeddings]));
        assert!(!tree.can_unlock(
            CapabilityKind::Retrieval,
            &[CapabilityKind::Coding]
        ));
        assert!(!tree.can_unlock(CapabilityKind::Agents, &[CapabilityKind::Embeddings]));
        assert!(tree.can_unlock(
            CapabilityKind::Agents,
            &[CapabilityKind::Embeddings, CapabilityKind::Retrieval]
        ));
        // Leaf unlocks trivially; unknown never unlocks.
        assert!(tree.can_unlock(CapabilityKind::Embeddings, &[]));
        assert!(!tree.can_unlock(CapabilityKind::Vision, &[CapabilityKind::Embeddings]));
    }

    #[test]
    fn direct_prerequisites_returns_sorted_or_empty() {
        let mut tree = TalentTree::new();
        tree.add(TalentNode::new(
            CapabilityKind::Agents,
            vec![CapabilityKind::Reasoning, CapabilityKind::Embeddings],
            512,
            None,
            1.0,
            false,
        ))
        .unwrap();
        tree.add(node(CapabilityKind::Coding)).unwrap();

        let got = tree.direct_prerequisites(CapabilityKind::Agents);
        // Sorted by declaration order (derive Ord): Reasoning < Embeddings.
        assert_eq!(got, vec![CapabilityKind::Reasoning, CapabilityKind::Embeddings]);
        // Leaf and unknown both yield empty.
        assert!(tree.direct_prerequisites(CapabilityKind::Coding).is_empty());
        assert!(tree.direct_prerequisites(CapabilityKind::Vision).is_empty());
    }

    #[test]
    fn resolve_path_returns_dependency_ordered_new_capabilities() {
        let tree = chain_tree();
        let path = tree.resolve_path(CapabilityKind::Agents, &[CapabilityKind::Embeddings]);
        // Dependencies first, target last.
        assert_eq!(
            path,
            vec![CapabilityKind::Retrieval, CapabilityKind::Agents]
        );
        // No duplicate steps and nothing already held is re-returned.
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn resolve_path_is_empty_when_target_already_held() {
        let tree = chain_tree();
        assert!(tree
            .resolve_path(CapabilityKind::Embeddings, &[CapabilityKind::Embeddings])
            .is_empty());
        assert!(tree
            .resolve_path(CapabilityKind::Agents, &[
                CapabilityKind::Embeddings,
                CapabilityKind::Retrieval,
                CapabilityKind::Agents,
            ])
            .is_empty());
    }

    #[test]
    fn resolve_path_is_empty_when_a_prerequisite_is_unknown() {
        // Agents references an unknown prerequisite, so the whole path is
        // unreachable: unknown capabilities are never assumed.
        let mut tree = TalentTree::new();
        tree.add(node(CapabilityKind::Embeddings)).unwrap();
        tree.add(TalentNode::new(
            CapabilityKind::Agents,
            vec![CapabilityKind::Retrieval], // unknown prereq
            512,
            None,
            1.0,
            false,
        ))
        .unwrap();
        assert!(tree
            .resolve_path(CapabilityKind::Agents, &[CapabilityKind::Embeddings])
            .is_empty());
        assert!(!tree.reachable(CapabilityKind::Agents, &[CapabilityKind::Embeddings]));
    }

    #[test]
    fn available_capabilities_filters_by_prerequisites_and_budget_sorted() {
        let mut tree = TalentTree::new();
        tree.add(node(CapabilityKind::Embeddings)).unwrap();
        tree.add(node(CapabilityKind::Coding)).unwrap();
        tree.add(TalentNode::new(
            CapabilityKind::Retrieval,
            vec![CapabilityKind::Embeddings],
            512,
            None,
            0.9,
            false,
        ))
        .unwrap();
        tree.add(TalentNode::new(
            CapabilityKind::Agents,
            vec![CapabilityKind::Retrieval],
            10_000, // too expensive for a small budget
            None,
            1.0,
            false,
        ))
        .unwrap();

        // With only Embeddings held, Retrieval (holds prereq) is available but
        // Agents is not (prereq not held) even though it's a leaf-cost node.
        // Coding is a leaf with no prerequisites, so it qualifies too.
        let available = tree.available_capabilities(&[CapabilityKind::Embeddings], 4096);
        assert_eq!(
            available,
            vec![
                CapabilityKind::Coding,
                CapabilityKind::Embeddings,
                CapabilityKind::Retrieval,
            ]
        );

        // Budget gates cost: Agents is out of reach even when prerequisites held.
        let pruned = tree.available_capabilities(
            &[CapabilityKind::Embeddings, CapabilityKind::Retrieval],
            4096,
        );
        assert_eq!(
            pruned,
            vec![
                CapabilityKind::Coding,
                CapabilityKind::Embeddings,
                CapabilityKind::Retrieval,
            ]
        );

        let funded = tree.available_capabilities(
            &[CapabilityKind::Embeddings, CapabilityKind::Retrieval],
            20_000,
        );
        assert_eq!(
            funded,
            vec![
                CapabilityKind::Coding,
                CapabilityKind::Agents,
                CapabilityKind::Embeddings,
                CapabilityKind::Retrieval,
            ]
        );
    }

    #[test]
    fn seed_tree_has_leaves_composites_and_is_acyclic() {
        let tree = seed_talent_tree();

        // Embeddings is a leaf.
        assert!(tree
            .direct_prerequisites(CapabilityKind::Embeddings)
            .is_empty());

        // Composite nodes are present.
        for kind in [
            CapabilityKind::Retrieval,
            CapabilityKind::DocumentUnderstanding,
            CapabilityKind::Agents,
            CapabilityKind::FunctionCalling,
            CapabilityKind::Classification,
            CapabilityKind::Reasoning,
            CapabilityKind::Summarization,
            CapabilityKind::Reranking,
        ] {
            assert!(tree.has(kind), "seed missing {kind:?}");
        }

        // Every node's prerequisites exist in the tree (or are base leaves).
        for kind in tree.capabilities() {
            let node = tree.get(kind).unwrap();
            for prereq in node.direct_prerequisites() {
                assert!(tree.has(*prereq), "{kind:?} references unknown {prereq:?}");
            }
        }

        // resolve_path never loops: with full base set, every node is reached
        // exactly once and its path is finite and acyclic.
        let base = vec![
            CapabilityKind::Embeddings,
            CapabilityKind::ToolCalling,
            CapabilityKind::Coding,
        ];
        for kind in tree.capabilities() {
            let path = tree.resolve_path(kind, &base);
            if path.is_empty() {
                assert!(base.contains(&kind), "{kind:?} should be reachable");
            } else {
                assert_eq!(path.last(), Some(&kind), "path must end at the target");
                let seen: BTreeSet<_> = path.iter().copied().collect();
                assert_eq!(seen.len(), path.len(), "{kind:?} path loops");
            }
        }
    }

    #[test]
    fn deeper_seed_nodes_are_experimental() {
        let tree = seed_talent_tree();
        // Deeper composite nodes carry the honesty flag and lower confidence.
        for kind in [
            CapabilityKind::DocumentUnderstanding,
            CapabilityKind::Agents,
            CapabilityKind::Classification,
            CapabilityKind::Reasoning,
            CapabilityKind::Reranking,
        ] {
            let node = tree.get(kind).expect("composite present");
            assert!(node.experimental, "{kind:?} should be experimental");
            assert!(node.confidence <= 0.7, "{kind:?} confidence too high");
        }
        // Base / shallow nodes are verified and confident.
        for kind in [
            CapabilityKind::Embeddings,
            CapabilityKind::ToolCalling,
            CapabilityKind::Coding,
            CapabilityKind::Retrieval,
            CapabilityKind::FunctionCalling,
            CapabilityKind::Summarization,
        ] {
            let node = tree.get(kind).expect("base present");
            assert!(!node.experimental, "{kind:?} should not be experimental");
        }
    }

    #[test]
    fn node_constructor_clamps_confidence_to_unit_range() {
        let high = node(CapabilityKind::Embeddings);
        let over = TalentNode::new(
            CapabilityKind::Coding,
            Vec::new(),
            256,
            None,
            7.5,
            false,
        );
        assert_eq!(over.confidence, 1.0);
        let under = TalentNode::new(
            CapabilityKind::Agents,
            Vec::new(),
            256,
            None,
            -2.0,
            false,
        );
        assert_eq!(under.confidence, 0.0);
        assert_eq!(high.confidence, 1.0);
        let mid = TalentNode::new(
            CapabilityKind::Vision,
            Vec::new(),
            256,
            None,
            0.37,
            false,
        );
        assert_eq!(mid.confidence, 0.37);
    }

    #[test]
    fn talent_tree_round_trips_over_json() {
        let tree = seed_talent_tree();
        let json = serde_json::to_string(&tree).unwrap();
        let back: TalentTree = serde_json::from_str(&json).unwrap();
        assert_eq!(tree, back);

        // CapabilityKind serializes in snake_case inside the map.
        assert_eq!(
            serde_json::to_string(&CapabilityKind::DocumentUnderstanding).unwrap(),
            "\"document_understanding\""
        );
    }
}
