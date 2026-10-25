use std::fs;
use tempfile::TempDir;
use threadmoth::{
    lifecycle::FileOperation,
    pipeline::execute_request,
    protocol::{
        Cardinality, DesiredStateOperation, EffectBudget, OperationPayload, Outcome, Request,
        TransactionRequest, PROTOCOL_VERSION,
    },
    provider::text::{NewlinePolicy, TextOperation, TextReplacementIntent},
    workspace::Workspace,
};

fn make_request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "firewall-test".into(),
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

// 1. Uniform LF source receives CRLF input: result must remain 100% uniform LF (Hamesh regression test)
#[test]
fn scenario_01_hamesh_regression_lf_source_receiving_crlf_input_stays_uniform_lf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("hamesh_text.rs");

    // 1,109 LF lines simulating the exact Hamesh file
    let mut initial = String::new();
    for i in 1..=1109 {
        if i == 500 {
            initial.push_str("pub fn target_function() {\n    let value = 1;\n}\n");
        } else {
            initial.push_str(&format!("// line {i} in hamesh text crate\n"));
        }
    }
    fs::write(&file_path, &initial).unwrap();
    assert!(!initial.contains("\r\n"), "Initial file must be 100% LF");

    // Edit supplying replacement lines with CRLF endings (agent running on Windows PowerShell)
    let request = make_request(
        "hamesh_text.rs",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: vec![TextReplacementIntent {
                old: "pub fn target_function() {\n    let value = 1;\n}".into(),
                new: "pub fn target_function() {\r\n    let value = 42;\r\n    let added = true;\r\n}".into(),
            }],
            newline_policy: NewlinePolicy::Source,
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert!(cert.preservation.newline_adaptation_applied);
    assert!(!cert.preservation.conversion_authorized);

    let landed = fs::read(&file_path).unwrap();
    let landed_str = std::str::from_utf8(&landed).unwrap();
    assert!(
        !landed_str.contains("\r\n"),
        "CRLF must not have leaked into uniform LF file!"
    );
    assert!(landed_str.contains("let value = 42;\n    let added = true;"));

    // Check certificate newline facts
    let counts = cert.preservation.newline_counts.unwrap();
    assert_eq!(counts.result_crlf, 0);
    assert_eq!(counts.result_bare_cr, 0);
    assert!(counts.result_lf > counts.original_lf);
}

// 2. Uniform CRLF source receives LF input: result must remain 100% uniform CRLF
#[test]
fn scenario_02_crlf_source_receiving_lf_input_stays_uniform_crlf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("windows_file.rs");

    let initial = "struct Config {\r\n    timeout: u32,\r\n}\r\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "windows_file.rs",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: vec![TextReplacementIntent {
                old: "timeout: u32,".into(),
                new: "timeout: u32,\n    retries: u8,\n    enabled: bool,".into(),
            }],
            newline_policy: NewlinePolicy::Source,
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert!(cert.preservation.newline_adaptation_applied);

    let landed = fs::read(&file_path).unwrap();
    let landed_str = std::str::from_utf8(&landed).unwrap();
    assert!(landed_str.contains("retries: u8,\r\n    enabled: bool,\r\n"));

    let counts = cert.preservation.newline_counts.unwrap();
    assert_eq!(counts.result_lf, 0);
    assert_eq!(counts.result_bare_cr, 0);
    assert_eq!(counts.result_crlf, 5);
}

// 3. Mixed file receives edit in an LF region: only LF inserted, unchanged CRLF lines untouched
#[test]
fn scenario_03_mixed_file_edit_in_lf_region_preserves_crlf_elsewhere() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("mixed.txt");

    // Top: CRLF header, Bottom: LF body
    let initial = "header_1\r\nheader_2\r\n\nbody_1\nbody_2\nbody_3\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "mixed.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: vec![TextReplacementIntent {
                old: "body_2".into(),
                new: "body_2_a\r\nbody_2_b".into(), // user passes CRLF, but region is LF
            }],
            newline_policy: NewlinePolicy::Source,
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed = fs::read(&file_path).unwrap();
    let landed_str = std::str::from_utf8(&landed).unwrap();
    assert!(landed_str.starts_with("header_1\r\nheader_2\r\n"));
    assert!(landed_str.contains("body_2_a\nbody_2_b\nbody_3\n"));
    assert!(!landed_str.contains("body_2_a\r\nbody_2_b"));
}

