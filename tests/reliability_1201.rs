use std::fs;
use tempfile::TempDir;
use threadmoth::engine::compute_sha256;
use threadmoth::pipeline::{
    apply_prepared_plan, commit_and_cleanup_journal, prepare_transaction_plan, rollback_entries,
};
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
        allow_newline_conversion: false,
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
    let sub_dir = temp.path().join("sub");
    fs::create_dir(&sub_dir).unwrap();
    let f2_path = sub_dir.join("f2.txt");
    fs::write(&f2_path, f2_orig).unwrap();

    // Create a transaction where f2 will fail during commit
    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-reconciliation".into(),
        requests: vec![
            text_request("f1.txt", "file1", "new1"),
            text_request("sub/f2.txt", "file2", "new2"),
        ],
        budget: Default::default(),
    };

    // Prepare plan with no extra assertions
    let plan = prepare_transaction_plan(&workspace, &tx, vec![]).unwrap();

    // Make destination unwritable so write_file_atomic_checked fails on f2 after f1 is committed:
    // - On Windows: read-only file fails ensure_windows_replaceable
    // - On Unix: read-only directory (0o555) prevents AtomicWriteFile temp file creation
    #[cfg(windows)]
    {
        let mut perms = fs::metadata(&f2_path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&f2_path, perms).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sub_dir, fs::Permissions::from_mode(0o555)).unwrap();
    }

    // Apply the plan
    let result = apply_prepared_plan(&workspace, &plan);

    // Clean up permissions early so tempdir cleanup is reliable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&sub_dir, fs::Permissions::from_mode(0o755));
    }
    #[cfg(windows)]
    {
        let mut reset_perms = fs::metadata(&f2_path).unwrap().permissions();
        reset_perms.set_readonly(false);
        let _ = fs::set_permissions(&f2_path, reset_perms);
    }

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

#[test]
fn commit_marker_failure_and_cleanup_failure_in_direct_transaction_triggers_rollback_and_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"alpha original\n";
    let f2_orig = b"beta original\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-direct-double-fail".into(),
        requests: vec![
            text_request("f1.txt", "alpha", "new-alpha"),
            text_request("f2.txt", "beta", "new-beta"),
        ],
        budget: Default::default(),
    };

    // Inject double failure: commit-marker persistence fails AND journal cleanup fails
    recovery::fault_injection::set_fail_commit_marker(true);
    recovery::fault_injection::set_fail_remove_journal(true);

    let cert = threadmoth::pipeline::execute_transaction(&workspace, &tx, false);

    // Reset fault injection immediately
    recovery::fault_injection::reset();

    // Critical invariant: Threadmoth MUST NOT report Applied when journal on disk is still Prepared!
    assert_eq!(
        cert.outcome,
        Outcome::Failed,
        "must fail if commit marker and cleanup both fail"
    );
    assert_eq!(
        cert.rollback_state, "rolled_back",
        "must have rolled back all committed members"
    );
    assert!(
        matches!(
            cert.failure_reason,
            Some(FailureReason::CommitFailure { .. })
        ),
        "must report CommitFailure"
    );

    // Both files on disk MUST be rolled back to their original content
    assert_eq!(
        fs::read(temp.path().join("f1.txt")).unwrap(),
        f1_orig,
        "f1.txt must be rolled back to original bytes"
    );
    assert_eq!(
        fs::read(temp.path().join("f2.txt")).unwrap(),
        f2_orig,
        "f2.txt must be rolled back to original bytes"
    );

    // Journal remains on disk (since cleanup failed), but its files are now Original.
    // When recovery runs later, it will find all files match Original and clean up safely without reversing anything!
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0);
    assert_eq!(report.restored, 0);
    assert_eq!(report.cleaned, 1);
}

