use std::fs;
use std::process::Command;

use tempfile::tempdir;
use threadmoth::{
    pipeline::execute_request,
    protocol::{Cardinality, EffectBudget, OperationPayload, Outcome, Request},
    provider::text::TextOperation,
    workspace::{Workspace, WorkspaceError},
};

fn ambiguous_request() -> Request {
    Request {
        version: "1.3.1".into(),
        request_id: "hardening-181-ambiguity".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: "x.txt".into(),
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
fn ambiguity_certificate_contains_complete_guarded_remedies() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("x.txt"), b"old\nold\n").unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let certificate = execute_request(&workspace, &ambiguous_request(), true);

    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(certificate.reason_code.as_deref(), Some("TARGET_AMBIGUOUS"));
    let recovery = certificate.recovery.expect("refusal recovery is present");
    assert!(recovery.requires_choice);
    assert_eq!(recovery.remedies.len(), 2);
    for remedy in recovery.remedies {
        assert_eq!(remedy.kind, "candidate_selection");
        let patch = remedy
            .request_patch
            .expect("candidate remedy has a request patch");
        assert_eq!(patch["expected_pre_hash"], certificate.pre_hash);
        assert!(patch["candidate_guard"]["selection_id"].is_string());
    }
}

#[test]
fn mutation_lock_is_cross_process_shaped_and_bounded() {
    let directory = tempdir().unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let lock = workspace.acquire_mutation_lock().unwrap();
    let error = match workspace.acquire_mutation_lock() {
        Ok(_) => panic!("second writer unexpectedly acquired the lock"),
        Err(error) => error,
    };
    assert!(matches!(error, WorkspaceError::Busy { .. }));
    drop(lock);
    assert!(workspace.acquire_mutation_lock().is_ok());
}

#[test]
fn schema_diagnostic_explains_pointer_typo() {
    let diagnostic = threadmoth::protocol::schema_diagnostic(
        "unknown field `pointer`, expected `path` or `value`",
    );
    assert_eq!(diagnostic.reason, "SCHEMA_INVALID");
    assert_eq!(diagnostic.field, "pointer");
    assert_eq!(diagnostic.location, "$.operation.operation.pointer");
    assert_eq!(diagnostic.expected_fields, vec!["path", "value"]);
    assert_eq!(diagnostic.suggested_field.as_deref(), Some("path"));
}

#[test]
fn cli_shorthands_and_doctor_json_use_safe_surfaces() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("x.txt"), b"old\n").unwrap();
    let replace = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(directory.path())
        .args(["replace-exact", "x.txt", "old", "new"])
        .output()
        .unwrap();
    assert!(replace.status.success());
    assert_eq!(fs::read(directory.path().join("x.txt")).unwrap(), b"new\n");

    fs::write(directory.path().join("config.json"), b"{\"port\":8080}\n").unwrap();
    let set = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(directory.path())
        .args(["set-value", "config.json", "$.port", "9090"])
        .output()
        .unwrap();
    assert!(set.status.success());
    assert_eq!(
        fs::read(directory.path().join("config.json")).unwrap(),
        b"{\"port\":9090}\n"
    );

    let doctor = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(directory.path())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(doctor.status.success());
    let doctor: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(doctor["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(doctor["protocol"], "1.4.0");
    assert_eq!(doctor["workspace_readiness"], "ready");
}
