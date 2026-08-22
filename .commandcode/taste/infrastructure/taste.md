# Infrastructure

## Production VPS (decentraai project)
- Connect with `ssh -o ConnectTimeout=10 decentraai@169.58.213.145`; root login is refused post-provisioning — don't retry root@. Confidence: 0.85
- Master API token lives on the VPS at `~/.decentraai/runtime/api.token` — read it into a shell variable and never echo/display it when calling authenticated endpoints. Confidence: 0.8
- Node runs as systemd unit `decentraai-node`, local API at http://127.0.0.1:8080 with Bearer master-token auth; `/v1/chat/completions` requires the exact served model filename (e.g. `Qwen3.5-4B-Q4_K_M.gguf`). Confidence: 0.75
- `/v1/peers` returns `[]` even when the mesh is fully connected — don't treat it as ground truth for mesh health; verify via `/v1/compute` workers list or a real end-to-end inference through the coordinator. Confidence: 0.6
- VPS is CPU-only and slow for big contexts: Qwen3.5-4B generates ~7.4 tok/s with prefill ~10–23 tok/s (fine for short chat turns / bootstrap-coordinator role); local laptop runs qwen2.5-3b at ~37 tok/s — route long-prompt interactive workloads through mesh workers, keep the VPS for coordination and short turns. Confidence: 0.55
