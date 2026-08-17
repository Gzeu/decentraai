//! P5 runtime: persistent collective memory (SQLite).
//!
//! The pure memory model lives in `decentraai_agents::memory` (scopes,
//! policies, `can_read`/`can_write` gates). That crate is deliberately I/O
//! free, so the persistence half — a durable, access-enforcing store — lives
//! here. This module is the runtime side of P5 (collective memory): it backs
//! the model's decision functions with a real SQLite file so scopes and
//! entries survive process restarts and can be shared by the memory
//! persistence task.
//!
//! # Why SQLite
//!
//! A memory scope's policy must be enforced even after a restart, and entries
//! must survive reboot. The pure model is stateless, so every `read`/`write`
//! here re-loads the scope, re-runs the pure `can_read`/`can_write` gate, and
//! applies expiry + `max_entries` pruning *in the store* before returning or
//! persisting. Access decisions are never cached: the store always asks the
//! pure model with the caller-supplied trust facts.
//!
//! # Enforced invariants
//!
//! - Every `read`/`search` runs `can_read`; every `write` runs `can_write`.
//! - Expired entries never surface to readers and are pruned on write.
//! - A scope never holds more than `policy.max_entries` live entries.
//! - `unregister_scope` removes a scope and all its entries atomically.

use anyhow::Result;
use decentraai_agents::memory::{
    MemoryAccessDecision, MemoryEntry, MemoryLevel, MemoryPolicy, MemoryScope, can_read, can_write,
};
use decentraai_hub::capability::Provenance;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

/// Errors from the persistent memory store. All recoverable and explainable.
#[derive(Debug, Error)]
pub enum MemoryStoreError {
    /// A SQLite failure (wrapped string to keep the public surface simple).
    #[error("sqlite error: {0}")]
    Sql(String),
    /// A scope with the same name is already registered.
    #[error("memory scope '{name}' is already registered")]
    DuplicateScope { name: String },
    /// The actor is not allowed to read or write the scope.
    #[error("access denied: {reason}")]
    AccessDenied { reason: String },
    /// The named scope does not exist.
    #[error("memory scope '{name}' does not exist")]
    UnknownScope { name: String },
}

impl From<rusqlite::Error> for MemoryStoreError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryStoreError::Sql(e.to_string())
    }
}

/// A persistent, access-enforcing collective-memory store.
///
/// Backed by a single SQLite connection guarded by a mutex: SQLite is not
/// sync-safe across threads, so every operation takes the lock. The store is
/// cheap to construct; callers hold it for the lifetime of the node's memory
/// task.
pub struct MemoryStore {
    conn: Mutex<Connection>,
}

const CREATE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS memory_scopes (
    name TEXT PRIMARY KEY,
    owner_agent TEXT NOT NULL,
    level TEXT NOT NULL,
    policy TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_entries (
    entry_id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    author_agent TEXT NOT NULL,
    author_node TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    provenance TEXT,
    FOREIGN KEY(scope) REFERENCES memory_scopes(name)
);
";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn level_to_str(level: MemoryLevel) -> &'static str {
    match level {
        MemoryLevel::Agent => "agent",
        MemoryLevel::Team => "team",
        MemoryLevel::Network => "network",
        MemoryLevel::Fabric => "fabric",
    }
}

fn level_from_str(s: &str) -> Result<MemoryLevel, MemoryStoreError> {
    match s {
        "agent" => Ok(MemoryLevel::Agent),
        "team" => Ok(MemoryLevel::Team),
        "network" => Ok(MemoryLevel::Network),
        "fabric" => Ok(MemoryLevel::Fabric),
        other => Err(MemoryStoreError::Sql(format!(
            "unrecognized memory level stored in db: '{other}'"
        ))),
    }
}

fn provenance_to_str(p: Provenance) -> &'static str {
    match p {
        Provenance::Verified => "verified",
        Provenance::Inferred => "inferred",
    }
}

