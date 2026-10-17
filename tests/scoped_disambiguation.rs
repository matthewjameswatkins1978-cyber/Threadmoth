#![forbid(unsafe_code)]

use std::fs;
use tempfile::TempDir;
use threadmoth::{
    pipeline::apply_prepared_plan,
    protocol::{Outcome, PlanApplyResult},
    source_prepare::{
        prepare, SourceAwareTextRequest, SourcePreparationResult, TextReplacementIntent,
    },
    workspace::Workspace,
};

#[test]
fn rust_function_scoped_disambiguation() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "src/main.rs";

    let original = r#"fn worker_a() {
    let timeout = 10;
    println!("timeout: {}", timeout);
}

fn worker_b() {
    let timeout = 10;
    println!("timeout: {}", timeout);
}
"#;
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join(file_path), original.as_bytes()).unwrap();

    // 1. Without scope, target is ambiguous:
    let request_unscoped = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "let timeout = 10;".into(),
            new: "let timeout = 60;".into(),
        }],
        newline_policy: Default::default(),
        within: None,
    };
    match prepare(&workspace, &request_unscoped) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(certificate.reason_code.as_deref(), Some("TARGET_AMBIGUOUS"));
            // Verify self-teaching remedy is present
            let remedies = &certificate.recovery.unwrap().remedies;
            assert!(remedies.iter().any(|r| r.kind == "use_scope_anchor"));
        }
        _ => panic!("unscoped edit should have been refused as ambiguous"),
    }

    // 2. With scope anchor identifying worker_b:
    let request_scoped = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "let timeout = 10;".into(),
            new: "let timeout = 60;".into(),
        }],
        newline_policy: Default::default(),
        within: Some("fn worker_b()".into()),
    };
    match prepare(&workspace, &request_scoped) {
        SourcePreparationResult::Prepared(prepared) => {
            assert_eq!(prepared.outcome, "PREPARED");
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            match apply {
                PlanApplyResult::Certificate(cert) => {
                    assert_eq!(cert.outcome, Outcome::Applied);
                }
                _ => panic!("expected single certificate"),
            }
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("scoped preparation refused: {:?}", certificate);
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    // worker_a remains unchanged with timeout = 10
    assert!(modified.contains("fn worker_a() {\n    let timeout = 10;"));
    // worker_b has timeout = 60
    assert!(modified.contains("fn worker_b() {\n    let timeout = 60;"));
}

#[test]
fn python_nested_scope_and_disambiguation() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "service.py";

    let original = r#"class AlphaService:
    def process(self):
        status = "pending"
        return status

class BetaService:
    def process(self):
        status = "pending"
        return status
"#;
    fs::write(temp.path().join(file_path), original.as_bytes()).unwrap();

    // Disambiguate to BetaService process
    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "status = \"pending\"".into(),
            new: "status = \"ready\"".into(),
        }],
        newline_policy: Default::default(),
        within: Some("class BetaService:".into()),
    };

    let result = prepare(&workspace, &request);
    match result {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            assert!(
                matches!(apply, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
            );
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("expected preparation to succeed: {:?}", certificate);
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    assert!(modified
        .contains("class AlphaService:\n    def process(self):\n        status = \"pending\""));
    assert!(
        modified.contains("class BetaService:\n    def process(self):\n        status = \"ready\"")
    );
}

#[test]
fn toml_section_disambiguation() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "config.toml";

    let original = r#"[server]
host = "127.0.0.1"
port = 8080

[metrics]
host = "127.0.0.1"
port = 8080
"#;
    fs::write(temp.path().join(file_path), original.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "port = 8080".into(),
            new: "port = 9090".into(),
        }],
        newline_policy: Default::default(),
        within: Some("[server]".into()),
    };

    let result = prepare(&workspace, &request);
    match result {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            assert!(
                matches!(apply, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
            );
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("preparation refused: {:?}", certificate);
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    assert!(modified.contains("[server]\nhost = \"127.0.0.1\"\nport = 9090"));
    assert!(modified.contains("[metrics]\nhost = \"127.0.0.1\"\nport = 8080"));
}

#[test]
fn markdown_heading_disambiguation_with_code_fences() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "README.md";

    let original = r#"# Project Docs

## Overview
Status: active

```bash
# This comment inside code fence must not prematurely end section
Status: active
```

## Maintenance
Status: active
"#;
    fs::write(temp.path().join(file_path), original.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "Status: active".into(),
            new: "Status: archived".into(),
        }],
        newline_policy: Default::default(),
        within: Some("## Maintenance".into()),
    };

    let result = prepare(&workspace, &request);
    match result {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            assert!(
                matches!(apply, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
            );
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("preparation refused: {:?}", certificate);
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    assert!(modified.contains("## Overview\nStatus: active"));
    assert!(modified.contains("## Maintenance\nStatus: archived"));
}

#[test]
fn missing_anchor_refuses_safely() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "test.rs";

    fs::write(temp.path().join(file_path), b"fn hello() {}\n").unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "hello".into(),
            new: "world".into(),
        }],
        newline_policy: Default::default(),
        within: Some("fn nonexistent()".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(
                certificate.reason_code.as_deref(),
                Some("CARDINALITY_MISMATCH")
            );
        }
        _ => panic!("missing anchor must be refused"),
    }
}

