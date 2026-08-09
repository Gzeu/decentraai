# Initial Execution Plan

## M0: repository bootstrap
- Create Rust workspace and workspace-wide format, lint, test, dependency-audit and secret-scan commands.
- Add configuration schema, example configuration, architecture, threat model, protocol and ADR documentation.

## M1: identity and system doctor
- Implement `decentraai init` to create identity and local directory structure.
- Implement `decentraai doctor` to report supported backends, CPU/RAM/GPU/disk, network status, and computed safe limits.
- Persist no private hardware details to public network announcements.

## M2: verified artifact core
- Implement manifest v1, canonical encoding, chunk hashing, Merkle verification, staging, quarantine, and model registry.
- Implement `scan`, `publish`, and `verify` CLI commands.

## M3: two-node LAN transfer
- Implement private libp2p transport and mDNS discovery.
- Implement manifest and chunk request/response messages with strict validation.
- Implement resumable concurrent downloads and peer scoring.

## M4: local inference
- Implement process-isolated llama-server adapter, localhost API, queue, cancellation, and resource governor.
- Do not enable remote inference until M3 and M4 adversarial tests pass.

## Definition of done
A milestone is done only when implementation, automated tests, error handling, documentation, and relevant security controls are committed together.