#[test]
fn commit_marker_failure_and_cleanup_failure_in_prepared_plan_triggers_rollback_and_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"gamma original\n";
    let f2_orig = b"delta original\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-plan-double-fail".into(),
        requests: vec![
            text_request("f1.txt", "gamma", "new-gamma"),
            text_request("f2.txt", "delta", "new-delta"),
        ],
        budget: Default::default(),
    };

    let plan = prepare_transaction_plan(&workspace, &tx, vec![]).unwrap();

    // Inject double failure: commit-marker persistence fails AND journal cleanup fails
    recovery::fault_injection::set_fail_commit_marker(true);
    recovery::fault_injection::set_fail_remove_journal(true);

    let result = apply_prepared_plan(&workspace, &plan);

    // Reset fault injection immediately
    recovery::fault_injection::reset();

    match result {
        PlanApplyResult::Transaction(cert) => {
            assert_eq!(
                cert.outcome,
                Outcome::Failed,
                "prepared plan must fail if commit marker and cleanup both fail"
            );
            assert_eq!(
                cert.rollback_state, "rolled_back",
                "must have rolled back all committed members"
            );
            assert!(
                matches!(
                    cert.failure_reason,
                    Some(FailureReason::CommitFailure { .. })
                ),
                "must report CommitFailure"
            );

            // Both files on disk MUST be restored to original bytes
            assert_eq!(
                fs::read(temp.path().join("f1.txt")).unwrap(),
                f1_orig,
                "f1.txt must be rolled back to original bytes"
            );
            assert_eq!(
                fs::read(temp.path().join("f2.txt")).unwrap(),
                f2_orig,
                "f2.txt must be rolled back to original bytes"
            );
        }
        _ => panic!("expected transaction result"),
    }
}

#[test]
fn committed_journal_with_mixed_states_is_never_rolled_back() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"file1 original\n";
    let f1_cand = b"file1 candidate\n";
    let f2_orig = b"file2 original\n";
    let f2_cand = b"file2 candidate\n";

    // f1 is Candidate, but f2 is Original (mixed state!)
    workspace.write_file_atomic("f1.txt", f1_cand).unwrap();
    workspace.write_file_atomic("f2.txt", f2_orig).unwrap();

    // Journal is explicitly marked Committed
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-committed-mixed".into(),
            entries: vec![
                make_journal_entry("f1.txt", f1_orig, f1_cand),
                make_journal_entry("f2.txt", f2_orig, f2_cand),
            ],
            phase: Some(JournalPhase::Committed),
        },
    )
    .unwrap();

    // List and inspect must truthfully report inconsistency and NOT mark it as automatic recovery safe!
    let list = recovery::list(&workspace);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].apparent_state, "inconsistent");
    assert!(
        !list.entries[0].automatic_recovery_safe,
        "committed mixed state must not be automatic recovery safe"
    );

    let inspect = recovery::inspect(&workspace, "tx-committed-mixed");
    assert!(
        !inspect.automatic_recovery_safe,
        "inspect must report automatic_recovery_safe = false for committed mixed state"
    );

    // Run recover_all: MUST NOT ROLL BACK f1!
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0, "must not complete");
    assert_eq!(report.restored, 0, "must NOT roll back committed member!");
    assert_eq!(report.cleaned, 0, "must preserve journal for inspection");
    assert!(
        !report.manual.is_empty(),
        "must report inconsistency for human inspection"
    );
    assert!(report.manual[0].contains("committed transaction has unexpected member states"));

    // Verify f1.txt REMAINS as Candidate content!
    assert_eq!(
        workspace.read_file("f1.txt").unwrap(),
        f1_cand,
        "committed candidate member must never be rolled back"
    );
}

#[test]
fn prepared_journal_with_all_candidates_is_uncommitted_and_rolls_back() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"alpha original\n";
    let f1_cand = b"alpha candidate\n";
    let f2_orig = b"beta original\n";
    let f2_cand = b"beta candidate\n";

    // Both files are in candidate state on disk
    workspace.write_file_atomic("f1.txt", f1_cand).unwrap();
    workspace.write_file_atomic("f2.txt", f2_cand).unwrap();

    // Journal is in Prepared phase (never reached commit!)
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-prepared-all-cand".into(),
            entries: vec![
                make_journal_entry("f1.txt", f1_orig, f1_cand),
                make_journal_entry("f2.txt", f2_orig, f2_cand),
            ],
            phase: Some(JournalPhase::Prepared),
        },
    )
    .unwrap();

    // Truthful listing: Prepared + all candidate is UNCOMMITTED, not "complete"!
    let list = recovery::list(&workspace);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(
        list.entries[0].apparent_state, "uncommitted",
        "Prepared journal with all candidates must be listed as uncommitted, never complete"
    );
    assert!(
        list.entries[0].automatic_recovery_safe,
        "Prepared journal is safe for rollback"
    );

    // Run recover_all: must roll back all candidates to original!
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0, "must not complete");
    assert_eq!(report.restored, 2, "must restore all candidate members");
    assert_eq!(
        report.cleaned, 1,
        "must clean up journal after complete rollback"
    );

    // Verify both files on disk were restored to original bytes!
    assert_eq!(workspace.read_file("f1.txt").unwrap(), f1_orig);
    assert_eq!(workspace.read_file("f2.txt").unwrap(), f2_orig);
}

