use std::fs;
use tempfile::TempDir;
use threadmoth::protocol::Outcome;
use threadmoth::toolbelt::structural::{
    discover, execute_structural, StructuralOutcome, StructuralRequest,
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
        err.contains("outside workspace root"),
        "escape attempt should be refused: {err}"
    );
}