#[test]
fn ambiguous_anchor_refuses_safely() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "test.rs";

    let content = r#"
// tag: handler
fn foo() { let x = 1; }

// tag: handler
fn bar() { let x = 1; }
"#;
    fs::write(temp.path().join(file_path), content.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "let x = 1;".into(),
            new: "let x = 2;".into(),
        }],
        newline_policy: Default::default(),
        within: Some("// tag: handler".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(
                certificate.reason_code.as_deref(),
                Some("CARDINALITY_MISMATCH")
            );
        }
        _ => panic!("ambiguous anchor must be refused"),
    }
}

#[test]
fn target_not_in_scope_refuses() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "test.rs";

    let content = r#"
fn worker_a() {
    let unique_to_a = true;
}

fn worker_b() {
    let other = false;
}
"#;
    fs::write(temp.path().join(file_path), content.as_bytes()).unwrap();

    // Target exists in worker_a, but within specifies worker_b
    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "unique_to_a".into(),
            new: "updated_a".into(),
        }],
        newline_policy: Default::default(),
        within: Some("fn worker_b()".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(certificate.reason_code.as_deref(), Some("TARGET_NOT_FOUND"));
        }
        _ => panic!("target outside scope must be refused as TARGET_NOT_FOUND"),
    }
}

#[test]
fn header_vs_body_rule_refuses_arbitrary_body_anchor() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "test.rs";

    let content = r#"
fn worker() {
    let mid_statement = 42;
    let target = 1;
}
"#;
    fs::write(temp.path().join(file_path), content.as_bytes()).unwrap();

    // Anchor is a statement inside the function body rather than the declaration header
    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "let target = 1;".into(),
            new: "let target = 2;".into(),
        }],
        newline_policy: Default::default(),
        within: Some("let mid_statement = 42;".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(
                certificate.reason_code.as_deref(),
                Some("PRESERVATION_UNAVAILABLE")
            );
        }
        _ => panic!("body anchor must be refused by header vs body rule"),
    }
}

#[test]
fn unsupported_file_format_fails_closed_without_guessing() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "notes.txt";

    let content = r#"Section A
  timeout: 10

Section B
  timeout: 10
"#;
    fs::write(temp.path().join(file_path), content.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "timeout: 10".into(),
            new: "timeout: 20".into(),
        }],
        newline_policy: Default::default(),
        within: Some("Section A".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(
                certificate.reason_code.as_deref(),
                Some("PRESERVATION_UNAVAILABLE")
            );
            let details = format!("{:?}", certificate.refusal_reason);
            assert!(details.contains("does not support structural scope resolution"));
        }
        _ => panic!("unstructured format must fail closed without guessing indentation"),
    }
}

#[test]
fn crlf_line_endings_preserved_exactly_under_scoped_disambiguation() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "crlf_test.rs";

    let crlf_content =
        "fn first() {\r\n    let val = 1;\r\n}\r\n\r\nfn second() {\r\n    let val = 1;\r\n}\r\n";
    fs::write(temp.path().join(file_path), crlf_content.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "let val = 1;".into(),
            new: "let val = 99;".into(),
        }],
        newline_policy: Default::default(),
        within: Some("fn second()".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            match apply {
                PlanApplyResult::Certificate(cert) => {
                    assert_eq!(cert.outcome, Outcome::Applied);
                    assert!(!cert.preservation.line_endings_changed);
                }
                _ => panic!("expected certificate"),
            }
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("preparation refused: {:?}", certificate);
        }
    }

    let modified_bytes = fs::read(temp.path().join(file_path)).unwrap();
    assert!(
        !modified_bytes.contains(&b'\n')
            || modified_bytes
                .windows(2)
                .all(|w| if w[1] == b'\n' { w[0] == b'\r' } else { true })
    );
    let expected =
        "fn first() {\r\n    let val = 1;\r\n}\r\n\r\nfn second() {\r\n    let val = 99;\r\n}\r\n";
    assert_eq!(modified_bytes, expected.as_bytes());
}

