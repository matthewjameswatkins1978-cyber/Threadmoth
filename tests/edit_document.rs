use std::fs;
use std::process::Command;
use tempfile::TempDir;
use threadmoth::{
    pipeline::apply_prepared_plan,
    protocol::{Outcome, PlanApplyResult, RefusalReason},
    workspace::Workspace,
};

fn block(old: &str, new: &str) -> String {
    format!("<<<<<<< SEARCH\n{old}\n=======\n{new}\n>>>>>>> REPLACE\n")
}

fn section(path: &str, edits: &[(&str, &str)]) -> String {
    let mut document = format!("*** File: {path}\n");
    for (old, new) in edits {
        document.push_str(&block(old, new));
    }
    document
}

fn prepared(
    workspace: &Workspace,
    document: &str,
) -> threadmoth::edit_document::EditDocumentPreparation {
    match threadmoth::edit_document::prepare(workspace, document) {
        Ok(prepared) => prepared,
        Err(error) => panic!("unexpected preparation failure: {error:?}"),
    }
}

fn refused(workspace: &Workspace, document: &str) -> threadmoth::protocol::TransactionCertificate {
    let certificate = match threadmoth::edit_document::prepare(workspace, document) {
        Err(threadmoth::edit_document::EditDocumentFailure::Refused(certificate)) => *certificate,
        other => panic!("expected Threadmoth refusal, got {other:?}"),
    };
    if document.contains("escape-link") {
        assert_eq!(
            certificate.reason_code.as_deref(),
            Some("WORKSPACE_ESCAPE"),
            "{certificate:?}"
        );
    }
    certificate
}
#[test]
fn parser_accepts_crlf_documents_and_unescapes_literal_control_lines() {
    let document = "*** File: file.txt\r\n<<<<<<< SEARCH\r\n\\=======\r\n=======\r\nreplacement\r\n>>>>>>> REPLACE\r\n";
    let files = threadmoth::edit_document::parse(document).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "file.txt");
    assert_eq!(files[0].edits[0].old, "=======");
    assert_eq!(files[0].edits[0].new, "replacement");
}

#[test]
fn parser_refuses_malformed_and_unescaped_delimiter_collisions() {
    for document in [
        "",
        "*** File: x.txt\n=======\n",
        "*** File: x.txt\n<<<<<<< SEARCH\nold\n=======\nnew\n",
        "*** File: x.txt\n<<<<<<< SEARCH\nold\n>>>>>>> REPLACE\n",
        "*** File: x.txt\n<<<<<<< SEARCH\nold\n=======\n=======\n>>>>>>> REPLACE\n",
    ] {
        assert!(
            threadmoth::edit_document::parse(document).is_err(),
            "{document:?}"
        );
    }
    let duplicate_path = format!(
        "{}{}",
        section("x.txt", &[("a", "b")]),
        section("x.txt", &[("b", "c")])
    );
    assert!(threadmoth::edit_document::parse(&duplicate_path).is_err());
}

#[test]
fn ordered_multi_edit_multifile_plan_preserves_crlf_and_lf_and_writes_nothing_until_apply() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("crlf.txt"),
        b"first=1\r\nsecond=2\r\nthird=3\r\n",
    )
    .unwrap();
    fs::write(temp.path().join("lf.txt"), b"mode=old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let document = format!(
        "{}{}",
        section(
            "crlf.txt",
            &[
                ("first=1", "one=1"),
                ("second=2", "two=2"),
                ("one=1", "first=1")
            ]
        ),
        section("lf.txt", &[("mode=old", "mode=new")]),
    );
    let prepared = prepared(&workspace, &document);
    assert_eq!(prepared.file_count, 2);
    assert_eq!(prepared.edit_count, 4);
    assert_eq!(prepared.plan.operations.len(), 2);
    assert!(prepared.plan.transaction_id.is_some());
    assert_eq!(
        fs::read(temp.path().join("crlf.txt")).unwrap(),
        b"first=1\r\nsecond=2\r\nthird=3\r\n"
    );
    assert_eq!(fs::read(temp.path().join("lf.txt")).unwrap(), b"mode=old\n");

    let applied = apply_prepared_plan(&workspace, &prepared.plan);
    assert!(
        matches!(applied, PlanApplyResult::Transaction(certificate) if certificate.outcome == Outcome::Applied && certificate.transaction_guarantee == "transactional_with_rollback")
    );
    assert_eq!(
        fs::read(temp.path().join("crlf.txt")).unwrap(),
        b"first=1\r\ntwo=2\r\nthird=3\r\n"
    );
    assert_eq!(fs::read(temp.path().join("lf.txt")).unwrap(), b"mode=new\n");
}

