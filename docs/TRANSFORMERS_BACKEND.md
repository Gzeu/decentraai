# Transformers Inference Backend — Technical Design

Status: IMPLEMENTED (subprocess + config + EngineKind + tests). Not yet wired
into `serve start` (Phase 5 — requires venv setup script + startup wiring in
`crates/runtime/src/lib.rs`). No model download automation yet.

## Design principle

The engine is ALWAYS an external process (never FFI). The Python subprocess
exposes an OpenAI-compatible surface so the same `InferenceBackend` adapter
used for llama-server, vLLM, SGLang and Ollama drives Transformers models with
zero runtime changes.

```text
┌─────────────────────────────────────────────────────┐
│  Node  (Rust)                                       │
│                                                     │
│  InferenceBackend (adapter)                         │
│    └─ POST /v1/chat/completions  →  base_url        │
│                                                     │
│  ┌──────────────────┐  ┌──────────────────────────┐ │
│  │ llama-server     │  │ transformers_server.py   │ │
│  │ (subprocess)     │  │ (subprocess)             │ │
│  └──────────────────┘  └──────────────────────────┘ │
│       both expose OpenAI-compatible /v1/*            │
└─────────────────────────────────────────────────────┘
```

## Files

| File | Role |
|---|---|
| `crates/runtime/src/transformers_server.py` | Python HTTP server (OpenAI-compatible) |
| `crates/runtime/src/tools.rs` | `TransformersServer` + `TransformersManager` (Rust subprocess manager) |
| `crates/config/src/lib.rs` | `TransformersSection` config struct + validation |
| `crates/fabric/src/engine.rs` | `EngineKind::Transformers` (planner + capabilities) |
| `crates/inference-adapter/src/lib.rs` | `EngineKind::Transformers` (adapter + parse) |
| `crates/runtime/src/api/fabric_intel.rs` | Model compatibility verdict |

## Config

```yaml
transformers:
  enabled: true
  model: "Qwen/Qwen2-0.5B"  # HF model ID or local path
  device: auto               # cpu | cuda | auto
```

Validation:
- `model` must not be empty when `enabled: true`
- `device` must be one of `cpu`, `cuda`, `auto`
- Config is absent by default (Transformers backend disabled)

## EngineKind::Transformers

```rust
EngineKind::Transformers.as_str()       // "transformers"
EngineKind::parse("transformers")       // Transformers
EngineKind::parse("hf")                 // Transformers
EngineKind::parse("huggingface")        // Transformers
```

Capabilities:
- `streaming: true`
- `expert_routing: false`

Model compatibility: `local_first` verdict (same as local engines).

## Python server endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/health` | GET | Health probe (`200 "ok"`) |
| `/v1/models` | GET | OpenAI-compatible model list |
| `/v1/chat/completions` | POST | Chat completion |
| `/v1/completions` | POST | Legacy text completion |

Design notes:
- Model loads lazily on first request (avoids blocking node startup)
- Threaded via `ThreadingHTTPServer` (concurrent requests handled)
- Loopback-only binding (security: same as llama-server)
- Binds to `127.0.0.1` with an ephemeral port (resolved LIVE per request)

## Subprocess lifecycle

```text
spawn(data_dir, model, device)
  ├─ write transformers_server.py → data_dir/tools/transformers/server.py
  ├─ spawn: python server.py --port {ephemeral} --model {model} --device {device}
  ├─ poll GET /health (120s timeout)
  └─ ready

stop()
  ├─ kill process + cleanup tmp file
  └─ done
```

The `ToolServer` abstraction (shared with OCR/STT/TTS/Skills) handles port
selection, process management, cleanup, and health probing.

## Setup script (not yet created)

A `scripts/setup-transformers.sh` is needed to:
1. Create `data_dir/tools/transformers/venv`
2. Install `torch`, `transformers`, `accelerate` into the venv
3. Optionally pre-download the model

This is the same pattern as `scripts/setup-skills.sh` for HF skills.

## What's missing to go live

1. **Startup wiring** in `crates/runtime/src/lib.rs`: when `transformers.enabled`
   is true AND `inference.engine` is `"transformers"`, spawn the Transformers
   server and set `backend_url` to `TransformersManager::base_url()`.

2. **scripts/setup-transformers.sh**: venv creation + dependency install.

3. **Model pre-download**: optional offline model download during setup (avoids
   first-request latency).

4. **Dashboard status**: add `transformers` section to `/status` output when
   the backend is enabled.

## Security

- Subprocess runs on loopback only (never exposed to network)
- No private keys stored in the Python process or passed via args
- Model loaded from HuggingFace hub (trust_remote_code for some models)
- Prompts/outputs are NEVER logged (security policy: only counters + latencies)
- Python venv is isolated (no system-wide pip install)
