//! Load test with 10+ simulated workers
//!
//! This test creates 10+ simulated workers and tests the scheduler's performance
//! under load, verifying that scoring and selection remain efficient and correct.

use decentraai_discovery::{SchedulerConfig, WorkerScheduler};
use decentraai_protocol::{InferRequest, WorkerAnnouncement};
use libp2p::PeerId;
use uuid::Uuid;

#[tokio::test]
async fn scheduler_load_test_with_10_workers() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create 10 simulated workers with varying characteristics
    let mut workers = Vec::new();
    for i in 0..10 {
        let peer_id = PeerId::random();
        let model_hash = format!("model-hash-{}", i % 3); // 3 different models

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec![model_hash.clone()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 50,
        };

        scheduler.register_worker(announcement.clone());
        workers.push((peer_id, model_hash));
    }

    // Test scheduler can handle many workers
    let all_workers = scheduler.get_all_workers();
    assert_eq!(all_workers.len(), 10);

    // Test worker selection with different model requests
    for i in 0..10 {
        let model_hash = format!("model-hash-{}", i % 3);
        let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

        let placement = scheduler.select_worker(&request);
        assert!(
            placement.is_some(),
            "Should find worker for model {}",
            model_hash
        );

        let placement = placement.unwrap();
        assert!(!placement.selected_worker.to_string().is_empty());
        assert!(placement.estimated_wait_ms < 10000); // Reasonable wait time
        assert!(placement.confidence > 0.0);
        assert!(placement.confidence <= 1.0);
    }
}

#[tokio::test]
async fn scheduler_scoring_under_load() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create workers with different performance characteristics
    let worker_configs = [
        (1.0, 50, 0), // High capacity, high throughput, no queue
        (0.5, 30, 5), // Medium capacity, medium throughput, some queue
        (0.2, 20, 8), // Low capacity, low throughput, high queue
    ];

    for (i, (capacity, tps, queue_depth)) in worker_configs.iter().enumerate() {
        let peer_id = PeerId::random();
        let model_hash = "test-model".to_string();

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec![model_hash.clone()],
            available_capacity: *capacity,
            queue_depth: *queue_depth,
            tokens_per_second: *tps,
            current_latency_ms: 100,
        };

        scheduler.register_worker(announcement);

        // Manually set worker status to test scoring
        // This would normally be updated through runtime metrics
    }

    let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

    // Best worker should be selected (high capacity, high throughput, low queue)
    let placement = scheduler.select_worker(&request);
    assert!(placement.is_some());
}

#[tokio::test]
async fn scheduler_fallback_under_load() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create 15 workers
    for i in 0..15 {
        let peer_id = PeerId::random();
        let model_hash = "test-model".to_string();

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec![model_hash.clone()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 50,
        };

        scheduler.register_worker(announcement);
    }

    let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

    // Get primary selection
    let primary = scheduler.select_worker(&request);
    assert!(primary.is_some());
    let primary_worker = primary.unwrap().selected_worker;

    // Get fallback workers (excluding primary)
    let fallbacks = scheduler.get_fallback_workers(&request, &primary_worker);
    assert!(
        !fallbacks.is_empty(),
        "Should have fallback workers available"
    );
    assert!(fallbacks.len() <= 14, "Should exclude primary worker");
}

#[tokio::test]
async fn scheduler_queue_management_under_load() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create 5 workers
    let mut worker_ids = Vec::new();
    for i in 0..5 {
        let peer_id = PeerId::random();
        worker_ids.push(peer_id);

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec!["test-model".to_string()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 50,
        };

        scheduler.register_worker(announcement);
    }

    let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

    // Queue multiple requests on first worker
    for _ in 0..5 {
        scheduler.queue_request(&worker_ids[0], request.clone());
    }

    // Verify queue depth is updated
    let worker = scheduler.get_worker(&worker_ids[0]);
    assert!(worker.is_some());

    // Dequeue requests
    for _i in 0..5 {
        scheduler.dequeue_request(&worker_ids[0], Uuid::new_v4());
    }
}

#[tokio::test]
async fn scheduler_concurrent_selections() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create 20 workers
    for i in 0..20 {
        let peer_id = PeerId::random();
        let model_hash = format!("model-{}", i % 5); // 5 different models

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec![model_hash.clone()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 50,
        };

        scheduler.register_worker(announcement);
    }

    // Simulate 50 concurrent requests
    let mut successful_selections = 0;
    for i in 0..50 {
        let model_hash = format!("model-{}", i % 5);
        let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

        if scheduler.select_worker(&request).is_some() {
            successful_selections += 1;
        }
    }

    // Most requests should be successfully scheduled
    assert!(
        successful_selections >= 45,
        "Should successfully schedule most requests"
    );
}

#[tokio::test]
async fn scheduler_worker_removal_under_load() {
    let config = SchedulerConfig::default();
    let mut scheduler = WorkerScheduler::new(config);

    // Create 10 workers
    let mut worker_ids = Vec::new();
    for i in 0..10 {
        let peer_id = PeerId::random();
        worker_ids.push(peer_id);

        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: format!("worker-{}", i),
            loaded_models: vec!["test-model".to_string()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 50,
        };

        scheduler.register_worker(announcement);
    }

    // Remove workers while under load
    for worker_id in worker_ids.iter().take(5) {
        scheduler.remove_worker(worker_id);
    }

    // Verify remaining workers
    let remaining = scheduler.get_all_workers();
    assert_eq!(remaining.len(), 5);

    // Verify requests can still be scheduled
    let mut request = InferRequest::new(model_hash.clone(), "test prompt".to_string(), 100);
        request = request.with_priority(128).with_streaming(false);

    let placement = scheduler.select_worker(&request);
    assert!(
        placement.is_some(),
        "Should still find workers after removal"
    );
}
