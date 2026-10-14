use std::{fs, process::Command};
use tempfile::TempDir;
use threadmoth::{
    protocol::{
        Cardinality, EffectBudget, OperationPayload, Request, TransactionRequest, PROTOCOL_VERSION,
    },
    provider::text::TextOperation,
    workspace::Workspace,
};

fn block(old: &str, new: &str) -> String {
    format!("<<<<<<< SEARCH\n{old}\n=======\n{new}\n>>>>>>> REPLACE\n")
}

fn request(path: &str, old: &str, new: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("legacy-{path}"),
        allow_generated: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            max_files: Some(1),
            max_matches: Some(1),
            max_changed_regions: Some(1),
            max_changed_lines: None,
            max_changed_bytes: Some(256),
            allowed_path_prefixes: Vec::new(),
        },
        operation: OperationPayload::Text(TextOperation::Replace {
            target: old.into(),
            replacement: new.into(),
        }),
    }
}

#[test]
fn utf8_and_escaped_delimiters_round_trip_as_literal_content() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("unicode.txt"), "label=old\n".as_bytes()).unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let document = "*** File: unicode.txt\n<<<<<<< SEARCH\nlabel=old\n=======\n雪\n\\<<<<<<< SEARCH\n\\=======\n\\>>>>>>> REPLACE\n>>>>>>> REPLACE\n";
    let prepared = threadmoth::edit_document::prepare(&workspace, document).unwrap();
    let result = threadmoth::pipeline::apply_prepared_plan(&workspace, &prepared.plan);
    assert!(
        matches!(result, threadmoth::protocol::PlanApplyResult::Certificate(c) if c.outcome == threadmoth::protocol::Outcome::Applied)
    );
    assert_eq!(
        fs::read(temp.path().join("unicode.txt")).unwrap(),
        "雪\n<<<<<<< SEARCH\n=======\n>>>>>>> REPLACE\n".as_bytes()
    );
}

#[test]
fn parser_enforces_file_edit_and_document_bounds() {
    use threadmoth::edit_document::{
        MAX_EDIT_DOCUMENT_EDITS, MAX_EDIT_DOCUMENT_EDITS_PER_FILE, MAX_EDIT_DOCUMENT_FILES,
    };
    let mut excessive_per_file = String::from("*** File: x.txt\n");
    for _ in 0..=MAX_EDIT_DOCUMENT_EDITS_PER_FILE {
        excessive_per_file.push_str(&block("x", "y"));
    }
    let per_file_error = threadmoth::edit_document::parse(&excessive_per_file).unwrap_err();
    assert_eq!(
        per_file_error.line,
        2 + MAX_EDIT_DOCUMENT_EDITS_PER_FILE * 5
    );
    assert!(per_file_error.message.contains("per-file edit limit of 64"));

    let mut many_edits = String::new();
    let mut remaining = MAX_EDIT_DOCUMENT_EDITS + 1;
    let mut file_index = 0;
    while remaining > 0 {
        let file_edits = remaining.min(MAX_EDIT_DOCUMENT_EDITS_PER_FILE);
        many_edits.push_str(&format!("*** File: {file_index}.txt\n"));
        for _ in 0..file_edits {
            many_edits.push_str(&block("x", "y"));
        }
        remaining -= file_edits;
        file_index += 1;
    }
    let total_error = threadmoth::edit_document::parse(&many_edits).unwrap_err();
    assert!(total_error.message.contains("edit count exceeds"));

    let many_files = (0..=MAX_EDIT_DOCUMENT_FILES)
        .map(|index| format!("*** File: {index}.txt\n{}", block("x", "y")))
        .collect::<String>();
    assert!(threadmoth::edit_document::parse(&many_files).is_err());

    assert!(threadmoth::edit_document::parse(
        &"x".repeat(threadmoth::protocol::MAX_REQUEST_BYTES + 1)
    )
    .is_err());
    assert!(threadmoth::edit_document::parse(
        "*** File: x.txt\n<<<<<<< SEARCH\nx\0y\n=======\nz\n>>>>>>> REPLACE\n"
    )
    .is_err());
    assert!(threadmoth::edit_document::parse("\u{feff}*** File: x.txt\n").is_err());
}

