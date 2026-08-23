//! Verified model downloads: fetch the manifest, download every chunk
//! with per-chunk BLAKE3 verification, persist progress for resume, and
//! finish with a full-file Merkle-root check and atomic rename.
//!
//! Two entry points:
//! - [`download`] — single provider, the M3/M5a behavior
//! - [`download_multi`] — M5c: parallel waves across score-ranked
//!   providers with deterministic assignment and fallback
//!
//! Every chunk outcome is recorded in the reputation store: verified
//! chunks raise the peer's score, corrupted ones count toward a ban.
//! Network errors never touch the score. Corrupted staging artifacts
//! are quarantined with metadata (M6), and security events are written
//! to the audit log.

use anyhow::{Result, bail};
use decentraai_manifest::{CHUNK_SIZE, Manifest, merkle_root};
use decentraai_protocol::{
    CURRENT_PROTOCOL_VERSION, ChunkRequest, ChunkResponse, ManifestRequest, ManifestResponse,
    deserialize_message, serialize_message,
};
use libp2p::PeerId;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::reputation::{ReputationStore, rank_providers};
use crate::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode};

/// Downloads the artifact described by `manifest_id` from a single `peer`.
///
/// Layout under `data_dir`:
/// - `staging/<manifest_id>.part` — assembled bytes so far
/// - `staging/<manifest_id>.done` — one `0`/`1` byte per chunk (resume bitmap)
/// - final artifact at `models/<file_name>` after Merkle verification
pub async fn download(
    node: &P2PNode,
    peer: PeerId,
    manifest_id: &str,
    data_dir: &Path,
    reputation: &mut ReputationStore,
) -> Result<PathBuf> {
    if let Some(until) = reputation.banned_until(&peer) {
        bail!("peer {peer} is banned until unix time {until} (too many invalid chunks)");
    }

    let manifest = fetch_manifest(node, peer, manifest_id).await?;
    let chunk_count = validate_manifest(&manifest)?;
    let (staging_path, bitmap_path, mut done) = prepare_staging(data_dir, &manifest)?;

    for index in 0..chunk_count {
        if done[index] {
            continue;
        }
        let data = fetch_chunk(node, peer, &manifest, index).await?;
        if let Err(e) = verify_chunk(&manifest, index, &data) {
            reputation.record_failure(&peer);
            if let Err(e) = reputation.save() {
                warn!(error = %e, "failed to persist reputation after invalid chunk");
            }
            audit_security_event(
                data_dir,
                "chunk_verification_failed",
                &peer,
                &manifest.model_id,
                index,
            );
            if reputation.is_banned(&peer) {
                audit_security_event(data_dir, "peer_banned", &peer, &manifest.model_id, index);
            }
            quarantine_staging(data_dir, &manifest, &peer, &e.to_string());
            return Err(e);
        }
        write_chunk(&staging_path, manifest.chunk_size as u64, index, &data)?;
        done[index] = true;
        save_bitmap(&bitmap_path, &done)?;
        reputation.record_success(&peer);
        info!(
            chunk = index,
            total = chunk_count,
            "chunk verified and stored"
        );
    }

    let final_path = finalize_download(&staging_path, &bitmap_path, data_dir, &manifest)?;
    if let Err(e) = reputation.save() {
        warn!(error = %e, "failed to persist reputation after download");
    }
    Ok(final_path)
}