fn provenance_from_str(s: &str) -> Result<Provenance, MemoryStoreError> {
    match s {
        "verified" => Ok(Provenance::Verified),
        "inferred" => Ok(Provenance::Inferred),
        other => Err(MemoryStoreError::Sql(format!(
            "unrecognized provenance stored in db: '{other}'"
        ))),
    }
}

impl MemoryStore {
    /// Opens (creating if needed) the SQLite store at `path`, ensuring the
    /// schema exists. Idempotent.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(CREATE_SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Registers a scope. Fails with [`MemoryStoreError::DuplicateScope`] if
    /// the name is already taken.
    pub fn register_scope(&self, scope: &MemoryScope) -> Result<(), MemoryStoreError> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_scopes WHERE name = ?1)",
                params![scope.name],
                |r| r.get(0),
            )
            .map_err(MemoryStoreError::from)?;
        if exists {
            return Err(MemoryStoreError::DuplicateScope {
                name: scope.name.clone(),
            });
        }
        conn.execute(
            "INSERT INTO memory_scopes (name, owner_agent, level, policy, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                scope.name,
                scope.owner_agent,
                level_to_str(scope.level),
                serde_json::to_string(&scope.policy)
                    .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
                scope.created_at_ms as i64,
            ],
        )?;
        Ok(())
    }

    /// Looks up a scope by name.
    pub fn get_scope(&self, name: &str) -> Result<Option<MemoryScope>, MemoryStoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT name, owner_agent, level, policy, created_at_ms FROM memory_scopes
                 WHERE name = ?1",
                params![name],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(scope_from_row).transpose()
    }

    /// All scopes, sorted by name (deterministic).
    pub fn list_scopes(&self) -> Result<Vec<MemoryScope>, MemoryStoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name, owner_agent, level, policy, created_at_ms FROM memory_scopes
             ORDER BY name ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut scopes = Vec::new();
        for row in rows {
            scopes.push(scope_from_row(row?)?);
        }
        Ok(scopes)
    }

    /// Removes a scope and all its entries atomically; returns whether it
    /// existed.
    pub fn unregister_scope(&self, name: &str) -> Result<bool, MemoryStoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let deleted = tx.execute(
            "DELETE FROM memory_entries WHERE scope = ?1",
            params![name],
        )?;
        let removed = tx.execute(
            "DELETE FROM memory_scopes WHERE name = ?1",
            params![name],
        )?;
        tx.commit()?;
        let _ = deleted;
        Ok(removed > 0)
    }

    /// Writes an entry to a scope, enforcing the scope's policy.
    ///
    /// Loads the scope, runs the pure [`can_write`] gate, prunes expired
    /// entries, enforces the `max_entries` cap (keeping the newest), then
    /// inserts. `now_ms` is derived from the system clock.
    pub fn write(
        &self,
        scope_name: &str,
        entry: &MemoryEntry,
        writer_agent: &str,
        writer_is_owner: bool,
        trusted: bool,
        verified_provenance: bool,
    ) -> Result<(), MemoryStoreError> {
        let scope = self.get_scope(scope_name)?.ok_or_else(|| {
            MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
            }
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
                return Err(MemoryStoreError::AccessDenied { reason });
            }
        }
        let now = now_ms();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Prune expired entries for this scope.
        tx.execute(
            "DELETE FROM memory_entries WHERE scope = ?1 AND expires_at_ms IS NOT NULL AND expires_at_ms < ?2",
            params![scope_name, now as i64],
        )?;
        // Drop the oldest live entries so that, after inserting the new one,
        // the scope holds at most `max_entries` live entries.
        let live: i64 = tx.query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE scope = ?1
             AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
            params![scope_name, now as i64],
            |r| r.get(0),
        )?;
        let to_delete = (live + 1).saturating_sub(scope.policy.max_entries as i64);
        if to_delete > 0 {
            tx.execute(
                "DELETE FROM memory_entries WHERE scope = ?1 AND entry_id IN (
                    SELECT entry_id FROM memory_entries
                    WHERE scope = ?1
                    ORDER BY created_at_ms ASC, entry_id DESC
                    LIMIT ?2
                )",
                params![scope_name, to_delete],
            )?;
        }
        tx.execute(
            "INSERT INTO memory_entries
                (entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.entry_id,
                entry.scope,
                entry.author_agent,
                entry.author_node,
                entry.content,
                serde_json::to_string(&entry.tags)
                    .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
                entry.created_at_ms as i64,
                entry.expires_at_ms.map(|ms| ms as i64),
                entry
                    .provenance
                    .map(|p| provenance_to_str(p).to_string()),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Reads a scope's non-expired entries, newest-first
    /// (`created_at_ms` desc, `entry_id` asc). Enforces [`can_read`].
    pub fn read(
        &self,
        scope_name: &str,
        reader_agent: &str,
        trusted: bool,
    ) -> Result<Vec<MemoryEntry>, MemoryStoreError> {
        let scope = self.get_scope(scope_name)?.ok_or_else(|| {
            MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
            }
        })?;
        let reader_is_owner = scope.owner_agent == reader_agent;
        match can_read(&scope, reader_agent, reader_is_owner, trusted) {
            MemoryAccessDecision::Granted => {}
            MemoryAccessDecision::Denied { reason } => {
                return Err(MemoryStoreError::AccessDenied { reason });
            }
        }
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance
             FROM memory_entries
             WHERE scope = ?1 AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)
             ORDER BY created_at_ms DESC, entry_id ASC",
        )?;
        let rows = stmt.query_map(params![scope_name, now as i64], entry_from_row)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Searches a scope's non-expired entries by tag, newest-first. Enforces
    /// the same read access as [`MemoryStore::read`], then filters by tag.
    pub fn search(
        &self,
        scope_name: &str,
        tag: &str,
        reader_agent: &str,
        trusted: bool,
    ) -> Result<Vec<MemoryEntry>, MemoryStoreError> {
        Ok(self
            .read(scope_name, reader_agent, trusted)?
            .into_iter()
            .filter(|e| e.tags.iter().any(|t| t == tag))
            .collect())
    }

    /// Number of live (non-expired) entries in a scope.
    pub fn entry_count(&self, scope_name: &str) -> Result<usize, MemoryStoreError> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE scope = ?1
             AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
            params![scope_name, now as i64],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Number of registered scopes.
    pub fn scope_count(&self) -> Result<usize, MemoryStoreError> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_scopes",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

