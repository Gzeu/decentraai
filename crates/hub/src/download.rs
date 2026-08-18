//! Verified model download from the HuggingFace Hub.
//!
//! The digest to verify against is **pinned before the download starts** (the
//! caller resolves the file's SHA-256 from the Hub tree API). The file streams
//! to a `.part` staging path and is renamed into place only after the digest
//! matches — an interrupted or corrupted download never becomes a registry
//! artifact.
//!
//! Two digest modes:
//! - `ExpectedSha256(Some(hex))` — the Hub reported a digest; verify exactly.
//! - `ExpectedSha256(None)` — no digest known (repo/file quirk); download
//!   still computes the digest and reports it, but does **not** reject on
//!   mismatch. The operator can pin it on a later pull. (TOFU-lite: bytes are
//!   still atomic, but integrity is only as good as the TLS channel.)

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::HfRef;
use crate::catalog::HubCatalog;

/// Result of a completed download.
#[derive(Debug, Clone)]
pub struct DownloadedModel {
    /// Final on-disk path (already renamed from `.part`).
    pub path: PathBuf,
    /// SHA-256 of the downloaded bytes (hex).
    pub sha256: String,
    /// Bytes written.
    pub bytes: u64,
}

/// Download a HuggingFace model reference into `dest_dir`, verified.
///
/// Resolution:
/// - When the reference pins a file (`hf:org/repo:file.gguf`), that file is
///   fetched and its Hub-reported SHA-256 (from the tree API) is enforced.
/// - When only a repository is given, the **largest** GGUF file is chosen —
///   a deterministic, honest default for "give me the best quantization of
///   this model" (largest = highest quality, matches the fabric picker's
///   size heuristic).
///
/// The file lands at `dest_dir/<file>` and is returned. The destination
/// directory is created if missing.
pub async fn download_model(reference: &HfRef, dest_dir: &Path) -> Result<DownloadedModel> {
    download_model_with_progress(reference, dest_dir, None).await
}

/// Like [`download_model`] but reports download progress (bytes received) via
/// `progress` when provided. The callback is invoked from the read loop; it
/// must be cheap and non-blocking.
pub async fn download_model_with_progress(
    reference: &HfRef,
    dest_dir: &Path,
    progress: Option<Box<dyn Fn(u64) + Send + Sync>>,
) -> Result<DownloadedModel> {
    let catalog = HubCatalog::new();

    let file = match &reference.file {
        Some(file) => file.clone(),
        None => {
            let files = catalog.list_gguf_files(&reference.repo).await?;
            let largest = files
                .iter()
                .max_by_key(|f| f.size.unwrap_or(0))
                .with_context(|| format!("repository '{}' has no GGUF files", reference.repo))?;
            largest.path.clone()
        }
    };

    // Pin the digest before any byte hits the disk.
    let expected = catalog
        .list_gguf_files(&reference.repo)
        .await?
        .into_iter()
        .find(|f| f.path == file)
        .and_then(|f| f.lfs.map(|lfs| lfs.oid));

    let url = reference.resolve_url(&file);
    let dest_file = dest_dir.join(&file);

    match expected {
        Some(sha) => {
            tracing::info!(repo = %reference.repo, file = %file, "downloading verified model");
            download_verified(&url, &dest_file, Some(&sha), progress).await
        }
        None => {
            tracing::warn!(repo = %reference.repo, file = %file, "no Hub SHA-256 available; downloading unverified");
            download_verified(&url, &dest_file, None, progress).await
        }
    }
}

/// Download `url` into `dest_file` and verify its SHA-256.
///
/// `expected_sha256` is the digest pinned by the catalog layer; see the module
/// docs for the `None` semantics. The download is atomic: data lands in
/// `dest_file.part` and is renamed over `dest_file` only on a verified match.
pub async fn download_verified(
    url: &str,
    dest_file: &Path,
    expected_sha256: Option<&str>,
    progress: Option<Box<dyn Fn(u64) + Send + Sync>>,
) -> Result<DownloadedModel> {
    let client = reqwest::Client::new();
    let mut req = client.get(url);
    if let Ok(token) =
        std::env::var("HF_TOKEN").or_else(|_| std::env::var("HUGGING_FACE_HUB_TOKEN"))
    {
        if !token.trim().is_empty() {
            req = req.bearer_auth(token.trim());
        }
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: HTTP {} for {url}", resp.status());
    }

    let parent = dest_file
        .parent()
        .context("destination has no parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let part_path = dest_file.with_extension("gguf.part");
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .with_context(|| format!("creating {}", part_path.display()))?;

    let mut stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let mut bytes: u64 = 0;

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("reading body of {url}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing {}", part_path.display()))?;
        bytes += chunk.len() as u64;
        if let Some(p) = &progress {
            p(bytes);
        }
    }
    file.flush()
        .await
        .with_context(|| format!("flushing {}", part_path.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("syncing {}", part_path.display()))?;

    let sha256 = hex(&hasher.finalize());

    if let Some(expected) = expected_sha256 {
        let expected = expected.trim().to_lowercase();
        if sha256 != expected {
            let _ = tokio::fs::remove_file(&part_path).await;
            anyhow::bail!(
                "sha256 mismatch for {}: expected {}, got {} (download discarded)",
                dest_file.display(),
                expected,
                sha256
            );
        }
    }

    tokio::fs::rename(&part_path, dest_file)
        .await
        .with_context(|| {
            format!(
                "renaming {} -> {}",
                part_path.display(),
                dest_file.display()
            )
        })?;

    Ok(DownloadedModel {
        path: dest_file.to_path_buf(),
        sha256,
        bytes,
    })
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_formats_lowercase() {
        assert_eq!(hex(&[0x00, 0xff, 0xAB]), "00ffab");
    }
}
