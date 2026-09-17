//! Contribution-backed quota ledger (Compute Contribution & Quota — Q3).
//!
//! DecentraAI's compute access model is **"contribute compute → earn quota →
//! use quota to consume compute"**. This module is the pure, deterministic,
//! auditable accounting core of that model. It deliberately contains NO I/O
//! and NO async — every type is serde-serializable and every transition is a
//! pure function, so tests drive it with synthetic inputs and the distributed
//! layer wraps it behind a mutex (never `await` under lock).
//!
//! # What it is
//!
//! A quota ledger answers "may this account consume compute, and how much?" It
//! keeps per-account balances through an explicit lifecycle:
//!
//! ```text
//! EARNED ──(credit)──> AVAILABLE ──(reserve)──> RESERVED
//!                                                  │
//!                                  ┌───────────────┤
//!                                  │               │
//!                             (settle)          (release)
//!                                  │               │
//!                                  v               v
//!                              CONSUMED        AVAILABLE
//! ```
//!
//! - [`credit`](QuotaLedger::credit) turns measured, already-verified work
//!   into quota (never fabricated; UNKNOWN measurements earn nothing).
//! - [`reserve`](QuotaLedger::reserve) books quota for an in-flight request so
//!   it cannot be overspent; it refuses on insufficient available balance.
//! - [`settle`](QuotaLedger::settle) converts a reservation into consumed
//!   quota from the *real* measured usage; unused reserved quota is released.
//! - [`release`](QuotaLedger::release) returns an unused reservation to the
//!   available pool (cancellation / failure without consumption).
//!
//! # Non-economics, on purpose
//!
//! This ledger does NOT establish a fair financial conversion. The
//! contribution→quota mapping is an explicit, **versioned, replaceable
//! policy** ([`ContributionPolicy`]); the default is a documented, arbitrary
//! placeholder that the operator tunes once real calibration data exists. The
//! ledger only ever moves already-converted integer quota units; it never
//! invents a token price or a hardware score.
//!
//! # Idempotency & safety
//!
//! Every mutation carries an explicit `ref_id` (an existing execution /
//! request / reservation identifier). Applying the same `ref_id` twice is a
//! no-op that returns the same outcome, so retried or replayed accounting
//! events can never double-credit, double-settle, or double-release. Balances
//! use checked/saturating arithmetic so they can never go negative or wrap.
//! The ledger also keeps an append-only audit trail (provenance) recording who
//! changed what and under which policy version.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Stable owner of a quota account.
///
/// Reuses the existing identity primitives — no new identity system. A worker
/// (provider) account is keyed by its libp2p peer id; a consumer account is
/// keyed by an explicit account identifier the operator supplies. The type is
/// an owned `String` so it is serializable over P2P and across account kinds.
pub type AccountId = String;

/// Versioned, replaceable policy that converts measured work into quota units.
///
/// The exact conversion is deliberately NOT baked into the ledger. It is an
/// explicit, inspectable configuration so an operator can see and replace the
/// economics without touching accounting logic. The default is a documented
/// placeholder: **1 token → 1 unit, 1 second of processing → 1 unit**. This is
/// an arbitrary starting point, not a fair market price — it exists only so the
/// pipeline is testable until real calibration data lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionPolicy {
    /// Policy version. Every credited unit and audit entry records the version
    /// that produced it, so historical balances stay explainable if the policy
    /// changes.
    pub version: u32,
    /// Quota units credited per measured token processed/generated.
    pub units_per_token: u64,
    /// Quota units credited per measured millisecond of processing time.
    pub units_per_processing_ms: u64,
}

impl Default for ContributionPolicy {
    fn default() -> Self {
        // Placeholder rates — NOT calibrated economics. 1 token → 1 unit,
        // 1 ms → 1 unit. Tune before production accounting is enabled.
        Self {
            version: 1,
            units_per_token: 1,
            units_per_processing_ms: 1,
        }
    }
}

