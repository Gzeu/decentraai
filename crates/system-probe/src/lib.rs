mod gpu;

pub use gpu::{GpuProbeStatus, GpuSnapshot, probe_gpu};

use decentraai_config::{GpuPolicy, ResourceSection};
use sysinfo::{Disks, System};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Reads the real battery charge percentage (0..100) on Linux, when the node
/// has a battery (mobile/laptop). Returns `None` on desktop / no battery /
/// unreadable — honest UNKNOWN, never fabricated. Used by the worker to
/// advertise `battery_percent` so the adaptive-contribution planner sends a
/// low-battery worker less work.
///
/// Sources, in order: `/sys/class/power_supply/*/capacity` (the first battery
/// with a valid percentage). A `capacity` of 0 or a path that is clearly a
/// charger (e.g. `AC`, `ADP`, `Mains`) is not a battery and is skipped.
pub fn probe_battery() -> Option<u8> {
    probe_battery_at(std::path::Path::new("/sys/class/power_supply"))
}

/// Pure battery-probe over an explicit sysfs-like directory, so tests drive
/// the real parsing/min-selection logic with synthetic entries. See
/// [`probe_battery`].
fn probe_battery_at(dir: &std::path::Path) -> Option<u8> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut batteries: Vec<u8> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        // Skip obvious AC/charger entries, not batteries.
        if name.starts_with("ac")
            || name.starts_with("adp")
            || name.contains("mains")
            || name.contains("charger")
        {
            continue;
        }
        let capacity_path = path.join("capacity");
        let Ok(raw) = std::fs::read_to_string(&capacity_path) else {
            continue;
        };
        let parsed: u8 = raw.trim().parse().ok()?;
        if parsed <= 100 {
            batteries.push(parsed);
        }
    }
    // If there are multiple batteries, report the minimum (most conservative:
    // the least charged cell bounds the device's usable life).
    batteries.into_iter().min()
}

