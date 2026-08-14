use decentraai_distributed::RequestQueueManager;
use decentraai_protocol::InferRequest;
use libp2p::identity::Keypair;
use std::time::Duration;

fn create_test_peer_id() -> libp2p::PeerId {
    let keypair = Keypair::generate_ed25519();
    libp2p::PeerId::from(keypair.public())
}

fn create_test_request(timeout_ms: u32) -> InferRequest {
    let mut req = InferRequest::new(
        "test-model-hash".to_string(),
        "test prompt".to_string(),
        100,
    );
    req.timeout_ms = timeout_ms;
    req
}

#[test]
fn test_queue_lifecycle_enqueue_dequeue_and_timeout() {
    // Short timeout for fast test
    let manager = RequestQueueManager::new(10, Duration::from_millis(50));

    let peer_id = create_test_peer_id();
    let req = create_test_request(10);

    // Queue it
    let queued = futures::executor::block_on(manager.queue_request(req.clone(), peer_id));
    assert!(queued, "request should be queued");

    // Should be one queued
    let total = futures::executor::block_on(manager.total_queued());
    assert_eq!(total, 1);

    // Wait for timeout to elapse
    std::thread::sleep(Duration::from_millis(30));

    // Cleanup timed out requests
    let timed_out = futures::executor::block_on(manager.cleanup_timed_out());
    assert!(
        !timed_out.is_empty(),
        "timed out requests should be returned"
    );

    // After cleanup total should be 0
    let total_after = futures::executor::block_on(manager.total_queued());
    assert_eq!(total_after, 0);

    // Now test enqueue/dequeue without timeout
    let req2 = create_test_request(10_000);
    let queued2 = futures::executor::block_on(manager.queue_request(req2.clone(), peer_id));
    assert!(queued2);

    let dequeued = futures::executor::block_on(manager.dequeue_request(&peer_id));
    assert!(dequeued.is_some());
}