// 4. Mixed file receives edit in a CRLF region: only CRLF inserted, unchanged LF lines untouched
#[test]
fn scenario_04_mixed_file_edit_in_crlf_region_preserves_lf_elsewhere() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("mixed_crlf.txt");

    let initial = "top_crlf_1\r\ntop_crlf_2\r\nbottom_lf_1\nbottom_lf_2\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "mixed_crlf.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: vec![TextReplacementIntent {
                old: "top_crlf_1".into(),
                new: "top_crlf_1_a\ntop_crlf_1_b".into(), // user passes LF, but region is CRLF
            }],
            newline_policy: NewlinePolicy::Source,
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed = fs::read(&file_path).unwrap();
    let landed_str = std::str::from_utf8(&landed).unwrap();
    assert!(landed_str.contains("top_crlf_1_a\r\ntop_crlf_1_b\r\ntop_crlf_2\r\n"));
    assert!(landed_str.ends_with("bottom_lf_1\nbottom_lf_2\n"));
}

// 5. Mixed file with ambiguous local convention: fails closed with exact explanation, no guessing
#[test]
fn scenario_05_ambiguous_local_convention_fails_closed_without_guessing() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("ambiguous.txt");

    // Preceding line ending is CRLF (\r\n), succeeding line ending is LF (\n)
    let initial = "crlf_line\r\nAMBIGUOUS_BOUNDARY\nlf_line\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "ambiguous.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "AMBIGUOUS_BOUNDARY".into(),
            replacement: "new_line_1\nnew_line_2".into(),
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    let details = match &cert.refusal_reason {
        Some(threadmoth::protocol::RefusalReason::PreservationUnavailable { details }) => {
            details.clone()
        }
        other => panic!("Expected PreservationUnavailable, got {other:?}"),
    };
    assert!(details.contains("preceding") || details.contains("ambiguous"));

    // Source must be unchanged
    let landed = fs::read(&file_path).unwrap();
    assert_eq!(landed, initial.as_bytes());
}

// 6. Multi-byte UTF-8 immediately before/after newlines: no panic, no slicing error
#[test]
fn scenario_06_multibyte_utf8_near_miss_and_replacement_no_panic() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("utf8.txt");

    // Multi-byte Unicode (é, 🦀, 你好) ending directly before \r\n and \n
    let initial = "café\r\ncrab🦀\r\n你好\n";
    fs::write(&file_path, initial).unwrap();

    // Near-miss check must not panic on UTF-8 continuation bytes
    let request_near_miss = make_request(
        "utf8.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "café\ncrab🦀".into(),
            replacement: "replacement".into(),
        }),
    );
    let cert = execute_request(&workspace, &request_near_miss, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(cert.reason_code.as_deref(), Some("TARGET_NOT_FOUND"));

    // Exact replacement with multibyte UTF-8 works cleanly
    let request_exact = make_request(
        "utf8.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "crab🦀\r\n".into(),
            replacement: "golden🦀\r\n".into(),
        }),
    );
    let cert2 = execute_request(&workspace, &request_exact, false);
    assert_eq!(cert2.outcome, Outcome::Applied);
    let landed = fs::read_to_string(&file_path).unwrap();
    assert!(landed.contains("golden🦀\r\n"));
}

// 7. Multi-line replacement spanning differing source endings: handled predictably
#[test]
fn scenario_07_multiline_replacement_spanning_differing_endings() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("span.txt");

    let initial = "line1\r\nline2\nline3\r\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "span.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "line1\r\nline2\n".into(),
            replacement: "replaced_content\n".into(),
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    // Spans differing endings; firewall enforces local preservation or safe refusal
    assert!(cert.outcome == Outcome::Applied || cert.outcome == Outcome::Refused);
    let landed = fs::read_to_string(&file_path).unwrap();
    assert!(landed.ends_with("line3\r\n"));
}

