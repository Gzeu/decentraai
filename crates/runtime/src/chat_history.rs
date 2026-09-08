//! Server-side chat history persistence (USER DATA, not logging).
//!
//! # Invariant #5 boundary — read this before touching this file
//!
//! "Prompts and outputs are never logged" means: message contents must NEVER
//! flow into telemetry, debug logs, audit logs, metrics, analytics, error
//! messages, or provider telemetry. Chat history is an explicit **USER DATA**
//! store (the user asked the node to remember their conversations), NOT
//! logging. Concretely:
//!
//! - [`ChatStore`] reads/writes ONLY `db/chat_history.json` (0600).
//! - No `tracing!` event in this module carries message content, conversation
//!   ids beyond safe identifiers, or any user text. Audit is never called.
//! - Error responses are static strings; they never echo request content.
//! - The proxy records a turn ONLY after the backend produced a successful
//!   reply, and persistence failures never break inference (best-effort).
//!
//! # Data model
//!
//! Conversations are scoped per authenticated caller (owner key, see
//! [`owner_key`]). Storage is a single JSON file persisted atomically
//! (tmp + sync + rename), mirroring the token registries. Retention is
//! enforced on every write: per-owner conversation cap + max age + per-
//! conversation message cap. Write semantics are last-write-wins on the full
//! message array: chat clients always send the complete history, so the
//! stored array is replaced wholesale and the assistant reply appended.
//!
//! Only user-visible roles (`user`, `assistant`) are stored. Server-injected
//! system prompts (see `apply_generation_defaults`) and `system`/`tool`/
//! `developer` roles are filtered out before storage.

use anyhow::{Context, Result, bail};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Field name a chat client may set in a `/v1/chat/completions` body to pin
/// the turn to an existing conversation. Stripped before forwarding so
/// backends always see a clean OpenAI body.
pub const CONVERSATION_ID_FIELD: &str = "conversation_id";
/// Response header carrying the conversation id (client-supplied or created).
pub const CONVERSATION_ID_HEADER: &str = "x-conversation-id";

/// Max conversations kept per owner (oldest `updated_at` pruned first).
pub const MAX_CONVERSATIONS_PER_OWNER: usize = 100;
/// Max messages kept per conversation (oldest pruned first).
pub const MAX_MESSAGES_PER_CONVERSATION: usize = 200;
/// Max age of a conversation since last update before retention prunes it.
pub const MAX_CONVERSATION_AGE_SECS: u64 = 90 * 24 * 3600;
/// Bounds for a client-supplied conversation id (`[A-Za-z0-9-_]`, 1–64).
pub const MAX_CONVERSATION_ID_LEN: usize = 64;
/// Title derivation: first user message truncated to this many chars.
pub const TITLE_MAX_CHARS: usize = 80;

/// One stored message. Only `user` / `assistant` roles are ever persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub ts: u64,
}

/// One stored conversation, owned by exactly one caller identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub owner: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<ChatMessage>,
}

/// List-view projection: metadata only, never message content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
}

