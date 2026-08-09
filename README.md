# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.

## v1 scope
- Content-addressed distribution for GGUF models.
- Signed manifests, chunk hashes, Merkle-root verification, resumable multi-peer transfers.
- Local inference through an isolated llama.cpp/llama-server runtime.
- Private-swarm remote inference where a worker serves a complete verified model.
- Hardware detection, resource governance, privacy-preserving telemetry, and local peer reputation.

## Explicit non-goals for v1
- Public DHT by default.
- Pipeline or tensor parallelism across the public Internet.
- On-chain token, payments, or public marketplace.
- Execution of peer-supplied code, plugins, or unverified updates.

## First acceptance scenario
1. Start two nodes on a LAN.
2. Node A publishes a signed GGUF manifest and verified chunks.
3. Node B discovers A, downloads chunks with resume support, and verifies every chunk plus the final Merkle root.
4. Node B runs the verified model locally via an OpenAI-compatible localhost API.
5. Resource limits pause transfer/inference under disk, RAM, VRAM, temperature, or policy pressure.

## Documentation
- `docs/ARCHITECTURE.md`
- `docs/THREAT_MODEL.md`
- `docs/ROADMAP.md`
- `action-plan.md`
- `docs/m2-implementation-handoff.md`
- `docs/local-model-registry.md`

## Current Status

### M2: Local Model Discovery ✅
- **Implemented**: Local model scanning and persistent registry
- **Supported formats**: `.gguf`, `.safetensors`, `.onnx`, `.bin`, `.pt`, `.pth`
- **CLI commands**:
  - `decentraai scan --directory <path> --registry <path>`: Scan directory for models
  - `decentraai registry list --registry <path>`: List registered models
- **Safety features**: Path validation, symlink rejection, canonical paths, idempotent operations

### Previous Milestones
- **M0**: Repository bootstrap ✅
- **M1**: Identity and system doctor ✅

## Security baseline
No artifact is usable before hash, manifest, and policy verification. Prompts and outputs are never logged by default. Private keys never enter Git or telemetry.