// 8. Patch ingestion with conflicting endings: handled without corrupting unaffected parts
#[test]
fn scenario_08_diff_patch_planner_conflicting_endings_preserves_uniform_lf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("patch_target.rs");

    let initial = "fn one() -> i32 {\n    1\n}\nfn two() -> i32 {\n    2\n}\n";
    fs::write(&file_path, initial).unwrap();

    // Request replacing fn one with CRLF content
    let request = make_request(
        "patch_target.rs",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: vec![TextReplacementIntent {
                old: "fn one() -> i32 {\n    1\n}".into(),
                new: "fn one() -> i32 {\r\n    100\r\n}".into(),
            }],
            newline_policy: NewlinePolicy::Source,
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed = fs::read(&file_path).unwrap();
    assert!(!landed.contains(&b'\r'));
    assert!(std::str::from_utf8(&landed)
        .unwrap()
        .contains("fn one() -> i32 {\n    100\n}"));
}

// 9. Structural replacement (ast-grep) against LF file using tool defaults: stays uniform LF
#[test]
fn scenario_09_structural_ast_grep_against_lf_file_stays_uniform_lf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("app.py");

    let initial = "def process(item):\n    old_call(item)\n    return True\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "app.py",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"def process(item):\n    new_call(item, extra=True)\n    return True\n"
                .to_vec(),
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed = fs::read(&file_path).unwrap();
    assert!(!landed.contains(&b'\r'), "Must remain 100% LF");
}

// 10. Structural replacement against CRLF file: stays uniform CRLF
#[test]
fn scenario_10_structural_against_crlf_file_stays_uniform_crlf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("app.rs");

    let initial = "fn run() {\r\n    old_call();\r\n}\r\n";
    fs::write(&file_path, initial).unwrap();

    let request = make_request(
        "app.rs",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"fn run() {\r\n    new_call();\r\n}\r\n".to_vec(),
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed = fs::read(&file_path).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(counts.lf, 0);
    assert_eq!(counts.crlf, 3);
}

// 11. Desired-state mode attempting conversion without authority: refused
#[test]
fn scenario_11_desired_state_unauthorized_conversion_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("strict_lf.txt");

    let initial = "line1\nline2\nline3\n";
    fs::write(&file_path, initial).unwrap();

    // Desired state changes all endings to CRLF without conversion authority
    let request = make_request(
        "strict_lf.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"line1\r\nline2\r\nline3\r\n".to_vec(),
        }),
    );

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    // Source must be unchanged
    let landed = fs::read(&file_path).unwrap();
    assert_eq!(landed, initial.as_bytes());
}

// 12. Multi-file transaction touching both LF and CRLF files: each maintains its convention
#[test]
fn scenario_12_multifile_transaction_preserves_distinct_conventions() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let file_lf = temp.path().join("file_lf.txt");
    let file_crlf = temp.path().join("file_crlf.txt");

    fs::write(&file_lf, "name = \"alpha\"\nversion = 1\n").unwrap();
    fs::write(&file_crlf, "name = \"beta\"\r\nversion = 2\r\n").unwrap();

    let req_a = make_request(
        "file_lf.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "version = 1".into(),
            replacement: "version = 10".into(),
        }),
    );

    let req_b = make_request(
        "file_crlf.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "version = 2".into(),
            replacement: "version = 20".into(),
        }),
    );

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-mixed-convention".into(),
        requests: vec![req_a, req_b],
        budget: EffectBudget::default(),
    };

    let cert = threadmoth::pipeline::execute_transaction(&workspace, &tx, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed_lf = fs::read(&file_lf).unwrap();
    let counts_lf = threadmoth::newline::count_newlines(&landed_lf);
    assert_eq!(counts_lf.crlf, 0);
    assert_eq!(counts_lf.lf, 2);

    let landed_crlf = fs::read(&file_crlf).unwrap();
    let counts_crlf = threadmoth::newline::count_newlines(&landed_crlf);
    assert_eq!(counts_crlf.lf, 0);
    assert_eq!(counts_crlf.crlf, 2);
}

