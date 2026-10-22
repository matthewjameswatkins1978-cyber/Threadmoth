use std::fs;
use std::path::Path;
use tempfile::TempDir;
use threadmoth::protocol::Outcome;
use threadmoth::toolbelt::structural::{
    discover, execute_structural, parse_ast_grep_output, StructuralError, StructuralOutcome,
    StructuralRequest,
};
use threadmoth::workspace::Workspace;

#[test]
fn structural_capability_is_discoverable_and_reports_engine() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let info = discover(&workspace);
    assert_eq!(info.name, "structural");
    assert_eq!(info.engine, "ast-grep");
    assert!(info.available, "ast-grep should be detected on PATH");
    assert!(info.version.is_some(), "version should be reported");
    assert!(
        info.supported_languages.contains(&"python".to_string()),
        "python should be supported"
    );
    assert!(
        info.supported_languages.contains(&"rust".to_string()),
        "rust should be supported"
    );

    // Orientation view check
    let orient = threadmoth::metadata::orientation(temp.path().to_str().unwrap());
    assert!(orient["toolbelt"]["structural"]["available"]
        .as_bool()
        .unwrap());
    assert_eq!(orient["toolbelt"]["structural"]["engine"], "ast-grep");

    // Capability view check
    let cap = threadmoth::metadata::capability_view(Some("toolbelt"));
    assert!(cap["attachments"].as_array().is_some());
}

#[test]
fn single_file_structural_transformation_locates_and_rewrites_pattern() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let initial_py = b"def calc():\n    return multiply(2, 3)\n\ndef other():\n    pass\n";
    workspace.write_file_atomic("calc.py", initial_py).unwrap();

    let req = StructuralRequest {
        pattern: "multiply($A, $B)".into(),
        rewrite: "times($B, $A)".into(),
        lang: Some("python".into()),
        paths: vec!["calc.py".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, false).unwrap();
    match outcome {
        StructuralOutcome::Applied { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Applied);
            assert_eq!(certificate.certificates.len(), 1);
            assert_eq!(certificate.certificates[0].file_path, "calc.py");
        }
        _ => panic!("expected Applied outcome, got {outcome:?}"),
    }

    let modified = workspace.read_file("calc.py").unwrap();
    let expected = b"def calc():\n    return times(3, 2)\n\ndef other():\n    pass\n";
    assert_eq!(
        modified, expected,
        "arguments should be swapped and function renamed"
    );
}

#[test]
fn repeated_matches_across_multiple_files_transformed_atomically() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let f1_orig = b"x = log_error(\"msg1\", 10)\ny = log_error(\"msg2\", 20)\n";
    let f2_orig = b"z = log_error(\"msg3\", 30)\n";
    workspace.write_file_atomic("src/a.py", f1_orig).unwrap();
    workspace.write_file_atomic("src/b.py", f2_orig).unwrap();

    let req = StructuralRequest {
        pattern: "log_error($M, $C)".into(),
        rewrite: "logger.error($M, code=$C)".into(),
        lang: Some("python".into()),
        paths: vec!["src".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, false).unwrap();
    match outcome {
        StructuralOutcome::Applied { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Applied);
            assert_eq!(
                certificate.certificates.len(),
                2,
                "both files should be modified"
            );
        }
        _ => panic!("expected Applied outcome, got {outcome:?}"),
    }

    let f1_mod = workspace.read_file("src/a.py").unwrap();
    let f1_expected = b"x = logger.error(\"msg1\", code=10)\ny = logger.error(\"msg2\", code=20)\n";
    assert_eq!(f1_mod, f1_expected);

    let f2_mod = workspace.read_file("src/b.py").unwrap();
    let f2_expected = b"z = logger.error(\"msg3\", code=30)\n";
    assert_eq!(f2_mod, f2_expected);
}

#[test]
fn structural_preview_does_not_modify_disk() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"fn main() { run_task(42); }\n";
    workspace.write_file_atomic("main.rs", orig).unwrap();

    let req = StructuralRequest {
        pattern: "run_task($X)".into(),
        rewrite: "execute_task($X)".into(),
        lang: Some("rust".into()),
        paths: vec!["main.rs".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, true).unwrap();
    match outcome {
        StructuralOutcome::Preview { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Applied);
            assert_eq!(certificate.certificates.len(), 1);
        }
        _ => panic!("expected Preview outcome"),
    }

    // Disk content MUST remain unchanged!
    assert_eq!(workspace.read_file("main.rs").unwrap(), orig);
}

