//! HuggingFace catalog and verified model download.
//!
//! DecentraAI's *public model source*: a node operator searches the
//! HuggingFace Hub for GGUF artifacts, inspects what files a repository
//! offers (with sizes and SHA-256 digests reported by the Hub), and pulls a
//! model into the local registry.
//!
//! Threat model / invariants:
//! - The Hub is **not trusted** for content — only for discovery. The SHA-256
//!   digest we verify against comes from the same Hub API that hands us the
//!   download URL, so a malicious Hub could lie about both. What makes the
//!   digest meaningful is that it is *pinned before the download starts* and
//!   the file is rejected if it does not match; this converts a content
//!   integrity check into an integrity check with a discoverable, attributable
//!   digest (the operator can cross-check the SHA-256 with the model card /
//!   upstream release notes).
//! - Downloads stream to a `.part` staging file and are renamed into place
//!   only after the digest matches — the registry never observes a torn file.
//! - No credentials, no write access to the Hub, no API keys involved.

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod catalog;
pub mod download;

pub use catalog::{HubCatalog, HubModel, HubModelFile, PipelineTag};
pub use download::download_model;

/// Scheme prefix for a model reference accepted by the CLI, e.g.
/// `hf:Qwen/Qwen2.5-1.5B-Instruct-GGUF:q4_k_m.gguf`.
pub const HF_SCHEME: &str = "hf:";

/// A parsed HuggingFace model reference.
///
/// Accepted forms:
/// - `hf:org/repo` — repository only; the largest GGUF file is chosen.
/// - `hf:org/repo:file.gguf` — a specific file inside the repository.
/// - `org/repo` and `org/repo:file.gguf` — the `hf:` scheme is optional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfRef {
    /// Repository id as on the Hub, e.g. `Qwen/Qwen2.5-1.5B-Instruct-GGUF`.
    pub repo: String,
    /// Optional GGUF file name inside the repository.
    pub file: Option<String>,
}

impl HfRef {
    /// Parse a user-supplied model reference.
    ///
    /// Validation rules:
    /// - The reference must not be empty.
    /// - The repository must be `org/name` (exactly two slash-free parts).
    /// - A file, when present, must end with `.gguf` (case-insensitive).
    pub fn parse(input: &str) -> Result<Self> {
        let raw = input.trim();
        if raw.is_empty() {
            anyhow::bail!("empty model reference");
        }
        let raw = raw.strip_prefix(HF_SCHEME).unwrap_or(raw);

        let (repo, file) = match raw.split_once(':') {
            Some((repo, file)) => (repo.trim(), Some(file.trim())),
            None => (raw, None),
        };

        if repo.is_empty() {
            anyhow::bail!("model reference '{input}' has an empty repository");
        }
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
            anyhow::bail!(
                "model reference '{input}' must be 'org/repo' (got '{repo}')"
            );
        }

        if let Some(file) = &file {
            if file.is_empty() {
                anyhow::bail!("model reference '{input}' has an empty file name");
            }
            if !file.to_lowercase().ends_with(".gguf") {
                anyhow::bail!("file '{file}' is not a .gguf file");
            }
        }

        Ok(HfRef {
            repo: repo.to_string(),
            file: file.map(str::to_string),
        })
    }

    /// The download URL for the resolved file on the Hub CDN.
    pub fn resolve_url(&self, file: &str) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.repo, file
        )
    }
}

impl std::fmt::Display for HfRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.file {
            Some(file) => write!(f, "hf:{}:{}", self.repo, file),
            None => write!(f, "hf:{}", self.repo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repo_only() {
        let r = HfRef::parse("hf:Qwen/Qwen2.5-1.5B-Instruct-GGUF").unwrap();
        assert_eq!(r.repo, "Qwen/Qwen2.5-1.5B-Instruct-GGUF");
        assert_eq!(r.file, None);
    }

    #[test]
    fn parse_repo_and_file() {
        let r =
            HfRef::parse("hf:Qwen/Qwen2.5-1.5B-Instruct-GGUF:q4_k_m.gguf").unwrap();
        assert_eq!(r.repo, "Qwen/Qwen2.5-1.5B-Instruct-GGUF");
        assert_eq!(r.file, Some("q4_k_m.gguf".into()));
    }

    #[test]
    fn scheme_is_optional() {
        let r = HfRef::parse("Qwen/Qwen2.5-1.5B-Instruct-GGUF").unwrap();
        assert_eq!(r.repo, "Qwen/Qwen2.5-1.5B-Instruct-GGUF");
        assert_eq!(r.file, None);

        let r = HfRef::parse("org/repo:file.gguf").unwrap();
        assert_eq!(r.file, Some("file.gguf".into()));
    }

    #[test]
    fn parse_trims_whitespace() {
        let r = HfRef::parse("  hf:org/repo:file.gguf  ").unwrap();
        assert_eq!(r.repo, "org/repo");
        assert_eq!(r.file, Some("file.gguf".into()));
    }

    #[test]
    fn reject_empty() {
        assert!(HfRef::parse("").is_err());
        assert!(HfRef::parse("   ").is_err());
    }

    #[test]
    fn reject_bad_repo_shape() {
        assert!(HfRef::parse("hf:no-slash").is_err());
        assert!(HfRef::parse("hf:a/b/c").is_err());
        assert!(HfRef::parse("hf:/repo").is_err());
        assert!(HfRef::parse("hf:org/").is_err());
    }

    #[test]
    fn reject_bad_file() {
        assert!(HfRef::parse("hf:org/repo:model.safetensors").is_err());
        assert!(HfRef::parse("hf:org/repo:").is_err());
    }

    #[test]
    fn resolve_url_is_pinned() {
        let r = HfRef::parse("hf:org/repo:file.gguf").unwrap();
        assert_eq!(
            r.resolve_url("file.gguf"),
            "https://huggingface.co/org/repo/resolve/main/file.gguf"
        );
    }

    #[test]
    fn display_round_trips() {
        let r = HfRef::parse("hf:org/repo:file.gguf").unwrap();
        assert_eq!(r.to_string(), "hf:org/repo:file.gguf");
        let r2 = HfRef::parse("hf:org/repo").unwrap();
        assert_eq!(r2.to_string(), "hf:org/repo");
    }
}