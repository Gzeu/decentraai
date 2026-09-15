//! Engine PID tracking + stale-orphan reaping (M24 hardening).
//!
//! Observed incident (2026-09-09): 11 orphaned llama-server processes held
//! ~4.5GB RAM on the VPS. Graceful paths already kill+wait; orphans come
//! from runs that never run cleanup (SIGKILL / OOM / power loss) — no
//! in-process `Drop` can fire then. Fix: every spawned engine is recorded
//! (`<data_dir>/engine-pids/<role>.pid`, content `pid + port`); at startup
//! [`reap_stale`] SIGKILLs recorded PIDs that are STILL llama-server
//! processes on the recorded port, then removes the record.
//!
//! Safety rules (never violated):
//! - Only PIDs recorded in OUR pid dir are candidates — never a scan.
//! - A candidate is killed only if `/proc/<pid>/cmdline` contains BOTH
//!   `llama-server` AND the recorded `--port <n>` (a sibling node's engine
//!   on another port is spared).
//! - Non-Linux: record/clear work; live-PID reaping is `LeftAlone`
//!   (documented, no silent behavior difference on Linux).

use std::io;
use std::path::{Path, PathBuf};

/// Role allow-list for pidfile names (no traversal, no surprises).
fn sanitize_role(role: &str) -> String {
    let clean: String = role
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim_matches('_').to_string();
    if clean.is_empty() {
        "engine".to_string()
    } else {
        clean.chars().take(64).collect()
    }
}

/// `<data_dir>/engine-pids` (created on record).
pub fn pid_dir_for(data_dir: &Path) -> PathBuf {
    data_dir.join("engine-pids")
}

fn pidfile_for(pid_dir: &Path, role: &str) -> PathBuf {
    pid_dir.join(format!("{}.pid", sanitize_role(role)))
}

/// Record a freshly spawned engine. Overwrites any previous record for the
/// role (a respawn replaces the old PID — the old process must already be
/// stopped by the caller; this file only tracks, never kills).
pub fn record(pid_dir: &Path, role: &str, pid: u32, port: u16) -> io::Result<()> {
    std::fs::create_dir_all(pid_dir)?;
    std::fs::write(pidfile_for(pid_dir, role), format!("{pid} {port}\n"))
}

/// Forget a cleanly stopped engine. Missing file is fine.
pub fn clear(pid_dir: &Path, role: &str) {
    let _ = std::fs::remove_file(pidfile_for(pid_dir, role));
}

/// Outcome of one pidfile examined by [`reap_stale`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReapOutcome {
    /// Live matching orphan was SIGKILLed and is gone; record removed.
    Reaped { pid: u32 },
    /// PID already dead; stale record removed.
    CleanedStale,
    /// Deliberately untouched: reason is human-readable.
    LeftAlone { reason: &'static str },
}

/// Examine every record in `pid_dir`: kill live matching orphans, drop
/// stale records. Returns `(role, outcome)` per record. Never errors as a
/// whole — per-record I/O failures are `LeftAlone`.
pub fn reap_stale(pid_dir: &Path) -> Vec<(String, ReapOutcome)> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(pid_dir);
    let Ok(entries) = entries else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        let role = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let mut parts = content.split_whitespace();
        let (Some(pid_s), Some(port_s)) = (parts.next(), parts.next()) else {
            out.push((
                role,
                ReapOutcome::LeftAlone {
                    reason: "unparseable-record",
                },
            ));
            continue;
        };
        let (Ok(pid), Ok(port)) = (pid_s.parse::<u32>(), port_s.parse::<u16>()) else {
            out.push((
                role,
                ReapOutcome::LeftAlone {
                    reason: "unparseable-record",
                },
            ));
            continue;
        };
        match reap_one(pid, port) {
            outcome @ (ReapOutcome::CleanedStale | ReapOutcome::Reaped { .. }) => {
                let _ = std::fs::remove_file(&path);
                out.push((role, outcome));
            }
            other => out.push((role, other)),
        }
    }
    out
}

/// Decide + act on one candidate PID.
fn reap_one(pid: u32, port: u16) -> ReapOutcome {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, port);
        return ReapOutcome::LeftAlone {
            reason: "unsupported-platform",
        };
    }
    #[cfg(target_os = "linux")]
    {
        if !pid_exists(pid) {
            return ReapOutcome::CleanedStale;
        }
        // NOTE: an empty cmdline (mid-exec window, kernel thread) is
        // "unknown", never "dead" — only a missing /proc entry means dead
        // (checked above). Killing requires a POSITIVE match below.
        let cmd = proc_cmdline(pid).unwrap_or_default();
        if !cmd.contains("llama-server") || !cmd.contains(&format!("--port {port}")) {
            return ReapOutcome::LeftAlone {
                reason: "not-our-engine",
            };
        }
        // SAFETY: pid/cmdline verified above; SIGKILL cannot be caught, so a
        // matching process that stays alive is a kernel-level anomaly we log
        // via the outcome, not a process we chase forever.
        let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        if rc != 0 {
            // Lost a race with its exit — treat as gone if it disappeared.
            return if proc_cmdline(pid).is_none() {
                ReapOutcome::CleanedStale
            } else {
                ReapOutcome::LeftAlone {
                    reason: "kill-failed",
                }
            };
        }
        for _ in 0..50 {
            if process_gone(pid) {
                return ReapOutcome::Reaped { pid };
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ReapOutcome::LeftAlone {
            reason: "still-alive",
        }
    }
}

/// `/proc/<pid>/cmdline` with NULs as spaces. `None` = unreadable (process
/// gone, or a transient mid-exec window — the caller must NOT treat `None`
/// as proof of death; use [`pid_exists`] for that).
#[cfg(target_os = "linux")]
fn proc_cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.iter()
            .map(|&b| if b == 0 { ' ' } else { b as char })
            .collect(),
    )
}