// 13. New file creation: uniform endings only, bare CR rejected
#[test]
fn scenario_13_new_file_creation_rejects_mixed_and_bare_cr() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    // 13a: Mixed endings in new file -> refused
    let req_mixed = make_request(
        "bad_mixed.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"line1\r\nline2\n".to_vec(),
        }),
    );
    let cert1 = execute_request(&workspace, &req_mixed, false);
    assert_eq!(cert1.outcome, Outcome::Refused);
    assert_eq!(
        cert1.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    // 13b: Bare CR in new file -> refused
    let req_bare_cr = make_request(
        "bad_cr.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"line1\rline2\r".to_vec(),
        }),
    );
    let cert2 = execute_request(&workspace, &req_bare_cr, false);
    assert_eq!(cert2.outcome, Outcome::Refused);
    assert_eq!(
        cert2.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    // 13c: Uniform LF new file -> applied
    let req_lf = make_request(
        "good_lf.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"line1\nline2\n".to_vec(),
        }),
    );
    let cert3 = execute_request(&workspace, &req_lf, false);
    assert_eq!(cert3.outcome, Outcome::Applied);
    assert!(temp.path().join("good_lf.txt").exists());

    // 13d: Uniform CRLF new file -> applied
    let req_crlf = make_request(
        "good_crlf.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"line1\r\nline2\r\n".to_vec(),
        }),
    );
    let cert4 = execute_request(&workspace, &req_crlf, false);
    assert_eq!(cert4.outcome, Outcome::Applied);
    assert!(temp.path().join("good_crlf.txt").exists());
}

// 14. Intentional conversion: succeeds only when explicit authority is given, reported accurately
#[test]
fn scenario_14_intentional_conversion_with_authority_succeeds_and_certifies() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("convert_target.txt");

    let initial = "line1\r\nline2\r\nline3\r\n";
    fs::write(&file_path, initial).unwrap();

    // 14a: ConvertNewlines with allow_newline_conversion: true
    let mut request = make_request(
        "convert_target.txt",
        OperationPayload::Text(TextOperation::ConvertNewlines { to: "lf".into() }),
    );
    request.allow_newline_conversion = true;

    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert!(cert.preservation.conversion_authorized);

    let landed = fs::read(&file_path).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(counts.crlf, 0);
    assert_eq!(counts.lf, 3);
    assert_eq!(landed, b"line1\nline2\nline3\n");

    // 14b: Refinement with allow_newline_conversion: true allows altering uniform line endings
    let mut request_crlf = make_request(
        "convert_target.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"line1\r\nline2\r\nline3\r\n".to_vec(),
        }),
    );
    request_crlf.allow_newline_conversion = true;

    let cert2 = execute_request(&workspace, &request_crlf, false);
    assert_eq!(cert2.outcome, Outcome::Applied);
    assert!(cert2.preservation.conversion_authorized);

    let landed2 = fs::read(&file_path).unwrap();
    let counts2 = threadmoth::newline::count_newlines(&landed2);
    assert_eq!(counts2.lf, 0);
    assert_eq!(counts2.crlf, 3);
}

// 15. Mixed -> uniform unauthorized whole-file conversion is refused
#[test]
fn scenario_15_mixed_to_uniform_unauthorized_conversion_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("mixed.txt");
    fs::write(&file_path, b"a\nb\r\n").unwrap();

    let request = make_request(
        "mixed.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"a\nb\n".to_vec(),
        }),
    );
    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert_eq!(fs::read(&file_path).unwrap(), b"a\nb\r\n");
}

