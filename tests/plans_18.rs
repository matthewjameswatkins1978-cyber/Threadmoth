use std::fs;

use tempfile::tempdir;
use threadmoth::{
    pipeline::{apply_prepared_plan, prepare_request_plan},
    protocol::{
        Assertion, Cardinality, EffectBudget, OperationPayload, Outcome, PlanApplyResult,
        RefusalReason, Request,
    },
    provider::text::TextOperation,
    workspace::Workspace,
};

fn request() -> Request {
    Request {
        version: "1.3.0".into(),
        request_id: "plan-test".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: "config.txt".into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::Text(TextOperation::Replace {
            target: "old".into(),
            replacement: "new".into(),
        }),
    }
}

#[test]
fn plans_are_deterministic_and_apply_exactly_once() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("config.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let assertions = vec![Assertion::LiteralCount {
        path: "config.txt".into(),
        literal: "new".into(),
        exactly: Some(1),
        minimum: None,
        maximum: None,
    }];
    let first = prepare_request_plan(&workspace, &request(), assertions.clone()).unwrap();
    let second = prepare_request_plan(&workspace, &request(), assertions).unwrap();
    assert_eq!(first.plan_id, second.plan_id);
    let rendered = serde_json::to_vec(&first).unwrap();
    let decoded = serde_json::from_slice(&rendered).unwrap();
    assert_eq!(first, decoded);
    let result = apply_prepared_plan(&workspace, &first);
    assert!(matches!(result, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied));
    assert_eq!(fs::read(temp.path().join("config.txt")).unwrap(), b"new\n");
}

#[test]
fn stale_plan_refuses_without_writing() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("config.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let plan = prepare_request_plan(&workspace, &request(), Vec::new()).unwrap();
    fs::write(temp.path().join("config.txt"), b"changed\n").unwrap();
    let result = apply_prepared_plan(&workspace, &plan);
    match result {
        PlanApplyResult::Certificate(certificate) => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert!(matches!(
                certificate.refusal_reason,
                Some(RefusalReason::PlanStale { .. })
            ));
        }
        PlanApplyResult::Transaction(_) => panic!("single-file plan returned a transaction"),
    }
    assert_eq!(
        fs::read(temp.path().join("config.txt")).unwrap(),
        b"changed\n"
    );
}

#[test]
fn prospective_assertion_failure_writes_nothing() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("config.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let error = prepare_request_plan(
        &workspace,
        &request(),
        vec![Assertion::LiteralCount {
            path: "config.txt".into(),
            literal: "never".into(),
            exactly: Some(1),
            minimum: None,
            maximum: None,
        }],
    )
    .unwrap_err();
    assert_eq!(error.outcome, Outcome::Refused);
    assert!(matches!(
        error.refusal_reason,
        Some(RefusalReason::PostconditionFailed { ref phase, .. }) if phase == "prospective"
    ));
    assert_eq!(fs::read(temp.path().join("config.txt")).unwrap(), b"old\n");
}
