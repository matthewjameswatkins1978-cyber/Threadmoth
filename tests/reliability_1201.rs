use std::fs;
use tempfile::TempDir;
use threadmoth::engine::compute_sha256;
use threadmoth::pipeline::{apply_prepared_plan, prepare_transaction_plan, rollback_entries};
use threadmoth::protocol::{
    Cardinality, FailureReason, OperationPayload, Outcome, PlanApplyResult, Request,
    TransactionRequest, PROTOCOL_VERSION,
};
use threadmoth::provider::text::TextOperation;
use threadmoth::recovery::{self, Journal, JournalEntry, JournalPhase};
use threadmoth::workspace::Workspace;

fn text_request(path: &str, target: &str, replacement: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("req-{path}"),
        allow_generated: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: Default::default(),
        operation: OperationPayload::Text(TextOperation::Replace {
            target: target.into(),
            replacement: replacement.into(),
        }),
    }
}

fn make_journal_entry(path: &str, original: &[u8], candidate: &[u8]) -> JournalEntry {
    JournalEntry {
        path: path.into(),
        pre_hash: compute_sha256(original),
        candidate_hash: compute_sha256(candidate),
        original: original.to_vec(),
        candidate: candidate.to_vec(),
    }
}

#[test]
fn every_committed_member_is_considered_for_rollback_even_if_another_restoration_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig_1 = b"file 1 original\n";
    let cand_1 = b"file 1 candidate\n";
    let orig_2 = b"file 2 original\n";
    let cand_2 = b"file 2 candidate\n";
    let orig_3 = b"file 3 original\n";
    let cand_3 = b"file 3 candidate\n";

    // Write candidate content as if all three were committed
    workspace.write_file_atomic("f1.txt", cand_1).unwrap();
    workspace.write_file_atomic("f2.txt", cand_2).unwrap();
    workspace.write_file_atomic("f3.txt", cand_3).unwrap();

    let e1 = make_journal_entry("f1.txt", orig_1, cand_1);
    let e2 = make_journal_entry("f2.txt", orig_2, cand_2);
    let e3 = make_journal_entry("f3.txt", orig_3, cand_3);

    // Corrupt f2.txt on disk so that write_file_atomic_checked will fail for f2 during rollback
    fs::write(temp.path().join("f2.txt"), b"corrupted file 2 content").unwrap();

    // Call exhaustive rollback on committed = [&e1, &e2, &e3]
    let committed = vec![&e1, &e2, &e3];
    let report = rollback_entries(&workspace, &committed);

    // In old short-circuiting rollback, e2 failed and e1 was NEVER ATTEMPTED!
    // With exhaustive reverse-order restoration:
    // e3 is restored, e2 fails, and e1 IS STILL RESTORED!
    assert_eq!(report.total, 3);
    assert_eq!(report.restored, 2);
    assert_eq!(report.failed, vec!["f2.txt".to_string()]);
    assert!(report.is_partial());
    assert_eq!(report.rollback_state(), "manual_recovery_required");

    // Verify f3 and f1 were restored back to original bytes on disk
    assert_eq!(fs::read(temp.path().join("f3.txt")).unwrap(), orig_3);
    assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), orig_1);

    // Verify error formatting accurately reports partial rollback
    let formatted = report.format_message("commit step failed");
    assert!(formatted.contains("rollback partial: 2/3 restored, failed: [f2.txt]"));
}

#[test]
fn failed_postconditions_never_become_successful_commits_during_recovery() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"int count = 0;\n";
    let cand = b"int count = 100;\n"; // candidate has value 100

    // Simulate an in-flight transaction where candidate was written to disk,
    // but verification failed or crashed before commit was recorded
    workspace.write_file_atomic("state.txt", cand).unwrap();

    // Journal was written with phase = Prepared (the default before commit)
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-uncommitted-crash".into(),
            entries: vec![make_journal_entry("state.txt", orig, cand)],
            phase: Some(JournalPhase::Prepared),
        },
    )
    .unwrap();

    // All files on disk match candidate!
    // Under 1.20.0, recovery would have marked this as completed and deleted the journal!
    // Under 1.20.1, failed postconditions / uncommitted transactions must roll back.
    let report = recovery::recover_all(&workspace);

    assert_eq!(report.completed, 0, "must never become a completed commit");
    assert_eq!(
        report.restored, 1,
        "uncommitted candidate must be restored to original"
    );
    assert_eq!(workspace.read_file("state.txt").unwrap(), orig);
}