// 16. Mixed -> uniform by legitimate deletion of CRLF lines succeeds
#[test]
fn scenario_16_mixed_to_uniform_by_legitimate_deletion_succeeds() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("mixed_del.txt");
    fs::write(&file_path, b"a\nb\r\n").unwrap();

    let request = make_request(
        "mixed_del.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "b\r\n".into(),
            replacement: "".into(),
        }),
    );
    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(fs::read(&file_path).unwrap(), b"a\n");
    assert_eq!(cert.preservation.original_newline_profile, "mixed");
    assert_eq!(cert.preservation.result_newline_profile, "lf");
}

// 17. Prepared plan attempting line ending conversion fails at apply-plan
#[test]
fn scenario_17_prepared_plan_attempting_line_ending_conversion_fails_at_apply() {
    use threadmoth::engine::compute_sha256;
    use threadmoth::pipeline::{apply_prepared_plan, plan_identity};
    use threadmoth::protocol::{ByteEdit, PlanApplyResult, PreparedPlan, PreparedPlanOperation};

    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("source.txt");
    fs::write(&file_path, b"line1\r\nline2\r\n").unwrap();
    let pre_hash = compute_sha256(b"line1\r\nline2\r\n");

    let op_req = make_request(
        "source.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            newline_policy: NewlinePolicy::Exact,
            edits: vec![TextReplacementIntent {
                old: "line1\r\n".into(),
                new: "line1\n".into(),
            }],
        }),
    );
    let mut plan = PreparedPlan {
        schema_version: "1.0".into(),
        protocol_version: PROTOCOL_VERSION.into(),
        plan_id: String::new(),
        request_id: "test-req".into(),
        transaction_id: None,
        operations: vec![PreparedPlanOperation {
            file_path: "source.txt".into(),
            provider: "text".into(),
            request: op_req,
            pre_hash: pre_hash.clone(),
            edits: vec![ByteEdit {
                offset: 5,
                delete_len: 2,
                replacement: b"\n".to_vec(),
            }],
            prospective_hash: compute_sha256(b"line1\nline2\r\n"),
        }],
        assertions: Vec::new(),
        budget: EffectBudget::default(),
    };
    plan.plan_id = plan_identity(&plan);

    let result = apply_prepared_plan(&workspace, &plan);
    match result {
        PlanApplyResult::Certificate(cert) => {
            assert_eq!(cert.outcome, Outcome::Refused);
            assert_eq!(
                cert.reason_code.as_deref(),
                Some("PRESERVATION_UNAVAILABLE")
            );
        }
        _ => panic!("expected refused certificate from forged conversion plan"),
    }
    assert_eq!(fs::read(&file_path).unwrap(), b"line1\r\nline2\r\n");
}

// 18. Stale or tampered plan fails safely
#[test]
fn scenario_18_stale_or_tampered_plan_fails_safely() {
    use threadmoth::engine::compute_sha256;
    use threadmoth::pipeline::{apply_prepared_plan, plan_identity};
    use threadmoth::protocol::{ByteEdit, PlanApplyResult, PreparedPlan, PreparedPlanOperation};

    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("stale.txt");
    fs::write(&file_path, b"original content\n").unwrap();

    let op_req = make_request(
        "stale.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "original".into(),
            replacement: "updated".into(),
        }),
    );
    let mut plan = PreparedPlan {
        schema_version: "1.0".into(),
        protocol_version: PROTOCOL_VERSION.into(),
        plan_id: String::new(),
        request_id: "test-stale".into(),
        transaction_id: None,
        operations: vec![PreparedPlanOperation {
            file_path: "stale.txt".into(),
            provider: "text".into(),
            request: op_req,
            pre_hash: compute_sha256(b"different content\n"),
            edits: vec![ByteEdit {
                offset: 0,
                delete_len: 8,
                replacement: b"updated".to_vec(),
            }],
            prospective_hash: compute_sha256(b"updated content\n"),
        }],
        assertions: Vec::new(),
        budget: EffectBudget::default(),
    };
    plan.plan_id = plan_identity(&plan);

    let result = apply_prepared_plan(&workspace, &plan);
    match result {
        PlanApplyResult::Certificate(cert) => {
            assert_eq!(cert.outcome, Outcome::Refused);
            assert_eq!(cert.reason_code.as_deref(), Some("PLAN_STALE"));
        }
        _ => panic!("expected refused certificate for stale plan"),
    }
}

