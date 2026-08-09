use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SUPPORTED_EXTENSIONS: &[&str] = &["gguf", "safetensors", "onnx", "bin", "pt", "pth"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub relative_path: String,
    pub canonical_path: String,
    pub size_bytes: u64,
    pub modification_time: u64,
    pub extension: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelRegistry {
    pub root: String,
    pub models: HashMap<String, ModelRecord>,
}

impl ModelRegistry {
    pub fn new(root: PathBuf) -> Result<Self> {
        let canonical_root = fs::canonicalize(&root)
            .with_context(|| format!("canonicalizing root path {}", root.display()))?;
        
        if !canonical_root.is_dir() {
            anyhow::bail!("not a directory: {}", canonical_root.display());
        }

        Ok(ModelRegistry {
            root: canonical_root.to_string_lossy().to_string(),
            models: HashMap::new(),
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
        let content = serde_json::to_string_pretty(self)
            .context("serializing registry to JSON")?;
        fs::write(path, content)
            .with_context(|| format!("writing registry to {}", path.display()))?;
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
        self.scan_recursive(&canonical_scan_root, &canonical_registry_root, &mut found_count)?;
        Ok(found_count)
    }

    fn scan_recursive(
        &mut self,
        dir: &Path,
        registry_root: &Path,
        found_count: &mut usize,
    ) -> Result<()> {
        let entries = fs::read_dir(dir)
            .with_context(|| format!("reading directory {}", dir.display()))?;

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

        let record = ModelRecord {
            relative_path: relative_path.clone(),
            canonical_path: canonical_path.to_string_lossy().to_string(),
            size_bytes: metadata.len(),
            modification_time: metadata.modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            extension,
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
        create_test_file(temp_dir.path(), "model.safetensors", b"safetensors");
        create_test_file(temp_dir.path(), "model.onnx", b"onnx");
        create_test_file(temp_dir.path(), "model.bin", b"bin");
        create_test_file(temp_dir.path(), "model.pt", b"pt");
        create_test_file(temp_dir.path(), "model.pth", b"pth");

        let count = registry.scan_directory(temp_dir.path()).unwrap();
        assert_eq!(count, 6);
        assert_eq!(registry.model_count(), 6);
    }

    #[test]
    fn test_scan_ignores_unsupported_extensions() {
        let temp_dir = TempDir::new().unwrap();
        let mut registry = ModelRegistry::new(temp_dir.path().to_path_buf()).unwrap();

        create_test_file(temp_dir.path(), "model.txt", b"text");
        create_test_file(temp_dir.path(), "model.json", b"json");
        create_test_file(temp_dir.path(), "model.unknown", b"unknown");

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
}