#[cfg(windows)]
#[test]
fn windows_path_case_variants_resolve_to_the_same_file() {
    use threadmoth::protocol::{Outcome, PlanApplyResult};
    use threadmoth::{edit_document::EditDocumentFailure, pipeline::apply_prepared_plan};
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("MixedCase.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let document = format!("*** File: mixedcase.TXT\n{}", block("old", "new"));
    let prepared = match threadmoth::edit_document::prepare(&workspace, &document) {
        Ok(plan) => plan,
        Err(EditDocumentFailure::Invalid(error)) => panic!("invalid document: {error:?}"),
        Err(EditDocumentFailure::Refused(error)) => panic!("path refused: {error:?}"),
    };
    assert!(
        matches!(apply_prepared_plan(&workspace, &prepared.plan), PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
    );
    assert_eq!(
        fs::read(temp.path().join("MixedCase.txt")).unwrap(),
        b"new\n"
    );
}

#[test]
fn cli_refusal_uses_exit_two_and_leaves_ambiguous_target_unchanged() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("same.txt"), b"same\nsame\n").unwrap();
    fs::write(
        temp.path().join("changes.tm"),
        format!("*** File: same.txt\n{}", block("same", "replacement")),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "plan.json",
            "--summary",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let summary = String::from_utf8_lossy(&output.stdout);
    assert!(
        summary.contains("TARGET_AMBIGUOUS")
            || summary.contains("TARGET_DUPLICATE")
            || summary.contains("REFUSED")
    );
    assert_eq!(
        fs::read(temp.path().join("same.txt")).unwrap(),
        b"same\nsame\n"
    );
    assert!(!temp.path().join("plan.json").exists());
}

#[test]
fn compact_multifile_fixture_measures_bytes_calls_retries_and_certificates() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("a.txt"), b"mode=old\n").unwrap();
    fs::write(temp.path().join("b.txt"), b"timeout=30\n").unwrap();

    let legacy = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "legacy-measurement".into(),
        requests: vec![
            request("a.txt", "mode=old", "mode=new"),
            request("b.txt", "timeout=30", "timeout=60"),
        ],
        budget: EffectBudget {
            max_files: Some(2),
            max_matches: Some(2),
            max_changed_regions: Some(2),
            max_changed_lines: None,
            max_changed_bytes: Some(512),
            allowed_path_prefixes: Vec::new(),
        },
    };
    let old_bytes = serde_json::to_vec(&legacy).unwrap().len();
    fs::write(
        temp.path().join("legacy.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let compact = format!(
        "*** File: a.txt\n{}*** File: b.txt\n{}",
        block("mode=old", "mode=new"),
        block("timeout=30", "timeout=60"),
    );
    let new_bytes = compact.len();
    fs::write(temp.path().join("changes.tm"), compact.as_bytes()).unwrap();

    let preview = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["transact", "--request", "legacy.json", "--preview"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stdout)
    );
    assert!(String::from_utf8_lossy(&preview.stdout).contains("\"outcome\": \"APPLIED\""));
    let commit = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["transact", "--request", "legacy.json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        commit.status.success(),
        "{}",
        String::from_utf8_lossy(&commit.stdout)
    );
    let old_calls = 2;

    // Reset the same fixture for the compact path.
    fs::write(temp.path().join("a.txt"), b"mode=old\n").unwrap();
    fs::write(temp.path().join("b.txt"), b"timeout=30\n").unwrap();
    let prepare = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "prepared.json",
            "--summary",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        prepare.status.success(),
        "{}",
        String::from_utf8_lossy(&prepare.stdout)
    );
    let review = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["explain", "--plan", "prepared.json", "--format", "diff"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stdout)
    );
    let apply = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["apply-plan", "--plan", "prepared.json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stdout)
    );
    let new_calls = 3;

    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"mode=new\n");
    assert_eq!(
        fs::read(temp.path().join("b.txt")).unwrap(),
        b"timeout=60\n"
    );
    assert!(
        new_bytes < old_bytes,
        "compact={new_bytes}, JSON={old_bytes}"
    );
    let old_retries = 0;
    let new_retries = 0;
    println!(
        "edit_document_metrics legacy_json_bytes={old_bytes} compact_intent_bytes={new_bytes} legacy_calls={old_calls} compact_calls={new_calls} legacy_retries={old_retries} compact_retries={new_retries} temp_helper_scripts=0"
    );
}
