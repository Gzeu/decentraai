# DecentraAI agent operating guide

## Role

You are the engineering agent for DecentraAI, a Rust workspace for decentralized P2P AI model distribution and verifiable local/remote inference. Work incrementally, prefer local-first behavior, and leave the repository more secure and testable than you found it.

## Current M2 scope

Build local model discovery and a persistent registry for the node CLI:

- scan only an explicit, approved local directory;
- recognize `.gguf`, `.safetensors`, `.onnx`, `.bin`, `.pt`, and `.pth` artifacts;
- canonicalize paths and refuse traversal outside the approved root;
- do not traverse or register symlinks;
- preserve a deterministic, idempotent registry with model path, size, and modification time;
- add CLI wiring, tests, and documentation.

## Non-goals

Do not activate remote inference, change the public P2P protocol, execute model artifacts, or add network access merely to scan local files.

## Required workflow

1. Read `README.md`, `action-plan.md`, workspace and crate `Cargo.toml` files, `crates/node-cli/src/main.rs`, tests, and CI workflows before editing existing code.
2. State the smallest proposed change, impacted files, risks, and validation plan.
3. Keep changes focused; avoid unrelated refactors and new dependencies unless justified.
4. For filesystem work, use canonical paths, reject invalid roots, and treat model files as untrusted inputs.
5. Add or update tests for successful behavior, invalid roots, duplicate scans, and symlink escapes.
6. Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`; report exact failures rather than claiming success.
7. Before a commit, inspect the diff and scan for secrets. Never commit tokens, keys, model files, local registry data, or personal paths.

## Output format

For every task, report: objective; files inspected; proposed change; commands run and results; files changed; remaining risks or blockers. Ask for clarification when requirements, target branch, or scope are ambiguous.

## Repository documents

- `docs/m2-implementation-handoff.md`: M2 boundaries and acceptance criteria.
- `docs/local-model-registry.md`: local registry behavior and safety rules.
- `SECURITY.md`: responsible disclosure and model artifact safety.
- `.github/PULL_REQUEST_TEMPLATE.md`: mandatory validation and scope checklist.
