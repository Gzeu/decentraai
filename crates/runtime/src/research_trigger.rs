//! M15 Research Pressure Trigger — the node-side half.
//!
//! The pure detector lives in [`decentraai_proposal::pressure`] (no I/O,
//! deterministic, golden-tested). THIS module owns node concerns only:
//!
//! ```text
//! World tick → snapshot JSON → detector → Fire | Skip
//!   → Fire spawns the EXISTING autonomous loop as a bounded child:
//!     experiment autonomous-cycle --world-url <self> --operator <cfg>
//!     (read-only/bounded lane — the trigger path can NEVER pass
//!     --enable-live-testnet; the master token travels via ENV, never argv)
//! ```
//!
//! Hard rules (all tested):
//! - Disabled/absent config = zero behavior (the tick handler early-outs).
//! - Trigger state persists atomically on EVERY evaluation, including
//!   Skips — a kill at any boundary resumes identically (reload-first).
//! - No credentials (open node without master token) = Skip without
//!   consuming pressure (the base does NOT advance, the next tick retries).
//! - Spawn failure is logged, never fatal to the tick.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use decentraai_proposal::pressure::{
    PressureDecision, PressureState, PressureThresholds, evaluate_pressure,
};
use decentraai_proposal::{ResearchJournal, parse_world_view};

/// Env var carrying the master token to the triggered child. Secret stays
/// out of argv (`ps` cannot see env of another user's process the way it
/// sees command lines; same-machine, same-operator by design).
pub const WORLD_TOKEN_ENV: &str = "DECENTRAAI_WORLD_TOKEN";

/// Node-side trigger configuration, built once from the validated
/// `research_trigger` config section (see `attach` call site in node-cli).
#[derive(Debug, Clone)]
pub struct ResearchTriggerRuntime {
    /// Allow-listed operator destination for the child's micro-budget.
    pub operator_address: String,
    /// Cycle budget ceiling (wei) for the triggered child.
    pub cycle_budget_wei: u64,
    /// Detector thresholds (mapped 1:1 from config at the edge).
    pub thresholds: PressureThresholds,
    /// This node's own World base URL (loopback; the API binds loopback).
    pub self_url: String,
    /// Master token for the child's World calls (None = open node → Skip).
    pub master_token: Option<String>,
    /// Trigger-state file (atomic JSON).
    pub state_path: PathBuf,
    /// Research journal path (best-effort read; absent = empty journal).
    pub journal_path: PathBuf,
    /// SEC-01 single-flight guard: at most one trigger cycle (evaluate →
    /// spawn → child run) is ever in flight per node process. Tick-time
    /// races (two concurrent POSTs) serialize here — the loser skips
    /// WITHOUT touching state, so pressure persists for the next tick.
    /// In-memory only: a node restart resets it (state file stays the
    /// durable truth; no stuck flag possible). No locks involved, so no
    /// deadlock is constructible — only a lock-free CAS.
    pub inflight: Arc<AtomicBool>,
}

impl ResearchTriggerRuntime {
    /// Build from the validated config section. `api_port` is the node's
    /// own API port; `data_dir` anchors the default state file.
    pub fn from_config(
        section: &decentraai_config::ResearchTriggerSection,
        api_port: u16,
        data_dir: &Path,
        master_token: Option<String>,
    ) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Self {
            operator_address: section.operator_address.clone(),
            cycle_budget_wei: section.cycle_budget_wei,
            thresholds: PressureThresholds {
                min_tick_drift: section.thresholds.min_tick_drift,
                min_new_events: section.thresholds.min_new_events,
                min_entity_delta: section.thresholds.min_entity_delta,
                min_treasury_delta: section.thresholds.min_treasury_delta,
                refuted_burst: section.thresholds.refuted_burst,
                revisit_ticks: section.thresholds.revisit_ticks,
                cooldown_ticks: section.cooldown_ticks,
                fire_threshold: section.thresholds.fire_threshold,
            },
            self_url: format!("http://127.0.0.1:{api_port}"),
            master_token,
            state_path: section
                .state_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("experiments/world-trigger.json")),
            journal_path: PathBuf::from(home).join(".decentraai/experiments/research-journal.json"),
            inflight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Claim the single-flight slot. `true` = this caller owns the cycle
    /// and must either spawn (which releases on child exit) or release via
    /// [`Self::end_cycle`]. `false` = a cycle is already running.
    #[must_use]
    pub fn try_begin_cycle(&self) -> bool {
        self.inflight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Release the slot without spawning (no-Fire path).
    pub fn end_cycle(&self) {
        self.inflight.store(false, Ordering::SeqCst);
    }

    /// Load trigger state (reload-first; garbage or absent = fresh state,
    /// which only establishes the baseline — never fires).
    #[must_use]
    pub fn load_state(&self) -> PressureState {
        std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|raw| PressureState::from_json(&raw).ok())
            .unwrap_or_default()
    }