impl From<&Conversation> for ConversationSummary {
    fn from(c: &Conversation) -> Self {
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            created_at: c.created_at,
            updated_at: c.updated_at,
            message_count: c.messages.len(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ChatFile {
    schema_version: u32,
    conversations: BTreeMap<String, Conversation>,
}

/// The chat history store: `conversation_id -> Conversation`, persisted
/// atomically. All methods take the caller `owner` explicitly so ownership
/// isolation holds by construction (no method can reach another owner's
/// conversations).
pub struct ChatStore {
    conversations: BTreeMap<String, Conversation>,
    path: PathBuf,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Generates a fresh conversation id (`conv-` + 16 hex chars).
pub fn new_conversation_id() -> String {
    let mut bytes = [0u8; 8];
    rand_core::OsRng.fill_bytes(&mut bytes);
    format!("conv-{}", hex::encode(bytes))
}

/// Validates a client-supplied conversation id: 1–64 chars of
/// `[A-Za-z0-9-_]`. Anything else is rejected (a fresh id is issued instead
/// of failing the chat turn).
pub fn is_valid_conversation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_CONVERSATION_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Extracts the caller's `conversation_id` from a chat request body, if
/// present and valid. Pure.
pub fn extract_conversation_id(body: &serde_json::Value) -> Option<String> {
    body.get(CONVERSATION_ID_FIELD)
        .and_then(|v| v.as_str())
        .filter(|s| is_valid_conversation_id(s))
        .map(str::to_string)
}

/// Removes the `conversation_id` field from a request body so backends see a
/// clean OpenAI contract. Returns true when the body was modified.
pub fn strip_conversation_id(body: &mut serde_json::Value) -> bool {
    match body.as_object_mut() {
        Some(obj) => obj.remove(CONVERSATION_ID_FIELD).is_some(),
        None => false,
    }
}

/// User-visible messages from a chat request body: keeps `user`/`assistant`
/// roles with non-empty string content only. Server-injected `system`
/// prompts and `tool`/`developer` messages are excluded by design (only
/// user-visible conversation is user data). Pure.
pub fn user_visible_messages(body: &serde_json::Value) -> Vec<(String, String)> {
    let Some(messages) = body.get("messages").and_then(|m| m.as_array()) else {
        return Vec::new();
    };
    messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role")?.as_str()?;
            if role != "user" && role != "assistant" {
                return None;
            }
            let content = m.get("content")?.as_str()?;
            if content.is_empty() {
                return None;
            }
            Some((role.to_string(), content.to_string()))
        })
        .collect()
}

/// Assistant reply from a non-streaming chat completion body
/// (`choices[0].message.content`). Empty/missing yields `None` (nothing
/// worth persisting). Pure.
pub fn parse_assistant_json(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Assistant reply from a streamed SSE transcript: joins every
/// `choices[0].delta.content` across `data:` lines (`[DONE]` and comments
/// ignored). Empty yields `None`. Pure.
pub fn parse_assistant_sse(transcript: &str) -> Option<String> {
    let mut out = String::new();
    for line in transcript.lines() {
        let line = line.trim_start();
        if !line.starts_with("data:") {
            continue;
        }
        let payload = line.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if let Some(text) = value
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            out.push_str(text);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Derives a conversation title from the first user message, truncated to
/// [`TITLE_MAX_CHARS`] chars with an ellipsis. Pure.
pub fn derive_title(messages: &[(String, String)]) -> String {
    let first = messages
        .iter()
        .find(|(role, _)| role == "user")
        .map(|(_, content)| content.as_str())
        .unwrap_or("New conversation");
    let flat: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TITLE_MAX_CHARS {
        flat
    } else {
        let truncated: String = flat.chars().take(TITLE_MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

/// Stable per-caller owner key for conversation scoping. Returns `None` for
/// unauthenticated (`Open`) callers — their turns are never persisted (no
/// stable identity to scope to).
///
/// Scoping (stable across key rotation where possible):
/// - master → `master`
/// - subscription token → `sub:<name>`
/// - consumer key → `acc:<owner_account>` (`dga_` gateway credentials use
///   `agt:<owner_account>` so agent traffic never mingles with app traffic)
/// - wallet session → `wal:<wallet_address>`
pub(crate) fn owner_key(auth: &crate::api::Auth) -> Option<String> {
    match auth {
        crate::api::Auth::Open => None,
        crate::api::Auth::Master => Some("master".to_string()),
        crate::api::Auth::Subscriber { name, .. } => Some(format!("sub:{name}")),
        crate::api::Auth::Consumer {
            key_id, account, ..
        } => {
            if key_id.starts_with("gk-") {
                Some(format!("agt:{account}"))
            } else {
                Some(format!("acc:{account}"))
            }
        }
        crate::api::Auth::Wallet { wallet_address, .. } => Some(format!("wal:{wallet_address}")),
    }
}

/// A single chat turn captured for persistence: who spoke, in which
/// conversation, with which user-visible messages. Built once per request in
/// the proxy; consumed by every serving path (local/fabric/provider,
/// streaming or not).
#[derive(Debug, Clone)]
pub struct ChatCapture {
    /// Stable per-caller owner key (see [`owner_key`]).
    pub owner: String,
    /// Client-pinned or pre-resolved conversation id.
    pub conversation_id: Option<String>,
    /// User-visible `(role, content)` pairs (`user`/`assistant` only).
    pub messages: Vec<(String, String)>,
}

impl ChatStore {
    /// Loads the store. A missing file starts empty; a corrupted file starts
    /// fresh (availability over strictness, same posture as the registries).
    /// Corruption never surfaces user content — only the fact is warned.
    pub fn load(path: &Path) -> Result<Self> {
        let conversations = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<ChatFile>(&content) {
                Ok(file) => file.conversations,
                Err(_) => {
                    tracing::warn!("corrupt chat history store, starting fresh");
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).context("reading chat history store"),
        };
        Ok(Self {
            conversations,
            path: path.to_path_buf(),
        })
    }

    /// Persists atomically (tmp + sync + rename) with 0600 permissions: chat
    /// history is user data, readable only by the node operator's user.
    pub fn save(&self) -> Result<()> {
        let file = ChatFile {
            schema_version: 1,
            conversations: self.conversations.clone(),
        };
        let content = serde_json::to_string(&file)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("tmp");
        {
            let mut out = std::fs::File::create(&tmp)?;
            out.write_all(content.as_bytes())?;
            out.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Rename may land on a pre-existing file with lax permissions;
            // enforce 0600 on the final path too.
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Records one assistant turn: replaces the conversation's message array
    /// with the request's user-visible messages plus the assistant reply
    /// (last-write-wins; clients always send full history), enforces
    /// retention, and persists. Returns the final conversation id.
    ///
    /// An existing conversation owned by someone else is NEVER overwritten:
    /// a fresh id is issued instead (id-guessing cannot cross owners).
    /// Empty replies are refused (nothing worth persisting).
    pub fn record(
        &mut self,
        owner: &str,
        conversation_id: Option<&str>,
        messages: &[(String, String)],
        assistant_reply: &str,
    ) -> Result<String> {
        let owner = owner.trim();
        if owner.is_empty() {
            bail!("owner must not be empty");
        }
        if assistant_reply.is_empty() {
            bail!("refusing to persist an empty assistant reply");
        }
        let now = now_secs();
        let wanted = conversation_id.filter(|id| is_valid_conversation_id(id));
        // Resolve the target id: reuse only when the existing conversation
        // belongs to this owner; otherwise issue a fresh one.
        let id = match wanted {
            Some(id) => match self.conversations.get(id) {
                Some(existing) if existing.owner == owner => id.to_string(),
                Some(_) => new_conversation_id(),
                None => id.to_string(),
            },
            None => new_conversation_id(),
        };
        let entry = self.conversations.entry(id.clone()).or_insert_with(|| {
            let title = derive_title(messages);
            Conversation {
                id: id.clone(),
                owner: owner.to_string(),
                title,
                created_at: now,
                updated_at: now,
                messages: Vec::new(),
            }
        });
        // Defensive: the entry could only be foreign-owned if the map was
        // mutated between the check above and now (single-threaded here —
        // impossible — but cheap to assert).
        if entry.owner != owner {
            bail!("conversation is owned by another account");
        }
        let mut stored: Vec<ChatMessage> = messages
            .iter()
            .map(|(role, content)| ChatMessage {
                role: role.clone(),
                content: content.clone(),
                ts: now,
            })
            .collect();
        stored.push(ChatMessage {
            role: "assistant".to_string(),
            content: assistant_reply.to_string(),
            ts: now,
        });
        if stored.len() > MAX_MESSAGES_PER_CONVERSATION {
            stored = stored[stored.len() - MAX_MESSAGES_PER_CONVERSATION..].to_vec();
        }
        entry.messages = stored;
        entry.updated_at = now;
        self.enforce_retention(owner, now);
        self.save()?;
        Ok(id)
    }

    /// Drops this owner's conversations beyond the count cap (oldest first)
    /// and older than the max age. The just-written conversation always has
    /// the newest `updated_at`, so it survives its own write.
    fn enforce_retention(&mut self, owner: &str, now: u64) {
        let mut owned: Vec<(String, u64)> = self
            .conversations
            .iter()
            .filter(|(_, c)| c.owner == owner)
            .map(|(id, c)| (id.clone(), c.updated_at))
            .collect();
        owned.sort_by_key(|(_, updated)| *updated);
        let expired: Vec<String> = owned
            .iter()
            .filter(|(_, updated)| now.saturating_sub(*updated) > MAX_CONVERSATION_AGE_SECS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.conversations.remove(id);
        }
        let mut owned: Vec<(String, u64)> = self
            .conversations
            .iter()
            .filter(|(_, c)| c.owner == owner)
            .map(|(id, c)| (id.clone(), c.updated_at))
            .collect();
        owned.sort_by_key(|(_, updated)| *updated);
        while owned.len() > MAX_CONVERSATIONS_PER_OWNER {
            if let Some((oldest, _)) = owned.first() {
                self.conversations.remove(oldest);
                owned.remove(0);
            } else {
                break;
            }
        }
    }

    /// This owner's conversations, newest first. Metadata only.
    pub fn list(&self, owner: &str) -> Vec<ConversationSummary> {
        let mut out: Vec<ConversationSummary> = self
            .conversations
            .values()
            .filter(|c| c.owner == owner)
            .map(ConversationSummary::from)
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        out
    }

    /// Full conversation, only when owned by the caller.
    pub fn get(&self, owner: &str, id: &str) -> Option<&Conversation> {
        self.conversations.get(id).filter(|c| c.owner == owner)
    }

    /// Deletes one conversation owned by the caller. Returns true when
    /// something was deleted. Unknown or foreign ids return false (no
    /// existence oracle across owners).
    pub fn delete(&mut self, owner: &str, id: &str) -> Result<bool> {
        let owned = self.conversations.get(id).is_some_and(|c| c.owner == owner);
        if !owned {
            return Ok(false);
        }
        self.conversations.remove(id);
        self.save()?;
        Ok(true)
    }

    /// Deletes ALL of the caller's conversations. Returns the count removed.
    pub fn delete_all(&mut self, owner: &str) -> Result<usize> {
        let ids: Vec<String> = self
            .conversations
            .iter()
            .filter(|(_, c)| c.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        let count = ids.len();
        for id in &ids {
            self.conversations.remove(id);
        }
        self.save()?;
        Ok(count)
    }

    #[cfg(test)]
    fn conversation_count(&self) -> usize {
        self.conversations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn open(dir: &Path) -> ChatStore {
        ChatStore::load(&dir.join("chat_history.json")).unwrap()
    }

    fn msgs() -> Vec<(String, String)> {
        vec![
            ("user".to_string(), "hello there".to_string()),
            ("assistant".to_string(), "hi!".to_string()),
            ("user".to_string(), "how are you?".to_string()),
        ]
    }

    #[test]
    fn record_creates_conversation_with_derived_title() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let id = store
            .record("acc:alice", None, &msgs(), "I'm well, thanks!")
            .unwrap();
        assert!(id.starts_with("conv-"));
        let conv = store.get("acc:alice", &id).unwrap();
        assert_eq!(conv.title, "hello there");
        assert_eq!(conv.messages.len(), 4);
        assert_eq!(conv.messages[3].role, "assistant");
        assert_eq!(conv.messages[3].content, "I'm well, thanks!");
        assert_eq!(conv.owner, "acc:alice");
    }

    #[test]
    fn record_replaces_messages_last_write_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let id = store
            .record("acc:alice", None, &msgs(), "reply one")
            .unwrap();
        let mut full = msgs();
        full.push(("assistant".to_string(), "reply one".to_string()));
        full.push(("user".to_string(), "and now?".to_string()));
        store
            .record("acc:alice", Some(&id), &full, "reply two")
            .unwrap();
        let conv = store.get("acc:alice", &id).unwrap();
        assert_eq!(conv.messages.len(), 6);
        assert_eq!(conv.messages[5].content, "reply two");
        // Title stays stable across turns.
        assert_eq!(conv.title, "hello there");
    }

    #[test]
    fn record_reuses_client_supplied_id_and_rejects_empty_reply() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let id = store
            .record("acc:alice", Some("my-chat_01"), &msgs(), "hi")
            .unwrap();
        assert_eq!(id, "my-chat_01");
        assert!(store.record("acc:alice", None, &msgs(), "").is_err());
        assert!(store.record("", None, &msgs(), "hi").is_err());
    }

    #[test]
    fn foreign_owned_id_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let id = store
            .record("acc:alice", Some("shared-id"), &msgs(), "alice reply")
            .unwrap();
        assert_eq!(id, "shared-id");
        // Bob guesses Alice's id: he gets a FRESH conversation, Alice's is
        // untouched.
        let bob_id = store
            .record("acc:bob", Some("shared-id"), &msgs(), "bob reply")
            .unwrap();
        assert_ne!(bob_id, "shared-id");
        assert_eq!(
            store.get("acc:alice", "shared-id").unwrap().messages.len(),
            4
        );
        assert_eq!(store.get("acc:bob", &bob_id).unwrap().owner, "acc:bob");
    }

    #[test]
    fn list_get_delete_are_owner_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let id = store.record("acc:alice", None, &msgs(), "hi").unwrap();
        assert!(store.get("acc:bob", &id).is_none());
        assert!(store.list("acc:bob").is_empty());
        assert_eq!(store.list("acc:alice").len(), 1);
        // Cross-owner delete is a silent no-op (no existence oracle).
        assert!(!store.delete("acc:bob", &id).unwrap());
        assert!(store.get("acc:alice", &id).is_some());
        assert!(store.delete("acc:alice", &id).unwrap());
        assert!(store.get("acc:alice", &id).is_none());
    }

    #[test]
    fn delete_all_only_removes_caller_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.record("acc:alice", None, &msgs(), "a1").unwrap();
        store.record("acc:alice", None, &msgs(), "a2").unwrap();
        store.record("acc:bob", None, &msgs(), "b1").unwrap();
        assert_eq!(store.delete_all("acc:alice").unwrap(), 2);
        assert!(store.list("acc:alice").is_empty());
        assert_eq!(store.list("acc:bob").len(), 1);
    }

    #[test]
    fn retention_caps_count_and_prunes_expired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat_history.json");
        let mut store = ChatStore::load(&path).unwrap();
        // Seed over the cap directly, then one record triggers enforcement.
        let now = now_secs();
        for i in 0..(MAX_CONVERSATIONS_PER_OWNER + 5) {
            store.conversations.insert(
                format!("conv-old-{i:03}"),
                Conversation {
                    id: format!("conv-old-{i:03}"),
                    owner: "acc:alice".to_string(),
                    title: "old".to_string(),
                    created_at: now - 1000,
                    updated_at: now - 1000 + i as u64,
                    messages: vec![],
                },
            );
        }
        // One expired conversation (older than max age).
        store.conversations.insert(
            "conv-expired".to_string(),
            Conversation {
                id: "conv-expired".to_string(),
                owner: "acc:alice".to_string(),
                title: "expired".to_string(),
                created_at: 1,
                updated_at: 1,
                messages: vec![],
            },
        );
        // Bob's conversations are untouched by Alice's retention.
        store.conversations.insert(
            "conv-bob".to_string(),
            Conversation {
                id: "conv-bob".to_string(),
                owner: "acc:bob".to_string(),
                title: "bob".to_string(),
                created_at: 1,
                updated_at: 1,
                messages: vec![],
            },
        );
        let fresh = store.record("acc:alice", None, &msgs(), "hi").unwrap();
        assert_eq!(store.list("acc:alice").len(), MAX_CONVERSATIONS_PER_OWNER);
        assert!(store.get("acc:alice", "conv-expired").is_none());
        assert!(store.get("acc:alice", &fresh).is_some());
        assert!(store.get("acc:bob", "conv-bob").is_some());
    }

    #[test]
    fn messages_cap_keeps_newest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let big: Vec<(String, String)> = (0..(MAX_MESSAGES_PER_CONVERSATION + 50))
            .map(|i| ("user".to_string(), format!("msg {i}")))
            .collect();
        let id = store.record("acc:alice", None, &big, "final").unwrap();
        let conv = store.get("acc:alice", &id).unwrap();
        assert_eq!(conv.messages.len(), MAX_MESSAGES_PER_CONVERSATION);
        assert_eq!(conv.messages.last().unwrap().content, "final");
    }

    #[test]
    fn store_survives_restart_and_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat_history.json");
        let id;
        {
            let mut store = ChatStore::load(&path).unwrap();
            id = store
                .record("acc:alice", None, &msgs(), "persist me")
                .unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "chat history must be owner-only");
        }
        let reloaded = ChatStore::load(&path).unwrap();
        let conv = reloaded.get("acc:alice", &id).unwrap();
        assert_eq!(conv.messages.len(), 4);
        assert_eq!(conv.messages[3].content, "persist me");
    }

    #[test]
    fn corrupt_store_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("chat_history.json"), b"not json").unwrap();
        let store = open(dir.path());
        assert_eq!(store.conversation_count(), 0);
    }