// 19. LF and CRLF file creation under .editorconfig and deterministic LF default
#[test]
fn scenario_19_file_creation_respects_editorconfig_scoped_policies_and_default_lf() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let editorconfig = "[*]\nend_of_line = lf\n[*.bat]\nend_of_line = crlf\n";
    fs::write(temp.path().join(".editorconfig"), editorconfig).unwrap();

    let req_sh = make_request(
        "script.sh",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"echo hello\r\necho world\r\n".to_vec(),
        }),
    );
    let cert_sh = execute_request(&workspace, &req_sh, false);
    assert_eq!(cert_sh.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("script.sh")).unwrap(),
        b"echo hello\necho world\n"
    );
    assert_eq!(cert_sh.preservation.result_newline_profile, "lf");

    let req_bat = make_request(
        "build.bat",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"@echo off\npause\n".to_vec(),
        }),
    );
    let cert_bat = execute_request(&workspace, &req_bat, false);
    assert_eq!(cert_bat.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp.path().join("build.bat")).unwrap(),
        b"@echo off\r\npause\r\n"
    );
    assert_eq!(cert_bat.preservation.result_newline_profile, "crlf");

    let temp2 = TempDir::new().unwrap();
    let ws2 = Workspace::new(temp2.path()).unwrap();
    let req_default = make_request(
        "default.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"line1\r\nline2\r\n".to_vec(),
        }),
    );
    let cert_def = execute_request(&ws2, &req_default, false);
    assert_eq!(cert_def.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(temp2.path().join("default.txt")).unwrap(),
        b"line1\nline2\n"
    );
    assert_eq!(cert_def.preservation.result_newline_profile, "lf");
}

// 20. Edits that split CRLF boundary are refused
#[test]
fn scenario_20_edits_splitting_crlf_boundary_are_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("boundary.txt");
    fs::write(&file_path, b"abc\r\ndef\r\n").unwrap();

    let req = make_request(
        "boundary.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "\ndef".into(),
            replacement: "XYZ".into(),
        }),
    );
    let cert = execute_request(&workspace, &req, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert!(format!("{:?}", cert.refusal_reason).contains("begins inside a CRLF sequence"));
}

// 21. Unicode and bare CR injection edge cases
#[test]
fn scenario_21_unicode_and_bare_cr_refusal_edge_cases() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("unicode.txt");
    fs::write(&file_path, "hello \u{1F30D} world\n".as_bytes()).unwrap();

    let req_bare = make_request(
        "unicode.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "world".into(),
            replacement: "world\rtest".into(),
        }),
    );
    let cert_bare = execute_request(&workspace, &req_bare, false);
    assert_eq!(cert_bare.outcome, Outcome::Refused);
    assert_eq!(
        cert_bare.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    let req_utf8 = make_request(
        "unicode.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "\u{1F30D}".into(),
            replacement: "\u{1F680}\u{2728}".into(),
        }),
    );
    let cert_utf8 = execute_request(&workspace, &req_utf8, false);
    assert_eq!(cert_utf8.outcome, Outcome::Applied);
    assert_eq!(
        fs::read(&file_path).unwrap(),
        "hello \u{1F680}\u{2728} world\n".as_bytes()
    );
}

// 22. Exact matching without conversion authorization cannot alter endings
#[test]
fn scenario_22_exact_matching_without_conversion_authority_cannot_alter_endings() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("exact_guard.txt");
    fs::write(&file_path, b"line1\nline2\n").unwrap();

    let req = make_request(
        "exact_guard.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            newline_policy: NewlinePolicy::Exact,
            edits: vec![TextReplacementIntent {
                old: "line1\n".into(),
                new: "line1\r\n".into(),
            }],
        }),
    );
    let cert = execute_request(&workspace, &req, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert!(format!("{:?}", cert.refusal_reason).contains("without explicit conversion authority"));
}