    /// Atomic persist (tmp + rename): torn writes are impossible.
    pub fn save_state(&self, state: &PressureState) -> std::io::Result<()> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = state
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let tmp = self.state_path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    /// Best-effort journal read (absent/unparseable = empty journal:
    /// journal-based signals score 0, honestly).
    #[must_use]
    pub fn load_journal(&self) -> ResearchJournal {
        std::fs::read_to_string(&self.journal_path)
            .ok()
            .and_then(|raw| ResearchJournal::from_json(&raw).ok())
            .unwrap_or_default()
    }

    /// Evaluate one World snapshot (pure detector + durable state).
    /// Returns the decision and persists the next state BEFORE the caller
    /// acts on it — crash recovery at the tick boundary.
    pub fn evaluate(
        &self,
        world_json: &serde_json::Value,
        journal: &ResearchJournal,
    ) -> (TickReport, PressureState) {
        let state = self.load_state();
        let view = match parse_world_view(world_json) {
            Ok(v) => v,
            Err(e) => {
                return (
                    TickReport {
                        fired: false,
                        note: format!("skip: world snapshot unparseable ({e})"),
                    },
                    state,
                );
            }
        };
        let (decision, next) = evaluate_pressure(&view, journal, &state, &self.thresholds);
        if self.save_state(&next).is_err() {
            return (
                TickReport {
                    fired: false,
                    note: "skip: trigger-state persist failed".to_string(),
                },
                next,
            );
        }
        match decision {
            PressureDecision::Skip { reason } => (
                TickReport {
                    fired: false,
                    note: format!("skip: {reason}"),
                },
                next,
            ),
            PressureDecision::Fire { points, signals } => {
                let names: Vec<&str> = {
                    let mut s: Vec<&str> = signals.iter().map(|sig| sig.slug()).collect();
                    s.sort();
                    s
                };
                // No credentials = the child could not act: Skip WITHOUT
                // consuming pressure (base already advanced by the detector
                // is kept — the next tick re-accumulates from the OLD base).
                // NOTE:evaluate_pressure advanced base_* on Fire; restore the
                // pre-fire base so un-actionable pressure persists honestly.
                if self.master_token.is_none() {
                    let mut kept = next.clone();
                    kept.base_tick = state.base_tick;
                    kept.base_events = state.base_events;
                    kept.base_entities = state.base_entities;
                    kept.base_minted = state.base_minted;
                    kept.base_burned = state.base_burned;
                    kept.base_mission.clone_from(&state.base_mission);
                    kept.refuted_seen = state.refuted_seen;
                    kept.total_triggers = state.total_triggers;
                    let _ = self.save_state(&kept);
                    return (
                        TickReport {
                            fired: false,
                            note: "skip: no credentials for child (needs api_auth token)"
                                .to_string(),
                        },
                        kept,
                    );
                }
                (
                    TickReport {
                        fired: true,
                        note: format!("fired: {} ({} pts)", names.join(","), points),
                    },
                    next,
                )
            }
        }
    }

