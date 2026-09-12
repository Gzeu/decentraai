//! MultiversX Supernova observer wiring for the node (O1-step2b).
//!
//! Read-only by construction: periodic [`poll_once`] snapshots served on
//! `GET /v1/mx/status`, on-demand [`track`] on `GET /v1/mx/track`.
//! Absent/disabled config = no state, endpoints 404, zero Proxy traffic.
//! This module never signs, never submits, never reads secrets.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use decentraai_mx_supernova::{MxProxy, MxTrack, ObserverConfig, ObserverSnapshot, SupernovaError};

/// Node-side observer handle: validated config + client + last snapshot.
pub struct MxObserverHandle {
    cfg: ObserverConfig,
    proxy: MxProxy,
    snapshot: tokio::sync::RwLock<Option<ObserverSnapshot>>,
}

impl MxObserverHandle {
    /// Build from the validated `mx_supernova` config section.
    /// `Ok(None)` when absent/disabled — the caller attaches nothing.
    pub fn from_section(
        cfg: &decentraai_config::MxSupernovaSection,
    ) -> Result<Option<Arc<Self>>, String> {
        if !cfg.enabled {
            return Ok(None);
        }
        let oc = ObserverConfig::new(
            &cfg.api_base,
            cfg.shard,
            cfg.poll_interval_secs * 1000,
            cfg.timeout_ms,
            cfg.max_bytes,
            match (cfg.enable_epoch, cfg.enable_round) {
                (Some(e), Some(r)) => Some(decentraai_mx_supernova::ActivationConfig {
                    supernova_enable_epoch: e,
                    supernova_enable_round: r,
                }),
                _ => None,
            },
            Some(cfg.chain_id.clone()),
        )
        .map_err(|e| format!("mx_supernova: {e}"))?;
        let proxy = oc.proxy().map_err(|e| format!("mx_supernova: {e}"))?;
        Ok(Some(Arc::new(Self {
            cfg: oc,
            proxy,
            snapshot: tokio::sync::RwLock::new(None),
        })))
    }

    /// Poll interval backing this handle (ms).
    pub fn poll_interval_ms(&self) -> u64 {
        self.cfg.poll_interval_ms
    }

    /// Spawn the bounded background poll loop. Failures are transient: the
    /// last snapshot stays (staleness is visible via `at_ms`).
    pub fn spawn(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(me.cfg.poll_interval_ms));
            loop {
                interval.tick().await;
                match decentraai_mx_supernova::poll_once(&me.proxy, &me.cfg, now_ms()).await {
                    Ok(snap) => {
                        *me.snapshot.write().await = Some(snap);
                    }
                    Err(e) => {
                        tracing::warn!("mx observer poll failed (transient): {e}");
                    }
                }
            }
        });
    }

    /// Last snapshot (`None` = no successful poll yet).
    pub async fn last(&self) -> Option<ObserverSnapshot> {
        self.snapshot.read().await.clone()
    }

    /// Track one tx hash. Uses the cached snapshot for the activation flag;
    /// polls once on demand when nothing was observed yet (bounded).
    pub async fn track(&self, hash: &str) -> Result<MxTrack, SupernovaError> {
        let snap = match self.last().await {
            Some(s) => s,
            None => {
                let fresh =
                    decentraai_mx_supernova::poll_once(&self.proxy, &self.cfg, now_ms()).await?;
                *self.snapshot.write().await = Some(fresh.clone());
                fresh
            }
        };
        decentraai_mx_supernova::track(&self.proxy, &snap, hash).await
    }
}

/// Wall-clock milliseconds (snapshot staleness marker only).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> decentraai_config::MxSupernovaSection {
        decentraai_config::MxSupernovaSection {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn disabled_section_attaches_nothing() {
        let off = decentraai_config::MxSupernovaSection::default();
        assert!(!off.enabled);
        assert!(MxObserverHandle::from_section(&off).unwrap().is_none());
    }

    #[test]
    fn enabled_section_builds_handle() {
        let h = MxObserverHandle::from_section(&section()).unwrap().unwrap();
        assert_eq!(h.poll_interval_ms(), 30_000);
    }

    #[test]
    fn bad_base_refused_at_attach() {
        let mut bad = section();
        bad.api_base = "ftp://x".to_string();
        assert!(MxObserverHandle::from_section(&bad).is_err());
    }
}