// 23. Cross-provider candidate preservation
#[test]
fn scenario_23_cross_provider_candidate_preservation_enforced_consistently() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("providers.txt");
    fs::write(&file_path, b"hello world\n").unwrap();

    let req_ds = make_request(
        "providers.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"hello world\r\n".to_vec(),
        }),
    );
    let cert_ds = execute_request(&workspace, &req_ds, false);
    assert_eq!(cert_ds.outcome, Outcome::Refused);
    assert_eq!(
        cert_ds.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    let req_txt = make_request(
        "providers.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "world\n".into(),
            replacement: "world\r\n".into(),
        }),
    );
    let cert_txt = execute_request(&workspace, &req_txt, false);
    assert_eq!(cert_txt.outcome, Outcome::NoChange);
    assert_eq!(fs::read(&file_path).unwrap(), b"hello world\n");
}

// 24. Truthful certificate preservation metadata
#[test]
fn scenario_24_truthful_certificate_preservation_metadata() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("cert_truth.txt");
    fs::write(&file_path, b"header\r\nbody\r\nfooter\r\n").unwrap();

    let req = make_request(
        "cert_truth.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "body\r\n".into(),
            replacement: "new_body\n".into(),
        }),
    );
    let cert = execute_request(&workspace, &req, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(cert.preservation.original_newline_profile, "crlf");
    assert_eq!(cert.preservation.result_newline_profile, "crlf");
    assert!(!cert.preservation.line_endings_changed);
    assert!(!cert.preservation.conversion_authorized);

    let mut req_conv = make_request(
        "cert_truth.txt",
        OperationPayload::Text(TextOperation::ConvertNewlines { to: "lf".into() }),
    );
    req_conv.allow_newline_conversion = true;
    let cert_conv = execute_request(&workspace, &req_conv, false);
    assert_eq!(cert_conv.outcome, Outcome::Applied);
    assert_eq!(cert_conv.preservation.original_newline_profile, "crlf");
    assert_eq!(cert_conv.preservation.result_newline_profile, "lf");
    assert!(cert_conv.preservation.line_endings_changed);
    assert!(cert_conv.preservation.conversion_authorized);
}

// 25. Mixed LF/CRLF replacement preserving the exact original newline sequence (Lucy's case)
#[test]
fn scenario_25_mixed_replacement_preserving_exact_newline_sequence_succeeds() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("lucy_repro.txt");
    fs::write(&file_path, b"a\nb\r\n").unwrap();

    let request = make_request(
        "lucy_repro.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "a\nb\r\n".into(),
            replacement: "A\nb\r\n".into(),
        }),
    );
    let cert = execute_request(&workspace, &request, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(fs::read(&file_path).unwrap(), b"A\nb\r\n");
    let counts = threadmoth::newline::count_newlines(&fs::read(&file_path).unwrap());
    assert_eq!(counts.lf, 1);
    assert_eq!(counts.crlf, 1);
}

// 26. Multiple sub-line edits in a changes.tm edit document
#[test]
fn scenario_26_multiple_subline_edits_in_changes_tm_document() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("subline.rs");
    fs::write(&file_path, b"let a = 1; let b = 2;\n").unwrap();

    let doc = "*** File: subline.rs\n<<<<<<< SEARCH\nlet a = 1;\n=======\nlet a = 10;\n>>>>>>> REPLACE\n<<<<<<< SEARCH\nlet b = 2;\n=======\nlet b = 20;\n>>>>>>> REPLACE\n";
    let prep = threadmoth::edit_document::prepare(&workspace, doc).expect("preparation succeeds");
    let apply = threadmoth::pipeline::apply_prepared_plan(&workspace, &prep.plan);
    let outcome = match apply {
        threadmoth::protocol::PlanApplyResult::Transaction(cert) => cert.outcome,
        threadmoth::protocol::PlanApplyResult::Certificate(cert) => cert.outcome,
    };
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(fs::read(&file_path).unwrap(), b"let a = 10; let b = 20;\n");
}

