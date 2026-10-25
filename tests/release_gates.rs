use tempfile::TempDir;
use threadmoth::engine::compute_sha256;
use threadmoth::pipeline::execute_request;
use threadmoth::protocol::{
    Cardinality, OperationPayload, Outcome, RefusalReason, Request, PROTOCOL_VERSION,
};
use threadmoth::provider::json::JsonOperation;
use threadmoth::provider::text::TextOperation;

fn request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: String::new(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: Default::default(),
        operation,
    }
}

#[test]
fn json_edit_preserves_unrelated_bytes() {
    let tmp = TempDir::new().unwrap();
    let original = b"{\n  \"target\" : 1,\n  \"secret-looking\" : \"do not echo\"\n}\n";
    std::fs::write(tmp.path().join("a.json"), original).unwrap();
    let cert = execute_request(
        &threadmoth::workspace::Workspace::new(tmp.path()).unwrap(),
        &request(
            "a.json",
            OperationPayload::Json(JsonOperation::Set {
                path: "$.target".into(),
                value: serde_json::json!(2),
            }),
        ),
        false,
    );
    assert_eq!(cert.outcome, Outcome::Applied);
    assert!(!cert
        .diff_summary
        .as_deref()
        .unwrap()
        .contains("do not echo"));
    assert_eq!(
        std::fs::read(tmp.path().join("a.json")).unwrap(),
        b"{\n  \"target\" : 2,\n  \"secret-looking\" : \"do not echo\"\n}\n"
    );
}

#[test]
fn duplicate_diagnostics_are_bounded_and_actionable() {
    let result = threadmoth::provider::text::TextProvider::plan(
        b"x x x",
        &TextOperation::Replace {
            target: "x".into(),
            replacement: "y".into(),
        },
        &Cardinality::ExactlyOne,
    );
    match result {
        Err(threadmoth::provider::text::TextProviderError::Refused(
            RefusalReason::DuplicateTarget { candidates, .. },
        )) => {
            assert_eq!(candidates.len(), 3);
            assert_eq!(candidates[1].offset, 2);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn protocol_version_is_fail_closed() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), b"old").unwrap();
    let mut req = request(
        "a.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "old".into(),
            replacement: "new".into(),
        }),
    );
    req.version = "9.9.9".into();
    let cert = execute_request(
        &threadmoth::workspace::Workspace::new(tmp.path()).unwrap(),
        &req,
        false,
    );
    assert_eq!(cert.outcome, Outcome::Refused);
    assert!(matches!(
        cert.refusal_reason,
        Some(RefusalReason::UnsupportedProtocolVersion { .. })
    ));
    assert_eq!(
        compute_sha256(&std::fs::read(tmp.path().join("a.txt")).unwrap()),
        compute_sha256(b"old")
    );
}
