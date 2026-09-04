# Code Review Findings — Settlement, Quest & Signer Layer

Deep code review of commits `f43825d` (quest dedup fix), `caba134` (nonce serialization +
dead-tx recovery), `fee7a02` (Phase 7 signing layer) and `a14d8ebe` (automated testnet
submission + operator wallet).

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

## F7 — Manual submit endpoint accepts an unvalidated sender (HIGH)

**Location:** `crates/runtime/src/api/mod.rs`, `world_settle_submit_handler` (from `a14d8ebe`)

**Problem:** The manual `/v1/world/settle/submit` endpoint reads `sender` straight from
the request body with `.unwrap_or("")` — no validation, no consistency check against
the operator. A caller can record a settlement under a forged or empty sender, corrupting
the sender-consistency invariant that `OnChainProof.sender` was added to protect. The
auto-submit path derives the sender from the injected signer correctly; only the manual
path is open.

**Recommendation:** In the manual handler, derive the sender from
`crate::settlement_tx::operator_address()` (fail closed when not configured), and make
`submit_settlement()` reject an empty `sender` outright. If a distinct operator wallet
per submission is ever needed, require the address to match a configured allowlist.

---

## F8 — `fetch_nonce` maps any HTTP 404 to nonce 0 (MEDIUM)

**Location:** `crates/runtime/src/settlement_tx.rs`, `fetch_nonce` (from `a14d8ebe`)

**Problem:** `if resp.status() == NOT_FOUND { return Ok(0); }` assumes 404 means "new
account, never transacted". A transient 404 from the API/gateway on an ACTIVE account
returns nonce 0. After a restart with an empty nonce tracker, that reserves nonce 0 and
broadcasts a doomed tx (invalid nonce), which then lands in the 404-sweep recovery loop
(interacting with F2's missing cap).

**Recommendation:** Distinguish "account genuinely unknown" from transport failure:
retry once on 404, or verify against the account endpoint's semantics (MultiversX returns
an account object with nonce 0 for new accounts rather than a hard 404 on newer API
versions). Treat an unconfirmed 404 as `Err`, never as `Ok(0)`.

---

## F9 — Settlement endpoints appear unauthenticated (HIGH — verify)

**Location:** `crates/runtime/src/api/mod.rs`, `build_router` (from `a14d8ebe`)

**Problem:** `/v1/world/settle/auto-submit`, `/settle/check`, `/settle/submit`,
`/settle/intent`, `/settle/sweep` are registered with no visible auth layer in
`build_router`. If the node API is reachable beyond localhost (PUBLIC_RELAY_NODE.md
suggests public exposure is a goal), anyone can trigger broadcasts that spend the
operator wallet's gas and mutate world state.

**Recommendation:** First verify whether a global middleware (authz.rs / wallet_auth.rs)
already covers these routes. If not, gate the entire `/v1/world/settle/*` family behind
operator authentication (token, wallet signature, or localhost-only binding) and add a
test asserting the endpoints reject unauthenticated calls.

---

## F10 — Signer reloaded from env on every call (LOW)

**Location:** `crates/runtime/src/settlement_tx.rs` — `operator_address()`, `sign_prepared()`

**Problem:** Both functions call `load_signer_from_env()` per invocation: file I/O on
every signature, and a swapped seed file silently rotates the operator identity
mid-process (proofs signed by two different keys in one run).

**Recommendation:** Load once into a `OnceLock<Ed25519Signer>` at first use; log the
derived address at startup so rotation is visible. Acceptable as-is for testnet volume,
but fix before any mainnet-adjacent use.

---

## F11 — Seed file written before permissions are tightened (LOW, security nitpick)

**Location:** `crates/economy/examples/gen_operator_wallet.rs` (from `a14d8ebe`)

**Problem:** `std::fs::write(&path, ...)` creates the file with default umask (often
0644 — world-readable), then `set_permissions(0o600)` runs afterwards. There is a window
where the operator seed sits on disk readable by other users.

**Recommendation:** Create the file with the mode atomically:

```rust
use std::os::unix::fs::OpenOptionsExt;
std::fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .mode(0o600)
    .open(&path)
    .expect("seed file create failed");
// then write via the handle
```

Also consider refusing to overwrite an existing seed file (`create_new` does this),
which `fs::write` silently does today.

---

## N1 — Enforce `cargo fmt --check` in CI (NITPICK)

**Evidence:** commits `caba134` and `a14d8ebe` both ship tests where a statement is glued
to the opening brace on the same line (`operator_address_fails_closed_without_injection`,
`debug_never_leaks_seed`), indicating `cargo fmt` was not run/enforced.

**Recommendation:** Add `cargo fmt --all -- --check` as a step (or separate job) in
`.github/workflows/ci.yml` so formatting drift cannot land.

---

## Suggested implementation order

1. F9 (verify auth coverage first — it gates whether the API can stay bound publicly)
2. F1 + F2 + F8 together (same risk area: sweep + nonce recovery paths)
3. F7 (sender validation on the manual submit path)
4. N1 (one line in CI)
5. F3 (quest id uniqueness)
6. F5 + F11 (zeroize + safe seed file creation)
7. F6 + F10 (bech32 validation, cached signer)
8. F4 (observe + metric only for now)
