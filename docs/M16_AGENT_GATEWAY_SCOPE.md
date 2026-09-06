# M16 Agent Gateway — Scope v0.1 (BYOA)

> Status: **SCOPE — no implementation.** Nothing below changes code. Each
> Phase-2 step ends with gates + report + STOP for review.
> Base: `main = 3557a6c` (SEC-01/SEC-02 closed, prod resynced). PR #94 stays
> an external review — untouched, unmerged; nothing here depends on it.

## 0. Invariant (non-negotiable)

```text
AI proposes → deterministic Rust decides → evidence verifies → execution
only through an authorized capability.
```

The Gateway MUST NOT become an alternate path around Governor, policy,
authorization, evidence, or kill-switch. Every new surface is a *client* of
the existing seams, never a bypass. Where this doc says "agent requests X",
the very next sentence names the deterministic decider.

## 1. Boundary map (verified current state — reuse, don't duplicate)

| Layer | Lives in | Decides | Gateway relationship |
|---|---|---|---|
| `AgentRuntime` | `crates/agent-runtime` (`AgentConfig{capabilities}`, `spawn`, `AgentHandle`) | local agent lifecycle + capability validation at `spawn()` | Gateway onboards EXTERNAL agents; never reimplements spawn. External agents are cognitive identities mapped onto gateway credentials, not runtime handles. |
| Governor | `crates/distributed/src/mp.rs` (`GovernorVerdict`: Local/Distributed/Queue/Reject) + `governor_execute_handler` | placement + saturation fate, deterministically from resource state | Any gateway-triggered work that needs placement goes through the Governor verdict. No direct-to-engine execution from gateway credentials. |
| MCP read surface | `crates/runtime/src/mcp.rs` (`all_tools`, `McpContext` precomputed snapshots) + `mcp_handler` (master) / `mcp_consumer_handler` (`dca_`) | what is visible, per credential | New agent-scoped tools extend `all_tools` + context; auth dispatch extends the two-handler pattern, never a third auth universe. |
| Policy gates | `crates/agents/src/policy.rs` (`PolicyEngine`: model allowlists, exploration egress) + `*_gate` attach points | what an agent may attempt | Gateway credentials are checked against the SAME gates (policy_gate on Delegate, check_model on local agents). |
| Credentials | `dca_` consumer keys (`crates/tokens/src/consumer.rs`: BLAKE3-hashed, revocable by id, optional expiry enforced at auth) · wallet sessions (`wallet_auth.rs`: challenge→verify→session, expiries enforced) · master token (0600 file) | who may act, until when | BYOA credentials are a NEW credential KIND reusing these three primitives: hashed-at-rest, revocable, expiring. No new crypto, no new storage format. |
| Audit/evidence | `crates/audit` (`record`/`record_best_effort`, append-only) + per-domain evidence (Hub execution evidence, testnet `ExperimentEvidence`, governor `ExecutedPlan`) | what happened, provably | Every gateway-mutating call appends audit + returns evidence ids. No evidence → the call didn't happen (same rule as the research loop). |
| Economic lanes | proposal/testnet lane (kill-switch `--enable-live-testnet`, budgets, allowlists) · M18 escrow/contracts · sweep (F1 typed errors, F2 resubmit cap) | money movement | Gateway NEVER arms economic execution. Testnet-economic stays behind its existing explicit flag, invoked only by the operator CLI — not reachable from any agent credential. |

Verified boundaries the scope preserves: *"wallet sessions are read-only in
MCP; use dca_ for execution or mutations"*; `execute_decision` requires
master (test `mcp_execute_decision_requires_master_not_operator`); consumer
`decide` is read-only (test `mcp_consumer_can_decide_read_only`); consumer
execute enforces quota + releases on failure
(test `mcp_consumer_execute_enforces_quota_and_releases_on_failure`).

## 2. Capability / RBAC model + least privilege

- The unit of authorization is the **capability** (`CapabilityKind` taxonomy
  in `decentraai_hub`), never a model name, never a raw endpoint.