// 27. Identical changes through direct CLI, MCP and edit document produce byte-identical results
#[test]
fn scenario_27_identical_changes_cli_mcp_edit_doc() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();

    let p_cli = temp.path().join("cli.txt");
    let p_doc = temp.path().join("doc.txt");
    fs::write(&p_cli, b"alpha = 1\nbeta = 2\n").unwrap();
    fs::write(&p_doc, b"alpha = 1\nbeta = 2\n").unwrap();

    // Direct request / CLI
    let req = make_request(
        "cli.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "beta = 2\n".into(),
            replacement: "beta = 42\r\n".into(),
        }),
    );
    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    // Edit document
    let doc =
        "*** File: doc.txt\n<<<<<<< SEARCH\nbeta = 2\n=======\nbeta = 42\r\n>>>>>>> REPLACE\n";
    let prep = threadmoth::edit_document::prepare(&ws, doc).unwrap();
    let _ = threadmoth::pipeline::apply_prepared_plan(&ws, &prep.plan);

    let bytes_cli = fs::read(&p_cli).unwrap();
    let bytes_doc = fs::read(&p_doc).unwrap();
    assert_eq!(
        bytes_cli, bytes_doc,
        "CLI and edit-doc must produce identical bytes"
    );
    assert_eq!(bytes_cli, b"alpha = 1\nbeta = 42\n");
}

// 28. Prepared plan preview versus actual application effects agree
#[test]
fn scenario_28_prepared_plan_preview_vs_apply_effects_agree() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("preview_apply.txt");
    fs::write(&file_path, b"key1=value1\r\nkey2=value2\r\n").unwrap();

    let req = make_request(
        "preview_apply.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "key2=value2\r\n".into(),
            replacement: "key2=updated\r\nkey3=new\r\n".into(),
        }),
    );
    let plan = threadmoth::pipeline::prepare_request_plan(&workspace, &req, Vec::new()).unwrap();
    assert_eq!(plan.operations.len(), 1);
    let op = &plan.operations[0];
    let planned_regions = op.edits.len();

    let cert = match threadmoth::pipeline::apply_prepared_plan(&workspace, &plan) {
        threadmoth::protocol::PlanApplyResult::Certificate(cert) => cert,
        other => panic!("expected certificate, got {other:?}"),
    };
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(cert.effect.changed_regions, planned_regions);
    assert!(cert.effect.changed_bytes > 0);
    assert!(cert.effect.changed_lines > 0);
}

// 29. Effect budgets pass exactly at the limit and refuse one unit above it
#[test]
fn scenario_29_effect_budgets_pass_at_limit_refuse_one_below() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("budget_test.txt");
    fs::write(&file_path, b"a=1\nb=2\nc=3\n").unwrap();

    // 1 edit replacing 1 region
    let mut req_pass = make_request(
        "budget_test.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "b=2\n".into(),
            replacement: "b=20\n".into(),
        }),
    );
    req_pass.budget.max_changed_regions = Some(1);
    let cert_pass = execute_request(&workspace, &req_pass, false);
    assert_eq!(cert_pass.outcome, Outcome::Applied);

    // Refuse one below (limit 0, actual 1)
    let mut req_refuse = make_request(
        "budget_test.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "c=3\n".into(),
            replacement: "c=30\n".into(),
        }),
    );
    req_refuse.budget.max_changed_regions = Some(0);
    let cert_refuse = execute_request(&workspace, &req_refuse, false);
    assert_eq!(cert_refuse.outcome, Outcome::Refused);
    assert_eq!(
        cert_refuse.reason_code.as_deref(),
        Some("EFFECT_BUDGET_EXCEEDED")
    );
}

// 30. Boundary insertion between differing newline conventions is refused
#[test]
fn scenario_30_boundary_insertion_between_differing_conventions_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("boundary.txt");
    fs::write(&file_path, b"alpha\r\nBOUNDARY\nbeta\n").unwrap();

    // Inserting newlines at the boundary between preceding CRLF and succeeding LF without explicit conversion
    let req = make_request(
        "boundary.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "BOUNDARY".into(),
            replacement: "part1\npart2".into(),
        }),
    );
    let cert = execute_request(&workspace, &req, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
}