    /// Pure child argv builder. The token NEVER appears here (env only),
    /// and `--enable-live-testnet` can NEVER appear here (the trigger
    /// path is read-only/bounded by construction — test asserts it).
    #[must_use]
    pub fn child_argv(&self, tick: u64) -> Vec<String> {
        vec![
            "experiment".to_string(),
            "autonomous-cycle".to_string(),
            "--world-url".to_string(),
            self.self_url.clone(),
            "--operator".to_string(),
            self.operator_address.clone(),
            "--cycle-id".to_string(),
            format!("cycle:trigger-{tick}"),
            "--cycle-budget-wei".to_string(),
            self.cycle_budget_wei.to_string(),
        ]
    }

    /// Fire-and-forget spawn of the existing loop. Returns immediately;
    /// the child outlives the tick and seals its own evidence/learning.
    pub fn spawn_child(&self) {
        let Ok(exe) = std::env::current_exe() else {
            tracing::warn!("research-trigger: current_exe unavailable, skipping spawn");
            return;
        };
        // Tick is informational for the cycle id; the child re-reads the
        // live World anyway (its own cursor gate stays authoritative).
        let tick = self.load_state().last_evaluated_tick;
        let argv = self.child_argv(tick);
        let token = self.master_token.clone().unwrap_or_default();
        let inflight = self.inflight.clone();
        tracing::info!(
            "research-trigger: spawning autonomous-cycle (cycle:trigger-{tick}, budget {} wei)",
            self.cycle_budget_wei
        );
        tokio::spawn(async move {
            let out = tokio::process::Command::new(&exe)
                .args(&argv)
                .env(WORLD_TOKEN_ENV, &token)
                .output()
                .await;
            match out {
                Ok(o) if o.status.success() => {
                    tracing::info!("research-trigger: child cycle:trigger-{tick} exited ok");
                }
                Ok(o) => {
                    tracing::warn!(
                        "research-trigger: child cycle:trigger-{tick} exited {}: {}",
                        o.status,
                        String::from_utf8_lossy(&o.stderr)
                            .chars()
                            .take(300)
                            .collect::<String>()
                    );
                }
                Err(e) => {
                    tracing::warn!("research-trigger: child spawn failed: {e}");
                }
            }
            // SEC-01: the slot releases exactly once, however the child
            // ended (ok / non-zero / spawn error) — no stuck guard.
            inflight.store(false, Ordering::SeqCst);
        });
    }
}