#[test]
fn sequential_edits_within_same_scope() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "script.py";

    let content = r#"def setup_a():
    x = 1
    y = 2

def setup_b():
    x = 1
    y = 2
"#;
    fs::write(temp.path().join(file_path), content.as_bytes()).unwrap();

    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![
            TextReplacementIntent {
                old: "x = 1".into(),
                new: "x = 100".into(),
            },
            TextReplacementIntent {
                old: "y = 2".into(),
                new: "y = 200".into(),
            },
        ],
        newline_policy: Default::default(),
        within: Some("def setup_b():".into()),
    };

    match prepare(&workspace, &request) {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            assert!(
                matches!(apply, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
            );
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!("preparation refused: {:?}", certificate);
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    assert!(modified.contains("def setup_a():\n    x = 1\n    y = 2"));
    assert!(modified.contains("def setup_b():\n    x = 100\n    y = 200"));
}

#[test]
fn cli_edit_with_within_flag() {
    use std::process::Command;

    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("main.rs");
    let content = r#"fn worker_one() {
    let mode = "old";
}

fn worker_two() {
    let mode = "old";
}
"#;
    fs::write(&file_path, content.as_bytes()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "edit",
            "main.rs",
            "-s",
            "let mode = \"old\";",
            "-r",
            "let mode = \"new\";",
            "--within",
            "fn worker_two()",
            "--json",
        ])
        .output()
        .expect("threadmoth edit runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"outcome\": \"APPLIED\""));

    let modified = fs::read_to_string(&file_path).unwrap();
    assert!(modified.contains("fn worker_one() {\n    let mode = \"old\";"));
    assert!(modified.contains("fn worker_two() {\n    let mode = \"new\";"));
}

#[test]
fn cli_prepare_text_with_within_flag() {
    use std::process::Command;

    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("settings.toml");
    let content = r#"[dev]
port = 3000

[prod]
port = 3000
"#;
    fs::write(&file_path, content.as_bytes()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "settings.toml",
            "--edit",
            "port = 3000",
            "port = 8080",
            "--within",
            "[prod]",
            "-o",
            "plan.json",
            "--summary",
        ])
        .output()
        .expect("threadmoth prepare-text runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join("plan.json").exists());

    // Now apply the prepared plan using threadmoth apply-plan
    let apply_output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan", "plan.json"])
        .output()
        .expect("threadmoth apply-plan runs");

    assert!(
        apply_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&apply_output.stderr)
    );

    let modified = fs::read_to_string(&file_path).unwrap();
    assert!(modified.contains("[dev]\nport = 3000"));
    assert!(modified.contains("[prod]\nport = 8080"));
}