/// True while `/proc/<pid>` exists (zombies included — see `process_gone`).
#[cfg(target_os = "linux")]
fn pid_exists(pid: u32) -> bool {
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
}

/// Gone = no /proc entry, or zombie (holds no RAM; init reaps it).
#[cfg(target_os = "linux")]
fn process_gone(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return true;
    };
    // comm is `(name)` possibly containing spaces/parens: state follows `) `.
    stat.rsplit(')')
        .next()
        .is_some_and(|after| after.trim_start().starts_with('Z'))
}

/// Whether a PID currently runs a llama-server (self-checkable in tests).
#[cfg(target_os = "linux")]
pub fn is_llama_server(pid: u32) -> bool {
    proc_cmdline(pid).is_some_and(|c| c.contains("llama-server"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_names_cannot_traverse() {
        assert_eq!(sanitize_role("../../etc/x"), "etc_x");
        assert_eq!(sanitize_role(""), "engine");
        assert_eq!(sanitize_role("main"), "main");
        let pidfile = pidfile_for(Path::new("/d"), "../../etc/x");
        assert!(!pidfile.to_string_lossy().contains(".."));
    }

    #[test]
    fn record_clear_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let pid_dir = pid_dir_for(dir.path());
        record(&pid_dir, "main", 1234, 8080).unwrap();
        assert!(pidfile_for(&pid_dir, "main").exists());
        clear(&pid_dir, "main");
        assert!(!pidfile_for(&pid_dir, "main").exists());
        clear(&pid_dir, "main"); // idempotent
    }

    #[test]
    fn dead_pid_record_is_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let pid_dir = pid_dir_for(dir.path());
        // PID 2^22+12345 is essentially never alive; and even if it were,
        // the cmdline guard refuses to kill non-matching processes.
        record(&pid_dir, "old", (1 << 22) + 12345, 8080).unwrap();
        let outcomes = reap_stale(&pid_dir);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0, "old");
        // That PID is dead on any test machine; the cmdline guard additionally
        // guarantees a live hit could never be killed by mistake.
        assert_eq!(outcomes[0].1, ReapOutcome::CleanedStale);
        assert!(!pidfile_for(&pid_dir, "old").exists());
    }

    #[test]
    fn missing_dir_reaps_nothing() {
        let outcomes = reap_stale(Path::new("/definitely/not/here/engine-pids"));
        assert!(outcomes.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn self_is_not_llama_server() {
        assert!(!is_llama_server(std::process::id()));
    }

    /// Real kill path: a `sleep` disguised with a llama-server argv[0] AND
    /// the recorded port is reaped; the record is removed.
    #[cfg(target_os = "linux")]
    #[test]
    fn live_matching_orphan_is_reaped() {
        use std::process::Command;
        let port = 54321u16;
        // `exec -a` disguises argv[0]; bash replaces itself so the recorded
        // PID is the disguised sleeper.
        let mut child = Command::new("bash")
            .args(["-c", "exec -a 'llama-server --port 54321' sleep 60"])
            .spawn()
            .expect("bash spawns in test env");
        let pid = child.id();
        // Sanity: our guard must recognize the disguise. Poll briefly: right
        // after spawn the child may sit in the execve window (empty cmdline)
        // especially with 400+ sibling tests loading the scheduler.
        let mut seen = false;
        for _ in 0..50 {
            if proc_cmdline(pid).is_some_and(|c| c.contains("llama-server")) {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(seen, "disguised sleeper must be recognizable");
        let dir = tempfile::tempdir().unwrap();
        let pid_dir = pid_dir_for(dir.path());
        record(&pid_dir, "test-orphan", pid, port).unwrap();
        let outcomes = reap_stale(&pid_dir);
        assert_eq!(
            outcomes,
            vec![("test-orphan".to_string(), ReapOutcome::Reaped { pid })]
        );
        assert!(!pidfile_for(&pid_dir, "test-orphan").exists());
        // …and must NOT match a wrong port (sibling's engine is spared).
        let mut child2 = Command::new("bash")
            .args(["-c", "exec -a 'llama-server --port 12345' sleep 60"])
            .spawn()
            .expect("bash spawns in test env");
        let pid2 = child2.id();
        record(&pid_dir, "sibling", pid2, 9999).unwrap();
        let outcomes2 = reap_stale(&pid_dir);
        assert_eq!(
            outcomes2,
            vec![(
                "sibling".to_string(),
                ReapOutcome::LeftAlone {
                    reason: "not-our-engine"
                }
            )]
        );
        assert!(pidfile_for(&pid_dir, "sibling").exists());
        let _ = child.try_wait();
        let _ = child2.kill();
        let _ = child2.wait();
    }
}
