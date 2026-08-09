# Resource Analyzer

## Purpose
The resource analyzer creates a local snapshot and derives a safe resource budget before a node downloads models, seeds artifacts, or starts inference. It is local-only and does not send raw hardware details to peers.

## Inputs
- Logical CPU count and current aggregate CPU use.
- Available RAM and swap use.
- Free disk space across mounted disks.
- Node configuration limits and reserves.

## Budget rules
- Reserve configured CPU cores, then apply `cpu_max_percent` to remaining logical CPUs.
- Reserve `reserve_ram_mb`, then apply `memory_max_percent` to currently available RAM.
- Preserve `min_free_disk_gb`; cache budget is the smaller of configured cache capacity and remaining disk space after that reserve.
- Network limits come exclusively from configuration.
- GPU policy is reported now; vendor-specific GPU, VRAM, temperature and power collection is deferred until a portable backend abstraction is selected.

## Usage
```bash
cargo run -p decentraai-cli -- config validate
cargo run -p decentraai-cli -- doctor --config configs/node.example.yaml
```

## Safety
The computed budget is a ceiling, not permission to immediately consume all resources. Future transfer and inference schedulers must re-check current memory, disk and GPU pressure before admitting work.
