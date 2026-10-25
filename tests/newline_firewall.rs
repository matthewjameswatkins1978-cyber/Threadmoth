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
