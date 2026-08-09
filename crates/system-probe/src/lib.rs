use decentraai_config::ResourceSection;
use sysinfo::{Disks, System};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub logical_cpus: usize,
    pub cpu_usage_percent: f32,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub used_swap_bytes: u64,
    pub total_disk_free_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct ResourceBudget {
    pub max_cpu_threads: usize,
    pub max_memory_bytes: u64,
    pub max_cache_bytes: u64,
    pub max_upload_mbps: u32,
    pub max_download_mbps: u32,
    pub gpu_policy: String,
    pub gpu_vram_limit_percent: u8,
    pub gpu_vram_reserve_bytes: u64,
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

    pub fn derive_budget(&self, policy: &ResourceSection, max_cache_gb: u32, min_free_disk_gb: u32) -> ResourceBudget {
        let reserved_cpu = usize::from(policy.reserve_cpu_cores).min(self.logical_cpus.saturating_sub(1));
        let usable_threads = self.logical_cpus.saturating_sub(reserved_cpu).max(1);
        let max_cpu_threads = ((usable_threads * usize::from(policy.cpu_max_percent)) / 100).max(1);

        let reserve_memory = u64::from(policy.reserve_ram_mb) * MIB;
        let memory_after_reserve = self.available_memory_bytes.saturating_sub(reserve_memory);
        let max_memory_bytes = memory_after_reserve.saturating_mul(u64::from(policy.memory_max_percent)) / 100;

        let requested_cache = u64::from(max_cache_gb) * GIB;
        let minimum_free = u64::from(min_free_disk_gb) * GIB;
        let max_cache_bytes = requested_cache.min(self.total_disk_free_bytes.saturating_sub(minimum_free));

        ResourceBudget {
            max_cpu_threads,
            max_memory_bytes,
            max_cache_bytes,
            max_upload_mbps: policy.max_upload_mbps,
            max_download_mbps: policy.max_download_mbps,
            gpu_policy: policy.gpu_enabled.clone(),
            gpu_vram_limit_percent: policy.gpu_max_vram_percent,
            gpu_vram_reserve_bytes: u64::from(policy.reserve_vram_mb) * MIB,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ResourceSection {
        ResourceSection {
            cpu_max_percent: 50,
            reserve_cpu_cores: 2,
            memory_max_percent: 50,
            reserve_ram_mb: 1024,
            gpu_enabled: "auto".into(),
            gpu_max_vram_percent: 75,
            reserve_vram_mb: 1024,
            stop_gpu_temperature_celsius: 83,
            max_upload_mbps: 20,
            max_download_mbps: 80,
        }
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