#[test]
fn twenty_four_edits_are_ordered_and_bounded() {
    let temp = TempDir::new().unwrap();
    let mut source = String::new();
    let mut edits = Vec::new();
    for index in 0..24 {
        source.push_str(&format!("value_{index}=old\n"));
        edits.push((format!("value_{index}=old"), format!("value_{index}=new")));
    }
    fs::write(temp.path().join("many.txt"), source.as_bytes()).unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let mut document = String::from("*** File: many.txt\n");
    for (old, new) in &edits {
        document.push_str(&block(old, new));
    }
    let prepared = prepared(&workspace, &document);
    assert_eq!(prepared.edit_count, 24);
    assert!(
        matches!(apply_prepared_plan(&workspace, &prepared.plan), PlanApplyResult::Certificate(cert) if cert.outcome == Outcome::Applied)
    );
    for index in 0..24 {
        assert!(
            String::from_utf8(fs::read(temp.path().join("many.txt")).unwrap())
                .unwrap()
                .contains(&format!("value_{index}=new\n"))
        );
    }
}

#[test]
fn duplicate_missing_overlapping_and_mixed_newline_targets_refuse_unchanged() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("duplicate.txt"), b"same\nsame\n").unwrap();
    fs::write(temp.path().join("overlap.txt"), b"abc\n").unwrap();
    fs::write(temp.path().join("mixed.txt"), b"first=1\r\nsecond=2\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let duplicate = refused(&workspace, &section("duplicate.txt", &[("same", "new")]));
    assert_eq!(duplicate.reason_code.as_deref(), Some("TARGET_AMBIGUOUS"));
    let missing = refused(&workspace, &section("duplicate.txt", &[("absent", "new")]));
    assert_eq!(missing.reason_code.as_deref(), Some("TARGET_NOT_FOUND"));
    let missing_file = refused(&workspace, &section("new.txt", &[("before", "after")]));
    assert_eq!(
        missing_file.reason_code.as_deref(),
        Some("WORKSPACE_ESCAPE")
    );
    assert!(!temp.path().join("new.txt").exists());
    let overlap = refused(
        &workspace,
        &section("overlap.txt", &[("abc", "axc"), ("bc", "zz")]),
    );
    assert!(matches!(
        overlap.refusal_reason,
        Some(RefusalReason::MissingTarget { .. }) | Some(RefusalReason::DuplicateTarget { .. })
    ));
    let mixed = refused(
        &workspace,
        &section("mixed.txt", &[("first=1\n", "first=2\n")]),
    );
    assert_eq!(
        mixed.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert_eq!(
        fs::read(temp.path().join("duplicate.txt")).unwrap(),
        b"same\nsame\n"
    );
    assert_eq!(fs::read(temp.path().join("overlap.txt")).unwrap(), b"abc\n");
    assert_eq!(
        fs::read(temp.path().join("mixed.txt")).unwrap(),
        b"first=1\r\nsecond=2\n"
    );
}

#[test]
fn traversal_and_absolute_paths_refuse_without_touching_outside_files() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("allowed/nested")).unwrap();
    fs::write(temp.path().join("outside.txt"), b"before\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    for path in [
        "outside.txt",
        "allowed/../outside.txt",
        "allowed/nested/../../outside.txt",
        "allowed\\..\\outside.txt",
    ] {
        let certificate = refused(&workspace, &section(path, &[("before", "after")]));
        assert_eq!(certificate.outcome, Outcome::Refused, "{path}");
    }
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("absolute.txt"), b"before\n").unwrap();
    let certificate = refused(
        &workspace,
        &section(
            &outside.path().join("absolute.txt").to_string_lossy(),
            &[("before", "after")],
        ),
    );
    assert_eq!(certificate.outcome, Outcome::Refused);
    assert_eq!(
        fs::read(outside.path().join("absolute.txt")).unwrap(),
        b"before\n"
    );
    assert_eq!(
        fs::read(temp.path().join("outside.txt")).unwrap(),
        b"before\n"
    );
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_refused_and_safe_internal_symlink_is_allowed() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("allowed/real")).unwrap();
    fs::create_dir_all(temp.path().join("outside")).unwrap();
    fs::write(temp.path().join("allowed/real/safe.txt"), b"before\n").unwrap();
    fs::write(temp.path().join("outside/target.txt"), b"before\n").unwrap();
    symlink(
        temp.path().join("allowed/real"),
        temp.path().join("allowed/safe-link"),
    )
    .unwrap();
    symlink(
        temp.path().join("outside"),
        temp.path().join("allowed/escape-link"),
    )
    .unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let safe = prepared(
        &workspace,
        &section("allowed/safe-link/safe.txt", &[("before", "inside")]),
    );
    assert!(
        matches!(apply_prepared_plan(&workspace, &safe.plan), PlanApplyResult::Certificate(cert) if cert.outcome == Outcome::Applied)
    );
    let escaped = refused(
        &workspace,
        &section("allowed/escape-link/target.txt", &[("before", "outside")]),
    );
    assert_eq!(escaped.outcome, Outcome::Refused);
    assert_eq!(
        fs::read(temp.path().join("outside/target.txt")).unwrap(),
        b"before\n"
    );
}

