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
    can_read, can_write, can_transition, MemoryAccessDecision, MemoryEntry, MemoryLevel,
    MemoryPolicy, MemoryScope, MemoryStatus, MemoryTransition, MAX_HISTORY, WriteOutcome,
};
use decentraai_agents::training_export::{training_candidates, TrainingCandidate};
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
    /// The referenced entry does not exist in the scope.
    #[error("memory entry '{entry_id}' does not exist in scope")]
    UnknownEntry { entry_id: String },
    /// A lifecycle transition violated the state machine.
    #[error("invalid memory transition for '{entry_id}': {from:?} → {to:?}")]
    InvalidTransition {
        entry_id: String,
        from: MemoryStatus,
        to: MemoryStatus,
    },
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
    content_hash TEXT NOT NULL DEFAULT '',
    meta TEXT,
    embedding BLOB,
    FOREIGN KEY(scope) REFERENCES memory_scopes(name)
);
";

/// Created after the M18 migration so pre-M18 databases get the column
/// before the index references it.
const CREATE_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_memory_entries_scope ON memory_entries(scope, content_hash);";

/// Adds columns introduced by M18/M19 to databases created before them.
/// Idempotent: existing installs get one `ALTER TABLE` per missing column,
/// fresh installs already have them from `CREATE_SCHEMA`.
fn ensure_m18_columns(conn: &Connection) -> Result<(), MemoryStoreError> {
    let mut stmt = conn.prepare("PRAGMA table_info(memory_entries)")?;
    let mut has_content_hash = false;
    let mut has_meta = false;
    let mut has_embedding = false;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for name in rows {
        match name? {
            ref n if n == "content_hash" => has_content_hash = true,
            ref n if n == "meta" => has_meta = true,
            ref n if n == "embedding" => has_embedding = true,
            _ => {}
        }
    }
    if !has_content_hash {
        conn.execute("ALTER TABLE memory_entries ADD COLUMN content_hash TEXT NOT NULL DEFAULT ''", [])?;
    }
    if !has_meta {
        conn.execute("ALTER TABLE memory_entries ADD COLUMN meta TEXT", [])?;
    }
    if !has_embedding {
        conn.execute("ALTER TABLE memory_entries ADD COLUMN embedding BLOB", [])?;
    }
    Ok(())
}

/// Serializes an embedding vector as little-endian f32 bytes (stable layout
/// across platforms; SQLite BLOB storage).
pub(crate) fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Inverse of [`vector_to_blob`]; `None` when the blob length is not a
/// multiple of 4 or is empty.
pub(crate) fn blob_to_vector(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.is_empty() || blob.len() % 4 != 0 {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Cosine similarity between two equal-length, non-zero vectors.
/// Deterministic pure function; `None` on length mismatch or a zero vector
/// (no direction → no meaningful similarity, never silently scored 0).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|y| y * y).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some(dot / (norm_a * norm_b))
}

use decentraai_protocol::memory_sync::SyncMemoryEntry;

/// Converts a wire sync entry into a domain entry, enforcing the trust
/// boundary: a remote claim ALWAYS lands as [`MemoryStatus::Candidate`] —
/// verified/trusted status is earned through LOCAL verification
/// (`transition_status` against local evidence), never imported from the
/// payload where a hostile peer could self-declare `trusted` and win
/// conflict resolution. Provenance detail (source/confidence/evidence_ref)
/// is preserved honestly as context.
#[must_use]
pub fn sync_entry_to_memory(entry: SyncMemoryEntry, target_scope: &str) -> MemoryEntry {
    let mut meta: decentraai_agents::memory::MemoryMeta =
        serde_json::from_value(serde_json::json!({
            "kind": entry.meta.kind,
            "status": entry.meta.status,
            "version": entry.meta.version,
            "subject_key": entry.meta.subject_key,
        }))
        .unwrap_or_default();
    // Downgrade whatever the wire claimed.
    meta.status = MemoryStatus::Candidate;
    meta.version = 1;
    if !entry.meta.source.is_empty() || !entry.meta.evidence_ref.is_empty() {
        let mut detail = decentraai_agents::memory::MemoryProvenance::new(
            if entry.meta.source.is_empty() { "remote" } else { entry.meta.source.as_str() },
            entry.author_agent.as_str(),
            entry.author_node.as_str(),
            entry.created_at_ms,
            entry.meta.confidence,
        );
        if !entry.meta.evidence_ref.is_empty() {
            detail = detail.with_evidence(entry.meta.evidence_ref.as_str());
        }
        meta.detail = Some(detail);
    }
    let mut e = MemoryEntry {
        entry_id: entry.entry_id,
        scope: target_scope.to_string(),
        author_agent: entry.author_agent,
        author_node: entry.author_node,
        content: entry.content,
        tags: Vec::new(),
        created_at_ms: entry.created_at_ms,
        expires_at_ms: None,
        provenance: None,
        meta,
    };
    // Local identity for this imported knowledge: subject keys still group
    // conflicts; ids stay stable for dedup across re-sends.
    let _ = &mut e;
    e
}

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
        MemoryLevel::Node => "node",
        MemoryLevel::Network => "network",
        MemoryLevel::Fabric => "fabric",
        MemoryLevel::System => "system",
    }
}

