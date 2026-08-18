//! Collective memory — multi-level scopes with explicit ownership and policy.
//!
//! # Why this design
//!
//! NOT all memory is shared. A "global brain" is both a privacy risk (private
//! scopes must never leak into audit logs) and a trust risk (unverified peers
//! must not pollute shared knowledge). Following `docs/COLLECTIVE_INTELLIGENCE.md`
//! §4.4, memory is a set of **scopes**, each owned by an agent, at one of four
//! levels (agent / team / network / fabric) and governed by an explicit
//! [`MemoryPolicy`]: who may read, who may write, how long entries live,
//! whether provenance is mandatory, and whether remote peers may contribute at
//! all (opt-in only).
//!
//! This module is the pure, decision-only model the runtime will enforce. It
//! holds no I/O: scopes, policies and entries are plain serde types that travel
//! over the P2P channel, and every gate is a pure function ([`can_read`],
//! [`can_write`], [`enforce_retention`]) that unit tests drive with synthetic
//! inputs. Persistence (SQLite) and replication (typed messages) live in the
//! runtime crate, later.
//!
//! Ownership and trust are facts the caller supplies: the runtime resolves
//! "is this writer the owner?" and "is this reader on the same team / trusted
//! node?" — this module only applies the declared policy to those facts.

use decentraai_hub::capability::Provenance;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// The reach of a memory scope: how far the scope extends by design.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLevel {
    /// Private to one agent: personal learnings, session context.
    #[default]
    Agent,
    /// Shared within an agent team.
    Team,
    /// Shared across trusted nodes.
    Network,
    /// Public across the fabric, opt-in.
    Fabric,
}

impl MemoryLevel {
    /// Breadth rank (Agent < Team < Network < Fabric), used by
    /// [`MemoryAccess::permits`]. Private by intent, never exposed on the wire.
    fn rank(self) -> u8 {
        match self {
            MemoryLevel::Agent => 0,
            MemoryLevel::Team => 1,
            MemoryLevel::Network => 2,
            MemoryLevel::Fabric => 3,
        }
    }
}

/// Read visibility of a scope, independent of its [`MemoryLevel`].
///
/// A scope's *level* says how far it is meant to reach; its *access* says who
/// may actually read it. The two are deliberately separate so a team scope can
/// stay private until its members decide to widen it — visibility is always an
/// explicit choice, never an automatic consequence of the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAccess {
    /// Readable only by the owning agent.
    #[default]
    Private,
    /// Readable by the owning agent and its team.
    TeamOnly,
    /// Readable by any trusted node.
    TrustedNetwork,
    /// Readable by everyone.
    Public,
}

impl MemoryAccess {
    /// Breadth rank (Private < TeamOnly < TrustedNetwork < Public).
    fn rank(self) -> u8 {
        match self {
            MemoryAccess::Private => 0,
            MemoryAccess::TeamOnly => 1,
            MemoryAccess::TrustedNetwork => 2,
            MemoryAccess::Public => 3,
        }
    }

    /// Whether a reader at the given level sits within this access's breadth.
    ///
    /// A `Private` scope is readable only by its owner agent (Agent level);
    /// `TeamOnly` extends to team members; `TrustedNetwork` to any trusted
    /// node; `Public` to everyone. A reader whose level is broader than the
    /// access breadth is denied.
    pub fn permits(&self, level: MemoryLevel) -> bool {
        level.rank() <= self.rank()
    }
}

/// The governance policy of one memory scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryPolicy {
    /// The scope's intended reach (agent / team / network / fabric).
    pub level: MemoryLevel,
    /// Read visibility.
    pub access: MemoryAccess,
    /// Retention window in seconds; `None` means keep forever.
    pub retention_secs: Option<u64>,
    /// If true, the scope only accepts entries whose provenance is
    /// [`Provenance::Verified`]. Enforced for every writer, owner included —
    /// a scope that demands provenance must not be poisoned by its own agent.
    pub require_verified_provenance: bool,
    /// If false (default), remote peers may not write; the scope is
    /// owner-only unless the owner explicitly opts in.
    pub allow_remote_write: bool,
    /// Bounded entry count per scope; the oldest entries are pruned first.
    pub max_entries: u32,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            level: MemoryLevel::Agent,
            access: MemoryAccess::Private,
            retention_secs: None,
            require_verified_provenance: false,
            allow_remote_write: false,
            max_entries: 1024,
        }
    }
}