/// Downloads from multiple providers in parallel waves (M5c).
///
/// Providers are ranked deterministically (score desc, PeerId asc);
/// chunk `i` is assigned to provider `i % N`. Each wave fetches up to N
/// chunks concurrently; verification and reputation updates happen
/// sequentially afterwards, keeping the store free of data races. A
/// failed chunk falls back to the next ranked provider in deterministic
/// order.
pub async fn download_multi(
    node: &P2PNode,
    peers: &[PeerId],
    manifest_id: &str,
    data_dir: &Path,
    reputation: &mut ReputationStore,
) -> Result<PathBuf> {
    let providers = rank_providers(peers, reputation);
    if providers.is_empty() {
        bail!("no eligible providers: every peer is banned or none were given");
    }

    let manifest = fetch_manifest(node, providers[0], manifest_id).await?;
    let chunk_count = validate_manifest(&manifest)?;
    let (staging_path, bitmap_path, mut done) = prepare_staging(data_dir, &manifest)?;
    let width = providers.len();

    let mut cursor = 0;
    while cursor < chunk_count {
        while cursor < chunk_count && done[cursor] {
            cursor += 1;
        }
        if cursor >= chunk_count {
            break;
        }
        let wave: Vec<usize> = (cursor..chunk_count)
            .filter(|i| !done[*i])
            .take(width)
            .collect();

        // Concurrent fetch without verification; reputation stays sequential.
        let results = futures::future::join_all(
            wave.iter()
                .map(|&i| fetch_chunk(node, providers[i % width], &manifest, i)),
        )
        .await;

        for (&i, result) in wave.iter().zip(results) {
            let assigned = providers[i % width];
            let data = match result {
                Ok(data) => match verify_chunk(&manifest, i, &data) {
                    Ok(()) => {
                        reputation.record_success(&assigned);
                        data
                    }
                    Err(_) => {
                        reputation.record_failure(&assigned);
                        fetch_verified_or_quarantine(
                            node,
                            &providers,
                            (i % width) + 1,
                            &manifest,
                            i,
                            reputation,
                            data_dir,
                        )
                        .await?
                    }
                },
                Err(_) => {
                    fetch_verified_or_quarantine(
                        node,
                        &providers,
                        (i % width) + 1,
                        &manifest,
                        i,
                        reputation,
                        data_dir,
                    )
                    .await?
                }
            };
            write_chunk(&staging_path, manifest.chunk_size as u64, i, &data)?;
            done[i] = true;
            save_bitmap(&bitmap_path, &done)?;
            info!(chunk = i, total = chunk_count, "chunk verified and stored");
        }
        cursor = wave.last().map(|last| last + 1).unwrap_or(chunk_count);
    }

    let final_path = finalize_download(&staging_path, &bitmap_path, data_dir, &manifest)?;
    if let Err(e) = reputation.save() {
        warn!(error = %e, "failed to persist reputation after download");
    }
    Ok(final_path)
}

/// Fetches and verifies one chunk via deterministic fallback, auditing
/// every verification failure. When every provider fails, the staging
/// artifact is quarantined before the error propagates.
async fn fetch_verified_or_quarantine(
    node: &P2PNode,
    providers: &[PeerId],
    start: usize,
    manifest: &Manifest,
    chunk_index: usize,
    reputation: &mut ReputationStore,
    data_dir: &Path,
) -> Result<Vec<u8>> {
    let mut last_err = anyhow::anyhow!("no providers available");
    let mut last_provider = providers[start % providers.len()];
    // A chunk is only quarantined on a CRYPTOGRAPHIC verification failure.
    // Pure network errors are transport failures (provider unreachable) —
    // quarantining on those contradicts the module docs and permanently
    // poisons the artifact for a retryable outage.
    let mut saw_crypto_failure = false;
    for attempt in 0..providers.len() {
        let provider = providers[(start + attempt) % providers.len()];
        last_provider = provider;
        match fetch_chunk(node, provider, manifest, chunk_index).await {
            Ok(data) => match verify_chunk(manifest, chunk_index, &data) {
                Ok(()) => {
                    reputation.record_success(&provider);
                    return Ok(data);
                }
                Err(e) => {
                    saw_crypto_failure = true;
                    reputation.record_failure(&provider);
                    audit_security_event(
                        data_dir,
                        "chunk_verification_failed",
                        &provider,
                        &manifest.model_id,
                        chunk_index,
                    );
                    if reputation.is_banned(&provider) {
                        audit_security_event(
                            data_dir,
                            "peer_banned",
                            &provider,
                            &manifest.model_id,
                            chunk_index,
                        );
                    }
                    last_err = e;
                }
            },
            Err(e) => {
                last_err = e;
            }
        }
    }
    if saw_crypto_failure {
        quarantine_staging(data_dir, manifest, &last_provider, &last_err.to_string());
    }
    Err(last_err)
}

