# Code Review Findings — Settlement, Quest & Signer Layer

Deep code review of commits `f43825d` (quest dedup fix), `caba134` (nonce serialization +
dead-tx recovery) and `fee7a02` (Phase 7 signing layer).

Overall verdict: the code is in very good shape — regression tests reproduce live bugs,
invariant tests (money-conservation) exist, event recording covers every settlement state
transition, and key material is handled with discipline. The findings below are all
small, localized fixes ordered by risk.

---

## F1 — Sweep 404 handling: brittle string matching (HIGH)

**Location:** `crates/runtime/src/api/mod.rs`, `world_settle_sweep_handler`

**Problem:** The dead-tx recovery path decides "transaction never landed" via
`e.contains("404")`. Any error text containing "404" (transient proxy errors, changed
API messages, URLs containing the substring) would trigger requeue + resubmit with a
fresh nonce — potentially double-anchoring a settlement.

**Recommendation:** Introduce a structured error type in `settlement_tx.rs` and match on
the variant, not on text:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SettlementTxError {
    #[error("tx not found on chain: {0}")]
    TxNotFound(String),
    #[error("api request failed: {0}")]
    Request(String),
    #[error("bad response: {0}")]
    BadResponse(String),
}
```

`tx_status()` returns `Err(SettlementTxError::TxNotFound(hash))` when the API responds
404, and the sweep matches `Err(SettlementTxError::TxNotFound(_))` instead of string
matching. Keep `requeue_settlement` + `reanchor_escrow` as-is — they are correct.

---

## F2 — No resubmit cap on the 404 sweep loop (HIGH)

**Location:** `crates/runtime/src/world.rs` (`OnChainProof`), `api/mod.rs` sweep

**Problem:** A proof whose tx 404s persistently (rejected contract call, invalid gas,
misconfigured receiver) is requeued and resubmitted on every sweep pass, forever, each
time consuming a fresh nonce. Nothing terminates the loop.

**Recommendation:**

1. Add a counter to the proof record:

```rust
/// Times this proof was requeued after a dead tx. Capped; terminal after MAX.
#[serde(default)]
pub resubmit_count: u32,
```

2. Increment it in `requeue_settlement()` and refuse (return Err) when
`resubmit_count >= MAX_RESUBMITS` (3 is a sane default), transitioning the proof to
`Failed` with reason `"resubmit cap exceeded"` via the existing `fail_settlement()`.
3. Regression test: after `MAX_RESUBMITS` requeues, `requeue_settlement` errors and the
sweep reports the proof as `failed` instead of `still_pending`.

---

## F3 — Quest id generation reuses ids: dedup treats the symptom (MEDIUM)

**Location:** `crates/runtime/src/world.rs` — quest generation + `dedup_quests()`

**Problem:** `dedup_quests()` (fix in `f43825d`) runs only from `load_world_state()`. If
quest generation after a restart re-uses ids already present in world.json, duplicates
re-accumulate at runtime and are only cleaned on the next cold start. The live freeze
was caused by id collision at generation time, not by load.

**Recommendation:** Make quest ids unique per generation — a monotonic per-world counter
persisted with the state:

```rust
#[serde(default)]
pub next_quest_seq: u64,

fn new_quest_id(&mut self) -> String {
    self.next_quest_seq += 1;
    format!("quest-{}-{}", self.tick, self.next_quest_seq)
}
```

Keep `dedup_quests()` in `load_world_state()` as defense-in-depth for legacy world.json
files, and add a test asserting two freshly generated quests never share an id, across a
save/load round-trip.

---

## F4 — `reserve_nonce` holds the lock across a network await (LOW, accepted tradeoff)

**Location:** `crates/runtime/src/settlement_tx.rs`

**Problem/observation:** The mutex is held across `fetch_nonce()` on purpose
(reservation + record is one atomic step — documented in the code). The cost is that
all submissions serialize behind one network call. Correct, but a latency bottleneck at
volume.

**Recommendation:** Do not change semantics now. If submission volume grows, switch to a
pre-reservation queue: reserve a batch of N nonces in one fetch and hand them out from
the queue, refilling when exhausted. Add a metric (nonces_reserved vs submits) before
optimizing.

---

## F5 — Seed zeroization gaps in `Ed25519Signer::from_seed_hex` (MEDIUM)

**Location:** `crates/economy/src/signer.rs`

**Problem:** `from_seed_hex` wipes the local `seed: [u8; 32]` copy, but:
- the `Vec<u8>` returned by `hex::decode` (containing the raw seed) is dropped unwiped;
- the caller's input `&str` is not (and cannot be) wiped — currently undocumented;
- `ed25519_dalek::SigningKey` is not zeroized on drop unless the `zeroize` feature is
  enabled.

**Recommendation:**

1. Add `zeroize` to the workspace and enable `ed25519-dalek`'s `zeroize` feature.
2. Wrap the decoded buffer: `let raw = Zeroizing::new(hex::decode(trimmed)...);`
3. Document the caller contract on `from_seed_hex`: "the input string buffer must be
   zeroized by the caller if it holds secret material."

---

## F6 — Weak sender validation in `submit_intent` (LOW)

**Location:** `crates/runtime/src/world.rs` — `submit_intent()`

**Problem:** Sender validation is `starts_with("erd1") && len >= 10`. This accepts any
malformed bech32 string, producing intents that will fail (or worse, be signed) with
garbage senders.

**Recommendation:** Validate the bech32 checksum. Either pull in a small bech32 crate or
add a strict length check for the operator address form (62 chars, `erd1` prefix, valid
bech32 charset) plus checksum verification. Failing closed on invalid addresses keeps
the deterministic-intent guarantee honest.

---

## N1 — Enforce `cargo fmt --check` in CI (NITPICK)

**Evidence:** commit `caba134` ships a test where a comment is glued to the opening
brace on the same line (`operator_address_fails_closed_without_injection`), indicating
`cargo fmt` was not run/enforced.

**Recommendation:** Add `cargo fmt --all -- --check` as a step (or separate job) in
`.github/workflows/ci.yml` so formatting drift cannot land.

---

## Suggested implementation order

1. F1 + F2 together (same files, same risk area: sweep recovery)
2. N1 (one line in CI)
3. F3 (quest id uniqueness)
4. F5 (zeroize)
5. F6 (bech32 validation)
6. F4 (observe + metric only for now)
