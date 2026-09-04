//! v0.2 testnet lane: the twelve hard-limit cases.
//!
//! Every denial is deterministic and typed; the single happy path proves
//! an authorized bounded action executes exactly once, captures its tx
//! hash into sealed evidence, and survives restarts without double-spend.

use std::cell::Cell;

use decentraai_proposal::{
    DenyAllEconomicAuthorization, DenyReason, EconomicAuthError, EconomicAuthorization,
    ExecutionMode, ExperimentBudget, ExperimentEvidence, ExperimentOutcome, ExperimentStore,
    PolicyDecision, TestnetApproval, TestnetAsset, TestnetAuthConfig, TestnetAuthRequest,
    TestnetEconomicAuthorization, TestnetExecutor, TestnetReport, assess, decide,
    execute_testnet_experiment, parse_proposal,
};

const NOW: u64 = 1_780_000_000;
const DEST: &str = "erd1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq6gq4hu";

fn budget() -> ExperimentBudget {
    ExperimentBudget {
        id: "budget:first".to_string(),
        max_amount_wei: 1_000,
        max_gas: 60_000,
        max_actions: 1,
        max_retries: 1,
        expiry_unix: NOW + 3_600,
        allowed_assets: vec![TestnetAsset::Xegld],
        allowed_destinations: vec![DEST.to_string()],
    }
}

fn proposal_json(budget: &ExperimentBudget) -> String {
    let b = serde_json::to_value(budget).unwrap();
    serde_json::json!({
        "version": 1,
        "id": "prop:first-testnet",
        "idea_id": "idea:first-testnet",
        "risk": "testnet_economic",
        "commitment": "cr",
        "budget": b,
        "created_by": "agent:primordial-1",
        "steps": [
            {"id": "s1", "rationale": "minimal self-transfer proves the loop",
             "action": {"kind": "testnet_transfer", "asset": "xegld",
                        "destination": DEST, "amount_wei": 1_000}}
        ]
    })
    .to_string()
}

fn enabled_auth() -> TestnetEconomicAuthorization {
    TestnetEconomicAuthorization::new(TestnetAuthConfig {
        enabled: true,
        chain_id: "T".to_string(),
    })
    .unwrap()
}

fn auth_request(budget: &ExperimentBudget, now: u64) -> TestnetAuthRequest {
    TestnetAuthRequest {
        proposal_id: "prop:first-testnet".to_string(),
        chain_id: "T".to_string(),
        asset: TestnetAsset::Xegld,
        destination: DEST.to_string(),
        amount_wei: 1_000,
        gas: 50_000,
        actions: 1,
        attempts_used: 0,
        now_unix: now,
        policy_allowed: true,
        budget: budget.clone(),
    }
}

struct CountingMock {
    calls: Cell<u32>,
    hash: String,
}

impl TestnetExecutor for CountingMock {
    fn execute_transfer(
        &self,
        _intent: &decentraai_proposal::AuthorizedTransfer,
    ) -> Result<String, decentraai_proposal::ProposalError> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.hash.clone())
    }
}

fn mock() -> CountingMock {
    CountingMock {
        calls: Cell::new(0),
        hash: "txhash:mock001".to_string(),
    }
}

fn happy_setup() -> (
    decentraai_proposal::ExperimentProposal,
    PolicyDecision,
    TestnetApproval,
    ExperimentBudget,
) {
    let b = budget();
    let p = parse_proposal(&proposal_json(&b)).expect("valid testnet proposal");
    let d = decide(&p, &DenyAllEconomicAuthorization, NOW);
    assert_eq!(
        d,
        PolicyDecision::Allow {
            mode: ExecutionMode::Testnet
        }
    );
    let approval = enabled_auth()
        .authorize_testnet(&auth_request(&b, NOW))
        .expect("bounded request approves");
    (p, d, approval, b)
}

// 9. authorized Testnet action → ALLOWED (policy + auth + execution).
// 10. tx hash captured in sealed evidence.
#[test]
fn authorized_testnet_action_allowed_and_evidenced() {
    let (p, d, approval, b) = happy_setup();
    let ex = mock();
    let mut store = ExperimentStore::new();
    let report: TestnetReport = execute_testnet_experiment(
        "exp:first",
        &p,
        &d,
        &approval,
        &b,
        NOW * 1_000,
        &mut store,
        &ex,
    )
    .expect("executes once");
    assert_eq!(ex.calls.get(), 1);
    assert_eq!(report.tx_hash, "txhash:mock001");
    assert!(!report.replayed);

    let ev = ExperimentEvidence::seal_testnet(
        &report,
        ExperimentOutcome::Success,
        b.max_amount_wei,
        NOW * 1_000,
        None,
    );
    assert!(ev.verify_seal());
    let t = ev.testnet.expect("testnet facts present");
    assert_eq!(t.tx_hash, "txhash:mock001");
    assert_eq!(t.amount_wei, 1_000);
    assert_eq!(t.authorized_wei, 1_000);
    assert_eq!(t.destination, DEST);

    // Learning separates the three successes (tx confirmed ≠ hypothesis).
    let l = assess(
        "exp:first",
        &p.id,
        &ev.id,
        true,
        true,
        decentraai_proposal::HypothesisVerdict::Supported,
        Some(report.tx_hash.clone()),
    );
    assert!(l.execution_success && l.experiment_success);
}