impl MemoryPolicy {
    /// Adds a retention window: entries expire `secs` after creation.
    pub fn with_retention(self, secs: u64) -> Self {
        Self {
            retention_secs: Some(secs),
            ..self
        }
    }

    /// Requires verified provenance on every entry written to the scope.
    pub fn with_provenance_required(self) -> Self {
        Self {
            require_verified_provenance: true,
            ..self
        }
    }

    /// Opts in to remote writes: peers may contribute under `access`.
    pub fn with_remote_write(self) -> Self {
        Self {
            allow_remote_write: true,
            ..self
        }
    }

    /// A shared team scope: team level, team-only visibility.
    pub fn team(self) -> Self {
        Self {
            level: MemoryLevel::Team,
            access: MemoryAccess::TeamOnly,
            ..self
        }
    }

    /// An opt-in public scope: fabric level, public visibility.
    pub fn public(self) -> Self {
        Self {
            level: MemoryLevel::Fabric,
            access: MemoryAccess::Public,
            ..self
        }
    }
}

/// One memory entry. Wire-safe: everything a peer needs to store or forward it.
///
/// `content` is deliberately opaque plain text; the scope policy — not this
/// type — decides who may see it. `created_at_ms`/`expires_at_ms` use unix
/// milliseconds, matching the rest of the repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Stable unique id within the scope, e.g. `"k:2026-08-17:1"`.
    pub entry_id: String,
    /// Name of the scope this entry belongs to.
    pub scope: String,
    /// Agent that authored the entry.
    pub author_agent: String,
    /// Node (peer id) that hosted the authoring agent.
    pub author_node: String,
    /// The remembered content.
    pub content: String,
    /// Free-form tags for [`MemoryRegistry::search`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
    /// Optional absolute expiry (unix ms); `None` means no expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    /// How the entry's content was obtained; `None` when unknown/not claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

impl MemoryEntry {
    /// A minimal entry with the given identity and author facts.
    pub fn new(
        entry_id: impl Into<String>,
        scope: impl Into<String>,
        author_agent: impl Into<String>,
        author_node: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            entry_id: entry_id.into(),
            scope: scope.into(),
            author_agent: author_agent.into(),
            author_node: author_node.into(),
            content: content.into(),
            tags: Vec::new(),
            created_at_ms: 0,
            expires_at_ms: None,
            provenance: None,
        }
    }

    /// Adds one tag used by scope search.
    pub fn tagged(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
        self
    }

    /// Sets the creation time (unix ms).
    pub fn created_at(mut self, ms: u64) -> Self {
        self.created_at_ms = ms;
        self
    }

    /// Sets an absolute expiry (unix ms).
    pub fn expires_at(mut self, ms: u64) -> Self {
        self.expires_at_ms = Some(ms);
        self
    }

    /// Declares the provenance of the entry's content.
    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = Some(provenance);
        self
    }
}

/// A named, owned memory scope on a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryScope {
    /// Unique scope name within the node, e.g. `"team.notes"`.
    pub name: String,
    /// Agent that owns (and can always read/write) this scope.
    pub owner_agent: String,
    /// The scope's level.
    #[serde(default)]
    pub level: MemoryLevel,
    /// Governance policy.
    #[serde(default)]
    pub policy: MemoryPolicy,
    /// Creation time (unix ms).
    #[serde(default)]
    pub created_at_ms: u64,
}

impl MemoryScope {
    /// A scope with a default policy at the given level: the policy starts at
    /// the same reach as the scope, conservative and owner-controlled.
    pub fn new(
        name: impl Into<String>,
        owner_agent: impl Into<String>,
        level: MemoryLevel,
    ) -> Self {
        Self {
            name: name.into(),
            owner_agent: owner_agent.into(),
            level,
            policy: MemoryPolicy {
                level,
                ..MemoryPolicy::default()
            },
            created_at_ms: 0,
        }
    }

    /// Replaces the governance policy.
    pub fn with_policy(mut self, policy: MemoryPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Outcome of an access check. Serialized so the runtime can report verdicts
/// over the API without leaking the policy internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryAccessDecision {
    Granted,
    Denied { reason: String },
}

impl MemoryAccessDecision {
    /// Whether access was granted.
    pub fn is_granted(&self) -> bool {
        matches!(self, MemoryAccessDecision::Granted)
    }

