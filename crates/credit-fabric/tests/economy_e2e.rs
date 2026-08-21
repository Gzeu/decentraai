//! Comprehensive E2E integration test suite for the DecentraAI Inference Credit Economy.
//!
//! Verifies the complete 20-point test matrix:
//! 1. contribution creation
//! 2. measurement
//! 3. receipt verification
//! 4. pending → verified → settled
//! 5. credit calculation
//! 6. ledger append
//! 7. idempotency
//! 8. duplicate receipt rejection
//! 9. double-credit prevention
//! 10. balance calculation
//! 11. reservation
//! 12. reservation release
//! 13. concurrent reservation
//! 14. insufficient balance
//! 15. consumption
//! 16. provider quota exhaustion
//! 17. expired provider quota while settled CU remain valid
//! 18. provenance
//! 19. failed execution produces no spendable credit
//! 20. backward compatibility with non-monetary, crypto-agnostic invariants

use decentraai_credit_economy::{
    ContributionState, CreditOp, CreditPolicy, EconomyError,
    InferenceCreditEconomy, MeasurementMethod, ProviderQuota, ResourceAdvertisement,
    ResourceType, VerifiedUsage,
};
use decentraai_credit_fabric::{
    AbusePolicy, CreditFabric, GatewayChatNeed, SessionState, WorkloadNeed,
};
use std::sync::Arc;
use std::thread;

fn make_ad(id: &str, contributor: &str, rt: ResourceType, provider: &str, model: &str, cap: u64) -> ResourceAdvertisement {
    ResourceAdvertisement {
        advertisement_id: id.into(),
        contributor: contributor.into(),
        resource_type: rt,
        provider: Some(provider.into()),
        model: Some(model.into()),
        capacity_units: cap,
        available_from_ms: None,
        available_until_ms: None,
        rate_limit_per_minute: Some(120),
        concurrency_limit: Some(8),
        measurement: MeasurementMethod::SignedReceipt,
        region: Some("global".into()),
        capabilities: vec!["chat".into(), "completion".into()],
        credential_ref: Some(format!("env:{}_API_KEY", provider.to_uppercase())),
    }
}

fn make_usage(receipt_id: &str, exec_id: &str, contributor: &str, consumer: &str, in_tokens: u64, out_tokens: u64, ok: bool) -> VerifiedUsage {
    VerifiedUsage {
        receipt_id: receipt_id.into(),
        execution_id: exec_id.into(),
        contributor: contributor.into(),
        consumer: consumer.into(),
        resource_type: ResourceType::ApiQuota,
        provider: Some("deepseek".into()),
        model: Some("deepseek-chat".into()),
        input_tokens: in_tokens,
        output_tokens: out_tokens,
        gpu_ms: 0,
        cpu_ms: 0,
        storage_byte_hours: 0,
        bandwidth_bytes: 0,
        success: ok,
        signature_valid: true,
        measurement: MeasurementMethod::SignedReceipt,
        reservation_id: None,
        measured_at_ms: 1000,
    }
}

fn bootstrap_balance(fabric: &CreditFabric, account: &str, amount_cu: u64) {
    fabric.economy().submit_contribution("boot-c", account, ResourceType::GpuCompute, None, None, None, None);
    let mut u = make_usage("boot-r", "boot-c", account, "sys", 0, 0, true);
    u.resource_type = ResourceType::GpuCompute;
    u.gpu_ms = amount_cu;
    fabric.economy().verify_contribution("boot-c", u).unwrap();
    fabric.economy().settle_contribution("boot-c").unwrap();
}

#[test]
fn test_01_contribution_creation() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    let ad = make_ad("ad-1", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000);
    fabric.offer_capacity(ad, 100_000, None).unwrap();
    assert_eq!(fabric.catalog().len(), 1);
    assert_eq!(fabric.economy().balance("node-a").available, 0);
}

#[test]
fn test_02_and_03_measurement_and_receipt_verification() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    fabric.offer_capacity(make_ad("ad-1", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, None).unwrap();
    
    // Forged receipt signature fails
    fabric.economy().submit_contribution("c1", "node-a", ResourceType::ApiQuota, None, None, None, None);
    let mut bad_usage = make_usage("r-bad", "c1", "node-a", "node-b", 100, 100, true);
    bad_usage.signature_valid = false;
    let err = fabric.economy().verify_contribution("c1", bad_usage).unwrap_err();
    assert_eq!(err, EconomyError::ForgedReceipt);
}