/// Moves the staging artifact into `quarantine/` and records why.
///
/// The FULL staging state is moved — both the `.part` bytes and the `.done`
/// resume bitmap — so a retry starts from a clean slate. Leaving the bitmap
/// behind was a permanent-corruption bug: a retried download would honor the
/// stale "verified" marks, skip chunks, and fail the final hash forever until
/// the bitmap was deleted manually.
///
/// Best-effort: a quarantine failure is logged, never fatal.
fn quarantine_staging(data_dir: &Path, manifest: &Manifest, peer: &PeerId, reason: &str) {
    let result = (|| -> Result<()> {
        let quarantine_dir = data_dir.join("quarantine");
        std::fs::create_dir_all(&quarantine_dir)?;
        let staging = data_dir
            .join("staging")
            .join(format!("{}.part", manifest.model_id));
        if staging.exists() {
            std::fs::rename(
                &staging,
                quarantine_dir.join(format!("{}.part", manifest.model_id)),
            )?;
        }
        // Move the resume bitmap too: a quarantined artifact must not leave
        // stale "verified" marks behind for a future retry.
        let bitmap = data_dir
            .join("staging")
            .join(format!("{}.done", manifest.model_id));
        if bitmap.exists() {
            std::fs::rename(
                &bitmap,
                quarantine_dir.join(format!("{}.done", manifest.model_id)),
            )?;
        }
        let metadata = serde_json::json!({
            "manifest_id": manifest.model_id,
            "file_name": manifest.file_name,
            "peer": peer.to_string(),
            "reason": reason,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
        std::fs::write(
            quarantine_dir.join(format!("{}.quarantine.json", manifest.model_id)),
            serde_json::to_string_pretty(&metadata)?,
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        warn!(error = %e, "failed to quarantine corrupted artifact");
    }
}

fn audit_security_event(
    data_dir: &Path,
    event: &str,
    peer: &PeerId,
    manifest_id: &str,
    chunk_index: usize,
) {
    decentraai_audit::record_best_effort(
        &data_dir.join("logs"),
        event,
        serde_json::json!({
            "peer": peer.to_string(),
            "manifest_id": manifest_id,
            "chunk": chunk_index,
        }),
    );
}

async fn fetch_manifest(node: &P2PNode, peer: PeerId, manifest_id: &str) -> Result<Manifest> {
    let request = ManifestRequest {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        manifest_id: manifest_id.to_string(),
        signature: None,
    };
    let payload = serialize_message(&request)?;
    let raw = node.request(peer, payload).await?;
    let response: ManifestResponse = deserialize_message(&raw, DEFAULT_MAX_MESSAGE_BYTES)?;
    if response.protocol_version != CURRENT_PROTOCOL_VERSION {
        bail!(
            "peer answered with protocol version {}",
            response.protocol_version
        );
    }
    if response.manifest.model_id != manifest_id {
        bail!(
            "peer served manifest {} for requested {}",
            response.manifest.model_id,
            manifest_id
        );
    }
    Ok(response.manifest)
}

fn validate_manifest(manifest: &Manifest) -> Result<usize> {
    if manifest.chunk_size != CHUNK_SIZE {
        bail!(
            "unsupported chunk size {} (expected {})",
            manifest.chunk_size,
            CHUNK_SIZE
        );
    }
    let chunk_count = manifest.chunk_hashes.len();
    if chunk_count == 0 {
        bail!("manifest has no chunks");
    }
    // `model_id` and `file_name` arrive from a remote peer and are
    // interpolated directly into staging/quarantine/models paths. A hostile
    // peer must not be able to steer those writes outside their directories
    // (path traversal / absolute-path write).
    validate_artifact_component(&manifest.model_id)?;
    validate_artifact_component(&manifest.file_name)?;
    Ok(chunk_count)
}

/// Rejects any name that cannot be used safely as a single path component.
///
/// A remote peer controls `model_id`/`file_name`, which are later joined into
/// `staging/`, `quarantine/` and `models/` paths. We accept only a plain file
/// name: no path separators, no `.`/`..`, no absolute/rooted path, no NUL
/// byte, and no empty name. Using `Path::components()` keeps the check
/// platform-aware (it also splits on Windows separators).
fn validate_artifact_component(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("artifact name is empty");
    }
    if name.contains('\0') {
        bail!("artifact name contains a NUL byte");
    }
    // Backslash is a separator on Windows; a name validated as safe on Linux
    // could be re-processed on Windows. Model filenames never legitimately
    // contain it, so reject it explicitly as a cross-platform guard.
    if name.contains('\\') {
        bail!("artifact name must be a plain file name, got {name:?}");
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(part)), None) => {
            if part == ".." || part == "." {
                bail!("artifact name must be a plain file name, got {name:?}");
            }
            Ok(())
        }
        _ => bail!("artifact name must be a plain file name, got {name:?}"),
    }
}

/// Fetches one chunk from a peer without verifying it; verification is
/// the caller's job (it owns the reputation consequences).
async fn fetch_chunk(
    node: &P2PNode,
    peer: PeerId,
    manifest: &Manifest,
    index: usize,
) -> Result<Vec<u8>> {
    let request = ChunkRequest {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        manifest_id: manifest.model_id.clone(),
        chunk_index: index as u32,
    };
    let payload = serialize_message(&request)?;
    let raw = node.request(peer, payload).await?;
    let response: ChunkResponse = deserialize_message(&raw, DEFAULT_MAX_CHUNK_MESSAGE_BYTES)?;
    if response.protocol_version != CURRENT_PROTOCOL_VERSION {
        bail!(
            "peer answered with protocol version {}",
            response.protocol_version
        );
    }
    if response.chunk_index as usize != index {
        bail!(
            "peer answered chunk {} for requested {}",
            response.chunk_index,
            index
        );
    }
    Ok(response.chunk_data)
}

/// BLAKE3-verifies a fetched chunk against the manifest.
fn verify_chunk(manifest: &Manifest, index: usize, data: &[u8]) -> Result<()> {
    let expected = &manifest.chunk_hashes[index];
    let actual = blake3::hash(data).to_hex().to_string();
    if &actual != expected {
        bail!(
            "chunk {} failed verification: expected {}, got {}",
            index,
            expected,
            actual
        );
    }
    Ok(())
}

/// Preallocates the staging file and loads the resume bitmap.
fn prepare_staging(data_dir: &Path, manifest: &Manifest) -> Result<(PathBuf, PathBuf, Vec<bool>)> {
    let staging_dir = data_dir.join("staging");
    std::fs::create_dir_all(&staging_dir)?;
    let staging_path = staging_dir.join(format!("{}.part", manifest.model_id));
    let bitmap_path = staging_dir.join(format!("{}.done", manifest.model_id));
    {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&staging_path)?;
        file.set_len(manifest.file_size)?;
    }
    let done = load_bitmap(&bitmap_path, manifest.chunk_hashes.len())?;
    Ok((staging_path, bitmap_path, done))
}

