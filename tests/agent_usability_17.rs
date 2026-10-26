use std::fs;

use tempfile::TempDir;
use threadmoth::{
    engine::compute_sha256,
    pipeline::execute_request,
    protocol::{
        CandidateGuard, Cardinality, EffectBudget, OperationPayload, Outcome, RefusalReason,
        Request, PROTOCOL_VERSION,
    },
    provider::code::CodeOperation,
    provider::text::TextOperation,
    workspace::Workspace,
};

fn text_request(path: &str, target: &str, replacement: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "candidate-test".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::Text(TextOperation::Replace {
            target: target.into(),
            replacement: replacement.into(),
        }),
    }
}

#[test]
fn identical_text_candidates_can_be_selected_without_relocation() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("x.txt"), b"dup\nkeep\ndup\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let request = text_request("x.txt", "dup", "one");

    let refusal = execute_request(&workspace, &request, true);
    assert_eq!(refusal.outcome, Outcome::Refused);
    assert_eq!(refusal.reason_code.as_deref(), Some("TARGET_AMBIGUOUS"));
    let candidates = match refusal.refusal_reason.as_ref().unwrap() {
        RefusalReason::DuplicateTarget { candidates, .. } => candidates,
        other => panic!("expected duplicate target, got {other:?}"),
    };
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].selection_id, candidates[1].selection_id);
    assert_eq!(candidates[0].target, "dup");
    assert_eq!(
        candidates[0].selection_id,
        threadmoth::protocol::candidate_selection_id(
            &refusal.pre_hash,
            "text",
            candidates[0].start,
            candidates[0].end,
            &candidates[0].anchor_sha256,
        )
    );

    let mut selected = request.clone();
    selected.expected_pre_hash = Some(refusal.pre_hash.clone());
    selected.candidate_guard = Some(CandidateGuard {
        offset: candidates[0].offset,
        selection_id: candidates[0].selection_id.clone(),
    });
    let preview = execute_request(&workspace, &selected, true);
    assert_eq!(preview.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("x.txt")).unwrap(),
        b"dup\nkeep\ndup\n"
    );
    let committed = execute_request(&workspace, &selected, false);
    assert_eq!(committed.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("x.txt")).unwrap(),
        b"one\nkeep\ndup\n"
    );
}

#[test]
fn candidate_selection_is_rejected_for_wrong_id_and_stale_bytes() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("x.txt"), b"dup\ndup\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let request = text_request("x.txt", "dup", "one");
    let refusal = execute_request(&workspace, &request, true);
    let candidates = match refusal.refusal_reason.as_ref().unwrap() {
        RefusalReason::DuplicateTarget { candidates, .. } => candidates,
        other => panic!("expected duplicate target, got {other:?}"),
    };

    let mut wrong = request.clone();
    wrong.expected_pre_hash = Some(refusal.pre_hash.clone());
    wrong.candidate_guard = Some(CandidateGuard {
        offset: candidates[0].offset,
        selection_id: candidates[1].selection_id.clone(),
    });
    let wrong_result = execute_request(&workspace, &wrong, false);
    assert_eq!(
        wrong_result.reason_code.as_deref(),
        Some("CANDIDATE_SELECTION_INVALID")
    );
    assert_eq!(fs::read(temp.path().join("x.txt")).unwrap(), b"dup\ndup\n");

    fs::write(temp.path().join("x.txt"), b"prefix\ndup\ndup\n").unwrap();
    let mut stale = request;
    stale.expected_pre_hash = Some(refusal.pre_hash);
    stale.candidate_guard = Some(CandidateGuard {
        offset: candidates[0].offset,
        selection_id: candidates[0].selection_id.clone(),
    });
    let stale_result = execute_request(&workspace, &stale, false);
    assert_eq!(stale_result.reason_code.as_deref(), Some("STALE_IDENTITY"));
}

#[test]
fn identical_code_nodes_receive_distinct_guarded_selection_ids() {
    let temp = TempDir::new().unwrap();
    let source = b"fn main() {\n    let first = 1;\n    let second = 1;\n}\n";
    fs::write(temp.path().join("main.rs"), source).unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let request = Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "code-candidate-test".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: "main.rs".into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::Code(CodeOperation::ReplaceNode {
            language: "rust".into(),
            target: "1".into(),
            replacement: "2".into(),
            node_kind: None,
        }),
    };
    let refusal = execute_request(&workspace, &request, true);
    let candidates = match refusal.refusal_reason.as_ref().unwrap() {
        RefusalReason::DuplicateTarget { candidates, .. } => candidates,
        other => panic!("expected duplicate code target, got {other:?}"),
    };
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].selection_id, candidates[1].selection_id);

    let mut selected = request;
    selected.expected_pre_hash = Some(compute_sha256(source));
    selected.candidate_guard = Some(CandidateGuard {
        offset: candidates[1].offset,
        selection_id: candidates[1].selection_id.clone(),
    });
    let result = execute_request(&workspace, &selected, false);
    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("main.rs")).unwrap(),
        b"fn main() {\n    let first = 1;\n    let second = 2;\n}\n"
    );
}

#[test]
fn provider_detection_handles_common_special_filenames_without_guessing() {
    use threadmoth::metadata::detect_provider;

    assert_eq!(detect_provider(".env.local", None).0, "dotenv");
    assert_eq!(detect_provider("Dockerfile", None).0, "text");
    assert_eq!(detect_provider("Makefile", None).0, "text");
    assert_eq!(detect_provider("Cargo.toml", None).0, "toml");
    assert_eq!(detect_provider("package.json", None).0, "json");
    assert_eq!(detect_provider("tsconfig.json", None).0, "json");
    assert_eq!(detect_provider("server.config.js", None).0, "code");
    assert_eq!(detect_provider("types.d.ts", None).0, "code");
    assert_eq!(
        detect_provider("deploy", Some(b"#!/bin/sh\necho ok\n")).0,
        "code"
    );
}
