use std::fs;

use tempfile::TempDir;
use threadmoth::{
    engine::compute_sha256,
    metadata,
    pipeline::execute_request,
    protocol::{
        DesiredStateOperation, EffectBudget, OperationPayload, Outcome, Request,
        TransactionRequest, PROTOCOL_VERSION,
    },
    provider::ini::IniOperation,
    provider::web::WebOperation,
    recovery::{self, Journal, JournalEntry},
    workspace::Workspace,
};

fn request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "threadmoth-15-test".into(),
        allow_generated: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Default::default(),
        budget: EffectBudget {
            allowed_path_prefixes: Vec::new(),
            ..Default::default()
        },
        operation,
    }
}

#[test]
fn desired_state_is_exactly_verified_and_no_op_is_clean() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    workspace.write_file_atomic("source.txt", b"old\n").unwrap();
    let operation = OperationPayload::DesiredState(DesiredStateOperation::Replace {
        desired_bytes: b"new\n".to_vec(),
    });
    let certificate = execute_request(&workspace, &request("source.txt", operation.clone()), true);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        certificate
            .desired_state
            .as_ref()
            .unwrap()
            .derived_region_count,
        1
    );
    assert_eq!(
        certificate.desired_state.as_ref().unwrap().desired_hash,
        compute_sha256(b"new\n")
    );
    let no_op = execute_request(&workspace, &request("source.txt", operation), false);
    assert_eq!(no_op.outcome, Outcome::Applied);
    assert_eq!(fs::read(temp.path().join("source.txt")).unwrap(), b"new\n");
}

#[test]
fn desired_state_budget_refusal_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    workspace.write_file_atomic("source.txt", b"old\n").unwrap();
    let mut req = request(
        "source.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"new\n".to_vec(),
        }),
    );
    req.budget.max_changed_bytes = Some(2);
    let certificate = execute_request(&workspace, &req, false);
    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(fs::read(temp.path().join("source.txt")).unwrap(), b"old\n");
}

#[test]
fn web_uses_the_shared_syntax_engine() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    workspace
        .write_file_atomic("index.html", b"<main><p>old</p></main>")
        .unwrap();
    let certificate = execute_request(
        &workspace,
        &request(
            "index.html",
            OperationPayload::Web(WebOperation::ReplaceNode {
                language: "html".into(),
                target: "<p>old</p>".into(),
                replacement: "<p>new</p>".into(),
                node_kind: None,
            }),
        ),
        false,
    );
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("index.html")).unwrap(),
        b"<main><p>new</p></main>"
    );
}

#[test]
fn registry_drives_capabilities_and_suggestions() {
    let capabilities = metadata::capabilities();
    assert!(capabilities.code_languages.contains(&"cpp"));
    assert!(capabilities.code_languages.contains(&"powershell"));
    assert!(capabilities.code_languages.contains(&"java"));
    assert!(capabilities
        .providers
        .iter()
        .any(|provider| provider.name == "ini"));
    assert_eq!(capabilities.web_formats, vec!["html", "css", "xml"]);
    let suggestion = metadata::suggest("script.ps1", None, None, "safe", None);
    assert_eq!(suggestion.provider, "code");
    assert_eq!(suggestion.language.as_deref(), Some("powershell"));
    assert_eq!(metadata::detect_provider("config.ini", None).0, "ini");
    assert_eq!(
        metadata::detect_provider("blob", Some(&[0, 1, 2])).0,
        "opaque"
    );
}

#[test]
fn ini_operation_uses_the_guarded_core_pipeline() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    workspace
        .write_file_atomic("setup.ini", b"[server]\r\nport = 8080\r\n")
        .unwrap();
    let certificate = execute_request(
        &workspace,
        &request(
            "setup.ini",
            OperationPayload::Ini(IniOperation::Set {
                path: "server.port".into(),
                value: "9090".into(),
            }),
        ),
        false,
    );
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("setup.ini")).unwrap(),
        b"[server]\r\nport = 9090\r\n"
    );
}

#[test]
fn recovery_inspection_is_read_only_and_classifies_members() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    workspace.write_file_atomic("x.txt", b"new").unwrap();
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "inspect-me".into(),
            entries: vec![JournalEntry {
                path: "x.txt".into(),
                pre_hash: compute_sha256(b"old"),
                candidate_hash: compute_sha256(b"new"),
                original: b"old".to_vec(),
                candidate: b"new".to_vec(),
            }],
            phase: Some(threadmoth::recovery::JournalPhase::Committed),
        },
    )
    .unwrap();
    let before = fs::read(temp.path().join("x.txt")).unwrap();
    let listed = recovery::list(&workspace);
    assert_eq!(listed.entries[0].apparent_state, "complete");
    let inspected = recovery::inspect(&workspace, "inspect-me");
    assert_eq!(inspected.members[0].classification, "CANDIDATE");
    assert_eq!(fs::read(temp.path().join("x.txt")).unwrap(), before);
}

#[test]
fn oversized_transaction_journal_is_refused_before_commit() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let original = vec![b'a'; 4 * 1024 * 1024];
    let mut desired = original.clone();
    let final_byte = desired.len() - 1;
    desired[final_byte] = b'b';
    workspace.write_file_atomic("large.txt", &original).unwrap();

    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "oversized-transaction".into(),
        requests: vec![request(
            "large.txt",
            OperationPayload::DesiredState(DesiredStateOperation::Replace {
                desired_bytes: desired,
            }),
        )],
        budget: EffectBudget::default(),
    };
    let certificate = threadmoth::pipeline::execute_transaction(&workspace, &transaction, false);

    assert_eq!(certificate.outcome, Outcome::Refused);
    assert!(matches!(
        certificate.refusal_reason,
        Some(threadmoth::protocol::RefusalReason::ResourceLimitExceeded {
            dimension,
            limit,
            actual,
        }) if dimension == "max_journal_bytes" && limit == 8 * 1024 * 1024 && actual > limit
    ));
    assert_eq!(fs::read(temp.path().join("large.txt")).unwrap(), original);
    assert!(!temp.path().join(".threadmoth-recovery").exists());
}