impl ContributionPolicy {
    /// Converts a measured execution into quota units under this policy.
    ///
    /// `None` measurements mean UNKNOWN (not measured) and contribute nothing
    /// (the ledger never turns UNKNOWN into a fake balance). An execution with
    /// no measured work at all yields 0 units — callers should not `credit` a
    /// fully-unmeasured execution in the first place; this just makes the
    /// mapping total and safe.
    pub fn units_for(&self, tokens_used: Option<u32>, processing_ms: Option<u32>) -> u64 {
        let tokens = tokens_used.unwrap_or(0) as u64;
        let ms = processing_ms.unwrap_or(0) as u64;
        tokens
            .saturating_mul(self.units_per_token)
            .saturating_add(ms.saturating_mul(self.units_per_processing_ms))
    }
}

/// Current balance of one quota account.
///
/// The invariant is `earned == consumed + reserved + available`. `earned` is
/// monotonic (total ever credited); the spendable pool is `available`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QuotaAccount {
    /// Total quota ever credited (monotonic; never decreases).
    pub earned: u64,
    /// Quota currently available to spend.
    pub available: u64,
    /// Quota booked for in-flight (unsettled) requests.
    pub reserved: u64,
    /// Quota consumed against settled executions.
    pub consumed: u64,
}

impl QuotaAccount {
    /// How much an account could still reserve right now (the spendable pool).
    pub fn spendable(&self) -> u64 {
        self.available
    }
}

/// A booked reservation awaiting settle or release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaReservation {
    /// Unique reservation id (the caller supplies an existing request/execution
    /// identifier, e.g. the request id). Idempotency key for settle/release.
    pub reservation_id: String,
    /// Account the reservation was booked against.
    pub account: AccountId,
    /// Quota units reserved.
    pub amount: u64,
    /// Whether this reservation was already settled. A settled reservation is
    /// a no-op target: settle/release on it are idempotent no-ops.
    pub settled: bool,
    /// Consumer key that booked this reservation, when known. Old
    /// reservations (and non-consumer paths) carry `None` — attribution is
    /// never backfilled, only recorded for new keyed reservations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// The reason a quota mutation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaError {
    /// The account had fewer spendable units than the reservation required.
    InsufficientQuota { available: u64, requested: u64 },
    /// The referenced reservation does not exist (unknown id).
    UnknownReservation,
    /// The referenced reservation was already settled (double settlement).
    AlreadySettled,
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientQuota {
                available,
                requested,
            } => write!(
                f,
                "insufficient quota: requested {requested}, available {available}"
            ),
            Self::UnknownReservation => write!(f, "unknown reservation"),
            Self::AlreadySettled => write!(f, "reservation already settled"),
        }
    }
}

impl std::error::Error for QuotaError {}

/// One auditable ledger mutation (provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaEvent {
    /// The operation: `credit`, `reserve`, `settle`, or `release`.
    pub op: String,
    /// The account affected.
    pub account: AccountId,
    /// The units involved.
    pub amount: u64,
    /// The idempotency key (execution/request/reservation id) this refers to.
    pub ref_id: String,
    /// The contribution policy version that governed the conversion (credit
    /// only; the ledger keeps it so the economics are explainable).
    pub policy_version: u32,
    /// Consumer key behind this mutation, when known. Old events carry
    /// `None` — history is never re-attributed, only new keyed mutations
    /// record it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// The deterministic quota accounting core.
///
/// Wrap this behind a `Mutex` (never `await` under the lock). All operations
/// are pure, idempotent by `ref_id`, and audited.
///
/// The ledger is plain data by design: [`Serialize`]/[`Deserialize`] plus
/// [`save_atomic`](QuotaLedger::save_atomic) let the host process persist it
/// across restarts. Balances that only lived in memory meant every node
/// reboot silently zeroed operator-granted consumer credit.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QuotaLedger {
    /// Per-account balances.
    accounts: HashMap<AccountId, QuotaAccount>,
    /// Active reservations keyed by reservation id.
    reservations: HashMap<String, QuotaReservation>,
    /// Idempotency: set of `(op, ref_id)` tuples already applied. Keeps a
    /// retried/replayed accounting event from double-applying.
    applied: HashSet<(String, String)>,
    /// Append-only audit trail (provenance). Bounded to avoid unbounded growth.
    events: std::collections::VecDeque<QuotaEvent>,
    /// Lifetime per-consumer-key consumption (key_id → units consumed by
    /// settle). Old snapshots load empty and are never backfilled: only
    /// settles of keyed reservations accumulate here.
    #[serde(default)]
    consumed_by_key: HashMap<String, u64>,
    /// The active contribution→quota policy.
    policy: ContributionPolicy,
    /// Set on every mutation; persistence layers use it to skip no-op writes.
    #[serde(skip)]
    dirty: std::sync::atomic::AtomicBool,
}

