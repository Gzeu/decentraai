mod gpu;

pub use gpu::{GpuProbeStatus, GpuSnapshot, probe_gpu};

use decentraai_config::{GpuPolicy, ResourceSection};
use sysinfo::{Disks, System};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Debug, Clone, Copy)]
pub struct SystemSnapshot {
    pub logical_cpus: usize,
    pub cpu_usage_percent: f32,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub used_swap_bytes: u64,
    pub total_disk_free_bytes: u64,
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
        };
        let budget = snapshot.derive_budget(&policy(), 100, 20);
        assert_eq!(budget.max_cache_bytes, 5 * GIB);
    }
}