#[test]
fn atomic_commit_marker_transition_preserves_valid_journal_across_interruption() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"key = 1\n";
    let cand = b"key = 2\n";
    workspace.write_file_atomic("file.txt", cand).unwrap();

    let journal = Journal {
        protocol_version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-atomic-transition".into(),
        entries: vec![make_journal_entry("file.txt", orig, cand)],
        phase: Some(JournalPhase::Prepared),
    };
    recovery::write_journal(&workspace, &journal).unwrap();

    // Simulate an interruption / early error
    recovery::fault_injection::set_fail_commit_marker(true);
    assert!(recovery::mark_journal_committed(&workspace, "tx-atomic-transition").is_err());
    recovery::fault_injection::reset();

    // Critical check for Blocker 2:
    // The journal on disk MUST NOT have been truncated or destroyed!
    // It must remain a valid, parseable Prepared journal!
    let journal_on_disk = recovery::inspect(&workspace, "tx-atomic-transition");
    assert_eq!(
        journal_on_disk.journal_validity, "valid",
        "journal must remain completely valid and non-empty after failed transition"
    );

    // Now complete the transition successfully using AtomicWriteFile
    let res = recovery::mark_journal_committed(&workspace, "tx-atomic-transition");
    assert!(
        res.is_ok(),
        "mark_journal_committed failed with {:?}",
        res.err()
    );

    let updated_inspect = recovery::inspect(&workspace, "tx-atomic-transition");
    assert_eq!(updated_inspect.journal_validity, "valid");
    let listed = recovery::list(&workspace);
    assert_eq!(listed.entries[0].apparent_state, "complete");
}

#[test]
fn atomic_commit_marker_transition_handles_partial_staged_write_without_corrupting_journal() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"data = initial\n";
    let cand = b"data = updated\n";
    workspace.write_file_atomic("state.txt", cand).unwrap();

    let journal = Journal {
        protocol_version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-partial-stage".into(),
        entries: vec![make_journal_entry("state.txt", orig, cand)],
        phase: Some(JournalPhase::Prepared),
    };
    recovery::write_journal(&workspace, &journal).unwrap();

    // Inject partial staged write failure: simulation of crash/interruption while writing staging file
    recovery::fault_injection::set_fail_stage_write(true);
    let res = recovery::mark_journal_committed(&workspace, "tx-partial-stage");
    assert!(res.is_err(), "must report stage write failure");
    recovery::fault_injection::reset();

    // The surviving journal at the main path MUST remain intact as a valid Prepared journal
    let inspected = recovery::inspect(&workspace, "tx-partial-stage");
    assert_eq!(inspected.journal_validity, "valid");
    let phase = recovery::read_journal_phase(&workspace, "tx-partial-stage").unwrap();
    assert_eq!(phase, Some(JournalPhase::Prepared));

    // Recovery sees Prepared + Candidate -> safely rolls back rather than guessing
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0);
    assert_eq!(report.restored, 1);
    assert_eq!(report.cleaned, 1);
    assert_eq!(workspace.read_file("state.txt").unwrap(), orig);
}