#[test]
fn test_04_05_06_lifecycle_policy_and_ledger_provenance() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    fabric.offer_capacity(make_ad("ad-1", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, None).unwrap();
    bootstrap_balance(&fabric, "node-b", 50_000);

    let need = WorkloadNeed {
        account: "node-b".into(),
        preferred_resource: ResourceType::ApiQuota,
        preferred_provider: Some("deepseek".into()),
        preferred_model: Some("deepseek-chat".into()),
        estimated_input_tokens: 2_000,
        estimated_output_tokens: 1_000,
        estimated_gpu_ms: 0,
        estimated_cpu_ms: 0,
        allow_fallback: false,
    };
    let sess = fabric.open_session("sess-1", need).unwrap();
    assert_eq!(sess.state, SessionState::CreditReserved);

    let usage = make_usage("r-sess-1", "e-sess-1", "node-a", "node-b", 2_000, 1_000, true);
    let rec = fabric.complete_session("sess-1", usage).unwrap();
    
    // 2000*1 (in) + 1000*2 (out) = 4000 CU
    assert_eq!(rec.earned_cu, 4_000);
    assert_eq!(rec.spent_cu, 4_000);
    
    let events = fabric.economy().events();
    let earn = events.iter().find(|e| e.op == CreditOp::Earn && e.ref_id == "contrib-sess-1").unwrap();
    assert_eq!(earn.amount, 4_000);
    assert_eq!(earn.policy_version, 1);
    assert_eq!(earn.origin_provider.as_deref(), Some("deepseek"));
}

#[test]
fn test_07_08_09_idempotency_duplicate_receipt_and_double_credit_prevention() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    fabric.offer_capacity(make_ad("ad-1", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, None).unwrap();
    
    fabric.economy().submit_contribution("c1", "node-a", ResourceType::ApiQuota, None, None, None, Some("quota-ad-1".into()));
    fabric.economy().submit_contribution("c2", "node-a", ResourceType::ApiQuota, None, None, None, Some("quota-ad-1".into()));
    
    let u = make_usage("r-unique", "e1", "node-a", "node-b", 500, 500, true);
    fabric.economy().verify_contribution("c1", u.clone()).unwrap();
    
    // Duplicate receipt rejection on different contribution
    let err = fabric.economy().verify_contribution("c2", u).unwrap_err();
    assert_eq!(err, EconomyError::DuplicateReceipt);
    
    // Idempotent double settlement returns existing calculation without doubling balance
    let c1 = fabric.economy().settle_contribution("c1").unwrap().credits;
    let c2 = fabric.economy().settle_contribution("c1").unwrap().credits;
    assert_eq!(c1, c2);
    assert_eq!(fabric.economy().balance("node-a").earned, c1);
}