    #[test]
    fn conversation_id_validation() {
        assert!(is_valid_conversation_id("abc-123_XY"));
        assert!(is_valid_conversation_id("s9f3x2ab"));
        assert!(!is_valid_conversation_id(""));
        assert!(!is_valid_conversation_id("has space"));
        assert!(!is_valid_conversation_id("has/slash"));
        assert!(!is_valid_conversation_id("../escape"));
        assert!(!is_valid_conversation_id(&"x".repeat(65)));
        assert!(is_valid_conversation_id(&"x".repeat(64)));
    }

    #[test]
    fn extract_and_strip_conversation_id() {
        let mut body = serde_json::json!({
            "model": "m",
            "conversation_id": "my-chat_1",
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(extract_conversation_id(&body).as_deref(), Some("my-chat_1"));
        assert!(strip_conversation_id(&mut body));
        assert!(body.get("conversation_id").is_none());
        assert_eq!(body["model"], "m");
        // Invalid ids are ignored, never stripped into the store path.
        let bad = serde_json::json!({"conversation_id": "../x"});
        assert_eq!(extract_conversation_id(&bad), None);
    }

    #[test]
    fn user_visible_messages_filters_hidden_roles() {
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "server injected — never stored"},
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi"},
                {"role": "tool", "content": "tool output"},
                {"role": "developer", "content": "dev note"},
                {"role": "user", "content": ""},
                {"role": "user", "content": 42}
            ]
        });
        let visible = user_visible_messages(&body);
        assert_eq!(
            visible,
            vec![
                ("user".to_string(), "hello".to_string()),
                ("assistant".to_string(), "hi".to_string()),
            ]
        );
    }

    #[test]
    fn parse_assistant_json_shapes() {
        let ok = br#"{"choices":[{"message":{"role":"assistant","content":"hello!"}}]}"#;
        assert_eq!(parse_assistant_json(ok).as_deref(), Some("hello!"));
        let empty = br#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#;
        assert_eq!(parse_assistant_json(empty), None);
        let err = br#"{"error":{"message":"boom"}}"#;
        assert_eq!(parse_assistant_json(err), None);
        assert_eq!(parse_assistant_json(b"not json"), None);
    }

    #[test]
    fn parse_assistant_sse_shapes() {
        let transcript = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
            data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
            : keepalive\n\n\
            data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"completion_tokens\":2}}\n\n\
            data: [DONE]\n\n";
        assert_eq!(parse_assistant_sse(transcript).as_deref(), Some("Hello"));
        assert_eq!(parse_assistant_sse("data: [DONE]\n\n"), None);
        assert_eq!(parse_assistant_sse(""), None);
    }

    #[test]
    fn derive_title_truncates_and_flattens() {
        let m = vec![("user".to_string(), "  what   is\nup?  ".to_string())];
        assert_eq!(derive_title(&m), "what is up?");
        let long = vec![("user".to_string(), "x".repeat(200))];
        let title = derive_title(&long);
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS + 1);
        assert!(title.ends_with('…'));
        assert_eq!(derive_title(&[]), "New conversation");
    }

    /// The invariant-#5 proof: run every store operation under a capturing
    /// tracing subscriber and assert no user content (messages, replies,
    /// titles derived from content) appears in any emitted event.
    #[test]
    fn store_operations_emit_no_content_to_tracing() {
        use std::sync::{Arc, Mutex};
        use tracing::field::{Field, Visit};
        use tracing::span::{Attributes, Id, Record};
        use tracing::{Event, Metadata, Subscriber};

        struct Capture(Arc<Mutex<Vec<String>>>);
        struct StrVisitor(Vec<String>);
        impl Visit for StrVisitor {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.push(format!("{field}={value:?}"));
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.push(format!("{field}={value}"));
            }
        }
        impl Subscriber for Capture {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &Attributes<'_>) -> Id {
                Id::from_u64(1)
            }
            fn record(&self, _: &Id, _: &Record<'_>) {}
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn event(&self, event: &Event<'_>) {
                let mut v = StrVisitor(Vec::new());
                event.record(&mut v);
                let mut guard = self.0.lock().unwrap();
                guard.extend(v.0);
                guard.push(format!("{:?}", event.metadata().target()));
            }
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
        }

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sub = Capture(Arc::clone(&captured));
        tracing::dispatcher::with_default(&tracing::dispatcher::Dispatch::new(sub), || {
            let dir = tempfile::tempdir().unwrap();
            let secret_prompt = "SECRET-PROMPT-zebra-4821";
            let secret_reply = "SECRET-REPLY-mango-7734";
            let mut store = open(dir.path());
            let id = store
                .record(
                    "acc:alice",
                    None,
                    &[("user".to_string(), secret_prompt.to_string())],
                    secret_reply,
                )
                .unwrap();
            let _ = store.list("acc:alice");
            let _ = store.get("acc:alice", &id);
            let _ = store.delete("acc:alice", &id);
            let _ = store.delete_all("acc:alice");
            // Corrupt-file path also warns without content.
            std::fs::write(dir.path().join("chat_history.json"), b"nope").unwrap();
            let _ = ChatStore::load(&dir.path().join("chat_history.json")).unwrap();
        });
        let events = captured.lock().unwrap().join("\n");
        assert!(
            !events.contains("SECRET-PROMPT-zebra-4821"),
            "prompt content leaked to tracing:\n{events}"
        );
        assert!(
            !events.contains("SECRET-REPLY-mango-7734"),
            "reply content leaked to tracing:\n{events}"
        );
    }
}
