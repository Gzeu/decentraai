//! Bridge between personal memory (Markdown per-agent) and collective memory
//! (SQLite scopes). When an agent writes a lesson, decision, or experience to
//! its personal memory, the bridge optionally mirrors it into a collective scope
//! so other agents can learn from it.
//!
//! # Design
//!
//! - **Write path**: `agent_memory_write` → `MemoryBridge::mirror_write` →
//!   `MemoryStore::write_checked` (policy-gated, dedup by content hash).
//! - **Read path**: `agent_memory_read` → `MemoryBridge::merge_read` →
//!   personal entries + collective entries from the bridged scope.
//!
//! The bridge is **opt-in per category**: each category maps to zero or one
//! collective scope. Categories without a mapping are unaffected.

use std::collections::HashMap;

use tracing::{debug, warn};

use decentraai_agents::memory::{KnowledgeKind, MemoryEntry, MemoryLevel, MemoryPolicy, MemoryScope};
use decentraai_agents::memory::MemoryAccess;
use decentraai_distributed::agent_memory::MemoryStore;

/// Configuration for one personal→collective bridge.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Collective scope name to mirror into.
    pub collective_scope: String,
    /// Knowledge kind for the mirrored entry.
    pub kind: KnowledgeKind,
    /// Tags to attach to the mirrored entry.
    pub tags: Vec<String>,
}

/// Maps personal memory categories to collective scope bridge configs.
#[derive(Debug, Clone, Default)]
pub struct BridgeMapping(HashMap<String, BridgeConfig>);

impl BridgeMapping {
    /// The default mapping: lessons, decisions, and experiences are bridged.
    pub fn default_mapping() -> Self {
        let mut m = HashMap::new();
        m.insert(
            "lessons".to_string(),
            BridgeConfig {
                collective_scope: "agent.lessons".to_string(),
                kind: KnowledgeKind::Learning,
                tags: vec!["bridge:personal".to_string(), "kind:lesson".to_string()],
            },
        );
        m.insert(
            "decisions".to_string(),
            BridgeConfig {
                collective_scope: "agent.decisions".to_string(),
                kind: KnowledgeKind::Decision,
                tags: vec!["bridge:personal".to_string(), "kind:decision".to_string()],
            },
        );
        m.insert(
            "experiences".to_string(),
            BridgeConfig {
                collective_scope: "agent.experiences".to_string(),
                kind: KnowledgeKind::Observation,
                tags: vec!["bridge:personal".to_string(), "kind:experience".to_string()],
            },
        );
        Self(m)
    }

    pub fn get(&self, category: &str) -> Option<&BridgeConfig> {
        self.0.get(category)
    }

    /// Register or override a bridge for a category.
    pub fn register(&mut self, category: String, config: BridgeConfig) {
        self.0.insert(category, config);
    }
}

/// Ensures the collective scopes required by the bridge mapping exist.
/// Called once at startup (or when the mapping changes).
///
/// When `bridge_sync` is true, scopes are created as propagation-eligible
/// (level=Network, allow_remote_write=true) so the memory propagator picks
/// them up and syncs them to connected peers. When false, scopes stay local
/// (level=Node, allow_remote_write=false).
pub fn ensure_bridged_scopes(
    memory: &MemoryStore,
    mapping: &BridgeMapping,
    owner_agent: &str,
    bridge_sync: bool,
) {
    for (category, config) in &mapping.0 {
        let scope_name = &config.collective_scope;
        // register_scope is idempotent (INSERT OR IGNORE).
        //
        // allow_remote_write is always true: the bridge writes as the agent
        // (not scope owner), so it needs this even for local-only mode.
        // Propagation eligibility is controlled by level: Node = local-only,
        // Network = propagator picks it up.
        let level = if bridge_sync {
            MemoryLevel::Network
        } else {
            MemoryLevel::Node
        };
        let policy = MemoryPolicy {
            level,
            access: MemoryAccess::Public,
            allow_remote_write: true,
            ..Default::default()
        };
        let scope = MemoryScope::new(scope_name, owner_agent, level)
            .with_policy(policy);
        if let Err(e) = memory.register_scope(&scope) {
            warn!(
                category,
                scope = scope_name,
                error = %e,
                "failed to register bridged collective scope"
            );
        } else {
            debug!(
                category,
                scope = scope_name,
                bridge_sync,
                "bridged collective scope ready"
            );
        }
    }
}

