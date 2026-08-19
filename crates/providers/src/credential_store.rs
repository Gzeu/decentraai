//! Secure, local credential store for provider API keys and secrets.
//!
//! Secrets are stored under unique `key_id`s and NEVER serialized in model
//! metadata, P2P advertisements, agent records, or audit events.
//!
//! Only the adapter boundary may read a secret by key_id — everywhere else
//! the operator/admin UI sees only a masked fingerprint (`••••a91f`).

use sha2::{Digest, Sha256};

/// A secure, local credential store for provider API keys and secrets.
#[derive(Debug, Clone, Default)]
pub struct CredentialStore {
    credentials: std::collections::HashMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
struct StoredCredential {
    /// The plaintext secret (API key, bearer token…). Kept in-memory while the
    /// node runs; never written to JSON or sent anywhere.
    secret: String,
    #[expect(dead_code)]
    created_at_ms: u64,
    last_used_at_ms: Option<u64>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new credential. Returns the generated `key_id` that callers
    /// use instead of storing/referencing the raw secret.
    ///
    /// Format: `dcrypt_{hex64}` where hex64 is a SHA-256 derived ID.
    pub fn add(&mut self, secret: impl Into<String>) -> String {
        let key_id = generate_key_id();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.credentials.insert(
            key_id.clone(),
            StoredCredential {
                secret: secret.into(),
                created_at_ms: now,
                last_used_at_ms: None,
            },
        );
        key_id
    }

    /// Look up the raw secret by key_id. This is the ONLY place the secret
    /// can be retrieved — the adapter passes the key_id and this method
    /// returns the plaintext. Returns `None` if not found.
    pub fn get_secret(&mut self, key_id: &str) -> Option<&str> {
        let entry = self.credentials.get_mut(key_id)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        entry.last_used_at_ms = Some(now);
        Some(&entry.secret)
    }

    /// Masked fingerprint shown to operators (last 4 chars of the key_id).
    pub fn fingerprint(&self, key_id: &str) -> String {
        match key_id.strip_prefix("dcrypt_") {
            Some(hex_part) => {
                let tail: String = hex_part.chars().rev().take(4).collect();
                format!("••••{}", tail)
            }
            None => "••••??".into(),
        }
    }

    pub fn has_key(&self, key_id: &str) -> bool {
        self.credentials.contains_key(key_id)
    }

    pub fn list_keys(&self) -> Vec<String> {
        self.credentials.keys().cloned().collect()
    }

    pub fn remove(&mut self, key_id: &str) -> bool {
        self.credentials.remove(key_id).is_some()
    }

    pub fn len(&self) -> usize {
        self.credentials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }
}

/// Generate a stable `dcrypt_` prefixed key id.
fn generate_key_id() -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dcrypt-seed-v1\0");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(ts.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("dcrypt_{}", hex)
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[test]
    fn credential_store_add_and_lookup() {
        let mut store = CredentialStore::new();
        let key_id = store.add("sk-test-key-abc123");
        assert!(key_id.starts_with("dcrypt_"));
        assert_eq!(store.get_secret(&key_id), Some("sk-test-key-abc123"));
    }

    #[test]
    fn credential_store_masked_fingerprint() {
        let store = CredentialStore::new();
        let fp = store.fingerprint("dcrypt_abcdef1234567890");
        // Must show exactly 8 Unicode scalar values (•••• + 4 hex).
        assert_eq!(fp.chars().count(), 8);
        assert!(fp.starts_with("••••"));
        assert!(!fp.contains("abcdef"));
    }

    #[test]
    fn credential_store_invalid_key_returns_none() {
        let mut store = CredentialStore::new();
        store.add("some-secret");
        assert!(store.get_secret("nonexistent").is_none());
    }

    #[test]
    fn credential_store_list_keys_no_secrets() {
        let mut store = CredentialStore::new();
        let k1 = store.add("secret1");
        let k2 = store.add("secret2");
        let keys = store.list_keys();
        assert_eq!(keys.len(), 2);
        let json = serde_json::to_string(&keys).unwrap();
        assert!(!json.contains("secret1"));
        assert!(!json.contains("secret2"));
    }

    #[test]
    fn credential_store_remove() {
        let mut store = CredentialStore::new();
        let k = store.add("remove-me");
        assert!(store.has_key(&k));
        assert!(store.remove(&k));
        assert!(!store.has_key(&k));
        assert!(store.get_secret(&k).is_none());
        assert!(!store.remove(&k));
    }
}
