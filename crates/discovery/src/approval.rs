//! Worker approval and trust records

use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use identity::Identity;
use chrono::{DateTime, Utc};

/// Worker approval from user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerApproval {
    pub worker_peer_id: PeerId,
    pub approver_peer_id: PeerId,
    pub signature: Vec<u8>,
    pub approved_at: u64,
    pub status: ApprovalStatus,
}

/// Approval status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalStatus {
    Approved,
    Revoked,
}

/// Trust record for persistent storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecord {
    pub worker_peer_id: PeerId,
    pub approved_at: DateTime<Utc>,
    pub trust_score: f32,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub last_seen: DateTime<Utc>,
}

impl WorkerApproval {
    pub fn create(
        worker_peer_id: PeerId,
        approver_identity: &Identity,
    ) -> anyhow::Result<Self> {
        let approver_peer_id = approver_identity.peer_id();
        let approved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Create message to sign
        let message = format!("approve_worker:{}:{}", worker_peer_id, approved_at);
        let signature = approver_identity.sign(message.as_bytes())?;

        Ok(Self {
            worker_peer_id,
            approver_peer_id,
            signature,
            approved_at,
            status: ApprovalStatus::Approved,
        })
    }

    pub fn verify(&self, approver_identity: &Identity) -> bool {
        let message = format!("approve_worker:{}:{}", self.worker_peer_id, self.approved_at);
        approver_identity.verify(message.as_bytes(), &self.signature)
    }

    pub fn revoke(&mut self, approver_identity: &Identity) -> anyhow::Result<()> {
        let revoked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let message = format!("revoke_worker:{}:{}", self.worker_peer_id, revoked_at);
        self.signature = approver_identity.sign(message.as_bytes())?;
        self.status = ApprovalStatus::Revoked;
        self.approved_at = revoked_at;

        Ok(())
    }
}

impl TrustRecord {
    pub fn new(worker_peer_id: PeerId) -> Self {
        let now = Utc::now();
        Self {
            worker_peer_id,
            approved_at: now,
            trust_score: 1.0,
            total_requests: 0,
            successful_requests: 0,
            last_seen: now,
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
        // Exponential moving average for trust score
        self.trust_score = 0.8 * self.trust_score + 0.2 * success_rate;
    }
}