#[test]
fn pre_rename_failure_preserves_prepared_journal_and_executes_rollback_on_double_failure() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"a1 original\n";
    let f2_orig = b"a2 original\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-pre-rename-fail".into(),
        requests: vec![
            text_request("f1.txt", "a1", "new-a1"),
            text_request("f2.txt", "a2", "new-a2"),
        ],
        budget: Default::default(),
    };

    let plan = prepare_transaction_plan(&workspace, &tx, vec![]).unwrap();

    // Inject failure before rename, and cleanup failure
    recovery::fault_injection::set_fail_before_commit_rename(true);
    recovery::fault_injection::set_fail_remove_journal(true);

    let result = apply_prepared_plan(&workspace, &plan);
    recovery::fault_injection::reset();

    match result {
        PlanApplyResult::Transaction(cert) => {
            assert_eq!(cert.outcome, Outcome::Failed);
            assert_eq!(cert.rollback_state, "rolled_back");
            // Original contents must be restored
            assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), f1_orig);
            assert_eq!(fs::read(temp.path().join("f2.txt")).unwrap(), f2_orig);
        }
        _ => panic!("expected transaction result"),
    }

    // Journal on disk must still be in Prepared phase
    let phase = recovery::read_journal_phase(&workspace, "tx-pre-rename-fail").unwrap();
    assert_eq!(phase, Some(JournalPhase::Prepared));

    // Subsequent recovery will clean it up safely because files on disk match original
    let rec = recovery::recover_all(&workspace);
    assert_eq!(rec.completed, 0);
    assert_eq!(rec.restored, 0);
    assert_eq!(rec.cleaned, 1);
}

#[test]
fn post_rename_fsync_error_treats_transaction_as_committed_and_never_rolls_back() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"f1 original\n";
    let f1_cand = b"f1 candidate\n";
    let f2_orig = b"f2 original\n";
    let f2_cand = b"f2 candidate\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-post-rename-fsync".into(),
        requests: vec![
            text_request("f1.txt", "f1 original", "f1 candidate"),
            text_request("f2.txt", "f2 original", "f2 candidate"),
        ],
        budget: Default::default(),
    };

    // Inject post-rename fsync failure (rename succeeded, Committed is on disk, but syscall returns Err)
    // AND cleanup failure (remove_journal fails)
    recovery::fault_injection::set_fail_after_commit_rename(true);
    recovery::fault_injection::set_fail_remove_journal(true);

    let cert = threadmoth::pipeline::execute_transaction(&workspace, &tx, false);
    recovery::fault_injection::reset();

    // Critical invariant for Lucy's blocker:
    // Even though mark_journal_committed returned Err, the journal on disk was atomically
    // published as Committed! Threadmoth MUST NOT roll back candidate files!
    assert_eq!(
        cert.outcome,
        Outcome::Applied,
        "committed transaction must remain Applied despite post-rename fsync failure"
    );
    assert_eq!(cert.rollback_state, "recovery_available");

    // Files on disk MUST retain candidate bytes!
    assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), f1_cand);
    assert_eq!(fs::read(temp.path().join("f2.txt")).unwrap(), f2_cand);

    // Journal on disk MUST be in Committed phase!
    let phase = recovery::read_journal_phase(&workspace, "tx-post-rename-fsync").unwrap();
    assert_eq!(phase, Some(JournalPhase::Committed));

    // Recovery sees Committed + all candidate files -> completes without rollback and cleans journal!
    let rec = recovery::recover_all(&workspace);
    assert_eq!(
        rec.completed, 1,
        "recovery must complete the committed transaction"
    );
    assert_eq!(
        rec.restored, 0,
        "recovery must NOT reverse any candidate files"
    );
    assert_eq!(
        rec.cleaned, 1,
        "recovery must clean up the committed journal"
    );

    // Candidate files are still intact after recovery
    assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), f1_cand);
    assert_eq!(fs::read(temp.path().join("f2.txt")).unwrap(), f2_cand);
}

