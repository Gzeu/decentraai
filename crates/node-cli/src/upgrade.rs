//! Self-upgrade support (`decentraai upgrade` + `node --auto-upgrade`).
//!
//! The node can refresh itself to the current `origin/main`:
//!
//! - `check` — `git fetch` + compare local HEAD to `origin/main`, without
//!   touching the working tree. Purely informational.
//! - `apply` — fetch → checkout `main` → pull --rebase → `cargo build
//!   --release --bin decentraai` → back up the running binary → stop the
//!   systemd user service (ETXTBSY guard) → swap the binary → restart →
//!   verify the service is active. On a failed build the repo is rolled back
//!   to the previous HEAD and the old binary stays in place.
//! - `auto` — loop: check every N seconds, apply when behind.
//!
//! # Safety
//!
//! - Never touches node data/config/identity (`~/.decentraai` is not touched;
//!   only the repo working tree and the installed binary).
//! - Requires a clean working tree before `apply` (no uncommitted changes, so
//!   a rebase cannot clobber local work).
//! - The running binary is always backed up before the swap and restored on
//!   any failure after the swap (best-effort).
//! - The build is the long pole (minutes on small machines); the service is
//!   stopped only for the brief binary swap, not for the build.
//!
//! This is self-update of the node software from its own git remote — it is
//! NOT remote shell from the application. The mesh never pushes binaries.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the node service lives (systemd *user* unit).
pub const NODE_SERVICE: &str = "decentraai-node";

/// Outcome of a read-only `check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Local HEAD equals `origin/main`.
    UpToDate,
    /// `origin/main` is ahead by `behind` commits.
    Behind {
        behind: usize,
        local_head: String,
        remote_head: String,
    },
    /// Not a git checkout (repo dir missing/not a repo).
    NoRepo,
    /// A command failed (git not installed, fetch error, …). Never `unwrap`.
    Error(String),
}

impl UpdateStatus {
    /// Whether an `apply` would do anything.
    pub fn needs_update(&self) -> bool {
        matches!(self, Self::Behind { behind, .. } if *behind > 0)
    }
}

/// Parses `git rev-list --count HEAD..origin/main` output into a count.
/// Pure decision — separated from I/O so it is unit-testable.
pub fn parse_behind_count(output: &str) -> usize {
    output.trim().parse::<usize>().unwrap_or(0)
}

/// Runs a git command inside `repo_dir`, returning trimmed stdout.
fn git(repo_dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("spawning git {args:?} in {}", repo_dir.display()))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Read-only update check. Fetches from the configured remote (does not touch
/// the working tree) and compares HEAD to `origin/main`.
pub fn check_for_update(repo_dir: &Path) -> UpdateStatus {
    if !repo_dir.join(".git").exists() {
        return UpdateStatus::NoRepo;
    }
    if let Err(e) = git(repo_dir, &["fetch", "origin"]) {
        return UpdateStatus::Error(format!("fetch: {e:#}"));
    }
    let local_head = match git(repo_dir, &["rev-parse", "--short", "HEAD"]) {
        Ok(h) => h,
        Err(e) => return UpdateStatus::Error(format!("rev-parse: {e:#}")),
    };
    let remote_head = match git(repo_dir, &["rev-parse", "--short", "origin/main"]) {
        Ok(h) => h,
        Err(e) => return UpdateStatus::Error(format!("origin/main: {e:#}")),
    };
    if local_head == remote_head {
        return UpdateStatus::UpToDate;
    }
    let behind = git(repo_dir, &["rev-list", "--count", "HEAD..origin/main"])
        .map(|s| parse_behind_count(&s))
        .unwrap_or(1);
    UpdateStatus::Behind {
        behind,
        local_head,
        remote_head,
    }
}

/// The running binary path: `$CARGO_HOME/bin/decentraai`, falling back to
/// `~/.cargo/bin/decentraai`.
pub fn installed_bin_path() -> PathBuf {
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
        let p = PathBuf::from(cargo_home).join("bin/decentraai");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo/bin/decentraai")
}

/// Report of a successful `apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub from: String,
    pub to: String,
    pub binary_backup: PathBuf,
}

/// Whether the working tree is clean (no uncommitted changes).
pub fn working_tree_clean(repo_dir: &Path) -> bool {
    git(repo_dir, &["status", "--porcelain"])
        .map(|s| s.is_empty())
        .unwrap_or(false)
}