// 12. duplicate execution is idempotent (executor untouched twice).
#[test]
fn duplicate_execution_is_idempotent() {
    let (p, d, approval, b) = happy_setup();
    let ex = mock();
    let mut store = ExperimentStore::new();
    let r1 = execute_testnet_experiment(
        "exp:dup",
        &p,
        &d,
        &approval,
        &b,
        NOW * 1_000,
        &mut store,
        &ex,
    )
    .unwrap();
    let r2 = execute_testnet_experiment(
        "exp:dup",
        &p,
        &d,
        &approval,
        &b,
        NOW * 1_000,
        &mut store,
        &ex,
    )
    .unwrap();
    assert_eq!(ex.calls.get(), 1, "executor ran exactly once");
    assert!(r2.replayed);
    assert_eq!(r1.tx_hash, r2.tx_hash);
}

// 11. restart preserves experiment state (JSON round-trip, no double-spend).
#[test]
fn restart_preserves_experiment_state() {
    let (p, d, approval, b) = happy_setup();
    let ex = mock();
    let mut store = ExperimentStore::new();
    execute_testnet_experiment(
        "exp:re",
        &p,
        &d,
        &approval,
        &b,
        NOW * 1_000,
        &mut store,
        &ex,
    )
    .unwrap();
    let json = store.to_json().unwrap();
    let mut reloaded = ExperimentStore::from_json(&json).unwrap();
    let ex2 = mock();
    let r = execute_testnet_experiment(
        "exp:re",
        &p,
        &d,
        &approval,
        &b,
        NOW * 1_000,
        &mut reloaded,
        &ex2,
    )
    .unwrap();
    assert!(r.replayed, "reloaded store replays, never re-executes");
    assert_eq!(ex2.calls.get(), 0);
}

// 1. budget exceeded → DENIED.
#[test]
fn budget_exceeded_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.amount_wei = b.max_amount_wei + 1;
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::BudgetExceeded { .. }
    ));
}

// 2. expired experiment → DENIED.
#[test]
fn expired_experiment_denied() {
    let b = budget();
    let req = auth_request(&b, NOW + 7_200);
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::Expired { .. }
    ));
}

// 3. mainnet target → DENIED.
#[test]
fn mainnet_target_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.chain_id = "1".to_string();
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::NotTestnet { .. }
    ));
}

// 4. wrong asset → DENIED.
#[test]
fn wrong_asset_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.asset = TestnetAsset::Dcai;
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::WrongAsset { .. }
    ));
}

// 5. arbitrary destination → DENIED.
#[test]
fn arbitrary_destination_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.destination = "erd1attacker".to_string();
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::ArbitraryDestination { .. }
    ));
}

// 6. kill switch → DENIED (everything, even perfect requests).
#[test]
fn kill_switch_denied() {
    let off = TestnetEconomicAuthorization::new(TestnetAuthConfig::default()).unwrap();
    let b = budget();
    let req = auth_request(&b, NOW);
    assert_eq!(
        off.authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::KillSwitch
    );
}

// 7. retry budget exceeded → DENIED.
#[test]
fn retry_budget_exceeded_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.attempts_used = b.max_retries + 1;
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::RetryBudgetExceeded { .. }
    ));
}

// 8. missing policy approval → DENIED.
#[test]
fn missing_policy_approval_denied() {
    let b = budget();
    let mut req = auth_request(&b, NOW);
    req.policy_allowed = false;
    assert!(matches!(
        enabled_auth().authorize_testnet(&req).unwrap_err(),
        EconomicAuthError::MissingPolicyApproval { .. }
    ));
}

// Testnet lane without budget → policy DENIED (no limit = no experiment).
#[test]
fn testnet_without_budget_denied_by_policy() {
    let b = budget();
    let mut v: serde_json::Value = serde_json::from_str(&proposal_json(&b)).unwrap();
    v.as_object_mut().unwrap().remove("budget");
    let p = parse_proposal(&v.to_string()).expect("parses without budget");
    assert!(matches!(
        decide(&p, &DenyAllEconomicAuthorization, NOW),
        PolicyDecision::Deny {
            reason: DenyReason::MissingBudget(_)
        }
    ));
}

// Always-denied actions stay denied in the testnet lane too.
#[test]
fn classic_economic_actions_denied_in_testnet_lane() {
    let b = budget();
    let mut v: serde_json::Value = serde_json::from_str(&proposal_json(&b)).unwrap();
    v["steps"][0]["action"] = serde_json::json!({"kind": "fund_transfer", "detail": "x"});
    let p = parse_proposal(&v.to_string()).expect("parses structurally");
    assert!(matches!(
        decide(&p, &DenyAllEconomicAuthorization, NOW),
        PolicyDecision::Deny {
            reason: DenyReason::EconomicAction { .. }
        }
    ));
}