    /// The denial reason, when denied.
    pub fn reason(&self) -> Option<&str> {
        match self {
            MemoryAccessDecision::Granted => None,
            MemoryAccessDecision::Denied { reason } => Some(reason),
        }
    }
}

/// Whether `reader_agent` may read `scope`.
///
/// The owner is always granted. Everyone else is gated by the scope's access:
/// `Private` is owner-only; `TeamOnly` and `TrustedNetwork` additionally admit
/// `trusted` readers; `Public` admits anyone. Team membership is a fact the
/// caller resolves (`trusted`), so this pure model stays honest: it never
/// guesses who belongs to a team.
pub fn can_read(
    scope: &MemoryScope,
    reader_agent: &str,
    reader_is_owner: bool,
    trusted: bool,
) -> MemoryAccessDecision {
    if reader_is_owner {
        return MemoryAccessDecision::Granted;
    }
    match scope.policy.access {
        MemoryAccess::Private => MemoryAccessDecision::Denied {
            reason: format!("scope '{}' is private to its owner", scope.name),
        },
        MemoryAccess::TeamOnly | MemoryAccess::TrustedNetwork if trusted => {
            MemoryAccessDecision::Granted
        }
        MemoryAccess::TeamOnly | MemoryAccess::TrustedNetwork => MemoryAccessDecision::Denied {
            reason: format!(
                "scope '{}' requires trust to read; '{}' is not trusted",
                scope.name, reader_agent
            ),
        },
        MemoryAccess::Public => MemoryAccessDecision::Granted,
    }
}

/// Whether `writer_agent` may write an entry to `scope`.
///
/// Order of gates: provenance (if the scope requires it, every writer — owner
/// included — must provide verified provenance), then ownership (owner always
/// writes), then the remote-write opt-in, then access breadth (a trusted
/// non-owner is at least a team member; an untrusted non-owner needs the scope
/// to be public).
pub fn can_write(
    scope: &MemoryScope,
    writer_agent: &str,
    writer_is_owner: bool,
    trusted: bool,
    verified_provenance: bool,
) -> MemoryAccessDecision {
    if scope.policy.require_verified_provenance && !verified_provenance {
        return MemoryAccessDecision::Denied {
            reason: format!(
                "scope '{}' requires verified provenance, writer '{}' did not provide it",
                scope.name, writer_agent
            ),
        };
    }
    if writer_is_owner {
        return MemoryAccessDecision::Granted;
    }
    if !scope.policy.allow_remote_write {
        return MemoryAccessDecision::Denied {
            reason: format!("scope '{}' does not opt in to remote writes", scope.name),
        };
    }
    // A non-owner is at least a team member when trusted, otherwise an
    // external actor; the scope's access must span that far.
    let writer_level = if trusted {
        MemoryLevel::Team
    } else {
        MemoryLevel::Fabric
    };
    if scope.policy.access.permits(writer_level) {
        MemoryAccessDecision::Granted
    } else {
        MemoryAccessDecision::Denied {
            reason: format!(
                "scope '{}' access does not permit writes by '{}'",
                scope.name, writer_agent
            ),
        }
    }
}

/// Whether an entry is expired at `now_ms` (an explicit expiry in the past).
///
/// Entries without an expiry never expire; an entry expiring exactly at
/// `now_ms` is still valid until the next millisecond.
pub fn entry_expired(entry: &MemoryEntry, now_ms: u64) -> bool {
    entry.expires_at_ms.is_some_and(|exp| exp < now_ms)
}

/// Applies a scope's retention deterministically: drops expired entries, then
/// keeps only the newest `max_entries` (entries arrive in insertion order, so
/// "newest" means the tail of the vector).
pub fn enforce_retention(
    mut entries: Vec<MemoryEntry>,
    policy: &MemoryPolicy,
    now_ms: u64,
) -> Vec<MemoryEntry> {
    entries.retain(|e| !entry_expired(e, now_ms));
    let overflow = entries.len().saturating_sub(policy.max_entries as usize);
    if overflow > 0 {
        entries.drain(..overflow);
    }
    entries
}

/// Memory policy/registry errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryError {
    /// The named scope does not exist.
    #[error("memory scope '{name}' does not exist")]
    UnknownScope { name: String },
    /// The actor is not allowed to read or write the scope.
    #[error("access denied: {reason}")]
    AccessDenied { reason: String },
    /// A scope with the same name is already registered.
    #[error("memory scope '{name}' is already registered")]
    DuplicateScope { name: String },
    /// The entry was already expired at write time.
    #[error("memory entry '{entry_id}' is already expired")]
    EntryExpired { entry_id: String },
}

