use tempfile::TempDir;
use threadmoth::engine::compute_sha256;
use threadmoth::lifecycle::FileOperation;
use threadmoth::pipeline::{
    apply_prepared_plan, execute_request, execute_transaction, prepare_request_plan,
};
use threadmoth::protocol::{
    Cardinality, EffectBudget, OperationPayload, Outcome, PlanApplyResult, RefusalReason, Request,
    TransactionRequest, PROTOCOL_VERSION,
};
use threadmoth::provider::text::TextOperation;
use threadmoth::workspace::Workspace;

fn text_request(path: &str, prefix: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "path-authorization-regression".into(),
        allow_generated: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            allowed_path_prefixes: vec![prefix.into()],
            ..EffectBudget::default()
        },
        operation: OperationPayload::Text(TextOperation::Replace {
            target: "before".into(),
            replacement: "after".into(),
        }),
    }
}

fn assert_refused_unchanged(workspace: &Workspace, base: &std::path::Path, path: &str) {
    let certificate = execute_request(workspace, &text_request(path, "allowed"), false);
    assert_eq!(certificate.outcome, Outcome::Refused, "{path}");
    assert_eq!(
        std::fs::read(base.join("outside.txt")).unwrap(),
        b"before\n"
    );
}

#[test]
fn outside_prefix_and_lexical_traversals_are_refused_before_write() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join("allowed")).unwrap();
    std::fs::create_dir(temp.path().join("allowed/nested")).unwrap();
    std::fs::write(temp.path().join("outside.txt"), b"before\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    for path in [
        "outside.txt",
        "allowed/../outside.txt",
        "allowed/nested/../../outside.txt",
        "allowed\\..\\outside.txt",
        "allowed/nested/../outside.txt",
    ] {
        assert_refused_unchanged(&workspace, temp.path(), path);
    }

    let outside_temp = TempDir::new().unwrap();
    let absolute_outside = outside_temp.path().join("absolute-outside.txt");
    std::fs::write(&absolute_outside, b"before\n").unwrap();
    let certificate = execute_request(
        &workspace,
        &text_request(&absolute_outside.to_string_lossy(), "allowed"),
        false,
    );
    assert_eq!(certificate.outcome, Outcome::Refused);
    let recovery_view = threadmoth::metadata::refusal_recovery(&certificate);
    assert!(recovery_view["suggestions"][0]["next"]
        .as_str()
        .unwrap()
        .contains("checkout-local Threadmoth CLI"));
    assert!(matches!(
        certificate.refusal_reason,
        Some(RefusalReason::WorkspaceRootMismatch { .. })
    ));
    let recovery = certificate
        .recovery
        .expect("workspace mismatch has deterministic recovery");
    assert!(recovery.requires_choice);
    assert_eq!(recovery.remedies[0].kind, "checkout_local_cli");
    assert!(recovery.remedies[0]
        .description
        .contains("checkout-local Threadmoth CLI"));
    assert!(recovery.remedies[0].request_patch.is_none());
    assert_eq!(std::fs::read(&absolute_outside).unwrap(), b"before\n");

    #[cfg(windows)]
    for path in [
        r"C:drive-relative.txt",
        r"\\server\share\outside.txt",
        r"\\?\C:\outside.txt",
    ] {
        assert!(
            workspace
                .resolve_namespaced_path(path, &Default::default())
                .is_err(),
            "{path}"
        );
    }

    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "outside-prefix-refusal".into(),
        requests: vec![text_request("outside.txt", "allowed")],
        budget: EffectBudget::default(),
    };
    let certificate = execute_transaction(&workspace, &transaction, false);
    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(
        std::fs::read(temp.path().join("outside.txt")).unwrap(),
        b"before\n"
    );
}

#[test]
fn safe_separator_variant_and_missing_leaf_under_safe_ancestor_are_allowed() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join("allowed/nested")).unwrap();
    std::fs::write(temp.path().join("allowed/nested/file.txt"), b"before\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let mut request = text_request("allowed\\nested\\file.txt", "allowed\\nested");
    let certificate = execute_request(&workspace, &request, false);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        std::fs::read(temp.path().join("allowed/nested/file.txt")).unwrap(),
        b"after\n"
    );

    request.file_path = "allowed/new/deep/created.txt".into();
    request.budget.allowed_path_prefixes = vec!["allowed".into()];
    request.expected_pre_hash = None;
    request.operation = OperationPayload::File(FileOperation::CreateFile {
        expected_absent: true,
        content: b"created\n".to_vec(),
    });
    let certificate = execute_request(&workspace, &request, false);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        std::fs::read(temp.path().join("allowed/new/deep/created.txt")).unwrap(),
        b"created\n"
    );
}

