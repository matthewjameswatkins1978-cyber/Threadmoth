use tempfile::TempDir;
#[cfg(target_os = "windows")]
use threadmoth::engine::compute_sha256;
use threadmoth::pipeline::{execute_request, execute_transaction};
use threadmoth::protocol::{
    Cardinality, EffectBudget, OperationPayload, Outcome, Request, TransactionRequest,
    PROTOCOL_VERSION,
};
use threadmoth::provider::text::TextOperation;
use threadmoth::workspace::Workspace;

fn request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "stress-regression".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation,
    }
}

#[test]
fn successful_transaction_leaves_no_recovery_directory() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    std::fs::write(temp.path().join("a.txt"), b"old-a\n").unwrap();
    std::fs::write(temp.path().join("b.txt"), b"old-b\n").unwrap();

    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "stress-success-cleanup".into(),
        requests: vec![
            request(
                "a.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "old-a".into(),
                    replacement: "new-a".into(),
                }),
            ),
            request(
                "b.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "old-b".into(),
                    replacement: "new-b".into(),
                }),
            ),
        ],
        budget: EffectBudget::default(),
    };

    let certificate = execute_transaction(&workspace, &transaction, false);

    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(workspace.read_file("a.txt").unwrap(), b"new-a\n");
    assert_eq!(workspace.read_file("b.txt").unwrap(), b"new-b\n");
    assert!(!temp.path().join(".threadmoth-recovery").exists());
}

#[test]
fn empty_set_replacement_refuses_instead_of_panicking() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    std::fs::write(temp.path().join("x.txt"), b"hello world\n").unwrap();

    let certificate = execute_request(
        &workspace,
        &request(
            "x.txt",
            OperationPayload::Text(TextOperation::Set {
                target: "missing".into(),
                replacement: String::new(),
            }),
        ),
        false,
    );

    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(workspace.read_file("x.txt").unwrap(), b"hello world\n");
}

#[test]
fn empty_rename_replacement_refuses_instead_of_panicking() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    std::fs::write(temp.path().join("x.txt"), b"hello world\n").unwrap();

    let certificate = execute_request(
        &workspace,
        &request(
            "x.txt",
            OperationPayload::Text(TextOperation::Rename {
                target: "missing".into(),
                replacement: String::new(),
            }),
        ),
        false,
    );

    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(workspace.read_file("x.txt").unwrap(), b"hello world\n");
}

#[cfg(target_os = "windows")]
#[test]
fn readonly_failure_leaves_no_staged_candidate_residue() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("readonly.txt");
    std::fs::write(&path, b"old\n").unwrap();

    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly_permissions = original_permissions.clone();
    readonly_permissions.set_readonly(true);
    std::fs::set_permissions(&path, readonly_permissions).unwrap();

    let workspace = Workspace::new(temp.path()).unwrap();
    let mut mutation = request(
        "readonly.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "old".into(),
            replacement: "new".into(),
        }),
    );
    mutation.expected_pre_hash = Some(compute_sha256(b"old\n"));

    let certificate = execute_request(&workspace, &mutation, false);
    let entries: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(certificate.outcome, Outcome::Failed);
    assert_eq!(std::fs::read(&path).unwrap(), b"old\n");
    assert!(!entries
        .iter()
        .any(|name| name.starts_with(".readonly.txt.")));

    std::fs::set_permissions(&path, original_permissions).unwrap();
}