/// A deterministic, per-node registry of memory scopes and their entries.
///
/// Scope metadata lives in `scopes`; entries are stored per scope name in
/// insertion order (oldest first), so retention can prune from the front and
/// "newest" is well-defined. This is the in-memory half; persistence is a
/// runtime concern.
#[derive(Debug, Clone, Default)]
pub struct MemoryRegistry {
    scopes: HashMap<String, MemoryScope>,
    entries: HashMap<String, Vec<MemoryEntry>>,
}

impl MemoryRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a scope. Fails on duplicate names so callers notice
    /// collisions instead of silently overwriting an existing scope.
    pub fn register_scope(&mut self, scope: MemoryScope) -> Result<(), MemoryError> {
        if self.scopes.contains_key(&scope.name) {
            return Err(MemoryError::DuplicateScope { name: scope.name });
        }
        self.entries.entry(scope.name.clone()).or_default();
        self.scopes.insert(scope.name.clone(), scope);
        Ok(())
    }

    /// Removes a scope and all its entries; returns whether it existed.
    pub fn unregister_scope(&mut self, name: &str) -> bool {
        let removed = self.scopes.remove(name).is_some();
        if removed {
            self.entries.remove(name);
        }
        removed
    }

    /// Looks up a scope by name.
    pub fn get_scope(&self, name: &str) -> Option<&MemoryScope> {
        self.scopes.get(name)
    }

    /// All registered scopes, sorted by name (deterministic).
    pub fn scopes(&self) -> Vec<MemoryScope> {
        let mut scopes: Vec<MemoryScope> = self.scopes.values().cloned().collect();
        scopes.sort_by(|a, b| a.name.cmp(&b.name));
        scopes
    }

    /// Writes an entry to a scope, enforcing its policy.
    ///
    /// Trust facts (`writer_is_owner`, `trusted`, `verified_provenance`) and
    /// the current time come from the caller — the runtime resolves them, this
    /// module only applies the declared policy. Existing expired entries are
    /// pruned and the `max_entries` cap is enforced before the new entry lands.
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &mut self,
        scope_name: &str,
        entry: MemoryEntry,
        writer_agent: &str,
        writer_is_owner: bool,
        trusted: bool,
        verified_provenance: bool,
        now_ms: u64,
    ) -> Result<(), MemoryError> {
        let scope =
            self.scopes
                .get(scope_name)
                .cloned()
                .ok_or_else(|| MemoryError::UnknownScope {
                    name: scope_name.to_string(),
                })?;
        match can_write(
            &scope,
            writer_agent,
            writer_is_owner,
            trusted,
            verified_provenance,
        ) {
            MemoryAccessDecision::Granted => {}
            MemoryAccessDecision::Denied { reason } => {
                return Err(MemoryError::AccessDenied { reason });
            }
        }
        if entry_expired(&entry, now_ms) {
            return Err(MemoryError::EntryExpired {
                entry_id: entry.entry_id,
            });
        }
        let bucket = self.entries.entry(scope_name.to_string()).or_default();
        bucket.push(entry);
        // Enforce expiry + cap including the just-written entry so the scope
        // never holds more than `max_entries` live entries.
        *bucket = enforce_retention(std::mem::take(bucket), &scope.policy, now_ms);
        Ok(())
    }

    /// Reads a scope's non-expired entries in insertion order.
    ///
    /// Ownership is derived honestly from the scope itself (`reader_agent ==
    /// owner_agent`); `trusted` is the caller-supplied trust fact for everyone
    /// else.
    pub fn read(
        &self,
        scope_name: &str,
        reader_agent: &str,
        trusted: bool,
        now_ms: u64,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let scope = self
            .scopes
            .get(scope_name)
            .ok_or_else(|| MemoryError::UnknownScope {
                name: scope_name.to_string(),
            })?;
        let reader_is_owner = scope.owner_agent == reader_agent;
        match can_read(scope, reader_agent, reader_is_owner, trusted) {
            MemoryAccessDecision::Granted => {}
            MemoryAccessDecision::Denied { reason } => {
                return Err(MemoryError::AccessDenied { reason });
            }
        }
        Ok(self
            .entries
            .get(scope_name)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| !entry_expired(e, now_ms))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Searches a scope's non-expired entries by tag, in insertion order.
    ///
    /// Deliberately access-free: this is an internal/trusted query the runtime
    /// calls only after it has already decided the caller may read the scope.
    pub fn search(&self, scope_name: &str, tag: &str, now_ms: u64) -> Vec<MemoryEntry> {
        self.entries
            .get(scope_name)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.tags.iter().any(|t| t == tag) && !entry_expired(e, now_ms))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_conservative() {
        let p = MemoryPolicy::default();
        assert_eq!(p.level, MemoryLevel::Agent);
        assert_eq!(p.access, MemoryAccess::Private);
        assert_eq!(p.retention_secs, None);
        assert!(!p.require_verified_provenance);
        assert!(
            !p.allow_remote_write,
            "peers never write without explicit opt-in"
        );
        assert_eq!(p.max_entries, 1024);
    }

    #[test]
    fn memory_access_permits_by_reader_level() {
        assert!(MemoryAccess::Private.permits(MemoryLevel::Agent));
        assert!(!MemoryAccess::Private.permits(MemoryLevel::Team));
        assert!(MemoryAccess::TeamOnly.permits(MemoryLevel::Team));
        assert!(!MemoryAccess::TeamOnly.permits(MemoryLevel::Network));
        assert!(MemoryAccess::TrustedNetwork.permits(MemoryLevel::Network));
        assert!(!MemoryAccess::TrustedNetwork.permits(MemoryLevel::Fabric));
        assert!(MemoryAccess::Public.permits(MemoryLevel::Fabric));
    }

    #[test]
    fn private_scope_reads_only_owner() {
        let scope = MemoryScope::new("agent-a.notes", "agent-a", MemoryLevel::Agent);
        assert!(can_read(&scope, "agent-a", true, false).is_granted());
        let denied = can_read(&scope, "agent-b", false, true);
        assert!(!denied.is_granted());
        assert!(denied.reason().unwrap().contains("private"));
    }

    #[test]
    fn team_and_network_scopes_require_trust_to_read() {
        let team = MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team)
            .with_policy(MemoryPolicy::default().team());
        assert!(can_read(&team, "agent-b", false, true).is_granted());
        assert!(!can_read(&team, "agent-b", false, false).is_granted());

        let net = MemoryScope::new("net.bench", "agent-a", MemoryLevel::Network).with_policy(
            MemoryPolicy {
                access: MemoryAccess::TrustedNetwork,
                ..MemoryPolicy::default()
            },
        );
        assert!(can_read(&net, "agent-c", false, true).is_granted());
        assert!(!can_read(&net, "agent-c", false, false).is_granted());
    }

    #[test]
    fn public_scope_reads_anyone() {
        let scope = MemoryScope::new("fabric.lessons", "agent-a", MemoryLevel::Fabric)
            .with_policy(MemoryPolicy::default().public());
        assert!(can_read(&scope, "stranger", false, false).is_granted());
    }

    #[test]
    fn owner_writes_always_allowed() {
        let private = MemoryScope::new("notes", "agent-a", MemoryLevel::Agent);
        assert!(can_write(&private, "agent-a", true, false, false).is_granted());

        let shared = MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team)
            .with_policy(MemoryPolicy::default().team().with_remote_write());
        assert!(can_write(&shared, "agent-a", true, false, false).is_granted());
    }

    #[test]
    fn remote_writers_need_access_and_opt_in() {
        let private = MemoryScope::new("notes", "agent-a", MemoryLevel::Agent);
        assert!(!can_write(&private, "agent-b", false, true, false).is_granted());

        // Team scope WITHOUT remote opt-in: trust alone is not enough.
        let closed_team = MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team)
            .with_policy(MemoryPolicy::default().team());
        assert!(!can_write(&closed_team, "agent-b", false, true, false).is_granted());

        // Team scope WITH remote opt-in: a trusted teammate may write,
        // a stranger may not.
        let open_team = closed_team
            .clone()
            .with_policy(MemoryPolicy::default().team().with_remote_write());
        assert!(can_write(&open_team, "agent-b", false, true, false).is_granted());
        assert!(!can_write(&open_team, "stranger", false, false, false).is_granted());

        // Public scope WITH remote opt-in: anyone may write.
        let public = MemoryScope::new("fabric.lessons", "agent-a", MemoryLevel::Fabric)
            .with_policy(MemoryPolicy::default().public().with_remote_write());
        assert!(can_write(&public, "stranger", false, false, false).is_granted());

        // Public scope WITHOUT remote opt-in is still owner-only for writes.
        let closed_public = MemoryScope::new("fabric.lessons", "agent-a", MemoryLevel::Fabric)
            .with_policy(MemoryPolicy::default().public());
        assert!(!can_write(&closed_public, "stranger", false, false, false).is_granted());
    }

    #[test]
    fn verified_provenance_is_enforced_for_every_writer() {
        let scope = MemoryScope::new("notes", "agent-a", MemoryLevel::Agent)
            .with_policy(MemoryPolicy::default().with_provenance_required());
        // The requirement binds the owner too: access would be granted, but an
        // unverified entry must not poison a provenance-demanding scope.
        let denied = can_write(&scope, "agent-a", true, false, false);
        assert!(!denied.is_granted());
        assert!(denied.reason().unwrap().contains("provenance"));
        assert!(can_write(&scope, "agent-a", true, false, true).is_granted());
    }

    #[test]
    fn entry_expiry_is_checked_against_now() {
        let entry = MemoryEntry::new("e1", "notes", "agent-a", "peer-1", "hello");
        assert!(
            !entry_expired(&entry, 1000),
            "no expiry means never expired"
        );
        let entry = entry.expires_at(500);
        assert!(entry_expired(&entry, 1000));
        assert!(
            !entry_expired(&entry, 500),
            "expiring exactly at now is still valid"
        );
        assert!(!entry_expired(&entry, 499));
    }

    #[test]
    fn enforce_retention_drops_expired_entries() {
        let policy = MemoryPolicy::default().with_retention(60);
        let live = MemoryEntry::new("e1", "notes", "agent-a", "peer-1", "live")
            .created_at(100)
            .expires_at(200);
        let dead = MemoryEntry::new("e2", "notes", "agent-a", "peer-1", "dead")
            .created_at(50)
            .expires_at(90);
        let kept = enforce_retention(vec![live.clone(), dead.clone()], &policy, 100);
        assert_eq!(kept, vec![live]);
    }

    #[test]
    fn enforce_retention_keeps_newest_entries_within_cap() {
        let policy = MemoryPolicy {
            max_entries: 3,
            ..MemoryPolicy::default()
        };
        let entries: Vec<MemoryEntry> = (0..5)
            .map(|i| {
                MemoryEntry::new(format!("e{i}"), "notes", "agent-a", "peer-1", "x")
                    .created_at(i as u64)
            })
            .collect();
        let kept = enforce_retention(entries, &policy, 10_000);
        let ids: Vec<&str> = kept.iter().map(|e| e.entry_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["e2", "e3", "e4"],
            "oldest entries are pruned first"
        );
    }

    #[test]
    fn registry_registers_scopes_and_rejects_duplicates() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(MemoryScope::new("alpha", "agent-a", MemoryLevel::Team))
            .unwrap();
        let dup = MemoryScope::new("alpha", "agent-a", MemoryLevel::Team);
        assert_eq!(
            reg.register_scope(dup),
            Err(MemoryError::DuplicateScope {
                name: "alpha".into()
            })
        );
        reg.register_scope(MemoryScope::new("zeta", "agent-a", MemoryLevel::Agent))
            .unwrap();
        assert!(reg.get_scope("alpha").is_some());
        assert!(reg.get_scope("nope").is_none());
        let names: Vec<String> = reg.scopes().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["alpha", "zeta"], "scopes() is sorted by name");
    }

    #[test]
    fn registry_write_enforces_scope_access_and_expiry() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(MemoryScope::new("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();

        let err = reg.write(
            "nope",
            MemoryEntry::new("e1", "nope", "agent-a", "peer-1", "x"),
            "agent-a",
            true,
            false,
            false,
            1000,
        );
        assert_eq!(
            err,
            Err(MemoryError::UnknownScope {
                name: "nope".into()
            })
        );

        let err = reg.write(
            "notes",
            MemoryEntry::new("e2", "notes", "agent-b", "peer-2", "x"),
            "agent-b",
            false,
            true,
            false,
            1000,
        );
        assert!(matches!(err, Err(MemoryError::AccessDenied { .. })));

        let expired = MemoryEntry::new("e3", "notes", "agent-a", "peer-1", "x").expires_at(100);
        let err = reg.write("notes", expired, "agent-a", true, false, false, 1000);
        assert_eq!(
            err,
            Err(MemoryError::EntryExpired {
                entry_id: "e3".into()
            })
        );

        reg.write(
            "notes",
            MemoryEntry::new("ok", "notes", "agent-a", "peer-1", "ok"),
            "agent-a",
            true,
            false,
            false,
            1000,
        )
        .unwrap();
        assert_eq!(reg.read("notes", "agent-a", false, 1000).unwrap().len(), 1);
    }

    #[test]
    fn registry_write_applies_max_entries_cap() {
        let mut reg = MemoryRegistry::new();
        let policy = MemoryPolicy {
            max_entries: 2,
            ..MemoryPolicy::default()
        };
        reg.register_scope(
            MemoryScope::new("notes", "agent-a", MemoryLevel::Agent).with_policy(policy),
        )
        .unwrap();
        for i in 1..=3 {
            reg.write(
                "notes",
                MemoryEntry::new(format!("e{i}"), "notes", "agent-a", "peer-1", "x")
                    .created_at(i * 100),
                "agent-a",
                true,
                false,
                false,
                1_000_000,
            )
            .unwrap();
        }
        let seen = reg.read("notes", "agent-a", false, 1_000_000).unwrap();
        let ids: Vec<&str> = seen.iter().map(|e| e.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["e2", "e3"], "the oldest entry is pruned first");
    }

    #[test]
    fn registry_write_prunes_expired_entries() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(MemoryScope::new("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        reg.write(
            "notes",
            MemoryEntry::new("old", "notes", "agent-a", "peer-1", "old").expires_at(100),
            "agent-a",
            true,
            false,
            false,
            50,
        )
        .unwrap();
        reg.write(
            "notes",
            MemoryEntry::new("fresh", "notes", "agent-a", "peer-1", "fresh"),
            "agent-a",
            true,
            false,
            false,
            150,
        )
        .unwrap();
        let seen = reg.read("notes", "agent-a", false, 150).unwrap();
        let ids: Vec<String> = seen.into_iter().map(|e| e.entry_id).collect();
        assert_eq!(
            ids,
            vec!["fresh"],
            "expired entries are pruned on the next write"
        );
    }

    #[test]
    fn registry_read_enforces_access_and_filters_expiry() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(MemoryScope::new("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        reg.write(
            "notes",
            MemoryEntry::new("secret", "notes", "agent-a", "peer-1", "secret").expires_at(100),
            "agent-a",
            true,
            false,
            false,
            50,
        )
        .unwrap();

        // Owner reads the live entry.
        assert_eq!(reg.read("notes", "agent-a", false, 50).unwrap().len(), 1);
        // Owner at now=150 sees it filtered as expired.
        assert!(reg.read("notes", "agent-a", false, 150).unwrap().is_empty());
        // A non-owner is denied regardless of time.
        let err = reg.read("notes", "agent-b", false, 50);
        assert!(matches!(err, Err(MemoryError::AccessDenied { .. })));
    }

    #[test]
    fn registry_search_filters_by_tag_and_expiry() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(
            MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team)
                .with_policy(MemoryPolicy::default().team()),
        )
        .unwrap();
        let mk = |id: &str, tag: &str, exp: Option<u64>| {
            let mut e = MemoryEntry::new(id, "team.notes", "agent-a", "peer-1", "x").tagged(tag);
            if let Some(ms) = exp {
                e = e.expires_at(ms);
            }
            e
        };
        reg.write(
            "team.notes",
            mk("e1", "architecture", Some(1000)),
            "agent-a",
            true,
            false,
            false,
            500,
        )
        .unwrap();
        reg.write(
            "team.notes",
            mk("e2", "flaky", Some(600)),
            "agent-a",
            true,
            false,
            false,
            500,
        )
        .unwrap();
        reg.write(
            "team.notes",
            mk("e3", "architecture", None),
            "agent-a",
            true,
            false,
            false,
            500,
        )
        .unwrap();

        let hits: Vec<String> = reg
            .search("team.notes", "architecture", 700)
            .into_iter()
            .map(|e| e.entry_id)
            .collect();
        assert_eq!(hits, vec!["e1", "e3"]);
        // The flaky entry expired by now=700 and must not surface.
        assert!(reg.search("team.notes", "flaky", 700).is_empty());
        // Unknown scope is simply empty, not an error.
        assert!(reg.search("nope", "x", 700).is_empty());
    }

    #[test]
    fn registry_unregister_removes_scope_and_entries() {
        let mut reg = MemoryRegistry::new();
        reg.register_scope(MemoryScope::new("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        reg.write(
            "notes",
            MemoryEntry::new("e1", "notes", "agent-a", "peer-1", "x"),
            "agent-a",
            true,
            false,
            false,
            1000,
        )
        .unwrap();
        assert!(reg.unregister_scope("notes"));
        assert!(!reg.unregister_scope("notes"));
        assert!(reg.get_scope("notes").is_none());
        assert!(matches!(
            reg.read("notes", "agent-a", false, 1000),
            Err(MemoryError::UnknownScope { .. })
        ));
    }

    #[test]
    fn registry_allows_trusted_teammate_writes_to_open_team_scope() {
        let mut reg = MemoryRegistry::new();
        let policy = MemoryPolicy::default().team().with_remote_write();
        reg.register_scope(
            MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team).with_policy(policy),
        )
        .unwrap();
        reg.write(
            "team.notes",
            MemoryEntry::new("e1", "team.notes", "agent-b", "peer-2", "shared"),
            "agent-b",
            false,
            true,
            false,
            1000,
        )
        .unwrap();
        let seen = reg.read("team.notes", "agent-b", true, 1000).unwrap();
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn wire_types_round_trip_over_json() {
        let level = MemoryLevel::Network;
        let lj = serde_json::to_string(&level).unwrap();
        assert_eq!(serde_json::from_str::<MemoryLevel>(&lj).unwrap(), level);

        let access = MemoryAccess::TrustedNetwork;
        let aj = serde_json::to_string(&access).unwrap();
        assert_eq!(serde_json::from_str::<MemoryAccess>(&aj).unwrap(), access);

        let policy = MemoryPolicy::default()
            .team()
            .with_remote_write()
            .with_retention(3600)
            .with_provenance_required();
        let pj = serde_json::to_string(&policy).unwrap();
        assert_eq!(serde_json::from_str::<MemoryPolicy>(&pj).unwrap(), policy);

        let entry = MemoryEntry::new("e1", "team.notes", "agent-a", "peer-1", "hello")
            .tagged("greeting")
            .created_at(123)
            .expires_at(999)
            .with_provenance(Provenance::Verified);
        let ej = serde_json::to_string(&entry).unwrap();
        assert_eq!(serde_json::from_str::<MemoryEntry>(&ej).unwrap(), entry);

        let no_prov = MemoryEntry::new("e2", "team.notes", "agent-a", "peer-1", "x");
        let nj = serde_json::to_string(&no_prov).unwrap();
        assert_eq!(serde_json::from_str::<MemoryEntry>(&nj).unwrap(), no_prov);

        let scope =
            MemoryScope::new("team.notes", "agent-a", MemoryLevel::Team).with_policy(policy);
        let sj = serde_json::to_string(&scope).unwrap();
        assert_eq!(serde_json::from_str::<MemoryScope>(&sj).unwrap(), scope);

        let decision = MemoryAccessDecision::Denied {
            reason: "nope".into(),
        };
        let dj = serde_json::to_string(&decision).unwrap();
        assert_eq!(
            serde_json::from_str::<MemoryAccessDecision>(&dj).unwrap(),
            decision
        );
    }

    #[test]
    fn wire_names_are_stable_snake_case() {
        assert_eq!(
            serde_json::to_string(&MemoryLevel::Agent).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryLevel::Team).unwrap(),
            "\"team\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryLevel::Network).unwrap(),
            "\"network\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryLevel::Fabric).unwrap(),
            "\"fabric\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryAccess::Private).unwrap(),
            "\"private\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryAccess::TeamOnly).unwrap(),
            "\"team_only\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryAccess::TrustedNetwork).unwrap(),
            "\"trusted_network\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryAccess::Public).unwrap(),
            "\"public\""
        );
    }
}
