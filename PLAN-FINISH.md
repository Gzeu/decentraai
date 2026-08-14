# DecentraAI — "Finish Remaining Milestones" plan (executed)

Tracking document for the plan to finish DecentraAI's remaining roadmap work.
Each phase lists what was done, the commit that landed it, and the tests that
prove it. All commits pass the quality gates (clippy `-D warnings`, workspace
tests green).

## Phase 0 — Commit pre-existing WIP + fix a runtime test regression

- Completed the in-progress node wiring in `crates/node-cli/src/main.rs`
  (dashboard-owned llama-server lifecycle, M24 engine supervisor, and the
  compute broadcaster resolved against the **live** engine port every beat so
  a respawn on a new port is reflected).
- Fixed a test-helper regression from `e58f13b`: the proxy/status now resolve
  the backend from `manager.base_url()`, but the test's fake `llama-server`
  was a dead `sleep 60`, so every proxied test returned 503. The fake engine is
  now a real axum listener wrapped by the manager — keeping `model_loaded`
  true while giving the proxy a live target (faithful to M24).
- Commit: `47c39dd`.

## Phase 1 — P5: Invites & join

- `decentraai invite --addr <a>` issues a fresh Tier-1 Guest token and prints a
  copy-pastable `<multiaddr>/p2p/<peer-id> <token>` string (peer id derived from
  the node identity, so it dials as-is).
- `decentraai join "<invite>"` parses the pair (pure `parse_invite`),
  auto-provisions identity + config, stores the guest token as the node's
  credential (`runtime/invite.token`, 0600), and verifies the coordinating peer
  is reachable over the verified P2P path. Audits `invite_created`/`joined`.
- Tests: multiaddr round-trip, malformed-invite rejection, least-privilege
  guest-seat guarantee. Commit: `f534349`.

## Phase 2 — M9-9: Reputation-based compensation

- `decentraai-compute::compensation` : `RewardPolicy` + `reward_tokens` —
  a deterministic, synthetic contribution-credits ledger (not a payment
  platform): `verified_requests × rate`, scaled by contribution quality and a
  reputation term (clean-service ratio `verified/(verified+failed)`). Zero
  verified work or a complete-failure record earns 0.
- Wired into `ContributionRow.reward_tokens`, surfaced on `/v1/compute` and the
  `tier suggest` table. Commit: `fa554d5`.

## Phase 3 — M10 acceptance gaps

- Per-request audit: `DistributedInference.set_logs_dir` → best-effort
  `inference_completed`/`inference_failed` audit events with request id, trace,
  session, worker id, model hash and status, from both routed and streamed
  paths. Wired on the node. Test asserts the correlation fields land in JSONL.
- Dashboard latency/success: `/status` exposes `latency_ms.{p50,p95,p99}`,
  `success_rate_percent`, `requests_failed`; rendered on the Inference card.
  Pure `inference_stats`/`percentile` helpers with synthetic tests.
- Commit: `cc9f330`.

## Phase 4 — Honest M21 / M22 / M23 readiness

- These milestones stay **parked / not-done (foundation only)** as AGENTS.md
  requires. ROADMAP now says plainly what each delivers and what it does NOT
  claim (no distributed MoE, no prefill/decode split, no autonomous planner).
- Added a pinned honesty test: every engine DecentraAI runs keeps
  `expert_routing` (and llama-server keeps `prefill_decode_separation`) off, so
  the whole-model fallback is provably correct for production.
- Commit: `49a830a`.

## Remaining (genuinely open) work

- **M10** implementation phase (contracts control-plane hardening): not itemized
  as acceptance-criteria gaps but left as the checklist's hardening phase.
- **P6/other**: none active — the roadmap's remaining milestone-scoped items are
  now checked or honestly annotated.
- Productization/installer, dashboard UI, subscription model: already done and
  outside the "finish remaining milestones" scope chosen for this plan.

## Quality gates run

```bash
cargo clippy --workspace --all-targets -- -D warnings   # clean
cargo test --workspace                                  # all green
```