/// The max number of audit events retained in memory (bounded provenance).
const MAX_EVENTS: usize = 4096;

impl QuotaLedger {
    /// A fresh ledger with the given conversion policy.
    pub fn new(policy: ContributionPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// The active contribution→quota policy (read-only).
    pub fn policy(&self) -> ContributionPolicy {
        self.policy
    }

    /// Swaps the active contribution→quota policy **in place**, preserving all
    /// historical account balances and the audit trail. Future `credit` calls
    /// use the new version; already-recorded events keep the version that
    /// produced them (historical records are never silently rewritten).
    pub fn set_policy(&mut self, policy: ContributionPolicy) {
        self.policy = policy;
    }

    /// Current balance of `account`, or `None` if the account has no record.
    /// Read-only, for observability.
    pub fn account(&self, account: &AccountId) -> Option<QuotaAccount> {
        self.accounts.get(account).copied()
    }

    /// Snapshot of every account balance (read-only, deterministic order).
    pub fn accounts(&self) -> BTreeMap<AccountId, QuotaAccount> {
        self.accounts.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    /// The audit trail so far (provenance). Read-only.
    pub fn events(&self) -> &std::collections::VecDeque<QuotaEvent> {
        &self.events
    }

    /// Converts measured work into quota units under the active policy and
    /// credits it to `account`, **exactly once**.
    ///
    /// `ref_id` is the existing execution/request identifier proving the work;
    /// re-applying the same `ref_id` returns the same credited amount without
    /// double-crediting. `None` measurements contribute nothing (UNKNOWN ≠ 0).
    ///
    /// Returns the quota units credited (0 when the measurement is fully
    /// unknown / under a zero-credit policy).
    pub fn credit(
        &mut self,
        account: &AccountId,
        ref_id: &str,
        tokens_used: Option<u32>,
        processing_ms: Option<u32>,
    ) -> u64 {
        if !self.mark_applied("credit", ref_id) {
            return 0; // duplicate: already credited this ref_id exactly once.
        }
        let units = self.policy.units_for(tokens_used, processing_ms);
        if units == 0 {
            return 0; // UNKNOWN / zero-credit execution: nothing to credit.
        }
        let acc = self.accounts.entry(account.clone()).or_default();
        acc.earned = acc.earned.saturating_add(units);
        acc.available = acc.available.saturating_add(units);
        self.record_event("credit", account, units, ref_id, None);
        units
    }

    /// Books `amount` quota for an in-flight request on `account`.
    ///
    /// Idempotent by `reservation_id`: reserving the same id twice returns the
    /// same reservation without double-booking. Refuses when the account lacks
    /// enough spendable quota. `ref_id` may equal `reservation_id` (reserving
    /// on behalf of an existing request id) or differ (a distinct reservation
    /// for a pre-allocated request) — either is fine; idempotency is keyed on
    /// `reservation_id`.
    pub fn reserve(
        &mut self,
        account: &AccountId,
        reservation_id: &str,
        amount: u64,
    ) -> Result<QuotaReservation, QuotaError> {
        self.reserve_with_key(account, reservation_id, amount, None)
    }

    /// Books `amount` quota like [`Self::reserve`], additionally recording
    /// which consumer key booked it so a later settle attributes consumption
    /// per key. Unkeyed callers keep using `reserve` (key stays `None`).
    pub fn reserve_with_key(
        &mut self,
        account: &AccountId,
        reservation_id: &str,
        amount: u64,
        key_id: Option<String>,
    ) -> Result<QuotaReservation, QuotaError> {
        if let Some(existing) = self.reservations.get(reservation_id) {
            // Already reserved this id: return the same reservation (no-op),
            // even if settled (caller learns it is done).
            return Ok(existing.clone());
        }
        let acc = self.accounts.entry(account.clone()).or_default();
        if acc.available < amount {
            return Err(QuotaError::InsufficientQuota {
                available: acc.available,
                requested: amount,
            });
        }
        acc.available = acc.available.saturating_sub(amount);
        acc.reserved = acc.reserved.saturating_add(amount);
        let reservation = QuotaReservation {
            reservation_id: reservation_id.to_string(),
            account: account.clone(),
            amount,
            settled: false,
            key_id: key_id.clone(),
        };
        self.reservations
            .insert(reservation_id.to_string(), reservation.clone());
        self.record_event("reserve", account, amount, reservation_id, key_id.as_deref());
        Ok(reservation)
    }

    /// Settles a reservation against real measured usage.
    ///
    /// Moves exactly `used` (clamped to the reserved amount, never more than
    /// reserved) from `reserved` to `consumed`, and releases the unused
    /// remainder back to `available`. A settled reservation is finalized and
    /// removed: a later settle or release of the same id reports
    /// [`QuotaError::UnknownReservation`] instead of silently re-reading stale
    /// state, so per-request reservation ids can never be re-used to consume
    /// without accounting. `used` may be 0 for a zero-output completion (the
    /// reservation is fully released).
    ///
    /// Returns the amount actually consumed (≤ the reserved amount).
    pub fn settle(&mut self, reservation_id: &str, used: u64) -> Result<u64, QuotaError> {
        let Some(mut res) = self.reservations.remove(reservation_id) else {
            return Err(QuotaError::UnknownReservation);
        };
        if res.settled {
            // Defensive: cannot happen while settled entries are removed on
            // settle, but never resurrect state from a tombstone.
            self.reservations.insert(reservation_id.to_string(), res);
            return Err(QuotaError::AlreadySettled);
        }
        res.settled = true;
        let account = res.account.clone();
        let amount = res.amount;
        let key_id = res.key_id.clone();
        let used = used.min(amount);
        let released = amount.saturating_sub(used);
        let acc = self.accounts.entry(account.clone()).or_default();
        acc.reserved = acc.reserved.saturating_sub(amount);
        acc.consumed = acc.consumed.saturating_add(used);
        // The unused remainder returns to the spendable pool.
        acc.available = acc.available.saturating_add(released);
        // Per-key attribution: only settles of keyed reservations with
        // real consumption accumulate. Zero settles record nothing, so a
        // key with only empty settles stays absent (no data ≠ 0).
        if used > 0 {
            if let Some(k) = key_id.as_deref() {
                *self.consumed_by_key.entry(k.to_string()).or_default() =
                    self.consumed_by_key.get(k).copied().unwrap_or(0).saturating_add(used);
            }
        }
        self.record_event("settle", &account, used, reservation_id, key_id.as_deref());
        Ok(used)
    }

    /// Releases an unused reservation back to the available pool.
    ///
    /// Used for cancellation or failure where no work was consumed. Idempotent:
    /// releasing an already-settled or already-released reservation is a no-op
    /// returning `Ok(())`. Unknown ids error (a caller referencing a
    /// reservation that never existed should know).
    pub fn release(&mut self, reservation_id: &str) -> Result<(), QuotaError> {
        let Some(res) = self.reservations.get_mut(reservation_id) else {
            return Err(QuotaError::UnknownReservation);
        };
        if res.settled {
            // Already settled; nothing to release (idempotent no-op).
            return Ok(());
        }
        res.settled = true;
        let account = res.account.clone();
        let amount = res.amount;
        let key_id = res.key_id.clone();
        let acc = self.accounts.entry(account.clone()).or_default();
        acc.reserved = acc.reserved.saturating_sub(amount);
        acc.available = acc.available.saturating_add(amount);
        self.record_event("release", &account, amount, reservation_id, key_id.as_deref());
        Ok(())
    }

    /// Moves `amount` from `available` to `consumed` for a payout
    /// redemption. Fails when `available` is short — nothing moves, so a
    /// failed payout never half-applies. Successful moves record a `payout`
    /// audit event. Idempotent on `ref_id` (the payout id): a retry after a
    /// crash converges instead of double-moving.
    pub fn payout_consume(
        &mut self,
        account: &AccountId,
        amount: u64,
        ref_id: &str,
        key_id: Option<&str>,
    ) -> Result<u64, QuotaError> {
        let available = self.accounts.get(account).map(|a| a.available).unwrap_or(0);
        if available < amount {
            return Err(QuotaError::InsufficientQuota { available, requested: amount });
        }
        if !self.mark_applied("payout", ref_id) {
            return Ok(0); // duplicate: already moved for this payout exactly once.
        }
        let acc = self.accounts.entry(account.clone()).or_default();
        acc.available = acc.available.saturating_sub(amount);
        acc.consumed = acc.consumed.saturating_add(amount);
        self.record_event("payout", account, amount, ref_id, key_id);
        Ok(amount)
    }

    /// Credits renewal-funded quota: adds `amount` to both `earned` and
    /// `available` of `account`, audited as a `renew` event. This is the
    /// ONLY mint path besides measured-work `credit` — callers must fund
    /// it from an already-earned balance (compensation redeemable), never
    /// from nothing; the method itself cannot verify funding, so the
    /// renewal flow (not this primitive) owns that invariant. Idempotent
    /// on `ref_id` (the renewal id): a crash-retry converges.
    pub fn renew_credit(
        &mut self,
        account: &AccountId,
        amount: u64,
        ref_id: &str,
        key_id: Option<&str>,
    ) -> u64 {
        if amount == 0 {
            return 0;
        }
        if !self.mark_applied("renew", ref_id) {
            return 0; // duplicate: already renewed under this id exactly once.
        }
        let acc = self.accounts.entry(account.clone()).or_default();
        acc.earned = acc.earned.saturating_add(amount);
        acc.available = acc.available.saturating_add(amount);
        self.record_event("renew", account, amount, ref_id, key_id);
        amount
    }

    /// Marks `(op, ref_id)` as applied; returns `false` if already applied.
    fn mark_applied(&mut self, op: &str, ref_id: &str) -> bool {
        self.applied.insert((op.to_string(), ref_id.to_string()))
    }

    fn record_event(
        &mut self,
        op: &str,
        account: &AccountId,
        amount: u64,
        ref_id: &str,
        key_id: Option<&str>,
    ) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(QuotaEvent {
            op: op.to_string(),
            account: account.clone(),
            amount,
            ref_id: ref_id.to_string(),
            policy_version: self.policy.version,
            key_id: key_id.map(str::to_string),
        });
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Lifetime quota units consumed via settles of `key_id`'s reservations.
    /// Read-only, for observability. Absent keys read as 0 — but a key with
    /// only zero-settles (or none) is indistinguishable here from "no data";
    /// consumers needing that distinction join with the requests counter.
    pub fn consumed_by_key(&self, key_id: &str) -> u64 {
        self.consumed_by_key.get(key_id).copied().unwrap_or(0)
    }

    /// Returns and clears the mutation flag. Persistence layers poll this to
    /// skip rewriting an unchanged ledger.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Writes a crash-safe snapshot (tmp file + rename) of the whole ledger.
    pub fn save_atomic(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }

    /// Loads a previously saved snapshot; `None` when absent or corrupt
    /// (callers then start from a fresh ledger — never fail the node for it).
    pub fn load_snapshot(path: &std::path::Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(bytes.as_slice()).ok()
    }

    /// Adopts another ledger's balances/reservations/history wholesale
    /// (used at boot to restore a saved snapshot into the live instance).
    pub fn restore(&mut self, other: QuotaLedger) {
        self.accounts = other.accounts;
        self.reservations = other.reservations;
        self.applied = other.applied;
        self.events = other.events;
        self.consumed_by_key = other.consumed_by_key;
        self.policy = other.policy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> QuotaLedger {
        // 1 token -> 1 unit, 1ms -> 1 unit (placeholder default).
        QuotaLedger::new(ContributionPolicy::default())
    }

    #[test]
    fn successful_execution_credits_exactly_once() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        let units = l.credit(&acct, "exec-1", Some(100), Some(5000));
        assert_eq!(units, 5100, "100 tokens + 5000ms -> 5100 units");
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.earned, 5100);
        assert_eq!(acc.available, 5100);
        assert_eq!(acc.consumed, 0);
        assert_eq!(acc.reserved, 0);
    }

    #[test]
    fn duplicate_execution_event_never_double_credits() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        let first = l.credit(&acct, "exec-1", Some(100), Some(5000));
        let second = l.credit(&acct, "exec-1", Some(100), Some(5000));
        assert_eq!(first, 5100);
        assert_eq!(second, 0, "replayed credit must be a no-op");
        assert_eq!(l.account(&acct).unwrap().earned, 5100);
        // Distinct executions credit independently.
        let third = l.credit(&acct, "exec-2", Some(10), Some(500));
        assert_eq!(third, 510);
        assert_eq!(l.account(&acct).unwrap().earned, 5610);
    }

