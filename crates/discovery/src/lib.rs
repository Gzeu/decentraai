//! Discovery service for worker pairing and scheduling

pub mod pairing;
pub mod scheduler;

pub use pairing::{PairingCode, TrustRecordPersisted, TrustStore};
pub use scheduler::{WorkerScheduler, SchedulerConfig};

// Re-export from protocol
pub use decentraai_protocol::{WorkerStatus, TaskPlacement};
