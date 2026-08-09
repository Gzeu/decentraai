# M2 Implementation Handoff

## Scope

M2 targets local model discovery and registry support for the node CLI. The intended boundaries are:

- local filesystem scanning only;
- a deterministic local registry;
- path validation and symlink-aware filesystem safety;
- CLI integration and automated tests;
- no public P2P protocol changes;
- no remote inference activation.

## Repository areas to update

- `crates/node-cli/src/main.rs`: expose the scan/registry command surface.
- `crates/node-cli/Cargo.toml`: add only dependencies required by the CLI implementation.
- A registry module under `crates/node-cli/src/`: own model records, persistence, and scan orchestration.
- Tests adjacent to the registry/scan modules: cover valid model discovery, invalid paths, duplicate records, and symlink escape handling.
- CI: run formatting, clippy, and workspace tests after the implementation is available.

## Acceptance criteria

1. A user can request a scan of an explicit local directory.
2. The scanner never traverses outside the approved root through a symlink.
3. Invalid, missing, unreadable, or non-directory roots produce actionable errors.
4. Repeated scans are idempotent: a model is not registered twice.
5. Registry output is deterministic and includes enough local metadata to identify a model artifact.
6. The workspace passes `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`.

## Current blocker

The connected GitHub read operation returns repository file metadata and blob SHA values, but not the source text of the existing Rust files. Therefore the CLI entry point, package manifest, and project-specific M2 requirements cannot be safely edited from this integration without risking an overwrite of existing implementation.

## Needed to complete code

Provide readable contents for the following files, or restore GitHub connector responses that include file text:

- `action-plan.md`
- `Cargo.toml`
- `crates/node-cli/Cargo.toml`
- `crates/node-cli/src/main.rs`
- any existing CI workflow under `.github/workflows/`

Once available, the implementation can be produced as a focused follow-up commit.