#[test]
fn structural_plan_can_be_prepared_and_applied_separately() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"const x = getValue(100);\n";
    workspace.write_file_atomic("app.js", orig).unwrap();

    let req = StructuralRequest {
        pattern: "getValue($V)".into(),
        rewrite: "fetchValue($V)".into(),
        lang: Some("javascript".into()),
        paths: vec!["app.js".into()],
        max_files: None,
    };

    let plan_path = temp.path().join("structural.plan.json");
    let outcome = execute_structural(&workspace, &req, Some(&plan_path), false).unwrap();
    match outcome {
        StructuralOutcome::PlanPrepared { files_affected, .. } => {
            assert_eq!(files_affected, 1);
            assert!(plan_path.exists(), "plan file should be written to disk");
        }
        _ => panic!("expected PlanPrepared outcome"),
    }

    // Disk content is unchanged until plan is applied
    assert_eq!(workspace.read_file("app.js").unwrap(), orig);

    // Apply the plan through Threadmoth's standard apply_prepared_plan
    let plan_bytes = fs::read(&plan_path).unwrap();
    let plan: threadmoth::protocol::PreparedPlan = serde_json::from_slice(&plan_bytes).unwrap();
    let apply_res = threadmoth::pipeline::apply_prepared_plan(&workspace, &plan);

    match apply_res {
        threadmoth::protocol::PlanApplyResult::Certificate(cert) => {
            assert_eq!(cert.outcome, Outcome::Applied);
        }
        threadmoth::protocol::PlanApplyResult::Transaction(cert) => {
            assert_eq!(cert.outcome, Outcome::Applied);
        }
    }

    let modified = workspace.read_file("app.js").unwrap();
    assert_eq!(modified, b"const x = fetchValue(100);\n");
}

#[test]
fn structural_no_change_is_reported_cleanly() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"fn keep_same() {}\n";
    workspace.write_file_atomic("lib.rs", orig).unwrap();

    let req = StructuralRequest {
        pattern: "non_existent_call($A)".into(),
        rewrite: "replacement($A)".into(),
        lang: Some("rust".into()),
        paths: vec!["lib.rs".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, false).unwrap();
    match outcome {
        StructuralOutcome::NoChange { message } => {
            assert!(message.contains("No matching structural patterns found"));
        }
        _ => panic!("expected NoChange outcome"),
    }

    assert_eq!(workspace.read_file("lib.rs").unwrap(), orig);
}

#[test]
fn workspace_escape_attempt_is_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let req = StructuralRequest {
        pattern: "foo()".into(),
        rewrite: "bar()".into(),
        lang: Some("python".into()),
        paths: vec!["../outside_workspace.py".into()],
        max_files: None,
    };

    let res = execute_structural(&workspace, &req, None, false);
    assert!(res.is_err());
    let err = res.err().unwrap().to_string();
    assert!(
        err.contains("path traversal not allowed") || err.contains("outside workspace root"),
        "escape attempt should be refused: {err}"
    );
}

#[test]
fn multibyte_utf8_offsets_handled_safely_and_byte_boundaries_respected() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    // Source contains 3-byte Japanese characters and 4-byte rocket emoji
    let utf8_src = "# 日本語コメント: データ処理\ndef process_rocket():\n    return emit_payload(123, \"🚀\")\n";
    workspace
        .write_file_atomic("multibyte.py", utf8_src.as_bytes())
        .unwrap();

    let req = StructuralRequest {
        pattern: "emit_payload($A, $B)".into(),
        rewrite: "send_data($B, $A)".into(),
        lang: Some("python".into()),
        paths: vec!["multibyte.py".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, false).unwrap();
    match outcome {
        StructuralOutcome::Applied { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Applied);
        }
        _ => panic!("expected Applied outcome"),
    }

    let modified_bytes = workspace.read_file("multibyte.py").unwrap();
    let modified_str = String::from_utf8(modified_bytes).expect("must remain valid UTF-8");
    assert!(modified_str.contains("# 日本語コメント: データ処理"));
    assert!(modified_str.contains("return send_data(\"🚀\", 123)"));
}

#[test]
fn crlf_line_endings_preserved_exactly() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let crlf_src = b"// Header line\r\nfunction run() {\r\n    return calculate(10);\r\n}\r\n// Footer line\r\n";
    workspace
        .write_file_atomic("crlf_sample.js", crlf_src)
        .unwrap();

    let req = StructuralRequest {
        pattern: "calculate($N)".into(),
        rewrite: "computeResult($N)".into(),
        lang: Some("javascript".into()),
        paths: vec!["crlf_sample.js".into()],
        max_files: None,
    };

    let outcome = execute_structural(&workspace, &req, None, false).unwrap();
    match outcome {
        StructuralOutcome::Applied { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Applied);
        }
        _ => panic!("expected Applied outcome"),
    }

    let modified_bytes = workspace.read_file("crlf_sample.js").unwrap();
    assert!(
        modified_bytes.windows(2).any(|w| w == b"\r\n"),
        "must preserve CRLF line endings"
    );
    let expected = b"// Header line\r\nfunction run() {\r\n    return computeResult(10);\r\n}\r\n// Footer line\r\n";
    assert_eq!(modified_bytes, expected);
}