/// Mirrors a personal memory write into the corresponding collective scope.
///
/// Called after a successful `agent_memory_write`. If the category has no
/// bridge mapping, this is a no-op.
///
/// The entry content is prefixed with the agent identity so collective readers
/// know the source. Dedup is handled by `MemoryStore::write_checked` (content
/// hash).
///
/// When `bridge_sync` is true, the mirrored entry is created with status
/// `Verified` so it qualifies as travel-worthy for the memory propagator.
/// When false, it stays `Candidate` (local-only).
pub fn mirror_write(
    memory: &MemoryStore,
    mapping: &BridgeMapping,
    agent_id: &str,
    category: &str,
    content: &str,
    now_ms: u64,
    bridge_sync: bool,
) {
    let Some(config) = mapping.get(category) else {
        return;
    };

    let entry_id = format!("bridge:{agent_id}:{category}:{now_ms}");
    let mut entry = MemoryEntry::new(
        &entry_id,
        &config.collective_scope,
        agent_id,
        "local",
        content,
    );
    entry.created_at_ms = now_ms;
    entry.meta.kind = config.kind;
    entry.meta.subject_key = format!("agent:{agent_id}");
    entry.tags = config.tags.clone();

    // When bridge_sync is enabled, promote the entry to Verified so the
    // propagator considers it travel-worthy (candidate entries stay local).
    if bridge_sync {
        entry.meta.status = decentraai_agents::memory::MemoryStatus::Verified;
    }

    // Writer is the agent itself (owner of the personal memory), but NOT the
    // scope owner (the scope is node-level). trusted=false, provenance=false
    // because personal memory has no verified provenance chain.
    match memory.write_checked(
        &config.collective_scope,
        &entry,
        agent_id,
        false, // writer_is_owner (scope owner is the node)
        false, // trusted
        false, // verified_provenance
    ) {
        Ok(outcome) => {
            debug!(
                agent_id,
                category,
                scope = config.collective_scope,
                entry_id,
                ?outcome,
                "mirrored personal memory to collective scope"
            );
        }
        Err(e) => {
            warn!(
                agent_id,
                category,
                scope = config.collective_scope,
                error = %e,
                "failed to mirror personal memory to collective scope"
            );
        }
    }
}

/// Reads collective entries from the bridged scope and returns them as
/// personal-memory-compatible summaries.
///
/// Called during `agent_memory_read` to augment the personal memory response
/// with cross-agent knowledge from the collective layer.
pub fn read_bridged(
    memory: &MemoryStore,
    mapping: &BridgeMapping,
    reader_agent: &str,
    category: &str,
    limit: usize,
) -> Vec<BridgedEntry> {
    let Some(config) = mapping.get(category) else {
        return Vec::new();
    };

    let entries = match memory.read(&config.collective_scope, reader_agent, true) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                scope = config.collective_scope,
                error = %e,
                "failed to read bridged collective scope"
            );
            return Vec::new();
        }
    };

    entries
        .into_iter()
        .filter(|e| e.author_agent != reader_agent) // exclude own entries (already in personal)
        .take(limit)
        .map(|e| BridgedEntry {
            author_agent: e.author_agent,
            content: e.content,
            kind: format!("{:?}", e.meta.kind),
            created_at_ms: e.created_at_ms,
            tags: e.tags,
        })
        .collect()
}

