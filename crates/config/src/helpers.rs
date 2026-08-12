use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HelperError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("file permissions are insecure: expected 0o600, got {0:#o}")]
    InsecureMode(u32),
    #[error("not a regular file: {0}")]
    NotRegular(String),
}

/// Ensure that the given path has permissions 0o600 (owner read/write only).
/// Returns Ok(()) if the file is missing (caller may create it) or meets the requirement.
pub fn ensure_mode_0600(path: &Path) -> Result<(), HelperError> {
    use std::os::unix::fs::PermissionsExt;
    if !path.exists() {
        // no file yet — caller may create it
        return Ok(());
    }
    let meta = std::fs::metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(HelperError::NotRegular(path.display().to_string()));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(HelperError::InsecureMode(mode));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::NamedTempFile;

    #[test]
    fn temp_file_ok_with_0600() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms).unwrap();
        assert!(ensure_mode_0600(path).is_ok());
    }

    #[test]
    fn temp_file_fails_with_0644() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(path, perms).unwrap();
        let err = ensure_mode_0600(path).unwrap_err();
        let s = format!("{}", err);
        assert!(s.contains("InsecureMode") || s.to_lowercase().contains("insecure"));
    }
}
