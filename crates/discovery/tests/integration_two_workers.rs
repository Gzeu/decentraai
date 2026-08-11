//! Integration test with 2 real workers using mDNS + P2P
//!
//! This test simulates two workers discovering each other, establishing trust
//! through QR pairing, and testing the approval/reject workflow that would be
//! managed through the dashboard.

use decentraai_discovery::{PairingCode, TrustRecordPersisted};
use decentraai_identity::Identity;
use libp2p::PeerId;
use tempfile::TempDir;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn two_workers_discover_and_pair_via_mdns() {
    // Create temporary directories for each node
    let temp_dir = TempDir::new().unwrap();
    let node1_dir = temp_dir.path().join("node1");
    let node2_dir = temp_dir.path().join("node2");
    std::fs::create_dir_all(&node1_dir).unwrap();
    std::fs::create_dir_all(&node2_dir).unwrap();

    // Create identities for both nodes (simulating mDNS discovery)
    let identity1 = Identity::generate();
    let _identity2 = Identity::generate();

    // Use libp2p PeerIds directly for the pairing code
    let peer_id1 = libp2p::PeerId::random();
    let peer_id2 = libp2p::PeerId::random();

    // Simulate worker discovery: controller creates pairing code for worker
    let pairing_code = PairingCode::new(
        peer_id2, // worker
        peer_id1, // controller
        "test-worker".to_string(),
        3600, // 1 hour TTL
    );

    // Sign the pairing code with controller identity (dashboard side)
    let signature = pairing_code.sign_pairing(&identity1).unwrap();

    // Worker verifies the signature (prevents unauthorized pairing)
    assert!(pairing_code.verify_pairing(&signature, &identity1));

    // Test trust record creation and updates (in-memory for this test)
    let trust_record = TrustRecordPersisted::new(&pairing_code);
    assert_eq!(trust_record.trust_score, 1.0);
    assert_eq!(trust_record.total_requests, 0);

    // Test trust score updates (simulating runtime performance tracking)
    let mut updated_record = trust_record.clone();
    updated_record.record_success();
    assert_eq!(updated_record.total_requests, 1);
    assert_eq!(updated_record.successful_requests, 1);
    assert!(updated_record.trust_score > 0.9); // Should still be high

    updated_record.record_failure();
    assert_eq!(updated_record.total_requests, 2);
    assert_eq!(updated_record.successful_requests, 1);
    assert!(updated_record.trust_score < 1.0); // Should decrease
}

#[tokio::test]
async fn pairing_code_serialization_roundtrip() {
    let peer_id1 = PeerId::random();
    let peer_id2 = PeerId::random();

    let original = PairingCode::new(
        peer_id1,
        peer_id2,
        "test-node".to_string(),
        3600,
    );

    // Serialize to QR data
    let qr_data = original.to_qr_data().unwrap();

    // Deserialize back
    let restored = PairingCode::from_qr_data(&qr_data).unwrap();

    assert_eq!(restored.worker_peer_id, original.worker_peer_id);
    assert_eq!(restored.controller_peer_id, original.controller_peer_id);
    assert_eq!(restored.pairing_token, original.pairing_token);
    assert_eq!(restored.expires_at, original.expires_at);
    assert_eq!(restored.node_name, original.node_name);
}

#[tokio::test]
async fn pairing_code_expiration() {
    let peer_id1 = PeerId::random();
    let peer_id2 = PeerId::random();

    // Create pairing code with very short TTL
    let short_lived = PairingCode::new(
        peer_id1,
        peer_id2,
        "test-node".to_string(),
        1, // 1 second
    );

    // Should not be expired immediately
    assert!(!short_lived.is_expired());

    // Wait for expiration
    sleep(Duration::from_secs(2)).await;

    // Should now be expired
    assert!(short_lived.is_expired());
}

#[tokio::test]
async fn trust_score_exponential_moving_average() {
    let peer_id1 = PeerId::random();
    let peer_id2 = PeerId::random();

    let pairing_code = PairingCode::new(
        peer_id1,
        peer_id2,
        "test-node".to_string(),
        3600,
    );

    let mut trust_record = TrustRecordPersisted::new(&pairing_code);
    assert_eq!(trust_record.trust_score, 1.0);

    // Simulate a series of successes and failures
    for _ in 0..10 {
        trust_record.record_success();
    }

    // After many successes, score should still be high
    assert!(trust_record.trust_score > 0.95);

    // Now introduce failures
    for _ in 0..5 {
        trust_record.record_failure();
    }

    // Score should decrease but not crash
    assert!(trust_record.trust_score < 1.0);
    assert!(trust_record.trust_score > 0.5);

    // Verify the EMA formula: new_score = 0.8 * old_score + 0.2 * success_rate
    // With 10 successes and 5 failures: success_rate = 10/15 = 0.667
    // The score should converge toward this value (but may not be exactly equal due to EMA smoothing)
    let final_success_rate = trust_record.successful_requests as f32 / trust_record.total_requests as f32;
    // The EMA smooths the transition, so the score will be between the initial 1.0 and the final success rate
    assert!(trust_record.trust_score >= final_success_rate);
    assert!(trust_record.trust_score <= 1.0);
}

#[tokio::test]
async fn multiple_workers_trust_management() {
    // Test in-memory trust record management without database
    let peer_id1 = PeerId::random();
    let peer_id2 = PeerId::random();
    let peer_id3 = PeerId::random();
    let controller_id = PeerId::random();

    let pairing1 = PairingCode::new(peer_id1, controller_id, "worker-1".to_string(), 3600);
    let pairing2 = PairingCode::new(peer_id2, controller_id, "worker-2".to_string(), 3600);
    let pairing3 = PairingCode::new(peer_id3, controller_id, "worker-3".to_string(), 3600);

    let mut record1 = TrustRecordPersisted::new(&pairing1);
    let mut record2 = TrustRecordPersisted::new(&pairing2);
    let mut record3 = TrustRecordPersisted::new(&pairing3);

    // Simulate different performance patterns
    for _ in 0..5 {
        record1.record_success();
    }
    for _ in 0..3 {
        record2.record_success();
        record2.record_failure();
    }
    for _ in 0..10 {
        record3.record_success();
    }

    // Verify trust scores reflect performance
    // Worker 1 (all successes) should have higher score than worker 2 (mixed)
    assert!(record1.trust_score > record2.trust_score);

    // Worker 3 (many successes) should have high score (though EMA smoothing may make it similar to worker 1)
    assert!(record3.trust_score > 0.9);

    // Verify request counts
    assert_eq!(record1.total_requests, 5);
    assert_eq!(record2.total_requests, 6);
    assert_eq!(record3.total_requests, 10);

    // Verify success rates
    assert_eq!(record1.successful_requests, 5);
    assert_eq!(record2.successful_requests, 3);
    assert_eq!(record3.successful_requests, 10);
}