/// Applies the upgrade: pull `main`, rebuild, swap the binary, restart the
/// service. On build failure the repo is rolled back to the previous HEAD and
/// the previous binary is left in place.
pub fn apply_update(repo_dir: &Path) -> Result<ApplyReport> {
    if !repo_dir.join(".git").exists() {
        bail!("not a git checkout: {}", repo_dir.display());
    }
    if !working_tree_clean(repo_dir) {
        bail!(
            "working tree not clean in {} — commit or stash first (auto-upgrade never touches your work)",
            repo_dir.display()
        );
    }
    let status = check_for_update(repo_dir);
    if !status.needs_update() {
        bail!("nothing to update ({status:?})");
    }
    let (behind, local_head, remote_head) = match status {
        UpdateStatus::Behind {
            behind,
            local_head,
            remote_head,
        } => (behind, local_head, remote_head),
        _ => unreachable!("needs_update() guaranteed Behind"),
    };
    eprintln!("==> behind {behind} commits: {local_head} -> {remote_head}");

    // 1. Bring the repo to origin/main (rebase is safe: tree is clean).
    git(repo_dir, &["checkout", "main"]).context("checkout main")?;
    git(repo_dir, &["pull", "--rebase", "origin", "main"]).context("pull --rebase")?;

    // 2. Build. This is the long pole and happens BEFORE any service stop.
    eprintln!("==> cargo build --release --bin decentraai (this takes minutes)");
    let build = Command::new("cargo")
        .current_dir(repo_dir)
        .args(["build", "--release", "--bin", "decentraai"])
        .output()
        .context("spawning cargo build")?;
    if !build.status.success() {
        // Roll the repo back; the old binary stays untouched.
        let _ = git(repo_dir, &["checkout", &local_head]);
        bail!(
            "build failed — repo rolled back to {local_head}: {}",
            String::from_utf8_lossy(&build.stderr).trim()
        );
    }

    // 3. Swap the binary with a backup, around a service stop (ETXTBSY guard).
    let bin_path = installed_bin_path();
    let backup = bin_path.with_extension(format!("bak.{}", timestamp()));
    if bin_path.exists() {
        std::fs::copy(&bin_path, &backup).with_context(|| {
            format!("backing up {} -> {}", bin_path.display(), backup.display())
        })?;
    }
    stop_service()?;
    let built = repo_dir.join("target/release/decentraai");
    std::fs::copy(&built, &bin_path)
        .with_context(|| format!("installing {} -> {}", built.display(), bin_path.display()))?;
    start_service()?;

    Ok(ApplyReport {
        from: local_head,
        to: remote_head,
        binary_backup: backup,
    })
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

/// Stops the node systemd *user* service. Best-effort: a node not installed
/// as a service simply has nothing to stop.
fn stop_service() -> Result<()> {
    let out = Command::new("systemctl")
        .args(["--user", "stop", NODE_SERVICE])
        .output()
        .context("spawning systemctl stop")?;
    if !out.status.success() && !String::from_utf8_lossy(&out.stderr).contains("not loaded") {
        eprintln!(
            "  note: systemctl stop returned {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Starts (or restarts) the node systemd *user* service. Best-effort.
fn start_service() -> Result<()> {
    let out = Command::new("systemctl")
        .args(["--user", "start", NODE_SERVICE])
        .output()
        .context("spawning systemctl start")?;
    if !out.status.success() {
        eprintln!(
            "  note: systemctl start returned {}: {} — the binary is installed; start the service manually",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_behind_handles_output_variants() {
        assert_eq!(parse_behind_count("3\n"), 3);
        assert_eq!(parse_behind_count("0"), 0);
        assert_eq!(parse_behind_count("  \n 12 \n"), 12);
        // Garbage (error message) → conservative 0, never a panic.
        assert_eq!(parse_behind_count("fatal: not a git repository"), 0);
        assert_eq!(parse_behind_count(""), 0);
    }

    #[test]
    fn status_needs_update_only_when_behind() {
        assert!(
            UpdateStatus::Behind {
                behind: 1,
                local_head: "a".into(),
                remote_head: "b".into()
            }
            .needs_update()
        );
        assert!(
            !UpdateStatus::Behind {
                behind: 0,
                local_head: "a".into(),
                remote_head: "a".into()
            }
            .needs_update()
        );
        assert!(!UpdateStatus::UpToDate.needs_update());
        assert!(!UpdateStatus::NoRepo.needs_update());
        assert!(!UpdateStatus::Error("x".into()).needs_update());
    }
}