/// One tick's trigger outcome (surfaced in the tick response + logs).
#[derive(Debug, Clone)]
pub struct TickReport {
    /// True when the child was spawned.
    pub fired: bool,
    /// Human-readable reason (`skip: …` / `fired: …`).
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> ResearchTriggerRuntime {
        ResearchTriggerRuntime {
            operator_address: "erd1operator".to_string(),
            cycle_budget_wei: 2_000,
            thresholds: PressureThresholds::default(),
            self_url: "http://127.0.0.1:8080".to_string(),
            master_token: Some("tok".to_string()),
            state_path: std::env::temp_dir().join(format!(
                "trigger-test-{}.json",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            journal_path: std::env::temp_dir().join("trigger-test-no-journal.json"),
            inflight: Arc::new(AtomicBool::new(false)),
        }
    }

    fn world(tick: u64, events: usize) -> serde_json::Value {
        serde_json::json!({
            "tick": tick,
            "entities": [{"id": "a"}, {"id": "b"}],
            "events": (0..events).map(|_| serde_json::json!({"kind": "e"})).collect::<Vec<_>>(),
            "mission_task_id": null,
            "treasury_minted": 0,
            "treasury_burned": 0
        })
    }

    #[test]
    fn child_argv_never_arms_live_testnet_and_never_carries_secret() {
        let r = rt();
        let argv = r.child_argv(42);
        assert!(!argv.iter().any(|a| a.contains("enable-live-testnet")));
        assert!(!argv.iter().any(|a| a.contains("tok")));
        assert!(argv.contains(&"cycle:trigger-42".to_string()));
        assert!(argv.contains(&"http://127.0.0.1:8080".to_string()));
        assert_eq!(WORLD_TOKEN_ENV, "DECENTRAAI_WORLD_TOKEN");
    }

    #[test]
    fn state_round_trips_atomically() {
        let r = rt();
        let loaded = r.load_state();
        assert!(loaded.is_fresh());
        let mut s = loaded;
        s.base_tick = 77;
        s.total_triggers = 2;
        r.save_state(&s).unwrap();
        assert_eq!(r.load_state(), s);
        assert!(!r.load_state().is_fresh());
        let _ = std::fs::remove_file(&r.state_path);
    }

    #[test]
    fn missing_journal_reads_as_empty() {
        let r = rt();
        assert_eq!(r.load_journal(), ResearchJournal::default());
    }

    #[test]
    fn tick_hook_baseline_then_fire_then_duplicate() {
        let r = rt();
        let journal = ResearchJournal::new();
        // Baseline tick: skip.
        let (rep1, _) = r.evaluate(&world(10, 1), &journal);
        assert!(!rep1.fired);
        assert!(rep1.note.contains("baseline"));
        // Pressure: drift + events → fire.
        let (rep2, _) = r.evaluate(&world(30, 9), &journal);
        assert!(rep2.fired, "{}", rep2.note);
        assert!(rep2.note.contains("fired"));
        // Same tick again: duplicate → no second spawn.
        let (rep3, _) = r.evaluate(&world(30, 9), &journal);
        assert!(!rep3.fired);
        assert!(rep3.note.contains("duplicate-tick"));
        let _ = std::fs::remove_file(&r.state_path);
    }

    #[test]
    fn no_credentials_skips_without_consuming_pressure() {
        let mut r = rt();
        r.master_token = None;
        let journal = ResearchJournal::new();
        let (rep1, _) = r.evaluate(&world(10, 1), &journal);
        assert!(!rep1.fired);
        // Fire-level pressure but no token: skip, base NOT advanced.
        let cfg_fire = PressureThresholds {
            fire_threshold: 1,
            cooldown_ticks: 0,
            ..PressureThresholds::default()
        };
        r.thresholds = cfg_fire;
        let (rep2, next) = r.evaluate(&world(40, 9), &journal);
        assert!(!rep2.fired);
        assert!(rep2.note.contains("no credentials"));
        assert_eq!(next.total_triggers, 0);
        assert_eq!(
            next.base_tick, 10,
            "pressure must persist for the next tick"
        );
        let _ = std::fs::remove_file(&r.state_path);
    }

    #[test]
    fn single_flight_gate_serializes_cycles() {
        // SEC-01: first claimant owns the cycle; the second skips WITHOUT
        // touching state; release re-arms. Deterministic, no spawn involved.
        let r = rt();
        assert!(r.try_begin_cycle(), "free slot claims");
        assert!(!r.try_begin_cycle(), "held slot refuses");
        r.end_cycle();
        assert!(r.try_begin_cycle(), "released slot claims again");
        r.end_cycle();
        let _ = std::fs::remove_file(&r.state_path);
    }

    #[test]
    fn concurrent_claimants_elect_exactly_one_owner() {
        // SEC-01 concurrency verification: N threads hammering the gate
        // elect EXACTLY one cycle owner (the tick-hook race). Outcome is
        // deterministic (exactly-once) regardless of scheduling.
        let r = rt();
        let shared = std::sync::Arc::new(r);
        let wins = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        std::thread::scope(|s| {
            for _ in 0..16 {
                let rt = std::sync::Arc::clone(&shared);
                let wins = std::sync::Arc::clone(&wins);
                s.spawn(move || {
                    if rt.try_begin_cycle() {
                        wins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(
            wins.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one concurrent claimant must own the cycle"
        );
        shared.end_cycle();
        assert!(shared.try_begin_cycle(), "slot releases after the cycle");
        shared.end_cycle();
        let _ = std::fs::remove_file(&shared.state_path);
    }
}