- Credential kinds form a strict ladder (each rung inherits NOTHING upward):
  1. `agent` (NEW, BYOA): only capabilities explicitly granted, all
     quota-gated, all evidence-sealed. Default: ZERO capabilities.
  2. `dca_` consumer: existing quota + capability tools (unchanged).
  3. wallet session: existing scopes, read-only in MCP (unchanged).
  4. operator/master: control-plane (unchanged, never delegable).
- Least privilege is structural: the gateway credential carries an explicit
  capability set; the MCP dispatcher intersects requested tool × granted
  set × policy gate. Deny is the default branch (closed schema,
  `deny_unknown_fields`, unknown tool = invalid params, as today).
- No ambient authority: possession of a gateway credential never implies
  master, never implies quota beyond its ceiling, never implies other
  agents' scopes.

## 3. BYOA credentials: scope, TTL, revocation, zeroization, log hygiene

- **Issuance**: operator-only (master) `gateway issue` command: agent name,
  capability set (must be non-empty, must exist in taxonomy), quota ceiling,
  TTL. Returns the secret ONCE (same ceremony as `token create`).
- **Storage**: secret hashed at rest (BLAKE3, same posture as `dca_` keys
  and the reputation/API-token files). Plaintext exists only at issuance
  output and in the agent's hands.
- **TTL**: MANDATORY expiry (unlike `dca_` where expiry is optional).
  Exact values are **policy constants M16 v0.1** — a security specification,
  NOT measured results: `default = 7 days`, `absolute max = 30 days`,
  minimum 1 day. Values outside `[1d, 30d]` are rejected at issuance.
  These constants are tunable only through a scope revision (future
  measurement may justify change; no implementation may silently drift
  them). Expired = unauthenticated, same code path as revoked.
- **Revocation**: by credential id, immediate effect (checked at auth time
  on every call — no caching of auth decisions beyond the request).
- **Zeroization**: secret buffers zeroed after hashing/comparison
  (`zeroize`-idiom; same class as the wallet-seed hardening already
  applied). No secret in `Debug`, `Display`, or error strings (error
  variants name the credential ID only).
- **Log/evidence hygiene**: audit records credential ID + capability +
  decision + evidence IDs. NEVER the secret, NEVER prompts/outputs
  (telemetry = counters and latencies only — fabric invariant §5).

### 3.1 Exact issue / revocation ceremony (normative for step 3)

- **Actor**: a human operator holding the master credential. No agent,
  no API caller, no automation may issue or revoke. (Enforcement: the
  issuance path requires master auth, same gate as `token create`.)
- **Issue command** (CLI-first; no HTTP issuance endpoint in v0.1):
  `decentraai gateway issue --agent <name> --capabilities <csv>`
  `--quota <ceiling> [--ttl-days <1..30, default 7>]`
  - Input validation (all fail-closed): name 1–32 chars
    `[a-zA-Z0-9_-]`; every capability must exist in the `CapabilityKind`
    taxonomy (unknown = reject, never coerce); quota > 0 and within the
    node-wide gateway ceiling; ttl-days within `[1, 30]`, default 7.
  - Secret generation: OS CSPRNG, 256-bit, `dca_`-distinct prefix reserved
    for gateway credentials (exact prefix fixed at implementation, must
    not collide with `dca_`/`dsk_`).
  - **Output (shown ONCE, never stored, never logged)**: the plaintext
    secret + credential id + capability set + quota + expiry timestamp.
    The operator transmits the secret to the agent owner out-of-band.
  - **Storage**: BLAKE3 hash of the secret + metadata row (id, agent,
    capabilities, quota ceiling + consumed, issued_at, expires_at,
    revoked flag, issuer). Same 0600 posture as token stores.
  - **Zeroization**: plaintext buffers zeroed immediately after hashing
    and after the once-only display write. Secrets never touch `Debug`,
    `Display`, error strings, audit, or metrics labels.