fn write_chunk(staging_path: &Path, chunk_size: u64, index: usize, data: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).open(staging_path)?;
    file.seek(SeekFrom::Start(index as u64 * chunk_size))?;
    file.write_all(data)?;
    file.sync_data()?;
    Ok(())
}

/// Final integrity gate: full-file streaming hash, Merkle root, atomic
/// rename into `models/`, bitmap cleanup.
fn finalize_download(
    staging_path: &Path,
    bitmap_path: &Path,
    data_dir: &Path,
    manifest: &Manifest,
) -> Result<PathBuf> {
    let mut full = blake3::Hasher::new();
    let mut file = std::fs::File::open(staging_path)?;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        full.update(&buffer[..n]);
    }
    let file_hash = full.finalize().to_hex().to_string();
    if file_hash != manifest.model_id {
        bail!(
            "assembled file hash mismatch: expected {}, got {}",
            manifest.model_id,
            file_hash
        );
    }
    let root = merkle_root(&manifest.chunk_hashes);
    if root != manifest.merkle_root {
        bail!(
            "merkle root mismatch: expected {}, got {}",
            manifest.merkle_root,
            root
        );
    }

    let models_dir = data_dir.join("models");
    std::fs::create_dir_all(&models_dir)?;
    let final_path = models_dir.join(&manifest.file_name);
    std::fs::rename(staging_path, &final_path)?;
    let _ = std::fs::remove_file(bitmap_path);
    info!(path = %final_path.display(), "download complete and verified");
    Ok(final_path)
}

fn load_bitmap(path: &Path, chunk_count: usize) -> Result<Vec<bool>> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() == chunk_count => Ok(bytes.iter().map(|b| *b == 1u8).collect()),
        _ => Ok(vec![false; chunk_count]),
    }
}

