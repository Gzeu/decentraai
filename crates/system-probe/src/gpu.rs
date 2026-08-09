use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum GpuProbeStatus {
    Nvidia(GpuSnapshot),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GpuSnapshot {
    pub name: String,
    pub total_vram_mib: u64,
    pub free_vram_mib: u64,
    pub utilization_percent: u8,
    pub temperature_celsius: u8,
    pub power_draw_watts: f32,
}

pub fn probe_gpu() -> GpuProbeStatus {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free,utilization.gpu,temperature.gpu,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else {
        return GpuProbeStatus::Unavailable("nvidia-smi not found".into());
    };
    if !output.status.success() {
        return GpuProbeStatus::Unavailable("nvidia-smi returned an error".into());
    }
    let binding = String::from_utf8_lossy(&output.stdout);
    let Some(line) = binding.lines().next() else {
        return GpuProbeStatus::Unavailable("nvidia-smi returned no GPU records".into());
    };
    let fields: Vec<_> = line.split(',').map(str::trim).collect();
    if fields.len() != 6 {
        return GpuProbeStatus::Unavailable("nvidia-smi returned an unexpected format".into());
    }
    let parsed = || -> Option<GpuSnapshot> {
        Some(GpuSnapshot {
            name: fields[0].to_owned(),
            total_vram_mib: fields[1].parse().ok()?,
            free_vram_mib: fields[2].parse().ok()?,
            utilization_percent: fields[3].parse().ok()?,
            temperature_celsius: fields[4].parse().ok()?,
            power_draw_watts: fields[5].parse().ok()?,
        })
    };
    parsed()
        .map(GpuProbeStatus::Nvidia)
        .unwrap_or_else(|| GpuProbeStatus::Unavailable("unable to parse nvidia-smi metrics".into()))
}