#[test]
fn mcp_edit_with_within_parameter() {
    use serde_json::json;
    use std::io::Write;
    use std::process::{Command, Stdio};

    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("app.go");
    let content = r#"package main

func HandlerA() {
	timeout := 10
	_ = timeout
}

func HandlerB() {
	timeout := 10
	_ = timeout
}
"#;
    fs::write(&file_path, content.as_bytes()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .arg("mcp")
        .current_dir(temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth mcp");

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" }
        }
    });

    let edit_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "threadmoth_edit",
            "arguments": {
                "file_path": "app.go",
                "search": "timeout := 10",
                "replace": "timeout := 99",
                "within": "func HandlerB()"
            }
        }
    });

    let mut stdin = child.stdin.take().expect("open mcp stdin");
    stdin
        .write_all(serde_json::to_string(&init_request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin
        .write_all(serde_json::to_string(&edit_request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let output = child.wait_with_output().expect("wait for mcp");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("mcp stdout utf8");
    assert!(
        stdout.contains("\"outcome\":\"APPLIED\"") || stdout.contains("\"outcome\": \"APPLIED\"")
    );

    let modified = fs::read_to_string(&file_path).unwrap();
    assert!(modified.contains("func HandlerA() {\n\ttimeout := 10"));
    assert!(modified.contains("func HandlerB() {\n\ttimeout := 99"));
}

#[test]
fn markdown_heading_in_fenced_code_block_cannot_be_structural_anchor() {
    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file_path = "guide.md";

    let original = r#"# User Guide

Here is an example snippet showing how a section is written:
```markdown
# Section A
Status: example_status
```

# Section A
Status: real_status
"#;
    fs::write(temp.path().join(file_path), original.as_bytes()).unwrap();

    // 1. Anchor "# Section A" appears both inside the fenced code block and subsequently as a real heading.
    // The fenced example must not cause TARGET_AMBIGUOUS or be selected; the real heading must be selected.
    let request = SourceAwareTextRequest {
        file_path: file_path.into(),
        edits: vec![TextReplacementIntent {
            old: "Status: real_status".into(),
            new: "Status: updated_real_status".into(),
        }],
        newline_policy: Default::default(),
        within: Some("# Section A".into()),
    };

    let result = prepare(&workspace, &request);
    match result {
        SourcePreparationResult::Prepared(prepared) => {
            let apply = apply_prepared_plan(&workspace, &prepared.plan);
            assert!(
                matches!(apply, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied)
            );
        }
        SourcePreparationResult::Refused { certificate } => {
            panic!(
                "expected scoped preparation to succeed, refused: {:?}",
                certificate
            );
        }
    }

    let modified = fs::read_to_string(temp.path().join(file_path)).unwrap();
    assert!(modified.contains("```markdown\n# Section A\nStatus: example_status\n```"));
    assert!(modified.contains("# Section A\nStatus: updated_real_status"));

    // 2. An anchor that only exists inside a fenced code block cannot be selected as a structural anchor.
    let fenced_only_original = r#"# Architecture

Here is a snippet:
```markdown
# Internal Component
config = default
```
"#;
    let fenced_file = "arch.md";
    fs::write(
        temp.path().join(fenced_file),
        fenced_only_original.as_bytes(),
    )
    .unwrap();

    let fenced_request = SourceAwareTextRequest {
        file_path: fenced_file.into(),
        edits: vec![TextReplacementIntent {
            old: "config = default".into(),
            new: "config = custom".into(),
        }],
        newline_policy: Default::default(),
        within: Some("# Internal Component".into()),
    };

    match prepare(&workspace, &fenced_request) {
        SourcePreparationResult::Refused { certificate } => {
            assert_eq!(certificate.outcome, Outcome::Refused);
            assert_eq!(
                certificate.reason_code.as_deref(),
                Some("PRESERVATION_UNAVAILABLE")
            );
        }
        _ => panic!("fenced code block heading anchor must be refused as structural anchor"),
    }
}

#[test]
fn cli_rejects_within_in_edit_document_mode() {
    use std::process::Command;

    let temp = TempDir::new().unwrap();
    let doc_path = temp.path().join("changes.tm");
    let doc_content = r#"*** File: dummy.txt
<<<<<<< SEARCH
foo
=======
bar
>>>>>>> REPLACE
"#;
    fs::write(&doc_path, doc_content.as_bytes()).unwrap();
    fs::write(temp.path().join("dummy.txt"), b"foo\n").unwrap();

    // 1. Invoking threadmoth edit with a .tm document and --within must exit 2
    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["edit", "changes.tm", "--within", "anchor"])
        .output()
        .expect("threadmoth edit runs");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("multi-file edit document mode does not accept --within"));

    // 2. Piping document via stdin with --within must also exit 2
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["edit", "-", "--within", "anchor"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth edit -");

    let mut stdin = child.stdin.take().expect("open stdin");
    stdin.write_all(doc_content.as_bytes()).unwrap();
    drop(stdin);

    let stdin_output = child.wait_with_output().expect("wait for stdin edit");
    assert_eq!(stdin_output.status.code(), Some(2));
    let stdin_stderr = String::from_utf8_lossy(&stdin_output.stderr);
    assert!(stdin_stderr.contains("multi-file edit document mode does not accept --within"));
}

#[test]
fn mcp_rejects_within_in_edit_document_mode() {
    use serde_json::json;
    use std::io::Write;
    use std::process::{Command, Stdio};

    let temp = TempDir::new().unwrap();
    let doc_content = "*** File: dummy.txt\n<<<<<<< SEARCH\nfoo\n=======\nbar\n>>>>>>> REPLACE\n";

    let mut child = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .arg("mcp")
        .current_dir(temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth mcp");

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" }
        }
    });

    let edit_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "threadmoth_edit",
            "arguments": {
                "document": doc_content,
                "within": "anchor"
            }
        }
    });

    let mut stdin = child.stdin.take().expect("open mcp stdin");
    stdin
        .write_all(serde_json::to_string(&init_request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin
        .write_all(serde_json::to_string(&edit_request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let output = child.wait_with_output().expect("wait for mcp");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("mcp stdout utf8");
    assert!(
        stdout.contains("multi-file edit document mode does not accept `within`"),
        "stdout was: {stdout}"
    );
}