fn level_from_str(s: &str) -> Result<MemoryLevel, MemoryStoreError> {
    match s {
        "agent" => Ok(MemoryLevel::Agent),
        "team" => Ok(MemoryLevel::Team),
        "node" => Ok(MemoryLevel::Node),
        "network" => Ok(MemoryLevel::Network),
        "fabric" => Ok(MemoryLevel::Fabric),
        "system" => Ok(MemoryLevel::System),
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

/// BLAKE3 content hash used for exact-match dedup (same function as the pure
/// model's dedup — one definition of "identical knowledge").
fn content_hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// Deletes expired entries of one scope inside a transaction.
fn prune_expired(
    tx: &rusqlite::Transaction<'_>,
    scope_name: &str,
    now: u64,
) -> Result<(), MemoryStoreError> {
    tx.execute(
        "DELETE FROM memory_entries WHERE scope = ?1 AND expires_at_ms IS NOT NULL AND expires_at_ms < ?2",
        params![scope_name, now as i64],
    )?;
    Ok(())
}

/// Drops the oldest live entries so that, counting `extra` rows about to be
/// inserted (`extra = 0` when called after the insert), the scope holds at
/// most `policy.max_entries` rows. Deterministic: created asc, id desc.
fn enforce_max_entries(
    tx: &rusqlite::Transaction<'_>,
    policy: &MemoryPolicy,
    scope_name: &str,
    now: u64,
    extra: i64,
) -> Result<(), MemoryStoreError> {
    let live: i64 = tx.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE scope = ?1
         AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
        params![scope_name, now as i64],
        |r| r.get(0),
    )?;
    let to_delete = (live + extra).saturating_sub(policy.max_entries as i64);
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
    Ok(())
}

impl MemoryStore {
    /// Opens (creating if needed) the SQLite store at `path`, ensuring the
    /// schema exists (including M18 columns on pre-M18 databases). Idempotent.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(CREATE_SCHEMA)?;
        ensure_m18_columns(&conn)?;
        conn.execute_batch(CREATE_INDEX)?;
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
        let deleted = tx.execute("DELETE FROM memory_entries WHERE scope = ?1", params![name])?;
        let removed = tx.execute("DELETE FROM memory_scopes WHERE name = ?1", params![name])?;
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
        let scope = self
            .get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
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
                return Err(MemoryStoreError::AccessDenied { reason });
            }
        }
        let now = now_ms();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Prune expired entries for this scope.
        prune_expired(&tx, scope_name, now)?;
        // Drop the oldest live entries so that, after inserting the new one,
        // the scope holds at most `max_entries` live entries.
        enforce_max_entries(&tx, &scope.policy, scope_name, now, 1)?;
        tx.execute(
            "INSERT INTO memory_entries
                (entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance, content_hash, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                content_hash(&entry.content),
                serde_json::to_string(&entry.meta)
                    .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Writes through the collective-memory path (M18): access policy,
    /// exact-duplicate rejection by BLAKE3 content hash, and subject-key
    /// conflict handling — competing claims are stored alongside existing
    /// ones and linked bidirectionally; nothing is ever overwritten.
    pub fn write_checked(
        &self,
        scope_name: &str,
        entry: &MemoryEntry,
        writer_agent: &str,
        writer_is_owner: bool,
        trusted: bool,
        verified_provenance: bool,
    ) -> Result<WriteOutcome, MemoryStoreError> {
        let scope = self
            .get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
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
                return Err(MemoryStoreError::AccessDenied { reason });
            }
        }
        let now = now_ms();
        let hash = content_hash(&entry.content);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Expired entries never count for dedup or conflicts.
        prune_expired(&tx, scope_name, now)?;
        if let Some(existing_id) = tx
            .query_row(
                "SELECT entry_id FROM memory_entries WHERE scope = ?1 AND content_hash = ?2 LIMIT 1",
                params![scope_name, hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(WriteOutcome::Duplicate { existing_id });
        }
        // Competing claims about the same subject: link both directions.
        let mut competitors: Vec<String> = Vec::new();
        let mut relink: Vec<(String, String)> = Vec::new(); // (entry_id, new_meta_json)
        if !entry.meta.subject_key.is_empty() {
            let mut stmt = tx.prepare(
                "SELECT entry_id, meta FROM memory_entries WHERE scope = ?1",
            )?;
            let rows = stmt.query_map(params![scope_name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (id, meta_json) = row?;
                let mut meta: decentraai_agents::memory::MemoryMeta = meta_json
                    .as_deref()
                    .and_then(|j| serde_json::from_str(j).ok())
                    .unwrap_or_default();
                if meta.subject_key != entry.meta.subject_key {
                    continue;
                }
                competitors.push(id.clone());
                if !meta.competes_with.contains(&entry.entry_id) {
                    meta.competes_with.push(entry.entry_id.clone());
                    relink.push((
                        id,
                        serde_json::to_string(&meta)
                            .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
                    ));
                }
            }
        }
        for (id, meta_json) in relink {
            tx.execute(
                "UPDATE memory_entries SET meta = ?2 WHERE entry_id = ?1",
                params![id, meta_json],
            )?;
        }
        let mut entry_meta = entry.meta.clone();
        entry_meta.competes_with = competitors.clone();
        tx.execute(
            "INSERT INTO memory_entries
                (entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance, content_hash, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                hash,
                serde_json::to_string(&entry_meta)
                    .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
            ],
        )?;
        enforce_max_entries(&tx, &scope.policy, scope_name, now, 0)?;
        tx.commit()?;
        if competitors.is_empty() {
            Ok(WriteOutcome::Stored)
        } else {
            Ok(WriteOutcome::CompetingClaim {
                stored_id: entry.entry_id.clone(),
                competes_with: competitors,
            })
        }
    }

    /// Applies a lifecycle transition (`candidate → verified → trusted`,
    /// any active → `obsolete`), recording it in the entry's bounded history
    /// and bumping its version. Obsolete entries remain in the store.
    pub fn transition_status(
        &self,
        scope_name: &str,
        entry_id: &str,
        to: MemoryStatus,
        actor: &str,
        reason: &str,
    ) -> Result<(), MemoryStoreError> {
        self.get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
            })?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let row: Option<Option<String>> = tx
            .query_row(
                "SELECT meta FROM memory_entries WHERE scope = ?1 AND entry_id = ?2",
                params![scope_name, entry_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(meta_json) = row else {
            return Err(MemoryStoreError::UnknownEntry {
                entry_id: entry_id.to_string(),
            });
        };
        let mut meta: decentraai_agents::memory::MemoryMeta = meta_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let from = meta.status;
        if !can_transition(from, to) {
            return Err(MemoryStoreError::InvalidTransition {
                entry_id: entry_id.to_string(),
                from,
                to,
            });
        }
        meta.history.push(MemoryTransition {
            from,
            to,
            actor: actor.to_string(),
            reason: decentraai_agents::memory::bounded(reason.to_string()),
            at_ms: now_ms(),
        });
        if meta.history.len() > MAX_HISTORY {
            meta.history.drain(..meta.history.len() - MAX_HISTORY);
        }
        meta.status = to;
        meta.version = meta.version.saturating_add(1);
        tx.execute(
            "UPDATE memory_entries SET meta = ?3 WHERE scope = ?1 AND entry_id = ?2",
            params![
                scope_name,
                entry_id,
                serde_json::to_string(&meta)
                    .map_err(|e| MemoryStoreError::Sql(e.to_string()))?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Collects Training Lab candidates from every scope readable by
    /// `reader_agent`: only VERIFIED/TRUSTED, evidence-backed generalizations
    /// qualify. Explicit export — nothing here trains anything.
    pub fn export_training_candidates(
        &self,
        reader_agent: &str,
        trusted: bool,
    ) -> Result<Vec<TrainingCandidate>, MemoryStoreError> {
        let mut out = Vec::new();
        for scope in self.list_scopes()? {
            let entries = match self.read(&scope.name, reader_agent, trusted) {
                Ok(e) => e,
                Err(_) => continue, // inaccessible scopes are silently skipped
            };
            out.extend(training_candidates(&entries));
        }
        Ok(out)
    }

    /// Stores the embedding vector for one entry (semantic retrieval index).
    /// Idempotent: re-indexing overwrites the previous vector.
    pub fn store_embedding(
        &self,
        scope_name: &str,
        entry_id: &str,
        vector: &[f32],
    ) -> Result<(), MemoryStoreError> {
        self.get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
            })?;
        if vector.is_empty() {
            return Err(MemoryStoreError::Sql(
                "refusing to store an empty embedding vector".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE memory_entries SET embedding = ?3 WHERE scope = ?1 AND entry_id = ?2",
            params![scope_name, entry_id, vector_to_blob(vector)],
        )?;
        if updated == 0 {
            return Err(MemoryStoreError::UnknownEntry {
                entry_id: entry_id.to_string(),
            });
        }
        Ok(())
    }

    /// How many live entries in a scope have / lack an embedding vector.
    /// Observability for the index backfill — gaps must be visible, never
    /// guessed.
    pub fn index_status(
        &self,
        scope_name: &str,
    ) -> Result<(usize, usize), MemoryStoreError> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        let indexed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE scope = ?1
             AND embedding IS NOT NULL
             AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
            params![scope_name, now as i64],
            |r| r.get(0),
        )?;
        let unindexed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE scope = ?1
             AND embedding IS NULL
             AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
            params![scope_name, now as i64],
            |r| r.get(0),
        )?;
        Ok((indexed as usize, unindexed as usize))
    }

    /// Semantic search inside one scope: cosine similarity between
    /// `query_vector` and each indexed entry's stored vector, deterministic
    /// order (score desc → entry_id asc), bounded by `top_k`. Enforces
    /// [`can_read`] like every read. Entries without vectors are invisible
    /// here (they remain reachable through lexical search).
    pub fn search_semantic(
        &self,
        scope_name: &str,
        reader_agent: &str,
        trusted: bool,
        query_vector: &[f32],
        top_k: usize,
    ) -> Result<Vec<(MemoryEntry, f32)>, MemoryStoreError> {
        let scope = self
            .get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
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
            "SELECT entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance, meta, embedding
             FROM memory_entries
             WHERE scope = ?1 AND embedding IS NOT NULL
               AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)",
        )?;
        let rows = stmt.query_map(params![scope_name, now as i64], entry_from_row_with_embedding)?;
        let mut scored: Vec<(MemoryEntry, f32)> = Vec::new();
        for row in rows {
            let (entry, blob) = row?;
            let Some(vec) = blob_to_vector(&blob) else {
                continue; // corrupt/legacy blob → skip, never guess a score
            };
            if let Some(score) = cosine_similarity(query_vector, &vec) {
                scored.push((entry, score));
            }
        }
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.entry_id.cmp(&b.0.entry_id))
        });
        scored.truncate(top_k);
        Ok(scored)
    }

    /// Reads a scope's non-expired entries, newest-first
    /// (`created_at_ms` desc, `entry_id` asc). Enforces [`can_read`].
    pub fn read(
        &self,
        scope_name: &str,
        reader_agent: &str,
        trusted: bool,
    ) -> Result<Vec<MemoryEntry>, MemoryStoreError> {
        let scope = self
            .get_scope(scope_name)?
            .ok_or_else(|| MemoryStoreError::UnknownScope {
                name: scope_name.to_string(),
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
            "SELECT entry_id, scope, author_agent, author_node, content, tags, created_at_ms, expires_at_ms, provenance, meta, embedding
             FROM memory_entries
             WHERE scope = ?1 AND (expires_at_ms IS NULL OR expires_at_ms >= ?2)
             ORDER BY created_at_ms DESC, entry_id ASC",
        )?;
        let rows = stmt.query_map(params![scope_name, now as i64], |r| {
            entry_from_row_with_embedding(r).map(|(e, _)| e)
        })?;
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
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM memory_scopes", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

fn scope_from_row(
    (name, owner_agent, level, policy_json, created_at_ms): (String, String, String, String, i64),
) -> Result<MemoryScope, MemoryStoreError> {
    let policy: MemoryPolicy =
        serde_json::from_str(&policy_json).map_err(|e| MemoryStoreError::Sql(e.to_string()))?;
    Ok(MemoryScope {
        name,
        owner_agent,
        level: level_from_str(&level)?,
        policy,
        created_at_ms: created_at_ms as u64,
    })
}

/// Row mapper that also extracts the optional embedding BLOB (M19).
fn entry_from_row_with_embedding(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(MemoryEntry, Vec<u8>)> {
    let tags_json: String = row.get(5)?;
    let provenance: Option<String> = row.get(8)?;
    let meta_json: Option<String> = row.get(9)?;
    let embedding: Option<Vec<u8>> = row.get(10)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let provenance = provenance
        .as_deref()
        .and_then(|s| provenance_from_str(s).ok());
    // Pre-M18 rows (meta NULL) deserialize to the conservative default:
    // observation / candidate / v1 / no detail.
    let meta = meta_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    Ok((
        MemoryEntry {
            entry_id: row.get(0)?,
            scope: row.get(1)?,
            author_agent: row.get(2)?,
            author_node: row.get(3)?,
            content: row.get(4)?,
            tags,
            created_at_ms: row.get::<_, i64>(6)? as u64,
            expires_at_ms: row.get::<_, Option<i64>>(7)?.map(|ms| ms as u64),
            provenance,
            meta,
        },
        embedding.unwrap_or_default(),
    ))
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
        store
            .register_scope(&scope("zeta", "agent-a", MemoryLevel::Agent))
            .unwrap();
        store
            .register_scope(&scope("alpha", "agent-b", MemoryLevel::Team))
            .unwrap();
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
        store
            .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        // Owner may write.
        store
            .write(
                "notes",
                &entry("e1", "notes", "agent-a", "mine"),
                "agent-a",
                true,
                false,
                false,
            )
            .unwrap();
        // A stranger (non-owner) without allow_remote_write is denied.
        let err = store.write(
            "notes",
            &entry("e2", "notes", "agent-b", "x"),
            "agent-b",
            false,
            true,
            false,
        );
        assert!(matches!(err, Err(MemoryStoreError::AccessDenied { .. })));
        // Unknown scope.
        let err = store.write(
            "nope",
            &entry("e3", "nope", "agent-a", "x"),
            "agent-a",
            true,
            false,
            false,
        );
        assert!(matches!(err, Err(MemoryStoreError::UnknownScope { .. })));
    }

    #[test]
    fn write_then_read_returns_persisted_entry_and_read_enforces_access() {
        let store = store_in_memory();
        store
            .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        store
            .write(
                "notes",
                &entry("e1", "notes", "agent-a", "hello").tagged("greeting"),
                "agent-a",
                true,
                false,
                false,
            )
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
        let ids: Vec<String> = store
            .read("notes", "agent-a", false)
            .unwrap()
            .into_iter()
            .map(|e| e.entry_id)
            .collect();
        assert_eq!(
            ids,
            vec!["e3", "e2"],
            "oldest entry is pruned, newest-first"
        );
        assert_eq!(store.entry_count("notes").unwrap(), 2);
    }

    #[test]
    fn expiry_drops_expired_entries_on_write_and_read() {
        let store = store_in_memory();
        store
            .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        // An entry that expires in the past (relative to now) is never returned.
        store
            .write(
                "notes",
                &entry("old", "notes", "agent-a", "old").expires_at(1),
                "agent-a",
                true,
                false,
                false,
            )
            .unwrap();
        assert!(
            store.read("notes", "agent-a", false).unwrap().is_empty(),
            "expired entry must not surface"
        );
        // Writing a fresh entry prunes the expired one.
        store
            .write(
                "notes",
                &entry("fresh", "notes", "agent-a", "fresh"),
                "agent-a",
                true,
                false,
                false,
            )
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
            .write(
                "team.notes",
                &entry("e1", "team.notes", "agent-a", "a").tagged("arch"),
                "agent-a",
                true,
                false,
                false,
            )
            .unwrap();
        store
            .write(
                "team.notes",
                &entry("e2", "team.notes", "agent-a", "b").tagged("flaky"),
                "agent-a",
                true,
                false,
                false,
            )
            .unwrap();
        store
            .write(
                "team.notes",
                &entry("e3", "team.notes", "agent-a", "c").tagged("arch"),
                "agent-a",
                true,
                false,
                false,
            )
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
        assert!(
            store
                .search("team.notes", "flaky", "agent-a", false)
                .unwrap()
                .len()
                == 1
        );
    }

    #[test]
    fn unregister_removes_scope_and_entries() {
        let store = store_in_memory();
        store
            .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent))
            .unwrap();
        store
            .write(
                "notes",
                &entry("e1", "notes", "agent-a", "x"),
                "agent-a",
                true,
                false,
                false,
            )
            .unwrap();
        assert_eq!(store.scope_count().unwrap(), 1);
        assert_eq!(store.entry_count("notes").unwrap(), 1);
        assert!(store.unregister_scope("notes").unwrap());
        assert!(
            !store.unregister_scope("notes").unwrap(),
            "second remove reports absent"
        );
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
            store
                .register_scope(&scope("notes", "agent-a", MemoryLevel::Agent))
                .unwrap();
            store
                .write(
                    "notes",
                    &entry("e1", "notes", "agent-a", "durable"),
                    "agent-a",
                    true,
                    false,
                    false,
                )
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
            .register_scope(
                &scope("net.bench", "agent-a", MemoryLevel::Network).with_policy(policy),
            )
            .unwrap();
        let got = store.get_scope("net.bench").unwrap().unwrap();
        assert!(got.policy.require_verified_provenance);
        assert!(got.policy.allow_remote_write);
        assert_eq!(got.policy.retention_secs, Some(7200));
        assert_eq!(got.policy, policy);
    }

    // ----- M18: collective-memory persistence -----

    fn team_scope() -> MemoryScope {
        let policy = MemoryPolicy::default().team().with_remote_write();
        MemoryScope::new("team.knowledge", "governor", MemoryLevel::Team).with_policy(policy)
    }

    fn knowledge(id: &str, content: &str, subject: &str) -> MemoryEntry {
        let mut e = MemoryEntry::new(id, "team.knowledge", "researcher", "node-1", content)
            .with_subject(subject);
        e.created_at_ms = 100;
        e
    }

    #[test]
    fn store_write_checked_dedups_and_links_conflicts() {
        let store = store_in_memory();
        store.register_scope(&team_scope()).unwrap();
        store
            .write_checked("team.knowledge", &knowledge("e1", "lesson A", "q:x"), "governor", true, false, false)
            .unwrap();
        // Exact duplicate → skipped.
        let out = store
            .write_checked("team.knowledge", &knowledge("e2", "lesson A", "q:x"), "governor", true, false, false)
            .unwrap();
        assert!(matches!(out, WriteOutcome::Duplicate { ref existing_id } if existing_id == "e1"));
        // Different content, same subject → competing claim, linked both ways.
        let out = store
            .write_checked("team.knowledge", &knowledge("e3", "lesson B", "q:x"), "peer-2", false, true, false)
            .unwrap();
        let competes = match out {
            WriteOutcome::CompetingClaim { competes_with, .. } => competes_with,
            other => panic!("expected CompetingClaim, got {other:?}"),
        };
        assert_eq!(competes, vec!["e1".to_string()]);
        let all = store.read("team.knowledge", "governor", true).unwrap();
        assert_eq!(all.len(), 2, "both claims persisted");
        let e1 = all.iter().find(|e| e.entry_id == "e1").unwrap();
        assert!(e1.meta.competes_with.contains(&"e3".to_string()), "bidirectional link");
    }

    #[test]
    fn store_transition_status_gates_and_persists() {
        let store = store_in_memory();
        store.register_scope(&team_scope()).unwrap();
        store
            .write_checked("team.knowledge", &knowledge("e1", "lesson", "q:t"), "governor", true, false, false)
            .unwrap();
        // Illegal jump rejected.
        assert!(matches!(
            store.transition_status("team.knowledge", "e1", MemoryStatus::Trusted, "gov", "skip"),
            Err(MemoryStoreError::InvalidTransition { .. })
        ));
        // Legal path persists status + history.
        store.transition_status("team.knowledge", "e1", MemoryStatus::Verified, "verifier", "evidence checked").unwrap();
        store.transition_status("team.knowledge", "e1", MemoryStatus::Trusted, "corroborator", "seen twice").unwrap();
        let e = &store.read("team.knowledge", "governor", true).unwrap()[0];
        assert_eq!(e.meta.status, MemoryStatus::Trusted);
        assert_eq!(e.meta.version, 3);
        assert_eq!(e.meta.history.len(), 2);
        // Unknown entry.
        assert!(matches!(
            store.transition_status("team.knowledge", "ghost", MemoryStatus::Obsolete, "gov", "x"),
            Err(MemoryStoreError::UnknownEntry { .. })
        ));
    }

    #[test]
    fn store_exports_only_verified_evidenced_candidates() {
        use decentraai_agents::memory::{KnowledgeKind, MemoryProvenance};
        let store = store_in_memory();
        store.register_scope(&team_scope()).unwrap();
        // Verified + evidenced learning → exports.
        let mut good = knowledge("g", "use backoff on 429", "q:backoff");
        good.meta.kind = KnowledgeKind::Learning;
        good.meta.detail = Some(
            MemoryProvenance::new("execution", "r", "n1", 1, 90).with_evidence("aud-1"),
        );
        store.write_checked("team.knowledge", &good, "governor", true, false, false).unwrap();
        store.transition_status("team.knowledge", "g", MemoryStatus::Verified, "v", "ok").unwrap();
        // Candidate with evidence → does NOT export.
        let mut cand = knowledge("c", "unverified hunch", "q:hunch");
        cand.meta.kind = KnowledgeKind::Learning;
        cand.meta.detail = Some(
            MemoryProvenance::new("agent_reasoning", "r", "n1", 2, 50).with_evidence("aud-2"),
        );
        store.write_checked("team.knowledge", &cand, "governor", true, false, false).unwrap();
        let got = store.export_training_candidates("governor", true).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].entry_id, "g");
        assert_eq!(got[0].evidence_ref, "aud-1");
    }

    #[test]
    fn pre_m18_database_migrates_and_legacy_rows_get_default_meta() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("legacy.db");
        // Create a PRE-M18 schema by hand (no content_hash/meta columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE memory_scopes (
                    name TEXT PRIMARY KEY, owner_agent TEXT NOT NULL, level TEXT NOT NULL,
                    policy TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
                 CREATE TABLE memory_entries (
                    entry_id TEXT PRIMARY KEY, scope TEXT NOT NULL, author_agent TEXT NOT NULL,
                    author_node TEXT NOT NULL, content TEXT NOT NULL, tags TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL, expires_at_ms INTEGER, provenance TEXT);
                 INSERT INTO memory_scopes VALUES ('notes','agent-a','agent','{}',0);
                 INSERT INTO memory_entries (entry_id, scope, author_agent, author_node, content, tags, created_at_ms)
                    VALUES ('old','notes','agent-a','n1','legacy content','[]',5);",
            )
            .unwrap();
        }
        // Opening with the new code migrates in place and preserves the row.
        let store = MemoryStore::open(&db_path).unwrap();
        let seen = store.read("notes", "agent-a", false).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].content, "legacy content");
        assert_eq!(seen[0].meta, Default::default(), "legacy row → candidate/observation/v1");
        // New collective path works on the migrated store.
        store.register_scope(&team_scope()).unwrap();
        let out = store
            .write_checked("team.knowledge", &knowledge("m1", "fresh", "q:m"), "governor", true, false, false)
            .unwrap();
        assert_eq!(out, WriteOutcome::Stored);
    }

    #[test]
    fn node_and_system_levels_round_trip() {
        let store = store_in_memory();
        for level in [MemoryLevel::Node, MemoryLevel::System] {
            let name = format!("s-{level:?}").to_lowercase();
            store.register_scope(&scope(&name, "governor", level)).unwrap();
            assert_eq!(store.get_scope(&name).unwrap().unwrap().level, level);
        }
    }

    #[test]
    fn cosine_similarity_is_deterministic_and_honest() {
        // Identical direction → 1.0; opposite → -1.0.
        let a = [1.0, 0.0, 2.0];
        let b = [2.0, 0.0, 4.0];
        assert!((cosine_similarity(&a, &b).unwrap() - 1.0).abs() < 1e-6);
        assert!((cosine_similarity(&a, &[-a[0], 0.0, -a[2]]).unwrap() + 1.0).abs() < 1e-6);
        // Orthogonal → 0.
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).unwrap().abs() < 1e-6);
        // Zero vector / length mismatch / empty → None, never a fake score.
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), None);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), None);
        assert_eq!(cosine_similarity(&[], &[1.0]), None);
    }

    #[test]
    fn vector_blob_round_trips() {
        let v = vec![0.25f32, -3.5, 1e-9, f32::MAX];
        let blob = vector_to_blob(&v);
        assert_eq!(blob.len(), 16);
        assert_eq!(blob_to_vector(&blob).unwrap(), v);
        assert_eq!(blob_to_vector(&[1, 2, 3]), None, "non-multiple-of-4 rejected");
        assert_eq!(blob_to_vector(&[]), None);
    }

    #[test]
    fn search_semantic_ranks_by_score_then_id_and_enforces_access() {
        use decentraai_agents::memory::KnowledgeKind;
        let store = store_in_memory();
        store.register_scope(&team_scope()).unwrap();
        // Three entries with hand-made vectors; query closest to "close1".
        for (id, vec) in [
            ("far", vec![1.0f32, 0.0]),
            ("mid", vec![0.9f32, 0.1]),
            ("close1", vec![0.8f32, 0.2]),
            ("close2", vec![0.8f32, 0.2]), // identical score to close1 → id tie-break
        ] {
            let mut e = knowledge(id, id, "q:sem");
            e.meta.kind = KnowledgeKind::Observation;
            e.created_at_ms = 100;
            store.write_checked("team.knowledge", &e, "governor", true, false, false).unwrap();
            store.store_embedding("team.knowledge", id, &vec).unwrap();
        }
        // An entry WITHOUT a vector: invisible to semantic mode.
        store
            .write_checked("team.knowledge", &knowledge("novector", "x", "q:sem"), "governor", true, false, false)
            .unwrap();

        let hits = store
            .search_semantic("team.knowledge", "governor", true, &[0.8f32, 0.2], 10)
            .unwrap();
        let ids: Vec<&str> = hits.iter().map(|(e, _)| e.entry_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["close1", "close2", "mid", "far"],
            "score desc then entry_id asc; unindexed 'novector' invisible"
        );
        let scores: Vec<f32> = hits.iter().map(|(_, s)| *s).collect();
        assert!(scores[0] > scores[2], "perfect match beats partial");
        // top_k bounds the result set.
        let bounded = store
            .search_semantic("team.knowledge", "governor", true, &[0.8f32, 0.2], 1)
            .unwrap();
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].0.entry_id, "close1");
        // Non-owner untrusted reader is denied like any other read path.
        assert!(matches!(
            store.search_semantic("team.knowledge", "stranger", false, &[0.8, 0.2], 10),
            Err(MemoryStoreError::AccessDenied { .. })
        ));
        // Unknown scope errors, empty vectors refused at store time.
        assert!(store.search_semantic("ghost", "governor", true, &[1.0], 5).is_err());
        assert!(store.store_embedding("team.knowledge", "far", &[]).is_err());
        assert!(matches!(
            store.store_embedding("team.knowledge", "ghost-entry", &[1.0]),
            Err(MemoryStoreError::UnknownEntry { .. })
        ));
    }

    #[test]
    fn index_status_reports_gaps_honestly() {
        let store = store_in_memory();
        store.register_scope(&team_scope()).unwrap();
        store.write_checked("team.knowledge", &knowledge("i1", "a", "q:i"), "governor", true, false, false).unwrap();
        store.write_checked("team.knowledge", &knowledge("i2", "b", "q:i"), "governor", true, false, false).unwrap();
        assert_eq!(store.index_status("team.knowledge").unwrap(), (0, 2));
        store.store_embedding("team.knowledge", "i1", &[1.0f32, 2.0]).unwrap();
        assert_eq!(store.index_status("team.knowledge").unwrap(), (1, 1));
    }
}