#[test]
fn commit_marker_failure_with_successful_journal_removal_triggers_rollback_and_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"beta1 original\n";
    let f2_orig = b"beta2 original\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-commit-fail-remove-ok".into(),
        requests: vec![
            text_request("f1.txt", "beta1 original", "beta1 new"),
            text_request("f2.txt", "beta2 original", "beta2 new"),
        ],
        budget: Default::default(),
    };

    // Inject commit marker failure BUT allow journal removal to succeed
    recovery::fault_injection::set_fail_commit_marker(true);
    // remove_journal failure is false by default

    let cert = threadmoth::pipeline::execute_transaction(&workspace, &tx, false);
    recovery::fault_injection::reset();

    // Critical invariant for PR #70 correctness:
    // Even though journal removal succeeded, the commit was NEVER recorded as Committed on disk.
    // Threadmoth MUST NOT acknowledge this transaction as Applied!
    // It must roll back all candidate files to original and report Failed!
    assert_eq!(
        cert.outcome,
        Outcome::Failed,
        "must fail if commit marker fails, even if journal removal succeeds"
    );
    assert_eq!(
        cert.rollback_state, "rolled_back",
        "must have rolled back all committed members"
    );
    assert!(
        matches!(
            cert.failure_reason,
            Some(FailureReason::CommitFailure { .. })
        ),
        "must report CommitFailure"
    );

    // Both files on disk MUST be rolled back to their original content
    assert_eq!(
        fs::read(temp.path().join("f1.txt")).unwrap(),
        f1_orig,
        "f1.txt must be rolled back to original bytes"
    );
    assert_eq!(
        fs::read(temp.path().join("f2.txt")).unwrap(),
        f2_orig,
        "f2.txt must be rolled back to original bytes"
    );

    // Journal was cleaned up during rollback
    assert!(
        recovery::list(&workspace).entries.is_empty(),
        "journal should be cleanly removed"
    );

    // Recovery sees nothing to recover
    let report = recovery::recover_all(&workspace);
    assert_eq!(report.completed, 0);
    assert_eq!(report.restored, 0);
    assert_eq!(report.cleaned, 0);
}

#[test]
fn post_rename_marker_error_with_successful_journal_removal_acknowledges_applied() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"gamma1 original\n";
    let f1_cand = b"gamma1 candidate\n";
    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-post-rename-remove-ok".into(),
        requests: vec![text_request(
            "f1.txt",
            "gamma1 original",
            "gamma1 candidate",
        )],
        budget: Default::default(),
    };

    // Post-rename fsync error, but remove_journal succeeds
    recovery::fault_injection::set_fail_after_commit_rename(true);

    let cert = threadmoth::pipeline::execute_transaction(&workspace, &tx, false);
    recovery::fault_injection::reset();

    // Since rename succeeded, Committed was written on disk.
    // Cleanup succeeded, so outcome is Applied with not_required rollback state.
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(cert.rollback_state, "not_required");

    // Candidate bytes preserved
    assert_eq!(fs::read(temp.path().join("f1.txt")).unwrap(), f1_cand);

    // Journal removed
    assert!(recovery::list(&workspace).entries.is_empty());
}

#[test]
fn commit_decision_on_missing_journal_triggers_rollback_and_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"original data\n";
    let cand = b"candidate data\n";
    workspace.write_file_atomic("data.txt", cand).unwrap();

    let entry = make_journal_entry("data.txt", orig, cand);
    let committed = vec![&entry];

    // Transaction ID has no journal written on disk
    let result = commit_and_cleanup_journal(&workspace, "tx-nonexistent", &committed);

    assert!(result.is_err(), "must fail when journal is missing");
    let (rollback, msg) = result.unwrap_err();

    assert_eq!(rollback.rollback_state(), "rolled_back");
    assert_eq!(rollback.restored, 1);
    assert_eq!(rollback.failed.len(), 0);
    assert!(msg.contains("commit decision failed"));

    // Candidate file must have been rolled back to original
    assert_eq!(workspace.read_file("data.txt").unwrap(), orig);
}

#[test]
fn commit_decision_on_unreadable_or_corrupt_journal_triggers_rollback_and_fails() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"secret original\n";
    let cand = b"secret candidate\n";
    workspace.write_file_atomic("secret.txt", cand).unwrap();

    let entry = make_journal_entry("secret.txt", orig, cand);
    let committed = vec![&entry];

    // Create a corrupted journal file on disk (invalid JSON)
    let rec_dir = temp.path().join(".threadmoth-recovery");
    fs::create_dir_all(&rec_dir).unwrap();
    fs::write(
        rec_dir.join("tx-corrupted.json"),
        b"invalid-json-payload-{{{",
    )
    .unwrap();

    let result = commit_and_cleanup_journal(&workspace, "tx-corrupted", &committed);

    assert!(result.is_err(), "must fail when journal is corrupt");
    let (rollback, msg) = result.unwrap_err();

    assert_eq!(rollback.rollback_state(), "rolled_back");
    assert_eq!(rollback.restored, 1);
    assert_eq!(rollback.failed.len(), 0);
    assert!(msg.contains("commit decision failed"));

    // Candidate file must have been rolled back to original
    assert_eq!(workspace.read_file("secret.txt").unwrap(), orig);
}

