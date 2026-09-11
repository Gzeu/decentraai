//! MultiversX Supernova v2.0.8 Observer types (O1).
//!
//! Invariant (inherited from the fabric contract):
//!
//! ```text
//!   CHAIN (untrusted input) → typed observation → deterministic DecentraAI
//! ```
//!
//! This crate holds ONLY the pure observation surface: closed internal
//! schemas plus explicit-field readers for the three real external
//! interfaces (Proxy / Indexer ES / Notifier). It never submits
//! transactions, never holds secrets, never selects peers, never mutates
//! trust. The Actor lane (existing `settlement_tx.rs` +
//! `proposal::economic::TestnetEconomicAuthorization`) stays the single
//! submission path — A1 only translates these types into it.
//!
//! Design notes:
//! - Types WE own (`SupernovaStatus`, `ExecutionResult`, `FinalityState`)
//!   are closed (`deny_unknown_fields`): unknown shape = rejection.
//! - Chain responses are read field-by-field (`Raw*` structs ignore unknown
//!   fields): forward-compatible across node versions; we validate the
//!   presence and shape of every field we ACT on, and treat malformed
//!   answers as rejections, never scrape around them.
//! - `nonce == 0` and `shard_id == 0` are VALID on MultiversX (first tx of
//!   an account; shard 0 exists). Validators must not reject them.
//! - Unknown activation config fails CLOSED (`active == false`), never open.

pub mod observer;
pub mod proxy;
pub mod types;

pub use observer::{
    MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS, MxTrack, ObserverConfig, ObserverSnapshot,
    poll_once, track,
};
pub use proxy::{
    DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT_MS, MAX_BYTES_CEILING, MxProxy, TIMEOUT_CEILING_MS,
};
pub use types::{
    BlockObservation, ChainStatus, ExecutionResult, FinalityState, MxTxRef, NetworkConfig,
    ObserverSnapshotDetails, ProofSummary, SupernovaError, SupernovaStatus, TxObservation,
};