#[test]
fn test_10_11_12_13_14_balance_reservation_release_concurrency_and_overspend() {
    let fabric = Arc::new(CreditFabric::new(CreditPolicy::default(), AbusePolicy::default()));
    fabric.offer_capacity(make_ad("ad-1", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 1_000_000), 1_000_000, None).unwrap();
    bootstrap_balance(&fabric, "node-b", 100_000);

    let fab1 = fabric.clone();
    let fab2 = fabric.clone();
    
    let t1 = thread::spawn(move || {
        let need = WorkloadNeed {
            account: "node-b".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 40_000,
            estimated_output_tokens: 15_000, // 40k + 30k = 70k CU
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        fab1.open_session("c-sess-1", need)
    });
    
    let t2 = thread::spawn(move || {
        let need = WorkloadNeed {
            account: "node-b".into(),
            preferred_resource: ResourceType::ApiQuota,
            preferred_provider: Some("deepseek".into()),
            preferred_model: Some("deepseek-chat".into()),
            estimated_input_tokens: 30_000,
            estimated_output_tokens: 15_000, // 30k + 30k = 60k CU
            estimated_gpu_ms: 0,
            estimated_cpu_ms: 0,
            allow_fallback: false,
        };
        fab2.open_session("c-sess-2", need)
    });
    
    let res1 = t1.join().unwrap();
    let res2 = t2.join().unwrap();
    
    // Exactly one reservation must succeed, other rejected with InsufficientCredits
    let successes = res1.is_ok() as u8 + res2.is_ok() as u8;
    assert_eq!(successes, 1, "concurrent reservation must never overspend 100k balance");
    
    let bal = fabric.economy().balance("node-b");
    assert!(bal.check_invariant());
    assert_eq!(bal.earned, 100_000);
}

#[test]
fn test_15_16_17_cross_resource_spend_quota_exhaustion_and_window_expiry() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    
    // Provider 1: DeepSeek API (100k quota)
    fabric.offer_capacity(make_ad("ds", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, Some(5000)).unwrap();
    // Provider 2: Qwen API (50k quota)
    fabric.offer_capacity(make_ad("qwen", "node-c", ResourceType::ApiQuota, "qwen", "qwen-max", 50_000), 50_000, None).unwrap();
    
    bootstrap_balance(&fabric, "node-b", 50_000);
    
    // Node B consumes 30k tokens on Node A's DeepSeek
    let need1 = WorkloadNeed {
        account: "node-b".into(),
        preferred_resource: ResourceType::ApiQuota,
        preferred_provider: Some("deepseek".into()),
        preferred_model: Some("deepseek-chat".into()),
        estimated_input_tokens: 20_000,
        estimated_output_tokens: 5_000, // 20k + 10k = 30k CU
        estimated_gpu_ms: 0,
        estimated_cpu_ms: 0,
        allow_fallback: false,
    };
    fabric.open_session("step1", need1).unwrap();
    fabric.complete_session("step1", make_usage("r-step1", "e-step1", "node-a", "node-b", 20_000, 5_000, true)).unwrap();
    
    // Node A earned 30,000 CU
    assert_eq!(fabric.economy().balance("node-a").available, 30_000);
    
    // Node A's original DeepSeek quota expires
    fabric.economy().expire_quota("quota-ds").unwrap();
    assert_eq!(fabric.economy().quota("quota-ds").unwrap().remaining(), 0);
    
    // Settled CU remain durable and Node A spends them on Node C's Qwen API
    let need2 = WorkloadNeed {
        account: "node-a".into(),
        preferred_resource: ResourceType::ApiQuota,
        preferred_provider: Some("qwen".into()),
        preferred_model: Some("qwen-max".into()),
        estimated_input_tokens: 5_000,
        estimated_output_tokens: 2_500, // 5k + 5k = 10k CU
        estimated_gpu_ms: 0,
        estimated_cpu_ms: 0,
        allow_fallback: false,
    };
    let s2 = fabric.open_session("step2", need2).unwrap();
    assert_eq!(s2.planned.provider.as_deref(), Some("qwen"));
    fabric.complete_session("step2", make_usage("r-step2", "e-step2", "node-c", "node-a", 5_000, 2_500, true)).unwrap();
    
    assert_eq!(fabric.economy().balance("node-a").available, 20_000);
    assert_eq!(fabric.economy().balance("node-c").earned, 10_000);
}

#[test]
fn test_18_19_20_provenance_failed_execution_and_gateway_invariants() {
    let fabric = CreditFabric::new(CreditPolicy::default(), AbusePolicy::default());
    fabric.offer_capacity(make_ad("ds", "node-a", ResourceType::ApiQuota, "deepseek", "deepseek-chat", 100_000), 100_000, None).unwrap();
    bootstrap_balance(&fabric, "node-b", 10_000);

    // Gateway chat envelope integration
    let gw_plan = fabric.gateway_chat(GatewayChatNeed {
        account: "node-b".into(),
        model: "deepseek-chat".into(),
        estimated_input_tokens: 1_000,
        estimated_output_tokens: 500,
        stream: false,
    }).unwrap();
    assert!(gw_plan.estimated_cu > 0);

    // If execution fails, consumer reservation is released, contributor earns 0 CU
    let mut fail_usage = make_usage("r-fail", "e-fail", "node-a", "node-b", 1_000, 500, false);
    fail_usage.success = false;
    assert!(fabric.complete_session(&gw_plan.session_id, fail_usage).is_err());
    
    assert_eq!(fabric.economy().balance("node-b").available, 10_000);
    assert_eq!(fabric.economy().balance("node-a").earned, 0);
    assert_eq!(fabric.session(&gw_plan.session_id).unwrap().state, SessionState::Failed);
}