#[test]
fn crash_recovery_cleans_prepared_journal_when_files_were_already_rolled_back() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"steady original\n";
    let cand = b"proposed candidate\n";
    // File on disk is already Original (e.g. process crashed after rollback but before journal removal)
    workspace.write_file_atomic("steady.txt", orig).unwrap();

    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-crashed-after-rollback".into(),
            entries: vec![make_journal_entry("steady.txt", orig, cand)],
            phase: Some(JournalPhase::Prepared),
        },
    )
    .unwrap();

    let report = recovery::recover_all(&workspace);

    assert_eq!(report.completed, 0);
    assert_eq!(report.restored, 0);
    assert_eq!(report.cleaned, 1, "prepared journal must be cleaned up");
    assert_eq!(workspace.read_file("steady.txt").unwrap(), orig);
    assert!(!temp.path().join(".threadmoth-recovery").exists());
}

#[test]
fn partial_rollback_on_unproven_commit_preserves_journal_evidence_when_deletion_is_permitted() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"file 1 original bytes\n";
    let f1_cand = b"file 1 candidate bytes\n";
    let f2_orig = b"file 2 original bytes\n";
    let f2_cand = b"file 2 candidate bytes\n";

    fs::write(temp.path().join("f1.txt"), f1_orig).unwrap();
    fs::write(temp.path().join("f2.txt"), f2_orig).unwrap();

    let e1 = make_journal_entry("f1.txt", f1_orig, f1_cand);
    let e2 = make_journal_entry("f2.txt", f2_orig, f2_cand);

    // Write initial candidate files as if commit step wrote them
    workspace.write_file_atomic("f1.txt", f1_cand).unwrap();
    workspace.write_file_atomic("f2.txt", f2_cand).unwrap();

    // Write the journal with phase Prepared (in-flight before commit decision)
    recovery::write_journal(
        &workspace,
        &Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "tx-partial-rollback-preserve-journal".into(),
            entries: vec![e1.clone(), e2.clone()],
            phase: Some(JournalPhase::Prepared),
        },
    )
    .unwrap();

    // Corrupt f1.txt so that write_file_atomic_checked fails during rollback of f1
    fs::write(temp.path().join("f1.txt"), b"corrupted content on disk").unwrap();

    // Inject commit-marker failure.
    // NOTE: remove_journal failure is NOT injected (journal deletion is permitted / would succeed).
    recovery::fault_injection::set_fail_commit_marker(true);

    let committed = vec![&e1, &e2];
    let res = commit_and_cleanup_journal(
        &workspace,
        "tx-partial-rollback-preserve-journal",
        &committed,
    );
    recovery::fault_injection::reset();

    assert!(res.is_err(), "must report failure when commit marker fails");
    let (rollback, msg) = res.unwrap_err();

    // Invariants:
    // 1. Rollback must be reported as partial and require manual recovery
    assert!(rollback.is_partial(), "rollback must be partial");
    assert_eq!(rollback.restored, 1);
    assert_eq!(rollback.failed, vec!["f1.txt".to_string()]);
    assert_eq!(rollback.rollback_state(), "manual_recovery_required");
    assert!(msg.contains("journal retained for manual recovery"));

    // 2. f2 must have been safely rolled back to original
    assert_eq!(workspace.read_file("f2.txt").unwrap(), f2_orig);

    // 3. THE JOURNAL MUST BE RETAINED ON DISK! It must NOT have been deleted!
    let journal_list = recovery::list(&workspace);
    assert_eq!(
        journal_list.entries.len(),
        1,
        "journal must remain on disk to preserve recovery evidence"
    );
    assert_eq!(
        journal_list.entries[0].transaction_id,
        "tx-partial-rollback-preserve-journal"
    );
    let rec_dir = temp.path().join(".threadmoth-recovery");
    assert!(rec_dir
        .join("tx-partial-rollback-preserve-journal.json")
        .exists());
}
