//! Verified model download: fetch the manifest, download every chunk with
//! per-chunk BLAKE3 verification, persist progress for resume, and finish
//! with a full-file Merkle-root check and atomic rename.
//!
//! Every chunk outcome is recorded in the reputation store: verified
//! chunks raise the peer's score, corrupted ones count toward a temporary
//! ban. Banned peers are refused before any network traffic.

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

use crate::reputation::ReputationStore;
use crate::{DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode};

/// Downloads the artifact described by `manifest_id` from `peer`.
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

    let staging_dir = data_dir.join("staging");
    std::fs::create_dir_all(&staging_dir)?;
    let staging_path = staging_dir.join(format!("{}.part", manifest.model_id));
    let bitmap_path = staging_dir.join(format!("{}.done", manifest.model_id));

    // Preallocate the staging file so chunk writes are pure seeks.
    {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&staging_path)?;
        file.set_len(manifest.file_size)?;
    }
    let mut done = load_bitmap(&bitmap_path, chunk_count)?;

    for index in 0..chunk_count {
        if done[index] {
            continue;
        }
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
        let expected = &manifest.chunk_hashes[index];
        let actual = blake3::hash(&response.chunk_data).to_hex().to_string();
        if &actual != expected {
            reputation.record_failure(&peer);
            if let Err(e) = reputation.save() {
                warn!(error = %e, "failed to persist reputation after invalid chunk");
            }
            bail!(
                "chunk {} failed verification: expected {}, got {}",
                index,
                expected,
                actual
            );
        }

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&staging_path)?;
        file.seek(SeekFrom::Start(index as u64 * manifest.chunk_size as u64))?;
        file.write_all(&response.chunk_data)?;
        file.sync_data()?;

        done[index] = true;
        save_bitmap(&bitmap_path, &done)?;
        reputation.record_success(&peer);
        info!(chunk = index, total = chunk_count, "chunk verified and stored");
    }

    // Final integrity gate: full-file streaming hash and Merkle root.
    let mut full = blake3::Hasher::new();
    let mut file = std::fs::File::open(&staging_path)?;
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
    std::fs::rename(&staging_path, &final_path)?;
    let _ = std::fs::remove_file(&bitmap_path);
    if let Err(e) = reputation.save() {
        warn!(error = %e, "failed to persist reputation after download");
    }
    info!(path = %final_path.display(), "download complete and verified");
    Ok(final_path)
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