    #[test]
    fn unmeasured_execution_earns_nothing() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        let units = l.credit(&acct, "exec-1", None, None);
        assert_eq!(units, 0, "UNKNOWN measurement must not fabricate quota");
        // Honest UNKNOWN: no measured work means no account record is created.
        assert_eq!(
            l.account(&acct),
            None,
            "no record for an unmeasured execution"
        );
    }

    #[test]
    fn keyed_reservations_attribute_consumption_per_key() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        // Keyed reservation: settle attributes to the key.
        let res = l
            .reserve_with_key(&acct, "res-k1", 200, Some("ck-aaa".to_string()))
            .unwrap();
        assert_eq!(res.key_id.as_deref(), Some("ck-aaa"));
        l.settle(&res.reservation_id, 170).unwrap();
        assert_eq!(l.consumed_by_key("ck-aaa"), 170);
        assert_eq!(l.consumed_by_key("ck-unknown"), 0);
        // Unkeyed reservation: account moves, no key attribution.
        let res2 = l.reserve(&acct, "res-plain", 100).unwrap();
        assert_eq!(res2.key_id, None);
        l.settle(&res2.reservation_id, 100).unwrap();
        assert_eq!(l.consumed_by_key("ck-aaa"), 170, "unkeyed settle changes nothing");
        // Zero settle records nothing (absent stays absent).
        let res3 = l
            .reserve_with_key(&acct, "res-k2", 50, Some("ck-bbb".to_string()))
            .unwrap();
        l.settle(&res3.reservation_id, 0).unwrap();
        assert_eq!(l.consumed_by_key("ck-bbb"), 0);
        assert!(!l.consumed_by_key.contains_key("ck-bbb"));
        // Events carry the key where known.
        let settle_ev = l
            .events()
            .iter()
            .rev()
            .find(|e| e.op == "settle" && e.ref_id == "res-k1")
            .unwrap();
        assert_eq!(settle_ev.key_id.as_deref(), Some("ck-aaa"));
    }

    #[test]
    fn old_snapshots_without_key_fields_load_clean() {
        // Pre-key ledgers (no key_id / consumed_by_key in JSON) must load
        // with empty attribution — history is never re-attributed.
        let legacy = serde_json::json!({
            "accounts": {},
            "reservations": {},
            "applied": [],
            "events": [{
                "op": "settle", "account": "a", "amount": 5,
                "ref_id": "r", "policy_version": 1
            }],
            "policy": { "units_per_token": 1, "units_per_processing_ms": 1, "version": 1 }
        });
        let back: QuotaLedger = serde_json::from_str(&legacy.to_string()).unwrap();
        assert_eq!(back.consumed_by_key("ck-aaa"), 0);
        assert!(back.events().iter().all(|e| e.key_id.is_none()));
        let mut live = ledger();
        live.restore(back);
        assert_eq!(live.consumed_by_key("ck-aaa"), 0);
    }

    #[test]
    fn payout_consume_moves_available_to_consumed() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let before = l.account(&acct).unwrap();
        assert_eq!(before.available, 1000);
        assert_eq!(l.payout_consume(&acct, 300, "po-1", Some("ck-x")).unwrap(), 300);
        let after = l.account(&acct).unwrap();
        assert_eq!(after.available, 700);
        assert_eq!(after.consumed, 300);
        assert_eq!(after.earned, 1000, "earned untouched by payout");
        // Short cover fails, nothing moves.
        assert!(l.payout_consume(&acct, 701, "po-2", None).is_err());
        assert_eq!(l.account(&acct).unwrap().available, 700);
        // Duplicate ref is a no-op (crash-retry converges).
        assert_eq!(l.payout_consume(&acct, 300, "po-1", Some("ck-x")).unwrap(), 0);
        assert_eq!(l.account(&acct).unwrap().consumed, 300);
        // Event carries the key.
        let ev = l.events().iter().rev().find(|e| e.op == "payout").unwrap();
        assert_eq!(ev.key_id.as_deref(), Some("ck-x"));
        assert_eq!(ev.ref_id, "po-1");
    }

    #[test]
    fn renew_credit_funds_from_nothing_but_marks_source() {
        // renew_credit itself cannot verify funding — the renewal FLOW owns
        // that invariant (it redeems compensation first). The primitive
        // only guarantees auditability (renew op + ref) and idempotency.
        let mut l = ledger();
        let acct = "peer-a".to_string();
        assert_eq!(l.renew_credit(&acct, 500, "renew:rq-1", Some("ck-a")), 500);
        let acc = l.account(&acct).unwrap();
        assert_eq!((acc.earned, acc.available, acc.consumed), (500, 500, 0));
        assert_eq!(l.renew_credit(&acct, 500, "renew:rq-1", Some("ck-a")), 0);
        let ev = l.events().iter().rev().find(|e| e.op == "renew").unwrap();
        assert_eq!(ev.ref_id, "renew:rq-1");
        assert_eq!(ev.key_id.as_deref(), Some("ck-a"));
    }

    #[test]
    fn reserve_settle_release_round_trip() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None); // 1000 units
        let res = l.reserve(&acct, "res-1", 200).unwrap();
        assert_eq!(res.amount, 200);
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.available, 800, "reserve moved 200 out of available");
        assert_eq!(acc.reserved, 200);
        assert_eq!(acc.spendable(), 800);
        // Settle using only 170 -> 30 released.
        let consumed = l.settle(&res.reservation_id, 170).unwrap();
        assert_eq!(consumed, 170);
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.consumed, 170);
        assert_eq!(acc.reserved, 0);
        assert_eq!(acc.available, 830);
    }

    #[test]
    fn partial_settlement_releases_the_remainder() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let res = l.reserve(&acct, "res-1", 200).unwrap();
        // Used > reserved clamps to reserved; nothing over-consumed.
        let consumed = l.settle(&res.reservation_id, 9999).unwrap();
        assert_eq!(consumed, 200, "cannot consume more than reserved");
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.consumed, 200);
        assert_eq!(acc.reserved, 0);
        assert_eq!(acc.available, 800);
    }

    #[test]
    fn release_returns_unused_reservation_to_pool() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let res = l.reserve(&acct, "res-1", 200).unwrap();
        l.release(&res.reservation_id).unwrap();
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.reserved, 0);
        assert_eq!(acc.consumed, 0);
        assert_eq!(acc.available, 1000, "release returns the full amount");
    }

    #[test]
    fn insufficient_quota_refuses_reservation() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(50), None); // 50 units
        let err = l.reserve(&acct, "res-1", 100).unwrap_err();
        assert!(matches!(
            err,
            QuotaError::InsufficientQuota {
                available: 50,
                requested: 100
            }
        ));
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.reserved, 0, "failed reservation books nothing");
        assert_eq!(acc.available, 50);
    }

    #[test]
    fn duplicate_reservation_is_idempotent() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let r1 = l.reserve(&acct, "res-1", 100).unwrap();
        let r2 = l.reserve(&acct, "res-1", 100).unwrap();
        assert_eq!(r1.reservation_id, r2.reservation_id);
        assert_eq!(r1.amount, r2.amount);
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.reserved, 100, "duplicate reserve must not double-book");
    }

    #[test]
    fn double_settle_is_refused() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let res = l.reserve(&acct, "res-1", 200).unwrap();
        l.settle(&res.reservation_id, 170).unwrap();
        // A settled reservation is finalized and removed: a second settle of
        // the same id must NOT re-read stale state or re-move any balance.
        let err = l.settle(&res.reservation_id, 170).unwrap_err();
        assert!(matches!(err, QuotaError::UnknownReservation));
        // No double-move of balance.
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.consumed, 170);
    }

    #[test]
    fn settled_reservation_cannot_be_reused_to_skip_accounting() {
        // Regression: a shared reservation id used to let every later request
        // ride the already-settled entry for free. Finalization must close
        // that hole — a fresh request gets a fresh id and reserves afresh.
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-0", Some(1000), None);
        let res = l.reserve(&acct, "req-1", 200).unwrap();
        assert_eq!(l.settle(&res.reservation_id, 40).unwrap(), 40);
        let err = l.settle(&res.reservation_id, 999).unwrap_err();
        assert!(matches!(err, QuotaError::UnknownReservation));
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.consumed, 40);
    }

    #[test]
    fn concurrent_mutations_keep_balances_consistent() {
        // Simulate many idempotent ops racing; the pure ledger is not itself
        // thread-safe (the wrapper is), but each op is atomic & idempotent, so
        // replayed sequences converge to one balance.
        let mut l = ledger();
        let acct = "peer-a".to_string();
        for i in 0..100u32 {
            l.credit(&acct, &format!("exec-{i}"), Some(10), None);
        }
        let acc = l.account(&acct).unwrap();
        assert_eq!(acc.earned, 1000);
        assert_eq!(acc.available, 1000);
    }

    #[test]
    fn audit_trail_records_provenance() {
        let mut l = ledger();
        let acct = "peer-a".to_string();
        l.credit(&acct, "exec-1", Some(100), Some(100));
        let res = l.reserve(&acct, "res-1", 100).unwrap();
        l.settle(&res.reservation_id, 50).unwrap();
        let events = l.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].op, "credit");
        assert_eq!(events[0].account, "peer-a");
        assert_eq!(events[0].ref_id, "exec-1");
        assert_eq!(events[0].policy_version, 1);
        assert_eq!(events[1].op, "reserve");
        assert_eq!(events[2].op, "settle");
        assert_eq!(events[2].amount, 50);
    }

    #[test]
    fn policy_version_is_recorded_in_events() {
        let mut l = QuotaLedger::new(ContributionPolicy {
            version: 7,
            units_per_token: 2,
            units_per_processing_ms: 0,
        });
        let acct = "peer-a".to_string();
        let units = l.credit(&acct, "exec-1", Some(100), Some(100));
        assert_eq!(units, 200, "v7 policy: 100 tokens x 2 units");
        assert_eq!(l.events()[0].policy_version, 7);
        assert_eq!(l.policy().version, 7);
    }

    #[test]
    fn account_snapshot_is_deterministic() {
        let mut l = ledger();
        l.credit(&"b".to_string(), "exec-1", Some(1), None);
        l.credit(&"a".to_string(), "exec-2", Some(1), None);
        l.credit(&"c".to_string(), "exec-3", Some(1), None);
        let snap = l.accounts();
        let keys: Vec<_> = snap.keys().cloned().collect();
        assert_eq!(
            keys,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