- **Inspect command** (master only): `decentraai gateway show --id <id>`
  returns metadata ONLY (capabilities, quota used/remaining, expiry,
  revoked flag) — never the secret, never the hash (hashes don't leave
  the store; comparison happens inside).
- **Revoke command** (master only): `decentraai gateway revoke --id <id>`
  flips the revoked flag; effect is immediate because auth resolves the
  live store on EVERY call (no cached auth decisions, no grace window).
  Revocation is terminal and audited (`gateway_credential_revoked` with
  id + issuer + reason; still no secret).
- **Expiry**: enforced at auth time like revocation; expired and revoked
  share one denial path and one failure reason (no oracle distinguishing
  them to unauthenticated callers).
- Step-3 implementation MUST NOT add issuance over HTTP, MUST NOT widen
  the actor set, and MUST NOT persist plaintext anywhere. Any deviation
  is a scope change requiring re-review.

## 4. Operation classes

| Class | Examples | Agent may request | Decided by |
|---|---|---|---|
| read-only | tools list/call (read tools), `decide`, status/snapshot reads, capability search | yes, with any valid credential | dispatcher (capability ∩ policy), no mutation possible by construction |
| compute | `execute_decision` (quota-gated), arena acts, hub task/bid/execute flows | yes, `agent`+`dca_` only | quota ledger (ceiling + reservation + lease) + Governor verdict where placement applies; `confirm:true` required |
| testnet-economic | anything that signs/broadcasts/settles | NO (not exposed on any agent credential) | operator CLI + explicit flag only (unchanged; out of gateway reach by construction) |
| privileged | key issuance, revocation, config, model admin, settlement sweep, manual submit | NO | master only (unchanged) |

## 5. Requestable vs deterministically-approved

- Agent-requestable: read-only calls; compute calls WITH quota + evidence;
  credential self-inspection (own id, capabilities, quota remaining, expiry).
- Deterministically approved (Rust, no LLM in the loop): capability grant
  check, quota reservation/release, Governor placement verdict, policy gates
  (model allowlist, egress), argument validation (closed schemas + bounds),
  rate/single-flight limits, evidence sealing, audit append.
- Explicitly NOT requestable: capability escalation, quota increase,
  another agent's scope, economic arming, control-plane tools, auth
  configuration. Each has a test asserting denial.

## 6. Audit trail + evidence per tool call

- Every mutating tool call appends ONE audit event
  (`gateway_tool_call`: credential id, tool, args hash — never args
  containing secrets, decision, evidence id, latency).
- Every execution returns evidence ids (existing per-domain evidence;
  gateway adds the linking record, not a parallel evidence system).
- Failure semantics for audit-write (binding, per class):
  - **Mutating / privileged operations are fail-closed**: if the audit or
    evidence record cannot be persisted, the call FAILS and no state
    effect is committed (reservation rolls back, nothing executes).
    "No evidence = it didn't happen" holds without exception here.
  - **Read-only operations** have explicitly separate, documented
    semantics: reads are counted via metrics (counters/latencies); the
    per-call audit record is best-effort. Rationale, stated plainly: a
    read cannot change state, so a dropped read-audit leaves no evidence
    gap with a state effect. Any future read that CAN change state
    (counters with side effects, lazy materialization) is reclassified
    as mutating and inherits fail-closed. No silent third category.
- Read-only calls are counted (metrics), not logged per-call (log hygiene).

## 7. Rate-limit + single-flight / concurrency limits

- Per-credential sliding-window rate limit (reuse the consumer
  `rate_windows` pattern; defaults: 60/min reads, 10/min mutations —
  tunable per credential, never above).
- Single-flight per (credential, tool): concurrent identical mutations
  serialize; the loser gets `retry_later`, never a duplicate execution
  (same CAS idiom as the SEC-01 trigger guard).
- Global mutation bound per node (operator-tunable, default small) so one
  agent cannot saturate execution while Governor says Queue/Reject.
- Every limit has a deterministic unit test (limit hit → denial shape,
  counters exact).

## 8. Kill-switch + failure semantics

