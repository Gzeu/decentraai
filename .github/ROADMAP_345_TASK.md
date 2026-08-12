# Agent Task: Execute Roadmap 345

## Mission

Implement DecentraAI as a secure decentralized intelligence fabric. Use `docs/ROADMAP_345_DECENTRALIZED_INTELLIGENCE.md` for scope and `docs/ROADMAP_345_EXECUTION_TRACKER.md` as the status ledger.

## First action

Read `AGENTS.md`, the 345-step roadmap, the execution tracker and `docs/ARCHITECTURE_OWNERSHIP.md`. Inspect the actual workspace, protocols, config loader, P2P handlers and frontend before editing. Select the lowest-numbered incomplete step whose dependencies are satisfied.

## Execution loop

```text
load tracker
→ select lowest incomplete dependency-safe step
→ inspect extension point
→ write smallest production-safe change
→ add unit/integration/security tests
→ run validation
→ update tracker with evidence
→ create focused commit
→ repeat
```

Do not jump to M11 optimization, economy or autonomous governance while M10 real inference is unvalidated, except for documentation or non-invasive scaffolding.

## 100% completion rule

A step reaches `100% COMPLETE` only when:

- implementation exists in the correct architecture boundary;
- unit tests pass;
- boundary integration tests pass;
- negative/security tests pass where relevant;
- release-gate E2E evidence exists;
- metrics/audit behavior is verified;
- docs and operator procedure are updated;
- rollback/failure behavior is documented;
- commit and exact commands are recorded in the tracker.

Never claim completion from a README, type, stub, mock, generated file, `cargo check` alone, simulated frontend data or unvalidated Docker file.

## PR template

```text
Title:
Roadmap steps:
Architecture boundary:
Files changed:
State-machine changes:
Protocol/config changes:
Tests:
Security review:
Observability:
Manual reproduction:
Rollback:
Known limitations:
Tracker updates:
```

## Required implementation order

1. Steps 1-30: identity, discovery and pairing.
2. Steps 31-58: capabilities and model registry.
3. Steps 59-96: admission, planning and reservations.
4. Steps 97-150: real dispatch, execution, streaming and recovery.
5. Steps 151-198: privacy, model supply chain and tenancy.
6. Steps 199-254: consistency, fault tolerance and security.
7. Steps 255-301: reputation, policy, metrics and audit.
8. Steps 302-330: compatibility and adaptive optimization.
9. Steps 331-345: agent operations and governance.

## M10 hard gate

A two-node LAN test must prove:

```text
pair → approve → publish model → reserve → dispatch
→ real backend inference → ordered streaming
→ cancellation/timeout → terminal event
→ metrics/audit → worker failure/recovery
```

No mock response is allowed on the production path. The P2P handler must be attached before node startup. Queue state must be shared, bounded and cancellation-safe.

## M11 hard gate

Do not enable tensor/pipeline parallelism or speculative execution by default until M10 E2E passes, capabilities are verified, topology is trusted and low-latency, benchmark artifacts show improvement, fallback is tested and unsafe public-peer sharding is rejected.

## Security requirements

Bind announcement PeerId to transport PeerId. Verify signatures, nonce and expiry. Reject stale, forged, revoked and malformed peers. Enforce tenant, model, context, token and locality policies. Do not log prompts, bearer tokens or private keys. Sandbox engines and restrict filesystem/GPU access. Keep destructive agent operations behind explicit approval.

## Validation commands

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd frontend && npm ci && npm run check && npm run build
```

For deployment changes:

```bash
docker compose -f deploy/docker-compose.m10.yml config
```

## Tracker update format

```text
Step: N
Status: VERIFIED
Completion: 100%
Commit: <sha>
Implementation: <files>
Tests: <commands and result>
E2E: <scenario and result>
Security: <negative tests/review>
Operations: <manual procedure>
Next: none
```

For incomplete work, record what exists, what prevents 100%, the next action and blocking step IDs.

## Stop conditions

Stop and report instead of improvising when an API contradicts the roadmap, a security boundary is unclear, a config key is unsupported, a Docker reference is missing, a dependency upgrade changes protocol behavior, a test needs a real secret, a retry could duplicate completed inference, a model artifact cannot be verified or a change would weaken trust checks.

## Final completion report

Before each release PR report completed step IDs and percentages, blocked steps, commits/files, validation commands, skipped tests, security findings, benchmarks, deployment/rollback, limitations and the next milestone.