#[test]
fn rename_destination_must_stay_within_the_declared_scope() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join("allowed")).unwrap();
    std::fs::create_dir_all(temp.path().join("outside-prefix")).unwrap();
    std::fs::write(temp.path().join("allowed/source.txt"), b"before\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let mut request = text_request("allowed/source.txt", "allowed");
    request.operation = OperationPayload::File(FileOperation::RenameFile {
        destination: "outside-prefix/dest.txt".into(),
        expected_source_hash: compute_sha256(b"before\n"),
        destination_absent: true,
    });

    let certificate = execute_request(&workspace, &request, false);
    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(
        std::fs::read(temp.path().join("allowed/source.txt")).unwrap(),
        b"before\n"
    );
    assert!(!temp.path().join("outside-prefix/dest.txt").exists());
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("Windows cmd is available for junction regression setup");
    assert!(
        output.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(any(unix, windows))]
#[test]
fn links_are_authorized_by_their_canonical_target() {
    let temp = TempDir::new().unwrap();
    let allowed = temp.path().join("allowed");
    let safe_target = allowed.join("real");
    let outside_prefix = temp.path().join("outside-prefix");
    let outside_temp = TempDir::new().unwrap();
    std::fs::write(outside_temp.path().join("external.txt"), b"before\n").unwrap();
    std::fs::create_dir_all(&safe_target).unwrap();
    std::fs::create_dir(&outside_prefix).unwrap();
    std::fs::write(safe_target.join("safe.txt"), b"before\n").unwrap();
    std::fs::write(outside_prefix.join("target.txt"), b"before\n").unwrap();
    create_directory_link(&safe_target, &allowed.join("safe-link"));
    create_directory_link(&outside_prefix, &allowed.join("escape-link"));
    create_directory_link(outside_temp.path(), &allowed.join("root-escape-link"));
    let workspace = Workspace::new(temp.path()).unwrap();

    let safe = execute_request(
        &workspace,
        &text_request("allowed/safe-link/safe.txt", "allowed"),
        false,
    );
    assert_eq!(safe.outcome, Outcome::Applied);
    assert_eq!(
        std::fs::read(safe_target.join("safe.txt")).unwrap(),
        b"after\n"
    );

    let escaped = execute_request(
        &workspace,
        &text_request("allowed/escape-link/target.txt", "allowed"),
        false,
    );
    assert_eq!(escaped.outcome, Outcome::Refused);
    assert!(matches!(
        escaped.refusal_reason,
        Some(RefusalReason::WorkspaceTraversal { .. })
    ));
    assert_eq!(
        std::fs::read(outside_prefix.join("target.txt")).unwrap(),
        b"before\n"
    );

    let escaped_workspace = execute_request(
        &workspace,
        &text_request("allowed/root-escape-link/external.txt", "allowed"),
        false,
    );
    assert_eq!(escaped_workspace.outcome, Outcome::Refused);
    assert!(matches!(
        escaped_workspace.refusal_reason,
        Some(RefusalReason::SymlinkEscape { .. })
    ));
    assert_eq!(
        std::fs::read(outside_temp.path().join("external.txt")).unwrap(),
        b"before\n"
    );
}

#[cfg(windows)]
#[test]
fn windows_case_variants_match_the_same_canonical_scope() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join("Allowed")).unwrap();
    std::fs::write(temp.path().join("Allowed/File.txt"), b"before\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let certificate = execute_request(
        &workspace,
        &text_request("allowed/file.TXT", "ALLOWED"),
        false,
    );
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(
        std::fs::read(temp.path().join("Allowed/File.txt")).unwrap(),
        b"after\n"
    );
}
#[cfg(any(unix, windows))]
#[test]
fn applying_a_plan_rechecks_the_physical_budget_scope() {
    let temp = TempDir::new().unwrap();
    let allowed = temp.path().join("allowed");
    let safe_target = allowed.join("safe");
    let outside_prefix = temp.path().join("outside-prefix");
    std::fs::create_dir_all(&safe_target).unwrap();
    std::fs::create_dir(&outside_prefix).unwrap();
    std::fs::write(safe_target.join("target.txt"), b"before\n").unwrap();
    std::fs::write(outside_prefix.join("target.txt"), b"before\n").unwrap();
    let link = allowed.join("link");
    create_directory_link(&safe_target, &link);
    let workspace = Workspace::new(temp.path()).unwrap();
    let plan = prepare_request_plan(
        &workspace,
        &text_request("allowed/link/target.txt", "allowed"),
        Vec::new(),
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::remove_file(&link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(&link).unwrap();
    create_directory_link(&outside_prefix, &link);
    let result = apply_prepared_plan(&workspace, &plan);
    assert!(matches!(
        result,
        PlanApplyResult::Certificate(certificate)
            if certificate.outcome == Outcome::Refused
                && matches!(certificate.refusal_reason, Some(RefusalReason::WorkspaceTraversal { .. }))
    ));
    assert_eq!(
        std::fs::read(safe_target.join("target.txt")).unwrap(),
        b"before\n"
    );
    assert_eq!(
        std::fs::read(outside_prefix.join("target.txt")).unwrap(),
        b"before\n"
    );
}