#[cfg(windows)]
#[test]
fn junction_escape_is_refused_and_safe_internal_junction_is_allowed() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("allowed/real")).unwrap();
    fs::create_dir_all(temp.path().join("outside")).unwrap();
    fs::write(temp.path().join("allowed/real/safe.txt"), b"before\n").unwrap();
    fs::write(temp.path().join("outside/target.txt"), b"before\n").unwrap();
    let link = |target: &str, name: &str| {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(temp.path().join("allowed").join(name))
            .arg(temp.path().join(target))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    link(r"allowed\real", "safe-link");
    link("outside", "escape-link");
    let workspace = Workspace::new(temp.path()).unwrap();
    let safe = prepared(
        &workspace,
        &section("allowed/safe-link/safe.txt", &[("before", "inside")]),
    );
    assert!(
        matches!(apply_prepared_plan(&workspace, &safe.plan), PlanApplyResult::Certificate(cert) if cert.outcome == Outcome::Applied)
    );
    let escaped = refused(
        &workspace,
        &section("allowed/escape-link/target.txt", &[("before", "outside")]),
    );
    assert_eq!(escaped.outcome, Outcome::Refused);
    assert_eq!(
        fs::read(temp.path().join("outside/target.txt")).unwrap(),
        b"before\n"
    );
}

#[test]
fn stale_source_refuses_whole_multifile_plan_without_partial_commit() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("a.txt"), b"a=old\n").unwrap();
    fs::write(temp.path().join("b.txt"), b"b=old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let document = format!(
        "{}{}",
        section("a.txt", &[("a=old", "a=new")]),
        section("b.txt", &[("b=old", "b=new")])
    );
    let prepared = prepared(&workspace, &document);
    fs::write(temp.path().join("b.txt"), b"b=external\n").unwrap();
    let result = apply_prepared_plan(&workspace, &prepared.plan);
    assert!(
        matches!(result, PlanApplyResult::Transaction(certificate) if certificate.outcome == Outcome::Refused && certificate.reason_code.as_deref() == Some("PLAN_STALE"))
    );
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a=old\n");
    assert_eq!(
        fs::read(temp.path().join("b.txt")).unwrap(),
        b"b=external\n"
    );
}

#[test]
fn cli_prepares_reviewable_multifile_plan_without_mutating_sources() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("a.txt"), b"a=old\r\n").unwrap();
    fs::write(temp.path().join("b.txt"), b"b=old\n").unwrap();
    fs::write(
        temp.path().join("changes.tm"),
        format!(
            "{}{}",
            section("a.txt", &[("a=old", "a=new")]),
            section("b.txt", &[("b=old", "b=new")])
        ),
    )
    .unwrap();
    let prepare = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "prepared plan.json",
            "--summary",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        prepare.status.success(),
        "{}",
        String::from_utf8_lossy(&prepare.stderr)
    );
    let summary = String::from_utf8_lossy(&prepare.stdout);
    assert!(summary.contains("Files          2"));
    assert!(summary.contains("Exact edits    2"));
    assert!(summary.contains("Review ("));
    assert!(summary.contains("Apply ("));
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a=old\r\n");
    assert_eq!(fs::read(temp.path().join("b.txt")).unwrap(), b"b=old\n");

    let review = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args([
            "explain",
            "--plan",
            "prepared plan.json",
            "--format",
            "diff",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    let apply = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["apply-plan", "--plan", "prepared plan.json", "--summary"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a=new\r\n");
    assert_eq!(fs::read(temp.path().join("b.txt")).unwrap(), b"b=new\n");
}