fn scope_from_row(
    (name, owner_agent, level, policy_json, created_at_ms): (
        String,
        String,
        String,
        String,
        i64,
    ),
) -> Result<MemoryScope, MemoryStoreError> {
    let policy: MemoryPolicy = serde_json::from_str(&policy_json)
        .map_err(|e| MemoryStoreError::Sql(e.to_string()))?;
    Ok(MemoryScope {
        name,
        owner_agent,
        level: level_from_str(&level)?,
        policy,
        created_at_ms: created_at_ms as u64,
    })
}

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let tags_json: String = row.get(5)?;
    let provenance: Option<String> = row.get(8)?;
    let tags: Vec<String> =
        serde_json::from_str(&tags_json).unwrap_or_default();
    let provenance = provenance
        .as_deref()
        .and_then(|s| provenance_from_str(s).ok());
    Ok(MemoryEntry {
        entry_id: row.get(0)?,
        scope: row.get(1)?,
        author_agent: row.get(2)?,
        author_node: row.get(3)?,
        content: row.get(4)?,
        tags,
        created_at_ms: row.get::<_, i64>(6)? as u64,
        expires_at_ms: row.get::<_, Option<i64>>(7)?.map(|ms| ms as u64),
        provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::memory::MemoryPolicy;

    fn scope(name: &str, owner: &str, level: MemoryLevel) -> MemoryScope {
        MemoryScope::new(name, owner, level)
    }

    fn entry(id: &str, scope_name: &str, author: &str, content: &str) -> MemoryEntry {
        MemoryEntry::new(id, scope_name, author, "peer-1", content)
    }

    fn store_in_memory() -> MemoryStore {
        MemoryStore::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn register_rejects_duplicates_and_list_scopes_is_sorted() {
        let store = store_in_memory();
        store.register_scope(&scope("zeta", "agent-a", MemoryLevel::Agent)).unwrap();
        store.register_scope(&scope("alpha", "agent-b", MemoryLevel::Team)).unwrap();
        // Duplicate name rejected.
        let err = store.register_scope(&scope("alpha", "agent-c", MemoryLevel::Network));
        assert!(matches!(err, Err(MemoryStoreError::DuplicateScope { name }) if name == "alpha"));
        let names: Vec<String> = store
            .list_scopes()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
        assert_eq!(store.scope_count().unwrap(), 2);
    }

    #[test]
    fn get_scope_round_trips_policy_and_level() {
        let store = store_in_memory();
        let policy = MemoryPolicy::default()
            .team()
            .with_remote_write()
            .with_retention(3600)
            .with_provenance_required();
        let sc = scope("team.notes", "agent-a", MemoryLevel::Team).with_policy(policy);
        store.register_scope(&sc).unwrap();
        let got = store.get_scope("team.notes").unwrap().unwrap();
        assert_eq!(got.name, "team.notes");
        assert_eq!(got.owner_agent, "agent-a");
        assert_eq!(got.level, MemoryLevel::Team);
        assert_eq!(got.policy, policy);
        assert!(got.policy.require_verified_provenance);
        assert!(got.policy.allow_remote_write);
        assert_eq!(got.policy.retention_secs, Some(3600));
        assert!(store.get_scope("nope").unwrap().is_none());
    }

    #[test]
    fn write_enforces_owner_write_and_denies_remote_without_opt_in() {
        let store = store_in_memory();
        store.register_scope(&scope("notes", "agent-a", MemoryLevel::Agent)).unwrap();
        // Owner may write.
        store
            .write("notes", &entry("e1", "notes", "agent-a", "mine"), "agent-a", true, false, false)
            .unwrap();
        // A stranger (non-owner) without allow_remote_write is denied.
        let err = store.write("notes", &entry("e2", "notes", "agent-b", "x"), "agent-b", false, true, false);
        assert!(matches!(err, Err(MemoryStoreError::AccessDenied { .. })));
        // Unknown scope.
        let err = store.write("nope", &entry("e3", "nope", "agent-a", "x"), "agent-a", true, false, false);
        assert!(matches!(err, Err(MemoryStoreError::UnknownScope { .. })));
    }

    #[test]
    fn write_then_read_returns_persisted_entry_and_read_enforces_access() {
        let store = store_in_memory();
        store.register_scope(&scope("notes", "agent-a", MemoryLevel::Agent)).unwrap();
        store
            .write("notes", &entry("e1", "notes", "agent-a", "hello").tagged("greeting"), "agent-a", true, false, false)
            .unwrap();
        let seen = store.read("notes", "agent-a", false).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].content, "hello");
        assert_eq!(seen[0].tags, vec!["greeting"]);
        // A stranger cannot read a private scope.
        let err = store.read("notes", "agent-b", true);
        assert!(matches!(err, Err(MemoryStoreError::AccessDenied { .. })));
    }

    #[test]
    fn max_entries_keeps_newest() {
        let store = store_in_memory();
        let policy = MemoryPolicy {
            max_entries: 2,
            ..MemoryPolicy::default()
        };
        store
            .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent).with_policy(policy))
            .unwrap();
        for i in 1..=3 {
            store
                .write(
                    "notes",
                    &entry(&format!("e{i}"), "notes", "agent-a", "x").created_at(i * 100),
                    "agent-a",
                    true,
                    false,
                    false,
                )
                .unwrap();
        }
        let ids: Vec<String> = store.read("notes", "agent-a", false).unwrap().into_iter().map(|e| e.entry_id).collect();
        assert_eq!(ids, vec!["e3", "e2"], "oldest entry is pruned, newest-first");
        assert_eq!(store.entry_count("notes").unwrap(), 2);
    }

    #[test]
    fn expiry_drops_expired_entries_on_write_and_read() {
        let store = store_in_memory();
        store.register_scope(&scope("notes", "agent-a", MemoryLevel::Agent)).unwrap();
        // An entry that expires in the past (relative to now) is never returned.
        store
            .write("notes", &entry("old", "notes", "agent-a", "old").expires_at(1), "agent-a", true, false, false)
            .unwrap();
        assert!(store.read("notes", "agent-a", false).unwrap().is_empty(), "expired entry must not surface");
        // Writing a fresh entry prunes the expired one.
        store
            .write("notes", &entry("fresh", "notes", "agent-a", "fresh"), "agent-a", true, false, false)
            .unwrap();
        let seen = store.read("notes", "agent-a", false).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].entry_id, "fresh");
        assert_eq!(store.entry_count("notes").unwrap(), 1);
    }

    #[test]
    fn search_filters_by_tag() {
        let store = store_in_memory();
        let policy = MemoryPolicy::default().team().with_remote_write();
        store
            .register_scope(&scope("team.notes", "agent-a", MemoryLevel::Team).with_policy(policy))
            .unwrap();
        store
            .write("team.notes", &entry("e1", "team.notes", "agent-a", "a").tagged("arch"), "agent-a", true, false, false)
            .unwrap();
        store
            .write("team.notes", &entry("e2", "team.notes", "agent-a", "b").tagged("flaky"), "agent-a", true, false, false)
            .unwrap();
        store
            .write("team.notes", &entry("e3", "team.notes", "agent-a", "c").tagged("arch"), "agent-a", true, false, false)
            .unwrap();
        let hits: Vec<String> = store
            .search("team.notes", "arch", "agent-b", true)
            .unwrap()
            .into_iter()
            .map(|e| e.entry_id)
            .collect();
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&"e1".to_string()));
        assert!(hits.contains(&"e3".to_string()));
        assert!(store.search("team.notes", "flaky", "agent-a", false).unwrap().len() == 1);
    }

    #[test]
    fn unregister_removes_scope_and_entries() {
        let store = store_in_memory();
        store.register_scope(&scope("notes", "agent-a", MemoryLevel::Agent)).unwrap();
        store
            .write("notes", &entry("e1", "notes", "agent-a", "x"), "agent-a", true, false, false)
            .unwrap();
        assert_eq!(store.scope_count().unwrap(), 1);
        assert_eq!(store.entry_count("notes").unwrap(), 1);
        assert!(store.unregister_scope("notes").unwrap());
        assert!(!store.unregister_scope("notes").unwrap(), "second remove reports absent");
        assert_eq!(store.scope_count().unwrap(), 0);
        // Entries are gone; reading the removed scope is UnknownScope.
        assert!(matches!(
            store.read("notes", "agent-a", false),
            Err(MemoryStoreError::UnknownScope { .. })
        ));
    }

    #[test]
    fn persistence_across_reopen_proves_real_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");
        {
            let store = MemoryStore::open(&db_path).unwrap();
            store.register_scope(&scope("notes", "agent-a", MemoryLevel::Agent)).unwrap();
            store
                .write("notes", &entry("e1", "notes", "agent-a", "durable"), "agent-a", true, false, false)
                .unwrap();
        }
        // Drop the store, reopen from the same file: the entry must survive.
        let store = MemoryStore::open(&db_path).unwrap();
        assert!(store.get_scope("notes").unwrap().is_some());
        let seen = store.read("notes", "agent-a", false).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].content, "durable");
    }

    #[test]
    fn policy_json_round_trips_flags() {
        let store = store_in_memory();
        let policy = MemoryPolicy::default()
            .with_provenance_required()
            .with_remote_write()
            .with_retention(7200);
        store
            .register_scope(&scope("net.bench", "agent-a", MemoryLevel::Network).with_policy(policy))
            .unwrap();
        let got = store.get_scope("net.bench").unwrap().unwrap();
        assert!(got.policy.require_verified_provenance);
        assert!(got.policy.allow_remote_write);
        assert_eq!(got.policy.retention_secs, Some(7200));
        assert_eq!(got.policy, policy);
    }
}
