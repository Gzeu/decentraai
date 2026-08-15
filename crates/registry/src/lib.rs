use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const SUPPORTED_EXTENSIONS: &[&str] = &["gguf"];

const REGISTRY_VERSION: u32 = 1;

/// A persisted capability claim on a model, hub-agnostic so the registry stays
/// a leaf crate (it does not depend on the hub). `capability` is the snake_case
/// name from the shared capability taxonomy (e.g. "ocr", "coding"), `provenance`
/// is "verified" or "inferred". This is a persistence *projection* of the
/// authoritative hub `ModelCapabilities` — not a second capability system.
/// Absent claims simply mean the model has no recorded capability data (UNKNOWN).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityClaimRecord {
    pub capability: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub relative_path: String,
    pub canonical_path: String,
    pub size_bytes: u64,
    pub modification_time: u64,
    pub extension: String,
    /// Persisted capability claims (projection of the hub taxonomy). Empty by
    /// default (UNKNOWN); `#[serde(default)]` keeps older registries valid.
    #[serde(default)]
    pub capability_claims: Vec<CapabilityClaimRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelRegistry {
    #[serde(default)]
    pub version: u32,
    pub root: String,
    pub models: BTreeMap<String, ModelRecord>,
}

impl ModelRegistry {
    pub fn new(root: PathBuf) -> Result<Self> {
        let canonical_root = fs::canonicalize(&root)
            .with_context(|| format!("canonicalizing root path {}", root.display()))?;

        if !canonical_root.is_dir() {
            anyhow::bail!("not a directory: {}", canonical_root.display());
        }

        Ok(ModelRegistry {
            version: REGISTRY_VERSION,
            root: canonical_root.to_string_lossy().to_string(),
            models: BTreeMap::new(),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("reading registry from {}", path.display()))?;
        let registry: ModelRegistry = serde_json::from_str(&content)
            .with_context(|| format!("parsing registry JSON from {}", path.display()))?;
        Ok(registry)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self).context("serializing registry to JSON")?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&tmp)
                .with_context(|| format!("creating temporary registry {}", tmp.display()))?;
            file.write_all(content.as_bytes())
                .with_context(|| format!("writing temporary registry {}", tmp.display()))?;
            file.sync_all()
                .with_context(|| format!("syncing temporary registry {}", tmp.display()))?;
        }
        fs::rename(&tmp, path).with_context(|| format!("replacing registry {}", path.display()))?;
        Ok(())
    }

    pub fn scan_directory(&mut self, scan_root: &Path) -> Result<usize> {
        let canonical_scan_root = fs::canonicalize(scan_root)
            .with_context(|| format!("canonicalizing scan root {}", scan_root.display()))?;

        let canonical_registry_root = PathBuf::from(&self.root);

        if !canonical_scan_root.starts_with(&canonical_registry_root) {
            anyhow::bail!(
                "scan root {} outside registry root {}",
                canonical_scan_root.display(),
                canonical_registry_root.display()
            );
        }

        let mut found_count = 0;
        self.scan_recursive(
            &canonical_scan_root,
            &canonical_registry_root,
            &mut found_count,
        )?;
        self.models
            .retain(|_, record| Path::new(&record.canonical_path).exists());
        Ok(found_count)
    }

    fn scan_recursive(
        &mut self,
        dir: &Path,
        registry_root: &Path,
        found_count: &mut usize,
    ) -> Result<()> {
        let entries =
            fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;

        for entry in entries {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();

            // Check for symlinks
            if path.is_symlink() {
                continue; // Skip symlinks entirely
            }

            if path.is_dir() {
                self.scan_recursive(&path, registry_root, found_count)?;
            } else if path.is_file() {
                if let Some(extension) = path.extension() {
                    let ext = extension.to_string_lossy().to_lowercase();
                    if SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
                        self.register_model(&path, registry_root)?;
                        *found_count += 1;
                    }
                }
            }
        }

        Ok(())
    }

    fn register_model(&mut self, file_path: &Path, registry_root: &Path) -> Result<()> {
        let canonical_path = fs::canonicalize(file_path)
            .with_context(|| format!("canonicalizing file path {}", file_path.display()))?;

        // Verify the canonical path is still under the registry root
        if !canonical_path.starts_with(registry_root) {
            anyhow::bail!(
                "canonical path {} outside registry root {}",
                canonical_path.display(),
                registry_root.display()
            );
        }

        let metadata = fs::metadata(&canonical_path)
            .with_context(|| format!("getting metadata for {}", canonical_path.display()))?;
        let relative_path = canonical_path
            .strip_prefix(registry_root)
            .map_err(|e| anyhow::anyhow!("{}: {}", canonical_path.display(), e))?
            .to_string_lossy()
            .to_string();

        let extension = canonical_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // Preserve previously persisted capability claims on rescan so a
        // periodic re-scan never wipes the capability data written at pull time
        // (idempotent: existing claims are kept, absent means UNKNOWN).
        let capability_claims = self
            .models
            .get(&relative_path)
            .map(|r| r.capability_claims.clone())
            .unwrap_or_default();

        let record = ModelRecord {
            relative_path: relative_path.clone(),
            canonical_path: canonical_path.to_string_lossy().to_string(),
            size_bytes: metadata.len(),
            modification_time: metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            extension,
            capability_claims,
        };

        // Idempotent: update existing record or add new one
        self.models.insert(relative_path, record);
        Ok(())
    }

    pub fn list_models(&self) -> Vec<&ModelRecord> {
        let mut models: Vec<_> = self.models.values().collect();
        models.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        models
    }

    /// Returns a model record by its relative path.
    pub fn record(&self, relative_path: &str) -> Option<&ModelRecord> {
        self.models.get(relative_path)
    }

    /// Sets the persisted capability claims for a model identified by its
    /// relative path (the projection of the hub taxonomy written at pull time).
    /// Returns `Ok(false)` when no such model exists (nothing persisted).
    pub fn set_capability_claims(
        &mut self,
        relative_path: &str,
        claims: Vec<CapabilityClaimRecord>,
    ) -> Result<bool> {
        let Some(record) = self.models.get_mut(relative_path) else {
            return Ok(false);
        };
        record.capability_claims = claims;
        Ok(true)
    }

    /// Authoritative local capability query: every `(relative_path, capability,
    /// provenance)` for each model whose `capability_claims` contains a claim
    /// whose `capability` matches `capability` (case-insensitive). When
    /// `require_verified` is true only claims with provenance exactly
    /// "verified" (case-insensitive) qualify; when false, "verified" OR
    /// "inferred" qualify. Provenance is preserved so callers can distinguish
    /// verified from inferred. A model with no matching claim is never returned
    /// (honest: UNKNOWN is absent). Sorted deterministically by `relative_path`
    /// ascending.
    pub fn models_with_capability(
        &self,
        capability: &str,
        require_verified: bool,
    ) -> Vec<(&str, &str, &str)> {
        let want = capability.to_lowercase();
        let mut results = Vec::new();
        for model in self.models.values() {
            for claim in &model.capability_claims {
                if claim.capability.to_lowercase() != want {
                    continue;
                }
                let is_verified = claim.provenance.eq_ignore_ascii_case("verified");
                if require_verified && !is_verified {
                    continue;
                }
                results.push((model.relative_path.as_str(), claim.capability.as_str(), claim.provenance.as_str()));
            }
        }
        results.sort_by(|a, b| a.0.cmp(b.0));
        results
    }

    /// Returns `(relative_path, claim_count)` for every model that has at least
    /// one capability claim, sorted by `relative_path` ascending. Useful for a
    /// "what capabilities are known locally" overview.
    pub fn models_with_any_claim(&self) -> Vec<(&str, usize)> {
        let mut results: Vec<(&str, usize)> = self
            .models
            .values()
            .filter(|m| !m.capability_claims.is_empty())
            .map(|m| (m.relative_path.as_str(), m.capability_claims.len()))
            .collect();
        results.sort_by(|a, b| a.0.cmp(b.0));
        results
    }

    /// Removes a model from the registry and deletes the underlying file.
    ///
    /// Security: the caller must supply a relative path — any attempt to
    /// escape the registry root via `..` or absolute path is rejected both
    /// syntactically (reject `..` and leading `/`) and structurally (the
    /// canonical path must still start with the registry root).
    pub fn remove_model(&mut self, relative_path: &str) -> Result<ModelRecord> {
        if relative_path.contains("..") || relative_path.starts_with('/') {
            anyhow::bail!("invalid relative path: {}", relative_path);
        }
        let _record = self.models.get(relative_path).cloned().ok_or_else(|| {
            anyhow::anyhow!("model not found in registry: {}", relative_path)
        })?;

        let full_path = PathBuf::from(&self.root).join(relative_path);
        let canonical = fs::canonicalize(&full_path)
            .with_context(|| format!("canonicalizing {}", full_path.display()))?;

        let root = PathBuf::from(&self.root);
        if !canonical.starts_with(&root) {
            anyhow::bail!(
                "canonical path {} escapes registry root {}",
                canonical.display(),
                root.display()
            );
        }

        fs::remove_file(&canonical)
            .with_context(|| format!("removing file {}", canonical.display()))?;

        let removed = self.models.remove(relative_path).expect("exists by lookup above");
        Ok(removed)
    }

    #[allow(dead_code)]
    pub fn get_model(&self, relative_path: &str) -> Option<&ModelRecord> {
        self.models.get(relative_path)
    }

    #[allow(dead_code)]
    pub fn model_count(&self) -> usize {
        self.models.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let file_path = dir.join(name);
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content).unwrap();
        file_path
    }

    #[test]
    fn test_registry_creation() {
        let temp_dir = TempDir::new().unwrap();
        let registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(
            registry.root,
            temp_dir
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(registry.model_count(), 0);
    }

    #[test]
    fn test_registry_rejects_nonexistent_root() {
        let nonexistent = PathBuf::from("/nonexistent/path/that/does/not/exist");
        let result = ModelRegistry::new(nonexistent);
        assert!(result.is_err());
    }

    #[test]
    fn test_registry_rejects_file_as_root() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("file.txt");
        File::create(&file_path).unwrap();

        let result = ModelRegistry::new(file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_supported_extensions() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");

        let count = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count, 1);
        assert_eq!(registry.model_count(), 1);
    }

    #[test]
    fn test_capability_claims_set_and_survive_rescan() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();
        create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");
        registry.scan_directory(temp_dir.path()).unwrap();

        // Set claims for the scanned model (projection written at pull time).
        let rel = "model.gguf".to_string();
        let claims = vec![
            CapabilityClaimRecord {
                capability: "ocr".into(),
                provenance: "verified".into(),
            },
            CapabilityClaimRecord {
                capability: "coding".into(),
                provenance: "inferred".into(),
            },
        ];
        assert!(registry.set_capability_claims(&rel, claims.clone()).unwrap());

        // A rescan must NOT wipe the persisted claims (idempotent projection).
        registry.scan_directory(temp_dir.path()).unwrap();
        let record = registry.record(&rel).unwrap();
        assert_eq!(record.capability_claims, claims);

        // Setting claims for an unknown path is a no-op (returns false).
        assert!(!registry.set_capability_claims("nope.gguf", vec![]).unwrap());
    }

    #[test]
    fn test_capability_claims_persist_across_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();
        create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");
        registry.scan_directory(temp_dir.path()).unwrap();
        registry
            .set_capability_claims(
                "model.gguf",
                vec![CapabilityClaimRecord {
                    capability: "vision".into(),
                    provenance: "verified".into(),
                }],
            )
            .unwrap();
        let path = temp_dir.path().join("registry.json");
        registry.save(&path).unwrap();

        let loaded = ModelRegistry::load(&path).unwrap();
        let record = loaded.record("model.gguf").unwrap();
        assert_eq!(record.capability_claims.len(), 1);
        assert_eq!(record.capability_claims[0].capability, "vision");
        assert_eq!(record.capability_claims[0].provenance, "verified");
    }

    #[test]
    fn test_scan_ignores_unsupported_extensions() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "model.txt", b"text");
        create_test_file(temp_dir.path(), "model.json", b"json");
        create_test_file(temp_dir.path(), "model.unknown", b"unknown");
        create_test_file(temp_dir.path(), "model.safetensors", b"safetensors");
        create_test_file(temp_dir.path(), "model.onnx", b"onnx");
        create_test_file(temp_dir.path(), "model.bin", b"bin");
        create_test_file(temp_dir.path(), "model.pt", b"pt");
        create_test_file(temp_dir.path(), "model.pth", b"pth");

        let count = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count, 0);
        assert_eq!(registry.model_count(), 0);
    }

    #[test]
    fn test_scan_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let sub_dir = temp_dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();

        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "root.gguf", b"root model");
        create_test_file(&sub_dir, "sub.gguf", b"sub model");

        let count = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count, 2);
        assert_eq!(registry.model_count(), 2);
    }

    #[test]
    fn test_scan_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        let file_path = create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");

        let count1 = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count1, 1);

        // Modify the file
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"modified content").unwrap();

        let count2 = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count2, 1);
        assert_eq!(registry.model_count(), 1); // Still 1, not 2
    }

    #[test]
    fn test_scan_removes_deleted_models() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        let file_path = create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");
        registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(registry.model_count(), 1);

        fs::remove_file(&file_path).unwrap();
        registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(registry.model_count(), 0);
    }

    #[test]
    fn test_scan_rejects_symlinks() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        let real_file = create_test_file(temp_dir.path(), "real.gguf", b"real model");
        let symlink_path = temp_dir.path().join("link.gguf");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &symlink_path).unwrap();
        }

        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(&real_file, &symlink_path).unwrap();
        }

        let count = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count, 1); // Only the real file, not the symlink
    }

    #[test]
    fn test_scan_rejects_path_outside_root() {
        let temp_dir = TempDir::new().unwrap();
        let other_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        let result = registry.scan_directory(other_dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let registry_path = temp_dir.path().join("registry.json");

        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();
        create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");
        registry.scan_directory(temp_dir.path()).unwrap();

        registry.save(&registry_path).unwrap();

        let loaded_registry = ModelRegistry::load(&registry_path).unwrap();
        assert_eq!(loaded_registry.model_count(), 1);
        assert_eq!(loaded_registry.root, registry.root);
    }

    #[test]
    fn test_list_models_sorted() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "z.gguf", b"z");
        create_test_file(temp_dir.path(), "a.gguf", b"a");
        create_test_file(temp_dir.path(), "m.gguf", b"m");

        registry.scan_directory(temp_dir.path()).unwrap();

        let models = registry.list_models();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].relative_path, "a.gguf");
        assert_eq!(models[1].relative_path, "m.gguf");
        assert_eq!(models[2].relative_path, "z.gguf");
    }

    #[test]
    fn test_get_model() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "model.gguf", b"GGUF magic");
        registry.scan_directory(temp_dir.path()).unwrap();

        let model = registry.get_model("model.gguf");
        assert!(model.is_some());
        assert_eq!(model.unwrap().extension, "gguf");

        let missing = registry.get_model("nonexistent.gguf");
        assert!(missing.is_none());
    }

    /// Builds a synthetic registry with three `.gguf` models and heterogeneous
    /// claims: `a.gguf` has a verified "ocr" + inferred "coding"; `m.gguf` has
    /// only an inferred "ocr"; `z.gguf` has no claims at all.
    fn claims_fixture() -> (TempDir, ModelRegistry) {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();
        create_test_file(temp_dir.path(), "a.gguf", b"a");
        create_test_file(temp_dir.path(), "m.gguf", b"m");
        create_test_file(temp_dir.path(), "z.gguf", b"z");
        registry.scan_directory(temp_dir.path()).unwrap();
        registry
            .set_capability_claims(
                "a.gguf",
                vec![
                    CapabilityClaimRecord {
                        capability: "ocr".into(),
                        provenance: "verified".into(),
                    },
                    CapabilityClaimRecord {
                        capability: "coding".into(),
                        provenance: "inferred".into(),
                    },
                ],
            )
            .unwrap();
        registry
            .set_capability_claims(
                "m.gguf",
                vec![CapabilityClaimRecord {
                    capability: "ocr".into(),
                    provenance: "inferred".into(),
                }],
            )
            .unwrap();
        (temp_dir, registry)
    }

    #[test]
    fn test_models_with_capability_verified_gating() {
        let (_dir, registry) = claims_fixture();

        // require_verified=true: only a.gguf's verified "ocr" claim qualifies.
        let results = registry.models_with_capability("ocr", true);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ("a.gguf", "ocr", "verified"));

        // require_verified=false: verified AND inferred both qualify; sorted.
        let results = registry.models_with_capability("ocr", false);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], ("a.gguf", "ocr", "verified"));
        assert_eq!(results[1], ("m.gguf", "ocr", "inferred"));

        // Provenance is preserved, never flattened.
        let provenances: Vec<_> = results.iter().map(|r| r.2).collect();
        assert!(provenances.contains(&"verified"));
        assert!(provenances.contains(&"inferred"));
    }

    #[test]
    fn test_models_with_capability_no_match_absent() {
        let (_dir, registry) = claims_fixture();

        // "vision" is claimed by no model -> empty regardless of gating.
        assert!(registry.models_with_capability("vision", true).is_empty());
        assert!(registry.models_with_capability("vision", false).is_empty());

        // "coding" is only inferred on a.gguf -> empty when require_verified.
        assert!(registry.models_with_capability("coding", true).is_empty());
        let results = registry.models_with_capability("coding", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ("a.gguf", "coding", "inferred"));

        // z.gguf has no claims at all -> never returned.
        assert!(registry
            .models_with_capability("ocr", false)
            .iter()
            .all(|r| r.0 != "z.gguf"));
    }

    #[test]
    fn test_models_with_capability_case_insensitive() {
        let (_dir, registry) = claims_fixture();

        // Query "OCR" matches the claim capability "ocr".
        let results = registry.models_with_capability("OCR", false);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, "ocr");
        assert_eq!(results[1].1, "ocr");
    }

    #[test]
    fn test_models_with_capability_sorted_by_relative_path() {
        let (_dir, registry) = claims_fixture();

        // Multiple qualifying models are ordered by relative_path ascending.
        let results = registry.models_with_capability("ocr", false);
        let paths: Vec<_> = results.iter().map(|r| r.0).collect();
        assert_eq!(paths, vec!["a.gguf", "m.gguf"]);
    }

    #[test]
    fn test_models_with_any_claim() {
        let (_dir, registry) = claims_fixture();

        // Only models with claims appear, with correct counts, sorted.
        let results = registry.models_with_any_claim();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], ("a.gguf", 2));
        assert_eq!(results[1], ("m.gguf", 1));

        // z.gguf (no claims) is absent.
        assert!(results.iter().all(|r| r.0 != "z.gguf"));
    }
}
