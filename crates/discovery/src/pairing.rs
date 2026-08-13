//! QR code pairing and trust persistence
//!
//! This module provides secure worker pairing through QR codes with Ed25519 signature verification.
//! Trust relationships are persisted in SQLite for recovery across restarts.
//!
//! # Pairing Flow
//!
//! 1. Controller generates a `PairingCode` with worker/controller peer IDs and expiration
//! 2. QR code is displayed containing the serialized pairing code
//! 3. Worker scans QR, extracts the code, and verifies the signature
//! 4. Upon successful verification, a `TrustRecordPersisted` is created and stored
//! 5. Trust scores are updated based on successful/failed requests over time
//!
//! # Security
//!
//! - Pairing codes expire after a configurable TTL to prevent replay attacks
//! - Ed25519 signatures ensure only the intended controller can authorize workers
//! - Trust scores use exponential moving averages to smooth reputation changes
//! - SQLite database stores trust records with atomic writes for consistency

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use decentraai_identity::Identity;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Pairing code for QR generation
///
/// Contains the information needed to establish a trust relationship between
/// a worker and controller, including peer IDs, a unique token, and expiration time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCode {
    /// The worker's libp2p peer ID
    pub worker_peer_id: PeerId,
    /// The controller's libp2p peer ID
    pub controller_peer_id: PeerId,
    /// Unique token for this pairing session
    pub pairing_token: String,
    /// Unix timestamp when this pairing code expires
    pub expires_at: u64,
    /// Human-readable name for the worker node
    pub node_name: String,
}

impl PairingCode {
    /// Creates a new pairing code with the given parameters and TTL
    ///
    /// # Arguments
    ///
    /// * `worker_peer_id` - The worker's libp2p peer ID
    /// * `controller_peer_id` - The controller's libp2p peer ID
    /// * `node_name` - Human-readable name for the worker
    /// * `ttl_secs` - Time-to-live in seconds before the code expires
    ///
    /// # Example
    ///
    /// ```
    /// use libp2p::PeerId;
    /// use decentraai_discovery::PairingCode;
    ///
    /// let worker_id = PeerId::random();
    /// let controller_id = PeerId::random();
    /// let code = PairingCode::new(worker_id, controller_id, "worker-1".to_string(), 3600);
    /// ```
    pub fn new(
        worker_peer_id: PeerId,
        controller_peer_id: PeerId,
        node_name: String,
        ttl_secs: u64,
    ) -> Self {
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + ttl_secs;

        let pairing_token = uuid::Uuid::new_v4().to_string();

        Self {
            worker_peer_id,
            controller_peer_id,
            pairing_token,
            expires_at,
            node_name,
        }
    }

    /// Serialize to JSON string for QR code encoding
    ///
    /// Returns a JSON string that can be encoded as a QR code for scanning.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization fails.
    pub fn to_qr_data(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    /// Deserialize from QR scan result
    ///
    /// Parses a JSON string obtained from scanning a QR code back into a PairingCode.
    ///
    /// # Arguments
    ///
    /// * `data` - The JSON string from the QR code scan
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is invalid or doesn't match the expected structure.
    pub fn from_qr_data(data: &str) -> Result<Self> {
        let code: Self = serde_json::from_str(data).context("Invalid QR code data")?;
        Ok(code)
    }

    /// Checks if the pairing code has expired
    ///
    /// Returns true if the current time is past the expiration timestamp.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > self.expires_at
    }

    /// Create signed pairing message
    ///
    /// Generates an Ed25519 signature over the pairing data using the controller's identity.
    /// The signature can be verified by the worker to ensure the pairing code is authentic.
    ///
    /// # Arguments
    ///
    /// * `identity` - The controller's identity with Ed25519 keypair
    ///
    /// # Errors
    ///
    /// Returns an error if signing fails.
    pub fn sign_pairing(&self, identity: &Identity) -> Result<Vec<u8>> {
        let message = format!(
            "pair:{}:{}:{}:{}",
            self.worker_peer_id, self.controller_peer_id, self.pairing_token, self.expires_at
        );
        let signature = identity.sign(message.as_bytes());
        Ok(signature.to_bytes().to_vec())
    }

    /// Verify pairing signature
    ///
    /// Verifies that the signature was created by the expected controller.
    ///
    /// # Arguments
    ///
    /// * `signature` - The 64-byte Ed25519 signature to verify
    /// * `identity` - The controller's identity with public key
    ///
    /// # Returns
    ///
    /// Returns true if the signature is valid, false otherwise.
    pub fn verify_pairing(&self, signature: &[u8], identity: &Identity) -> bool {
        let message = format!(
            "pair:{}:{}:{}:{}",
            self.worker_peer_id, self.controller_peer_id, self.pairing_token, self.expires_at
        );

        if signature.len() != 64 {
            return false;
        }

        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(signature);

        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        decentraai_identity::verify_signature(identity.public_key(), message.as_bytes(), &sig)
            .is_ok()
    }
}

