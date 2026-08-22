use std::path::Path;
use std::time::Duration;
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

/// Environment variable that overrides every inference-timeout layer
/// (backend HTTP, P2P request/response, remote route) with one value.
pub const BACKEND_TIMEOUT_ENV: &str = "DECENTRAAI_BACKEND_TIMEOUT_SECS";
/// Default inference budget in seconds. Slow-CPU prefill on large agent
/// prompts legitimately exceeds 5 minutes (e.g. ~11.5k-token prompts on a
/// 6-vCPU node at ~20 tok/s), so operators can raise it via the env var.
pub const DEFAULT_BACKEND_TIMEOUT_SECS: u64 = 300;

/// Single source of truth for the inference timeout shared by every
/// transport layer. All strata must derive from THIS value — a remote hop
/// whose limit is shorter than the backend's would cut a healthy worker
/// mid-prefill, which surfaces to callers as "connection dropped".
///
/// Overridable for slow-CPU nodes: set `DECENTRAAI_BACKEND_TIMEOUT_SECS`
/// in the service environment.
pub fn backend_request_timeout() -> Duration {
    backend_request_timeout_from(std::env::var(BACKEND_TIMEOUT_ENV).ok())
}

/// Pure decision behind [`backend_request_timeout`], separated from the
/// environment so tests can drive it with synthetic inputs.
pub fn backend_request_timeout_from(env_value: Option<String>) -> Duration {
    let secs = env_value
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_BACKEND_TIMEOUT_SECS);
    Duration::from_secs(secs)
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

    /// The shared timeout helper must honor the env override and reject
    /// nonsense values (0 would mean "no budget" for a hung engine).
    #[test]
    fn backend_request_timeout_env_override() {
        assert_eq!(
            backend_request_timeout_from(Some("1800".into())),
            Duration::from_secs(1800),
            "env override must win over the default"
        );
        assert_eq!(
            backend_request_timeout_from(Some("0".into())),
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECS),
            "0 must fall back to the default, not disable the budget"
        );
        assert_eq!(
            backend_request_timeout_from(Some("garbage".into())),
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECS)
        );
        assert_eq!(
            backend_request_timeout_from(None),
            Duration::from_secs(DEFAULT_BACKEND_TIMEOUT_SECS)
        );
    }
}
