//! QR code pairing and trust persistence

use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use identity::Identity;
use chrono::{DateTime, Utc};
use anyhow::{Context, Result};
use std::path::Path;

/// Pairing code for QR generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCode {
    pub worker_peer_id: PeerId,
    pub controller_peer_id: PeerId,
    pub pairing_token: String,
    pub expires_at: u64,
    pub node_name: String,
}

impl PairingCode {
    pub fn new(
        worker_peer_id: PeerId,
        controller_peer_id: PeerId,
        node_name: String,
        ttl_secs: u64,
    ) -> Self {
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() + ttl_secs;

        let pairing_token = uuid::Uuid::new_v4().to_string();

        Self {
            worker_peer_id,
            controller_peer_id,
            pairing_token,
            expires_at,
            node_name,
        }
    }

    /// Serialize to JSON string for QR code
    pub fn to_qr_data(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    /// Deserialize from QR scan result
    pub fn from_qr_data(data: &str) -> Result<Self> {
        let code: Self = serde_json::from_str(data)
            .context("Invalid QR code data")?;
        Ok(code)
    }

    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > self.expires_at
    }

    /// Create signed pairing message
    pub fn sign_pairing(&self, identity: &Identity) -> Result<Vec<u8>> {
        let message = format!(
            "pair:{}:{}:{}:{}",
            self.worker_peer_id,
            self.controller_peer_id,
            self.pairing_token,
            self.expires_at
        );
        identity.sign(message.as_bytes())
    }

    /// Verify pairing signature
    pub fn verify_pairing(&self, signature: &[u8], identity: &Identity) -> bool {
        let message = format!(
            "pair:{}:{}:{}:{}",
            self.worker_peer_id,
            self.controller_peer_id,
            self.pairing_token,
            self.expires_at
        );
        identity.verify(&message.as_bytes(), signature)
    }
}

/// Trust record with persistent storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecordPersisted {
    pub worker_peer_id: String,
    pub controller_peer_id: String,
    pub node_name: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub trust_score: f32,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub pairing_token: String,
}

impl TrustRecordPersisted {
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

    pub fn record_success(&mut self) {
        self.total_requests += 1;
        self.successful_requests += 1;
        self.last_seen = Utc::now();
        self.update_trust_score();
    }

    pub fn record_failure(&mut self) {
        self.total_requests += 1;
        self.last_seen = Utc::now();
        self.update_trust_score();
    }

    fn update_trust_score(&mut self) {
        if self.total_requests == 0 {
            return;
        }
        let success_rate = self.successful_requests as f32 / self.total_requests as f32;
        self.trust_score = 0.8 * self.trust_score + 0.2 * success_rate;
    }
}

/// Trust store with SQLite persistence
pub struct TrustStore {
    db_path: String,
    conn: Option<rusqlite::Connection>,
}

impl TrustStore {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db_path = db_path.as_ref().to_string_lossy().to_string();
        let conn = rusqlite::Connection::open(&db_path)
            .context("Failed to open trust store database")?;

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

        Ok(Self { db_path, conn: Some(conn) })
    }

    pub fn add_trust(&self, record: &TrustRecordPersisted) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO trust_records 
             (worker_peer_id, controller_peer_id, node_name, paired_at, last_seen, 
              trust_score, total_requests, successful_requests, pairing_token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            [
                &record.worker_peer_id,
                &record.controller_peer_id,
                &record.node_name,
                &record.paired_at.to_rfc3339(),
                &record.last_seen.to_rfc3339(),
                &record.trust_score.to_string(),
                &record.total_requests.to_string(),
                &record.successful_requests.to_string(),
                &record.pairing_token,
            ],
        )?;
        Ok(())
    }

    pub fn get_trust(&self, worker_peer_id: &str) -> Result<Option<TrustRecordPersisted>> {
        let conn = self.conn.as_ref().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM trust_records WHERE worker_peer_id = ?1"
        )?;

        let row = stmt.query_row([&worker_peer_id], |row| {
            Ok(TrustRecordPersisted {
                worker_peer_id: row.get(0)?,
                controller_peer_id: row.get(1)?,
                node_name: row.get(2)?,
                paired_at: row.get::<_, String>(3)?.parse()?,
                last_seen: row.get::<_, String>(4)?.parse()?,
                trust_score: row.get::<_, String>(5)?.parse()?,
                total_requests: row.get::<_, String>(6)?.parse()?,
                successful_requests: row.get::<_, String>(7)?.parse()?,
                pairing_token: row.get(8)?,
            })
        });

        match row {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_trusted(&self) -> Result<Vec<TrustRecordPersisted>> {
        let conn = self.conn.as_ref().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM trust_records")?;
        let records = stmt.query_map([], |row| {
            Ok(TrustRecordPersisted {
                worker_peer_id: row.get(0)?,
                controller_peer_id: row.get(1)?,
                node_name: row.get(2)?,
                paired_at: row.get::<_, String>(3)?.parse()?,
                last_seen: row.get::<_, String>(4)?.parse()?,
                trust_score: row.get::<_, String>(5)?.parse()?,
                total_requests: row.get::<_, String>(6)?.parse()?,
                successful_requests: row.get::<_, String>(7)?.parse()?,
                pairing_token: row.get(8)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(records)
    }

    pub fn remove_trust(&self, worker_peer_id: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        conn.execute("DELETE FROM trust_records WHERE worker_peer_id = ?1", [&worker_peer_id])?;
        Ok(())
    }
}