/// Trust record with persistent storage
///
/// Represents the trust relationship between a controller and worker,
/// including success/failure statistics and a computed trust score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecordPersisted {
    /// The worker's peer ID as a string
    pub worker_peer_id: String,
    /// The controller's peer ID as a string
    pub controller_peer_id: String,
    /// Human-readable name for the worker
    pub node_name: String,
    /// When the pairing was established
    pub paired_at: DateTime<Utc>,
    /// Last time this worker was seen/active
    pub last_seen: DateTime<Utc>,
    /// Computed trust score (0.0 to 1.0, higher is better)
    pub trust_score: f32,
    /// Total number of requests sent to this worker
    pub total_requests: u64,
    /// Number of successful requests
    pub successful_requests: u64,
    /// The original pairing token for reference
    pub pairing_token: String,
}

impl TrustRecordPersisted {
    /// Creates a new trust record from a pairing code
    ///
    /// Initializes the trust score to 1.0 (maximum trust) and sets timestamps
    /// to the current time.
    ///
    /// # Arguments
    ///
    /// * `pairing` - The pairing code that established this trust relationship
    pub fn new(pairing: &PairingCode) -> Self {
        let now = Utc::now();
        Self {
            worker_peer_id: pairing.worker_peer_id.to_string(),
            controller_peer_id: pairing.controller_peer_id.to_string(),
            node_name: pairing.node_name.clone(),
            paired_at: now,
            last_seen: now,
            trust_score: 1.0,
            total_requests: 0,
            successful_requests: 0,
            pairing_token: pairing.pairing_token.clone(),
        }
    }

    /// Records a successful request and updates the trust score
    ///
    /// Increments both total and successful request counters, updates the
    /// last seen timestamp, and recalculates the trust score using an
    /// exponential moving average.
    pub fn record_success(&mut self) {
        self.total_requests += 1;
        self.successful_requests += 1;
        self.last_seen = Utc::now();
        self.update_trust_score();
    }

    /// Records a failed request and updates the trust score
    ///
    /// Increments only the total request counter, updates the last seen
    /// timestamp, and recalculates the trust score.
    pub fn record_failure(&mut self) {
        self.total_requests += 1;
        self.last_seen = Utc::now();
        self.update_trust_score();
    }

    /// Updates the trust score using exponential moving average
    ///
    /// The trust score is updated as: `new_score = 0.8 * old_score + 0.2 * success_rate`
    /// This smooths out fluctuations while responding to recent performance.
    fn update_trust_score(&mut self) {
        if self.total_requests == 0 {
            return;
        }
        let success_rate = self.successful_requests as f32 / self.total_requests as f32;
        self.trust_score = 0.8 * self.trust_score + 0.2 * success_rate;
    }
}

/// Trust store with SQLite persistence
///
/// Provides persistent storage for trust records using SQLite. The store
/// maintains a table of trust records with atomic writes for consistency.
pub struct TrustStore {
    _db_path: String,
    conn: Option<rusqlite::Connection>,
}