#[derive(Debug, Clone, Copy)]
pub struct SystemSnapshot {
    pub logical_cpus: usize,
    pub cpu_usage_percent: f32,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub used_swap_bytes: u64,
    pub total_disk_free_bytes: u64,
    /// Real battery charge percentage (0..100), when the node has a battery.
    /// `None` on desktop / no battery / unreadable (UNKNOWN).
    pub battery_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceBudget {
    pub max_cpu_threads: usize,
    pub max_memory_bytes: u64,
    pub max_cache_bytes: u64,
    pub max_upload_mbps: u32,
    pub max_download_mbps: u32,
    pub gpu_policy: GpuPolicy,
    pub gpu_vram_limit_percent: u8,
    pub gpu_vram_reserve_bytes: u64,
    /// Absolute floor for free RAM; inference is rejected below it.
    pub ram_reserve_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdmissionDecision {
    Admit,
    Reject(String),
}

impl SystemSnapshot {
    pub fn collect() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        let disks = Disks::new_with_refreshed_list();
        Self {
            logical_cpus: system.cpus().len().max(1),
            cpu_usage_percent: system.global_cpu_usage(),
            total_memory_bytes: system.total_memory(),
            available_memory_bytes: system.available_memory(),
            used_swap_bytes: system.used_swap(),
            total_disk_free_bytes: disks.list().iter().map(|disk| disk.available_space()).sum(),
            battery_percent: probe_battery(),
        }
    }

    pub fn derive_budget(
        &self,
        policy: &ResourceSection,
        max_cache_gb: u32,
        min_free_disk_gb: u32,
    ) -> ResourceBudget {
        let reserved_cpu =
            usize::from(policy.reserve_cpu_cores).min(self.logical_cpus.saturating_sub(1));
        let usable_threads = self.logical_cpus.saturating_sub(reserved_cpu).max(1);
        ResourceBudget {
            max_cpu_threads: ((usable_threads * usize::from(policy.cpu_max_percent)) / 100).max(1),
            max_memory_bytes: self
                .available_memory_bytes
                .saturating_sub(u64::from(policy.reserve_ram_mb) * MIB)
                .saturating_mul(u64::from(policy.memory_max_percent))
                / 100,
            max_cache_bytes: (u64::from(max_cache_gb) * GIB).min(
                self.total_disk_free_bytes
                    .saturating_sub(u64::from(min_free_disk_gb) * GIB),
            ),
            max_upload_mbps: policy.max_upload_mbps,
            max_download_mbps: policy.max_download_mbps,
            gpu_policy: policy.gpu_enabled,
            gpu_vram_limit_percent: policy.gpu_max_vram_percent,
            gpu_vram_reserve_bytes: u64::from(policy.reserve_vram_mb) * MIB,
            ram_reserve_bytes: u64::from(policy.reserve_ram_mb) * MIB,
        }
    }

    pub fn admit_inference(
        &self,
        budget: &ResourceBudget,
        gpu: &GpuProbeStatus,
        temperature_limit: u8,
    ) -> AdmissionDecision {
        if self.available_memory_bytes < budget.ram_reserve_bytes {
            return AdmissionDecision::Reject(
                "available RAM is below the configured reserve".into(),
            );
        }
        match gpu {
            GpuProbeStatus::Unavailable(_) if budget.gpu_policy == GpuPolicy::Required => {
                AdmissionDecision::Reject("GPU is required by policy but unavailable".into())
            }
            GpuProbeStatus::Nvidia(info) if info.temperature_celsius >= temperature_limit => {
                AdmissionDecision::Reject(
                    "GPU temperature exceeds the configured safety limit".into(),
                )
            }
            GpuProbeStatus::Nvidia(info)
                if info.free_vram_mib * MIB <= budget.gpu_vram_reserve_bytes =>
            {
                AdmissionDecision::Reject("free VRAM is below the configured reserve".into())
            }
            _ => AdmissionDecision::Admit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ResourceBudget {
        ResourceBudget {
            max_cpu_threads: 2,
            max_memory_bytes: 2 * GIB,
            max_cache_bytes: GIB,
            max_upload_mbps: 20,
            max_download_mbps: 80,
            gpu_policy: GpuPolicy::Required,
            gpu_vram_limit_percent: 75,
            gpu_vram_reserve_bytes: 1024 * MIB,
            ram_reserve_bytes: 1024 * MIB,
        }
    }

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            logical_cpus: 4,
            cpu_usage_percent: 0.0,
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: 4 * GIB,
            used_swap_bytes: 0,
            total_disk_free_bytes: 50 * GIB,
            battery_percent: None,
        }
    }

    fn policy() -> ResourceSection {
        ResourceSection {
            cpu_max_percent: 50,
            reserve_cpu_cores: 2,
            memory_max_percent: 50,
            reserve_ram_mb: 1024,
            gpu_enabled: GpuPolicy::Auto,
            gpu_max_vram_percent: 75,
            reserve_vram_mb: 1024,
            stop_gpu_temperature_celsius: 83,
            max_upload_mbps: 20,
            max_download_mbps: 80,
        }
    }

    #[test]
    fn rejects_required_gpu_when_unavailable() {
        assert!(matches!(
            snapshot().admit_inference(
                &budget(),
                &GpuProbeStatus::Unavailable("missing".into()),
                83
            ),
            AdmissionDecision::Reject(_)
        ));
    }

    #[test]
    fn rejects_hot_gpu() {
        let gpu = GpuProbeStatus::Nvidia(GpuSnapshot {
            name: "test".into(),
            total_vram_mib: 8192,
            free_vram_mib: 4096,
            utilization_percent: 20,
            temperature_celsius: 85,
            power_draw_watts: 100.0,
        });
        assert!(matches!(
            snapshot().admit_inference(&budget(), &gpu, 83),
            AdmissionDecision::Reject(_)
        ));
    }

    #[test]
    fn rejects_when_ram_below_reserve() {
        let low_ram = SystemSnapshot {
            available_memory_bytes: 512 * MIB,
            ..snapshot()
        };
        let decision =
            low_ram.admit_inference(&budget(), &GpuProbeStatus::Unavailable("none".into()), 83);
        match decision {
            AdmissionDecision::Reject(reason) => assert!(reason.contains("RAM")),
            AdmissionDecision::Admit => {
                panic!("512 MiB free must be rejected with a 1 GiB reserve")
            }
        }
    }

    #[test]
    fn admits_when_ram_meets_reserve() {
        let mut flexible = budget();
        flexible.gpu_policy = GpuPolicy::Auto;
        assert_eq!(
            snapshot().admit_inference(&flexible, &GpuProbeStatus::Unavailable("none".into()), 83),
            AdmissionDecision::Admit
        );
    }

    #[test]
    fn budget_respects_cpu_memory_and_disk_reserves() {
        let snapshot = SystemSnapshot {
            logical_cpus: 8,
            cpu_usage_percent: 10.0,
            total_memory_bytes: 16 * GIB,
            available_memory_bytes: 12 * GIB,
            used_swap_bytes: 0,
            total_disk_free_bytes: 200 * GIB,
            battery_percent: None,
        };
        let budget = snapshot.derive_budget(&policy(), 100, 20);
        assert_eq!(budget.max_cpu_threads, 3);
        assert_eq!(budget.max_memory_bytes, 11 * GIB / 2);
        assert_eq!(budget.max_cache_bytes, 100 * GIB);
        assert_eq!(budget.ram_reserve_bytes, 1024 * MIB);
    }

    #[test]
    fn cache_never_consumes_reserved_free_space() {
        let snapshot = SystemSnapshot {
            logical_cpus: 2,
            cpu_usage_percent: 0.0,
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: 4 * GIB,
            used_swap_bytes: 0,
            total_disk_free_bytes: 25 * GIB,
            battery_percent: None,
        };
        let budget = snapshot.derive_budget(&policy(), 100, 20);
        assert_eq!(budget.max_cache_bytes, 5 * GIB);
    }

    #[test]
    fn battery_probe_reads_real_capacity_and_skips_chargers() {
        let dir = tempfile::tempdir().unwrap();
        // A real battery reports capacity.
        let bat = dir.path().join("BAT0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::write(bat.join("capacity"), "42\n").unwrap();
        // A charger must be skipped (not a battery).
        let ac = dir.path().join("AC");
        std::fs::create_dir_all(&ac).unwrap();
        std::fs::write(ac.join("capacity"), "100\n").unwrap();

        assert_eq!(probe_battery_at(dir.path()), Some(42));
    }

    #[test]
    fn battery_probe_reports_conservative_min_across_cells() {
        let dir = tempfile::tempdir().unwrap();
        for (name, cap) in [("BAT0", "80\n"), ("BAT1", "35\n")] {
            let b = dir.path().join(name);
            std::fs::create_dir_all(&b).unwrap();
            std::fs::write(b.join("capacity"), cap).unwrap();
        }
        // Two cells: report the least charged (most conservative).
        assert_eq!(probe_battery_at(dir.path()), Some(35));
    }

    #[test]
    fn battery_probe_returns_none_without_a_battery() {
        let dir = tempfile::tempdir().unwrap();
        // A directory with only a charger -> no battery.
        let ac = dir.path().join("ADP1");
        std::fs::create_dir_all(&ac).unwrap();
        std::fs::write(ac.join("capacity"), "100\n").unwrap();
        assert_eq!(probe_battery_at(dir.path()), None);

        // Missing directory -> None (honest UNKNOWN).
        assert_eq!(probe_battery_at(&dir.path().join("does-not-exist")), None);
    }
}
