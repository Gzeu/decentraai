# M10 File Map

This map prevents duplicate architecture and directs the agent to existing extension points.

## Existing areas to inspect first

| Area | Existing location | Agent responsibility |
|---|---|---|
| CLI/node lifecycle | `crates/node-cli/src/main.rs` | Wire real startup/config/API without hiding errors |
| P2P transport | `crates/p2p` | Verify handler context and transport peer identity |
| Protocol types | `crates/protocol` | Extend versioned messages/lifecycle types |
| Identity | `crates/identity` | Reuse signing and verification; do not duplicate keys |
| Distributed routing | `crates/distributed/src` | Integrate adapter, queues, routing and fallback |
| Registry/models | `crates/registry`, `crates/manifest` | Resolve model hash and readiness |
| Discovery/trust | `crates/discovery` | Reuse pairing/trust persistence and scheduler primitives |
| Audit | `crates/audit` | Emit request/admin events |
| Monitoring | `crates/monitoring` | Add metrics without a second metrics system |
| Frontend | `frontend/src` | Consume API contracts and live events |

## New files only when justified

| Concern | Preferred path | Notes |
|---|---|---|
| Backend adapter | `crates/inference-adapter/src` | Add workspace member only if no existing suitable crate exists |
| HTTP/control plane | `crates/api/src` or existing node API | First inspect current node server dependencies |
| Security policy | `crates/security/src` or existing identity/discovery boundary | Avoid duplicated token/signature validation |
| E2E harness | `tests/e2e` | Use deterministic backend fixture for CI |
| Deployment | `deploy/` | Every referenced file must exist or be marked opt-in |

## Contract ownership

- Wire protocol owns P2P serialization and compatibility.
- API owns HTTP DTOs, auth errors and SSE event format.
- Backend adapter owns provider-specific JSON.
- Frontend owns presentation state, not routing/security policy.
- Audit owns event schema; monitoring owns measurements.

## Forbidden shortcuts

- Frontend-to-worker direct calls.
- A second identity/key format.
- Provider JSON types in protocol messages.
- Logging bearer tokens or full prompts by default.
- Making worker registration imply backend readiness.
- Adding tokenomics to routing before reliability tests pass.

## PR slicing

1. `feat(adapter): add backend trait and deterministic HTTP adapter`
2. `fix(distributed): wire real inference handler and lifecycle`
3. `feat(security): bind worker announcements to authenticated peers`
4. `feat(api): expose inference and operational endpoints`
5. `feat(frontend): connect chat and worker operations`
6. `test(e2e): add two-node lifecycle and failure scenarios`
7. `chore(deploy): validate local stack and operator documentation`
