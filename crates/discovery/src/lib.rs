//! Discovery service for worker pairing and scheduling
//!
//! This crate provides the core discovery functionality for DecentraAI, enabling:
//! - QR code-based worker pairing with cryptographic verification
//! - Persistent trust storage with SQLite
//! - Multi-factor worker scheduling with load balancing
//! - Task placement across distributed workers
//!
//! # Architecture
//!
//! The discovery service is split into two main modules:
//! - [`pairing`]: Handles QR code generation, pairing verification, and trust persistence
//! - [`scheduler`]: Manages worker registration, scoring, and task placement
//!
//! # Security Model
//!
//! Pairing uses Ed25519 signatures to ensure that only authorized workers can join the network.
//! Trust scores are persisted and updated based on successful/failed requests, with exponential
//! moving averages to smooth fluctuations.
//!
//! # Thread Safety
//!
//! The `TrustStore` uses internal mutex protection for SQLite access, while `WorkerScheduler`
//! is designed for single-threaded access (typically behind an actor pattern in the p2p layer).

pub mod pairing;
pub mod scheduler;

pub use pairing::{PairingCode, TrustRecordPersisted, TrustStore};
pub use scheduler::{WorkerScheduler, SchedulerConfig};

// Re-export from protocol
pub use decentraai_protocol::{WorkerStatus, TaskPlacement};
