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
//! Network errors never touch the score.

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
            return Err(e);
        }
        write_chunk(&staging_path, manifest.chunk_size as u64, index, &data)?;
        done[index] = true;
        save_bitmap(&bitmap_path, &done)?;
        reputation.record_success(&peer);
        info!(chunk = index, total = chunk_count, "chunk verified and stored");
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
                        fetch_verified_chunk(
                            node,
                            &providers,
                            (i % width) + 1,
                            &manifest,
                            i,
                            reputation,
                        )
                        .await?
                    }
                },
                Err(_) => {
                    fetch_verified_chunk(
                        node,
                        &providers,
                        (i % width) + 1,
                        &manifest,
                        i,
                        reputation,
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

/// Fetches and verifies one chunk, trying providers in deterministic
/// order starting at `start` (round-robin position plus attempt offset).
/// Verification failures are recorded; network errors are not.
async fn fetch_verified_chunk(
    node: &P2PNode,
    providers: &[PeerId],
    start: usize,
    manifest: &Manifest,
    chunk_index: usize,
    reputation: &mut ReputationStore,
) -> Result<Vec<u8>> {
    let mut last_err = anyhow::anyhow!("no providers available");
    for attempt in 0..providers.len() {
        let provider = providers[(start + attempt) % providers.len()];
        match fetch_chunk(node, provider, manifest, chunk_index).await {
            Ok(data) => match verify_chunk(manifest, chunk_index, &data) {
                Ok(()) => {
                    reputation.record_success(&provider);
                    return Ok(data);
                }
                Err(e) => {
                    reputation.record_failure(&provider);
                    last_err = e;
                }
            },
            Err(e) => {
                last_err = e;
            }
        }
    }
    Err(last_err)
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
        bail!("peer answered with protocol version {}", response.protocol_version);
    }
    if response.manifest.model_id != manifest_id {
        bail!("peer served manifest {} for requested {}", response.manifest.model_id, manifest_id);
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
    Ok(chunk_count)
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
    let response: ChunkResponse =
        deserialize_message(&raw, DEFAULT_MAX_CHUNK_MESSAGE_BYTES)?;
    if response.protocol_version != CURRENT_PROTOCOL_VERSION {
        bail!("peer answered with protocol version {}", response.protocol_version);
    }
    if response.chunk_index as usize != index {
        bail!("peer answered chunk {} for requested {}", response.chunk_index, index);
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
fn prepare_staging(
    data_dir: &Path,
    manifest: &Manifest,
) -> Result<(PathBuf, PathBuf, Vec<bool>)> {
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
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(staging_path)?;
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
        bail!("assembled file hash mismatch: expected {}, got {}", manifest.model_id, file_hash);
    }
    let root = merkle_root(&manifest.chunk_hashes);
    if root != manifest.merkle_root {
        bail!("merkle root mismatch: expected {}, got {}", manifest.merkle_root, root);
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
        Ok(bytes) if bytes.len() == chunk_count => {
            Ok(bytes.iter().map(|b| *b == 1u8).collect())
        }
        _ => Ok(vec![false; chunk_count]),
    }
}

fn save_bitmap(path: &Path, done: &[bool]) -> Result<()> {
    let bytes: Vec<u8> = done.iter().map(|d| u8::from(*d)).collect();
    std::fs::write(path, bytes)?;
    Ok(())
}