fn save_bitmap(path: &Path, done: &[bool]) -> Result<()> {
    let bytes: Vec<u8> = done.iter().map(|d| u8::from(*d)).collect();
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(model_id: &str, file_name: &str) -> Manifest {
        Manifest {
            version: 1,
            model_id: model_id.to_string(),
            file_name: file_name.to_string(),
            file_size: 0,
            chunk_size: CHUNK_SIZE,
            chunk_hashes: vec!["ab".to_string()],
            merkle_root: "cd".to_string(),
        }
    }

    /// A peer-controlled model_id/file_name must never escape the
    /// staging/quarantine/models directories, even under platform-specific
    /// path handling. This is the security invariant for `validate_manifest`.
    #[test]
    fn rejects_path_traversal_in_artifact_names() {
        let malicious: &[(&str, &str)] = &[
            ("../escape", "safe.bin"),
            ("..", "safe.bin"),
            ("/etc/passwd", "safe.bin"),
            ("safe", "../escape.gguf"),
            ("safe", "/tmp/evil"),
            ("a/b", "safe"),
            ("safe", "a\\b"),
            ("safe", ""),
        ];
        for &(model_id, file_name) in malicious {
            let e = validate_manifest(&manifest_with(model_id, file_name))
                .expect_err(&format!("{model_id:?}/{file_name:?} must be rejected"));
            assert!(!e.to_string().is_empty(), "error should carry a message");
        }
    }

    #[test]
    fn accepts_plain_artifact_names() {
        // Real artifacts use the BLAKE3 file hash as model_id and the source
        // basename as file_name — both are plain components.
        let ok = manifest_with(
            "9c4a5f3f9f1d9b7f5a8e2d0b1f6c7d8e9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d",
            "model-7b-q4_k_m.gguf",
        );
        assert!(validate_manifest(&ok).is_ok());
    }

    #[test]
    fn quarantine_moves_bitmap_so_retry_starts_fresh() {
        // Regression (review, data plane): quarantine_staging used to move
        // only the `.part` and leave the `.done` resume bitmap behind. A retry
        // would then honor the stale "verified" marks, skip chunks, and fail
        // the final hash forever. The bitmap must move with the artifact.
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let m = manifest_with("quar-test", "m.gguf");
        let part = staging.join(format!("{}.part", m.model_id));
        let bitmap = staging.join(format!("{}.done", m.model_id));
        std::fs::write(&part, b"partial bytes").unwrap();
        // Bitmap marks chunk 0 as verified (stale after quarantine).
        std::fs::write(&bitmap, [1u8]).unwrap();

        quarantine_staging(
            dir.path(),
            &m,
            &PeerId::random(),
            "chunk verification failed",
        );

        // Both files moved out of staging.
        assert!(!part.exists(), "staging .part must be moved away");
        assert!(!bitmap.exists(), "staging .done must be moved away");
        let quarantine = dir.path().join("quarantine");
        assert!(quarantine.join(format!("{}.part", m.model_id)).exists());
        assert!(
            quarantine.join(format!("{}.done", m.model_id)).exists(),
            "the resume bitmap must be quarantined with the artifact"
        );
        // Metadata recorded.
        assert!(
            quarantine
                .join(format!("{}.quarantine.json", m.model_id))
                .exists()
        );

        // A retry prepares a fresh staging state: empty bitmap (no stale marks).
        let (_part, _bitmap, done) = prepare_staging(dir.path(), &m).unwrap();
        assert!(
            done.iter().all(|d| !*d),
            "a quarantined artifact must retry from a clean bitmap"
        );
    }

    #[test]
    fn quarantine_moves_only_existing_files() {
        // A network-only failure path never calls quarantine_staging, but the
        // function must be safe when only a bitmap exists (no .part yet).
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let m = manifest_with("quar-only-bitmap", "m.gguf");
        std::fs::write(staging.join(format!("{}.done", m.model_id)), [0u8, 1u8]).unwrap();
        quarantine_staging(dir.path(), &m, &PeerId::random(), "test");
        let quarantine = dir.path().join("quarantine");
        assert!(
            quarantine.join(format!("{}.done", m.model_id)).exists(),
            "bitmap-only quarantine must still move the bitmap"
        );
    }
}