- Master kill-switch: one config flag + one runtime file-gate; when OFF,
  ALL gateway mutations deny with a stable reason string; reads unaffected.
  Default OFF (consistent with every other autonomous surface).
- Failure semantics (closed set, each tested): `denied` (policy/quota/
  capability), `retry_later` (rate/single-flight/Governor-Queue),
  `rejected` (Governor-Reject/overload), `failed` (execution error —
  quota released, evidence sealed with failure, audit appended),
  `expired_or_revoked` (credential dead — indistinguishable handling for
  expired vs revoked, no oracle).
- Crash between reservation and evidence = released on next evaluation
  (lease expiry; same rule as compute reservations — every lease expires).
- No partial-credit states: a call is exactly one of the above.

## 9. MCP exposure + argument validation

- New tools appear in `all_tools()` with capability annotations; the
  dispatcher filters `tools/list` by credential (agents never see tools
  they cannot call — no oracle for privileged surface).
- Arguments: closed JSON schemas (`deny_unknown_fields`), string bounds
  (≤128/512 idiom from the mission bridge), numeric ranges, enum-closed
  capability names. Malformed = `invalid_params`, never coercion.
- `execute_decision` keeps `confirm:true`; agent credentials additionally
  require quota reservation success BEFORE execution starts.
- Fuzz-adjacent coverage: unknown tool, unknown field, oversized string,
  negative price, missing confirm — all denial-tested (existing tests
  `unknown_tool_is_invalid_params` et al. are the template).

## 10. Acceptance criteria + mandatory tests

- [ ] Credential lifecycle: issue → use → expire → deny; issue → revoke →
  deny; wrong secret → deny; all WITHOUT restart (auth reads live store).
- [ ] Least privilege: fresh credential denies everything; grant-one →
  exactly-one works; escalation attempts denied (one test per class §4).
- [ ] Quota: exceed → deny; failure → release (ledger exact); concurrent
  same-mutation → single execution (CAS test).
- [ ] Evidence/audit: every mutation returns evidence ids; audit contains
  the call; no secret/prompt/output bytes in audit (byte-scan test).
- [ ] Governor interplay: Queue/Reject verdicts surface as
  `retry_later`/`rejected`, never silent execution.
- [ ] Kill-switch OFF → all mutations deny with stable reason; reads live.
- [ ] Rate limits exact at the boundary (N ok, N+1 denied).
- [ ] Zeroization: secret buffers zero after use (test via instrumented
  comparison hook, same class as seed handling).
- [ ] Gates per step: `cargo test --workspace`, `cargo clippy
  --workspace --all-targets --all-features -- -D warnings`,
  `cargo fmt --all -- --check`, CI green. Baseline: 1835 green.
- [ ] Live proof (before merge): real node, real agent credential, real
  quota-gated execution + evidence + audit; then credential revoked and
  denial verified. No testnet, no mainnet, no exceptions.

## 11. Explicitly OUT of scope for M16

- Real economic execution from agent credentials (no testnet/mainnet
  signing path reachable from the gateway — a separate explicit decision
  would be required, as with the research lane).
- Cross-node delegation / collective orchestration (that's M17).
- New cryptography or credential formats (reuse BLAKE3-hashed stores).
- Model training, tensor parallelism, public-internet exposure (no domain/
  TLS work here), changes to consensus/reputation math.
- Touching PR #94 (external review — stays untouched) or the dead
  `tokens::tokenomics` module (wire-or-delete belongs to its author).
- Any bypass: if a step cannot be built as a client of the existing
  seams, the step is redesigned, not the seam bypassed.

## Appendix: Phase-2 step order (respected in implementation)

1. types/capabilities (credential record, operation classes, failure enum)
2. authorization boundary (dispatcher ∩, deny-default, per-class tests)
3. credential isolation (issue/revoke/TTL/zeroize, hashed store)
4. MCP tool boundary (list filtering, schemas, confirm+quota wiring)
5. audit/evidence (linking records, hygiene scan tests)
6. integration tests (live node proof + revocation proof)

Each step: implement → gates → report → **STOP for review**.