#[test]
fn acknowledged_successful_transaction_is_never_silently_reversed_by_recovery() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"mode = development\n";
    let cand = b"mode = production\n";

    workspace.write_file_atomic("config.ini", cand).unwrap();

    // Transaction completed and verified: journal was marked Committed
    // but journal cleanup failed (or system crashed during cleanup)
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-acknowledged-commit".into(),
            entries: vec![make_journal_entry("config.ini", orig, cand)],
            phase: Some(JournalPhase::Committed),
        },
    )
    .unwrap();

    let report = recovery::recover_all(&workspace);

    assert_eq!(
        report.completed, 1,
        "acknowledged commit must be recognized as complete"
    );
    assert_eq!(
        report.restored, 0,
        "acknowledged commit must NOT be reversed"
    );
    assert_eq!(
        workspace.read_file("config.ini").unwrap(),
        cand,
        "candidate bytes preserved"
    );
}

#[test]
fn ambiguous_legacy_journal_handled_conservatively_for_human_attention() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"legacy original\n";
    let cand = b"legacy candidate\n";

    workspace.write_file_atomic("legacy.txt", cand).unwrap();

    // Legacy journal without a phase field, all members matching candidate
    let legacy_json = serde_json::json!({
        "protocol_version": "1.1.0",
        "transaction_id": "tx-legacy-ambiguous",
        "entries": [{
            "path": "legacy.txt",
            "pre_hash": compute_sha256(orig),
            "candidate_hash": compute_sha256(cand),
            "original": orig,
            "candidate": cand
        }]
    });

    let rec_dir = temp.path().join(".threadmoth-recovery");
    fs::create_dir_all(&rec_dir).unwrap();
    fs::write(
        rec_dir.join("tx-legacy-ambiguous.json"),
        serde_json::to_vec_pretty(&legacy_json).unwrap(),
    )
    .unwrap();

    // Inspect list report
    let list = recovery::list(&workspace);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].apparent_state, "ambiguous");
    assert!(!list.entries[0].automatic_recovery_safe);

    // Run recover_all: must not guess! Must not complete, must not restore, must report for manual attention
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0);
    assert_eq!(report.restored, 0);
    assert!(!report.manual.is_empty());
    assert!(report.manual[0].contains("ambiguous legacy journal state"));

    // Journal file must remain for human inspection
    assert!(rec_dir.join("tx-legacy-ambiguous.json").exists());
    assert_eq!(workspace.read_file("legacy.txt").unwrap(), cand);
}

#[test]
#[allow(clippy::permissions_set_readonly_false)]
fn direct_and_prepared_transactions_reconcile_rollback_and_error_handling() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"file1\n";
    let f2_orig = b"file2\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    // Create a transaction where f2 will fail post-commit verification by assertion
    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-reconciliation".into(),
        requests: vec![
            text_request("f1.txt", "file1", "new1"),
            text_request("f2.txt", "file2", "new2"),
        ],
        budget: Default::default(),
    };

    // Prepare plan with no extra assertions
    let plan = prepare_transaction_plan(&workspace, &tx, vec![]).unwrap();

    // Make f2.txt read-only so write_file_atomic_checked fails on f2 after f1 is committed
    let mut perms = fs::metadata(temp.path().join("f2.txt"))
        .unwrap()
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(temp.path().join("f2.txt"), perms).unwrap();

    // Apply the plan
    let result = apply_prepared_plan(&workspace, &plan);

    // Clean up permissions early so tempdir cleanup is reliable
    let mut reset_perms = fs::metadata(temp.path().join("f2.txt"))
        .unwrap()
        .permissions();
    reset_perms.set_readonly(false);
    let _ = fs::set_permissions(temp.path().join("f2.txt"), reset_perms);

    match result {
        PlanApplyResult::Transaction(cert) => {
            assert_eq!(cert.outcome, Outcome::Failed);
            assert_eq!(cert.rollback_state, "rolled_back");
            assert!(matches!(
                cert.failure_reason,
                Some(FailureReason::CommitFailure { .. })
            ));
            // f1 must have been rolled back to original
            assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), f1_orig);
        }
        _ => panic!("expected transaction result"),
    }
}
