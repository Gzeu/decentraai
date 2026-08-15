use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(String);

impl PeerId {
    pub fn from_public_key(public_key: &VerifyingKey) -> Self {
        let hash = blake3::hash(public_key.as_bytes());
        Self(hex::encode(hash.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct Identity {
    signing_key: SigningKey,
    public_key: VerifyingKey,
    peer_id: PeerId,
}

impl Identity {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        let public_key = signing_key.verifying_key();
        let peer_id = PeerId::from_public_key(&public_key);
        Self {
            signing_key,
            public_key,
            peer_id,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("reading identity from {}", path.display()))?;

        if bytes.len() != 32 {
            anyhow::bail!(
                "invalid identity file: expected 32 bytes, got {}",
                bytes.len()
            );
        }

        let signing_key = SigningKey::from_bytes(&bytes.try_into().unwrap());
        let public_key = signing_key.verifying_key();
        let peer_id = PeerId::from_public_key(&public_key);
        Ok(Self {
            signing_key,
            public_key,
            peer_id,
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        // Atomic, permission-safe write: the key must never exist with default
        // (world-readable) permissions, even for an instant, and a partially
        // written key must never be readable. tmp + fsync + rename matches the
        // persistence pattern used elsewhere in the repo (AGENTS.md §4).
        let bytes = self.signing_key.to_bytes();
        Self::atomic_write_0600(path, &bytes)
            .with_context(|| format!("writing identity to {}", path.display()))?;
        Ok(())
    }

    /// Writes `bytes` to `path` atomically (tmp + fsync + rename) with mode
    /// 0600 on Unix. The secret key file is created permission-safe from the
    /// first byte, so no other user can read it during or after the write.
    fn atomic_write_0600(path: &Path, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let tmp = path.with_extension("tmp");
        {
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            opts.mode(0o600);
            let mut f = opts
                .open(&tmp)
                .with_context(|| format!("opening temp file {}", tmp.display()))?;
            f.write_all(bytes)
                .with_context(|| format!("writing temp file {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("syncing temp file {}", tmp.display()))?;
        }
        fs::rename(&tmp, path).with_context(|| {
            format!(
                "renaming {} to {}",
                tmp.display(),
                path.display()
            )
        })?;
        // Durability: fsync the parent directory so the rename is persistent.
        if let Some(parent) = path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        // Defense in depth: force 0600 even if the mode flag was not honored.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path)
                .with_context(|| format!("getting metadata for {}", path.display()))?
                .permissions();
            perms.set_mode(0o600);
            fs::set_permissions(path, perms)
                .with_context(|| format!("setting permissions for {}", path.display()))?;
        }
        Ok(())
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn public_key(&self) -> &VerifyingKey {
        &self.public_key
    }

    /// Returns the raw 32-byte Ed25519 secret key.
    ///
    /// Sensitive: callers must not log, persist unprotected, or transmit these
    /// bytes. Used to derive the libp2p transport keypair so the network
    /// PeerId is bound to this node identity.
    pub fn signing_key_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}

pub fn verify_signature(
    public_key: &VerifyingKey,
    message: &[u8],
    signature: &Signature,
) -> Result<()> {
    public_key
        .verify(message, signature)
        .context("signature verification failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_identity() {
        let identity = Identity::generate();
        assert_eq!(identity.peer_id().as_str().len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let identity = Identity::generate();
        let message = b"test message";
        let signature = identity.sign(message);

        let public_key = identity.public_key();
        assert!(verify_signature(public_key, message, &signature).is_ok());
    }

    #[test]
    fn test_wrong_key_rejection() {
        let identity1 = Identity::generate();
        let identity2 = Identity::generate();

        let message = b"test message";
        let signature = identity1.sign(message);

        let public_key2 = identity2.public_key();
        assert!(verify_signature(public_key2, message, &signature).is_err());
    }

    #[test]
    fn test_persistence_across_reloads() {
        let temp_dir = TempDir::new().unwrap();
        let identity_path = temp_dir.path().join("identity.pem");

        let identity1 = Identity::generate();
        let peer_id1 = identity1.peer_id().clone();
        identity1.save(&identity_path).unwrap();

        let identity2 = Identity::load(&identity_path).unwrap();
        assert_eq!(identity2.peer_id(), &peer_id1);

        // Verify the reloaded identity can sign correctly
        let message = b"test message";
        let signature1 = identity1.sign(message);
        let signature2 = identity2.sign(message);
        assert_eq!(signature1, signature2);
    }

    #[test]
    fn test_peer_id_from_public_key() {
        let identity = Identity::generate();
        let public_key = identity.public_key();
        let peer_id1 = PeerId::from_public_key(public_key);
        let peer_id2 = PeerId::from_public_key(public_key);

        assert_eq!(peer_id1, peer_id2);
        assert_eq!(peer_id1.as_str().len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn test_signing_key_bytes_roundtrip() {
        let identity = Identity::generate();
        let bytes = identity.signing_key_bytes();
        let restored = SigningKey::from_bytes(&bytes);
        assert_eq!(restored.verifying_key(), *identity.public_key());
    }

    #[test]
    fn test_invalid_identity_file_length() {
        let temp_dir = TempDir::new().unwrap();
        let identity_path = temp_dir.path().join("invalid.pem");

        // Write invalid length (not 32 bytes)
        fs::write(&identity_path, b"invalid").unwrap();

        assert!(Identity::load(&identity_path).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_unix_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let identity_path = temp_dir.path().join("identity.pem");

        let identity = Identity::generate();
        identity.save(&identity_path).unwrap();

        let perms = fs::metadata(&identity_path).unwrap().permissions();
        let mode = perms.mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    #[cfg(unix)]
    fn save_is_atomic_and_leaves_no_tmp() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let identity_path = temp_dir.path().join("key.pem");

        let identity = Identity::generate();
        identity.save(&identity_path).unwrap();

        // Atomic write: no partially-written temp file is left behind.
        assert!(
            !identity_path.with_extension("tmp").exists(),
            "temp file must be renamed away after save"
        );
        // The real key is 0600 and reloadable.
        let perms = fs::metadata(&identity_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
        assert!(Identity::load(&identity_path).is_ok());
    }
}