/// A collective entry surfaced through the bridge, formatted for display
/// alongside personal memory entries.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BridgedEntry {
    pub author_agent: String,
    pub content: String,
    pub kind: String,
    pub created_at_ms: u64,
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::memory::MemoryStatus;

    use std::path::Path;

    fn in_memory_store() -> MemoryStore {
        MemoryStore::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn default_mapping_covers_three_categories() {
        let m = BridgeMapping::default_mapping();
        assert!(m.get("lessons").is_some());
        assert!(m.get("decisions").is_some());
        assert!(m.get("experiences").is_some());
        assert!(m.get("identity").is_none(), "identity is not bridged");
        assert!(m.get("goals").is_none(), "goals are not bridged");
    }

    #[test]
    fn bridge_config_fields_are_correct() {
        let m = BridgeMapping::default_mapping();
        let lessons = m.get("lessons").unwrap();
        assert_eq!(lessons.collective_scope, "agent.lessons");
        assert!(matches!(lessons.kind, KnowledgeKind::Learning));
        assert!(lessons.tags.contains(&"bridge:personal".to_string()));
    }

    #[test]
    fn register_override_works() {
        let mut m = BridgeMapping::default_mapping();
        m.register(
            "custom".to_string(),
            BridgeConfig {
                collective_scope: "custom.scope".to_string(),
                kind: KnowledgeKind::Execution,
                tags: vec!["custom".to_string()],
            },
        );
        assert!(m.get("custom").is_some());
        assert_eq!(
            m.get("custom").unwrap().collective_scope,
            "custom.scope"
        );
    }

    #[test]
    fn ensure_bridged_scopes_local_only_by_default() {
        let store = in_memory_store();
        let mapping = BridgeMapping::default_mapping();
        ensure_bridged_scopes(&store, &mapping, "governor", false);
        let scopes = store.list_scopes().unwrap();
        for s in &scopes {
            if s.name == "agent.lessons" || s.name == "agent.decisions" || s.name == "agent.experiences" {
                assert_eq!(s.level, MemoryLevel::Node, "scope {} should be Node level when bridge_sync=false", s.name);
                assert!(s.policy.allow_remote_write, "scope {} needs allow_remote_write for bridge writes", s.name);
            }
        }
    }

    #[test]
    fn ensure_bridged_scopes_propagation_eligible_when_sync_enabled() {
        let store = in_memory_store();
        let mapping = BridgeMapping::default_mapping();
        ensure_bridged_scopes(&store, &mapping, "governor", true);
        let scopes = store.list_scopes().unwrap();
        for s in &scopes {
            if s.name == "agent.lessons" || s.name == "agent.decisions" || s.name == "agent.experiences" {
                assert_eq!(s.level, MemoryLevel::Network, "scope {} should be Network level when bridge_sync=true", s.name);
                assert!(s.policy.allow_remote_write, "scope {} should allow remote write when bridge_sync=true", s.name);
                assert!(matches!(s.policy.access, MemoryAccess::Public), "scope {} should be Public access", s.name);
            }
        }
    }

    #[test]
    fn mirror_write_creates_verified_entry_when_sync_enabled() {
        let store = in_memory_store();
        let mapping = BridgeMapping::default_mapping();
        ensure_bridged_scopes(&store, &mapping, "governor", true);
        mirror_write(&store, &mapping, "agent-1", "lessons", "test lesson content", 1000, true);
        let entries = store.read("agent.lessons", "reader", true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].meta.status, MemoryStatus::Verified, "entry should be Verified when bridge_sync=true");
        assert!(entries[0].content.contains("test lesson content"));
        assert_eq!(entries[0].author_agent, "agent-1");
    }

    #[test]
    fn mirror_write_creates_candidate_entry_when_sync_disabled() {
        let store = in_memory_store();
        let mapping = BridgeMapping::default_mapping();
        ensure_bridged_scopes(&store, &mapping, "governor", false);
        mirror_write(&store, &mapping, "agent-1", "lessons", "test lesson content", 2000, false);
        let entries = store.read("agent.lessons", "reader", true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].meta.status, MemoryStatus::Candidate, "entry should be Candidate when bridge_sync=false");
    }
}
