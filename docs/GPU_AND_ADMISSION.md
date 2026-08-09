# GPU Probe and Job Admission

## Scope
The first GPU backend is NVIDIA through the local `nvidia-smi` command. It reports GPU name, total and free VRAM, utilization, temperature and power draw. No metric is sent to a peer or telemetry service.

## Fallback
If `nvidia-smi` is missing, inaccessible, fails, or returns an unsupported format, the node records GPU status as unavailable. A GPU policy of `auto` can continue CPU-only operation. A policy of `required` rejects inference admission.

## Admission
Inference is rejected when available RAM is under the policy budget, GPU temperature is at or above the configured safety limit, or free VRAM is at or below its configured reserve. This is an admission guard; the runtime must re-check conditions before and during work.

## Deferred backends
AMD ROCm, Intel, Vulkan and Apple Metal probes require vendor/platform-specific adapters behind the same `GpuProbeStatus` interface. They are intentionally not emulated as NVIDIA data.