impl TrustStore {
    /// Creates a new trust store with the given database path
    ///
    /// Opens the SQLite database and creates the trust_records table if it
    /// doesn't exist. The table stores all trust relationship data.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the SQLite database file
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the table cannot be created.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db_path = db_path.as_ref().to_string_lossy().to_string();
        let conn =
            rusqlite::Connection::open(&db_path).context("Failed to open trust store database")?;

        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS trust_records (
                worker_peer_id TEXT PRIMARY KEY,
                controller_peer_id TEXT NOT NULL,
                node_name TEXT NOT NULL,
                paired_at TEXT NOT NULL,
                last_seen TEXT NOT NULL,
                trust_score REAL NOT NULL,
                total_requests INTEGER NOT NULL,
                successful_requests INTEGER NOT NULL,
                pairing_token TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            _db_path: db_path,
            conn: Some(conn),
        })
    }

    /// Adds or updates a trust record in the database
    ///
    /// Uses INSERT OR REPLACE to atomically update existing records or create new ones.
    ///
    /// # Arguments
    ///
    /// * `record` - The trust record to persist
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub fn add_trust(&self, record: &TrustRecordPersisted) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO trust_records
             (worker_peer_id, controller_peer_id, node_name, paired_at, last_seen,
              trust_score, total_requests, successful_requests, pairing_token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                record.worker_peer_id,
                record.controller_peer_id,
                record.node_name,
                record.paired_at.to_rfc3339(),
                record.last_seen.to_rfc3339(),
                record.trust_score as f64,
                record.total_requests as i64,
                record.successful_requests as i64,
                record.pairing_token,
            ],
        )?;
        Ok(())
    }

    /// Retrieves a trust record by worker peer ID
    ///
    /// # Arguments
    ///
    /// * `worker_peer_id` - The worker's peer ID string
    ///
    /// # Returns
    ///
    /// Returns Some(trust_record) if found, None if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_trust(&self, worker_peer_id: &str) -> Result<Option<TrustRecordPersisted>> {
        let conn = self.conn.as_ref().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM trust_records WHERE worker_peer_id = ?1")?;

        let row = stmt.query_row([&worker_peer_id], |row| {
            let paired_at_str: String = row.get(3)?;
            let last_seen_str: String = row.get(4)?;
            let trust_score: f64 = row.get(5)?;
            let total_requests: i64 = row.get(6)?;
            let successful_requests: i64 = row.get(7)?;

            Ok(TrustRecordPersisted {
                worker_peer_id: row.get(0)?,
                controller_peer_id: row.get(1)?,
                node_name: row.get(2)?,
                paired_at: paired_at_str
                    .parse()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                last_seen: last_seen_str
                    .parse()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                trust_score: trust_score as f32,
                total_requests: total_requests as u64,
                successful_requests: successful_requests as u64,
                pairing_token: row.get(8)?,
            })
        });

        match row {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Lists all trusted workers
    ///
    /// Returns a vector of all trust records in the database.
    ///
    /// # Returns
    ///
    /// A vector of all trust records, or an error if the query fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn list_trusted(&self) -> Result<Vec<TrustRecordPersisted>> {
        let conn = self.conn.as_ref().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM trust_records")?;
        let records = stmt
            .query_map([], |row| {
                let paired_at_str: String = row.get(3)?;
                let last_seen_str: String = row.get(4)?;
                let trust_score: f64 = row.get(5)?;
                let total_requests: i64 = row.get(6)?;
                let successful_requests: i64 = row.get(7)?;

                Ok(TrustRecordPersisted {
                    worker_peer_id: row.get(0)?,
                    controller_peer_id: row.get(1)?,
                    node_name: row.get(2)?,
                    paired_at: paired_at_str
                        .parse()
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    last_seen: last_seen_str
                        .parse()
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    trust_score: trust_score as f32,
                    total_requests: total_requests as u64,
                    successful_requests: successful_requests as u64,
                    pairing_token: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(records)
    }

    /// Removes a trust record by worker peer ID
    ///
    /// # Arguments
    ///
    /// * `worker_peer_id` - The worker's peer ID string to remove
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub fn remove_trust(&self, worker_peer_id: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        conn.execute(
            "DELETE FROM trust_records WHERE worker_peer_id = ?1",
            [&worker_peer_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn record(worker: &str) -> TrustRecordPersisted {
        TrustRecordPersisted {
            worker_peer_id: worker.to_string(),
            controller_peer_id: "controller".to_string(),
            node_name: "worker-node".to_string(),
            paired_at: Utc::now(),
            last_seen: Utc::now(),
            trust_score: 1.0,
            total_requests: 3,
            successful_requests: 2,
            pairing_token: "tok".to_string(),
        }
    }

    fn store_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("trust.db")
    }

    #[test]
    fn trust_record_round_trips_through_sqlite() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = store_path(&dir);
        let store = TrustStore::new(&path).unwrap();

        let worker = "12D3KooWtestpeer000000000000000000000000000000000000";
        store.add_trust(&record(worker)).unwrap();

        // A fresh store (as a separate CLI invocation would open) must see it.
        let reopened = TrustStore::new(&path).unwrap();
        let listed = reopened.list_trusted().unwrap();
        assert_eq!(listed.len(), 1, "the stored record must round-trip");
        let got = listed[0].clone();
        assert_eq!(got.worker_peer_id, worker);
        assert_eq!(got.node_name, "worker-node");
        assert!((got.trust_score - 1.0).abs() < f32::EPSILON);
        assert_eq!(got.total_requests, 3);
        assert_eq!(got.successful_requests, 2);

        let by_id = reopened.get_trust(worker).unwrap().expect("record by id");
        assert_eq!(by_id.pairing_token, "tok");

        reopened.remove_trust(worker).unwrap();
        assert!(reopened.list_trusted().unwrap().is_empty());
    }
}