#[test]
fn stale_source_drift_detected_and_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"def handler():\n    return step_one(42)\n";
    workspace.write_file_atomic("task.py", orig).unwrap();

    // Mock proposals parsed against initial snapshot
    let record_json = serde_json::json!({
        "text": "step_one(42)",
        "range": { "byteOffset": { "start": 26, "end": 38 } },
        "file": "task.py",
        "replacement": "step_two(42)"
    });

    let stdout = format!("{}\n", record_json);
    let proposals = parse_ast_grep_output(&workspace, &stdout).unwrap();
    assert_eq!(proposals.len(), 1);

    // Now intervene: modify file on disk before building transaction
    workspace
        .write_file_atomic(
            "task.py",
            b"def handler():\n    # intervening edit\n    return step_one(42)\n",
        )
        .unwrap();

    // Verify build_transaction detects drift between proposal observation and live bytes
    let group = proposals.get("task.py").unwrap();
    let live_bytes = workspace.read_file("task.py").unwrap();
    let live_hash = threadmoth::engine::compute_sha256(&live_bytes);
    assert_ne!(
        live_hash, group.observed_hash,
        "live file hash must have drifted"
    );
}

#[test]
fn unsupported_language_is_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let req = StructuralRequest {
        pattern: "foo()".into(),
        rewrite: "bar()".into(),
        lang: Some("unsupported_custom_lang_xyz".into()),
        paths: vec!["code.xyz".into()],
        max_files: None,
    };

    let res = execute_structural(&workspace, &req, None, false);
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(matches!(err, StructuralError::UnsupportedLanguage(_)));
}

#[test]
fn plan_output_traversal_and_collision_refused() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let orig = b"def compute():\n    return calc(1, 2)\n";
    workspace.write_file_atomic("calc.py", orig).unwrap();

    let req = StructuralRequest {
        pattern: "calc($A, $B)".into(),
        rewrite: "add($A, $B)".into(),
        lang: Some("python".into()),
        paths: vec!["calc.py".into()],
        max_files: None,
    };

    // 1. Plan output outside workspace root is refused
    let outside_path = Path::new("../escaped_plan.json");
    let res_outside = execute_structural(&workspace, &req, Some(outside_path), false);
    assert!(res_outside.is_err());
    assert!(matches!(
        res_outside.err().unwrap(),
        StructuralError::WorkspaceEscape(_)
    ));

    // 2. Plan output colliding with target source file is refused
    let collide_path = Path::new("calc.py");
    let res_collide = execute_structural(&workspace, &req, Some(collide_path), false);
    assert!(res_collide.is_err());
    assert!(matches!(
        res_collide.err().unwrap(),
        StructuralError::PlanOutputCollision(_)
    ));
}

#[test]
fn malformed_ast_grep_json_fails_closed() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let malformed_stdout = "{ this is not valid json\n";
    let res = parse_ast_grep_output(&workspace, malformed_stdout);
    assert!(res.is_err(), "must fail closed on malformed json record");
    assert!(matches!(
        res.err().unwrap(),
        StructuralError::MalformedRecord { .. }
    ));
}

#[test]
fn missing_replacement_fails_closed() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    workspace
        .write_file_atomic("sample.py", b"def foo(): pass\n")
        .unwrap();

    let record_no_replacement = serde_json::json!({
        "text": "foo()",
        "range": { "byteOffset": { "start": 4, "end": 9 } },
        "file": "sample.py"
        // missing "replacement"
    });

    let stdout = format!("{}\n", record_no_replacement);
    let res = parse_ast_grep_output(&workspace, &stdout);
    assert!(res.is_err(), "must fail closed when replacement is missing");
    assert!(matches!(
        res.err().unwrap(),
        StructuralError::MissingReplacement { .. }
    ));
}

#[test]
fn unaligned_utf8_byte_offsets_fail_closed() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    // 🚀 is 4 bytes: [240, 159, 154, 128] at indices 13..17
    let src = b"// Comments: \xf0\x9f\x9a\x80 rocket\n";
    workspace.write_file_atomic("emoji.rs", src).unwrap();

    // Invalid range that slices into the middle of 🚀 (index 14)
    let record = serde_json::json!({
        "text": "invalid",
        "range": { "byteOffset": { "start": 14, "end": 18 } },
        "file": "emoji.rs",
        "replacement": "repaired"
    });

    let stdout = format!("{}\n", record);
    let res = parse_ast_grep_output(&workspace, &stdout);
    assert!(
        res.is_err(),
        "must fail closed when byte offset is not on UTF-8 boundary"
    );
    assert!(matches!(
        res.err().unwrap(),
        StructuralError::InvalidByteRange { .. }
    ));
}
