use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("threadmoth binary executes")
}

fn call_mcp(workspace: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .arg("mcp")
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth mcp");

    let mut stdin = child.stdin.take().expect("open mcp stdin");
    for request in requests {
        let line = serde_json::to_string(request).expect("serialize request");
        stdin.write_all(line.as_bytes()).expect("write mcp line");
        stdin.write_all(b"\n").expect("write newline");
    }
    stdin.flush().expect("flush stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for mcp");
    let text = String::from_utf8(output.stdout).expect("utf8 stdout");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse mcp json response"))
        .collect()
}

#[test]
fn cli_edit_with_search_file_and_replace_file_bypasses_quoting() {
    let temp = TempDir::new().unwrap();
    let target_path = temp.path().join("main.rs");
    let original = r#"fn main() {
    println!("Hello, $USER! `pwd` \"escaped\"");
}
"#;
    fs::write(&target_path, original).unwrap();

    let search_path = temp.path().join("search.txt");
    let search_content = r#"    println!("Hello, $USER! `pwd` \"escaped\"");"#;
    fs::write(&search_path, search_content).unwrap();

    let replace_path = temp.path().join("replace.txt");
    let replace_content = r#"    println!("Hello, World!");"#;
    fs::write(&replace_path, replace_content).unwrap();

    let out = cli(
        temp.path(),
        &[
            "edit",
            "main.rs",
            "--search-file",
            "search.txt",
            "--replace-file",
            "replace.txt",
            "--json",
        ],
    );

    assert_eq!(out.status.code(), Some(0));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["observed_cardinality"], 1);
    assert_eq!(cert["preservation"]["unrelated_bytes_changed"], false);

    let written = fs::read_to_string(&target_path).unwrap();
    assert!(written.contains(r#"println!("Hello, World!");"#));
    assert!(!written.contains("$USER"));
}

#[test]
fn cli_edit_rejects_contradictory_and_invalid_inputs() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("file.txt"), "hello world\n").unwrap();
    fs::write(temp.path().join("old.txt"), "hello").unwrap();
    fs::write(temp.path().join("new.txt"), "hi").unwrap();

    // 1. Combining --search and --search-file is rejected by CLI parser
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search",
            "hello",
            "--search-file",
            "old.txt",
            "--replace",
            "hi",
        ],
    );
    assert_ne!(out.status.code(), Some(0));

    // 2. Combining --replace and --replace-file is rejected by CLI parser
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search-file",
            "old.txt",
            "--replace",
            "hi",
            "--replace-file",
            "new.txt",
        ],
    );
    assert_ne!(out.status.code(), Some(0));

    // 3. Search without replace
    let out = cli(
        temp.path(),
        &["edit", "file.txt", "--search-file", "old.txt"],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("requires replacement input"));

    // 4. Expect count 0 is refused
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search",
            "hello",
            "--replace",
            "hi",
            "--expect-count",
            "0",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--expect-count must be at least 1"));

    // 5. Combining --expect-count and --all is rejected
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search",
            "hello",
            "--replace",
            "hi",
            "--expect-count",
            "2",
            "--all",
        ],
    );
    assert_ne!(out.status.code(), Some(0));

    // 6. Invalid newline policy is rejected
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search",
            "hello",
            "--replace",
            "hi",
            "--newline-policy",
            "bogus",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid newline policy"));

    // 7. Directory traversal with search file is refused
    let out = cli(
        temp.path(),
        &[
            "edit",
            "file.txt",
            "--search-file",
            "../outside.txt",
            "--replace",
            "hi",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("path refused"));
}

#[test]
fn cli_edit_expect_count_cardinality_enforcement() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("counts.txt");
    fs::write(&target, "foo\nbar\nfoo\nbaz\nfoo\n").unwrap();

    // Default without --expect-count or --all fails on duplicate target
    let out = cli(
        temp.path(),
        &[
            "edit",
            "counts.txt",
            "--search",
            "foo",
            "--replace",
            "qux",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "REFUSED");
    assert_eq!(cert["reason_code"], "TARGET_AMBIGUOUS");

    // --expect-count 2 fails when actual count is 3 (overflow -> TARGET_AMBIGUOUS)
    let out = cli(
        temp.path(),
        &[
            "edit",
            "counts.txt",
            "--search",
            "foo",
            "--replace",
            "qux",
            "--expect-count",
            "2",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "REFUSED");
    assert_eq!(cert["reason_code"], "TARGET_AMBIGUOUS");

    // --expect-count 4 fails when actual count is 3 (shortfall -> TARGET_NOT_FOUND)
    let out = cli(
        temp.path(),
        &[
            "edit",
            "counts.txt",
            "--search",
            "foo",
            "--replace",
            "qux",
            "--expect-count",
            "4",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "REFUSED");
    assert_eq!(cert["reason_code"], "TARGET_NOT_FOUND");

    // --expect-count 3 succeeds and replaces all 3 occurrences
    let out = cli(
        temp.path(),
        &[
            "edit",
            "counts.txt",
            "--search",
            "foo",
            "--replace",
            "qux",
            "--expect-count",
            "3",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["observed_cardinality"], 3);
    assert_eq!(
        cert["expected_cardinality"],
        serde_json::json!({"type": "exactly", "value": 3})
    );

    let written = fs::read_to_string(&target).unwrap();
    assert_eq!(written, "qux\nbar\nqux\nbaz\nqux\n");
}

#[test]
fn cli_edit_all_flag_replaces_all_occurrences() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("all.txt");
    fs::write(&target, "item item item item\n").unwrap();

    let out = cli(
        temp.path(),
        &[
            "edit",
            "all.txt",
            "--search",
            "item",
            "--replace",
            "token",
            "--all",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["observed_cardinality"], 4);

    let written = fs::read_to_string(&target).unwrap();
    assert_eq!(written, "token token token token\n");
}

#[test]
fn cli_edit_mixed_newline_exact_bytes_support() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("mixed.txt");
    // 3 LF lines, 1 CRLF line
    let content = b"line1\nline2\nline3\r\nline4\n";
    fs::write(&target, content).unwrap();

    // Default policy source refuses on mixed endings
    let out = cli(
        temp.path(),
        &[
            "edit",
            "mixed.txt",
            "--search",
            "line3",
            "--replace",
            "replaced3",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["reason_code"], "PRESERVATION_UNAVAILABLE");

    // With --newline-policy exact, exact search file with CRLF succeeds!
    let search_file = temp.path().join("old_crlf.txt");
    fs::write(&search_file, b"line3\r\n").unwrap();
    let replace_file = temp.path().join("new_crlf.txt");
    fs::write(&replace_file, b"line3_new\r\n").unwrap();

    let out = cli(
        temp.path(),
        &[
            "edit",
            "mixed.txt",
            "--search-file",
            "old_crlf.txt",
            "--replace-file",
            "new_crlf.txt",
            "--newline-policy",
            "exact",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["preservation"]["original_newline_profile"], "mixed");
    assert_eq!(cert["preservation"]["result_newline_profile"], "mixed");
    assert_eq!(cert["preservation"]["unrelated_bytes_changed"], false);
    assert_eq!(cert["preservation"]["line_endings_changed"], false);
    assert!(cert["preservation"]["newline_counts"].is_object());

    let written = fs::read(&target).unwrap();
    assert_eq!(written, b"line1\nline2\nline3_new\r\nline4\n");
}

#[test]
fn hamesh_forensic_reproduction_diagnose_near_miss_truthfulness() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("lib.rs");

    // Replicate Hamesh GUI state: 20 LF lines, but 2 CRLF lines elsewhere in file
    let mut file_content = String::new();
    file_content.push_str("fn first() {\r\n    // crlf line elsewhere\r\n}\r\n");
    for i in 0..20 {
        file_content.push_str(&format!("fn func_{i}() {{\n    let x = {i};\n}}\n"));
    }
    fs::write(&target, file_content.as_bytes()).unwrap();

    // Agent queries an LF function, but accidentally submits CRLF line endings in search
    let old_crlf_target = "fn func_5() {\r\n    let x = 5;\r\n}\r\n";
    let search_file = temp.path().join("search.txt");
    fs::write(&search_file, old_crlf_target.as_bytes()).unwrap();
    let replace_file = temp.path().join("replace.txt");
    fs::write(&replace_file, "fn func_5() {\n    let x = 50;\n}\n").unwrap();

    let out = cli(
        temp.path(),
        &[
            "edit",
            "lib.rs",
            "--search-file",
            "search.txt",
            "--replace-file",
            "replace.txt",
            "--newline-policy",
            "exact",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let cert: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cert["reason_code"], "TARGET_NOT_FOUND");

    let diff_summary = cert["diff_summary"].as_str().unwrap();
    // Must truthfully report LF vs CRLF discrepancy
    assert!(diff_summary.contains("detected LF vs CRLF line ending discrepancy"));
    // Must NEVER falsely claim the matching source region uses mixed endings
    assert!(!diff_summary.contains("matching source region uses mixed LF and CRLF"));

    // Now test a source region that genuinely HAS mixed endings
    let mixed_target = temp.path().join("mixed_region.rs");
    fs::write(
        &mixed_target,
        b"fn mixed() {\r\n    let x = 1;\n    let y = 2;\r\n}\n",
    )
    .unwrap();

    let out_mixed = cli(
        temp.path(),
        &[
            "edit",
            "mixed_region.rs",
            "--search",
            "fn mixed() {\n    let x = 1;\n    let y = 2;\n}\n",
            "--replace",
            "fn mixed() {\n    let x = 2;\n}\n",
            "--newline-policy",
            "exact",
            "--json",
        ],
    );
    assert_eq!(out_mixed.status.code(), Some(2));
    let cert_mixed: Value = serde_json::from_slice(&out_mixed.stdout).unwrap();
    assert_eq!(cert_mixed["reason_code"], "TARGET_NOT_FOUND");
    let diff_mixed = cert_mixed["diff_summary"].as_str().unwrap();
    assert!(diff_mixed.contains("matching source region uses mixed LF and CRLF"));
}

#[test]
fn changes_tm_five_line_quickstart_format() {
    let temp = TempDir::new().unwrap();
    let doc_path = temp.path().join("hello.txt");
    fs::write(&doc_path, "old text\n").unwrap();

    let changes_path = temp.path().join("changes.tm");
    let changes_content = "*** File: hello.txt\n<<<\nold text\n===\nnew text\n>>>\n";
    fs::write(&changes_path, changes_content).unwrap();

    let out = cli(temp.path(), &["edit", "changes.tm"]);
    assert_eq!(out.status.code(), Some(0));

    let written = fs::read_to_string(&doc_path).unwrap();
    assert_eq!(written, "new text\n");
}

#[test]
fn mcp_edit_with_search_file_and_expect_count() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("target.rs");
    fs::write(&target, "fn a() { test(); test(); }\n").unwrap();

    let search_file = temp.path().join("search.txt");
    fs::write(&search_file, "test()").unwrap();

    let replace_file = temp.path().join("replace.txt");
    fs::write(&replace_file, "verified()").unwrap();

    let mcp_res = call_mcp(
        temp.path(),
        &[
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "threadmoth_edit",
                    "arguments": {
                        "file_path": "target.rs",
                        "search_file": "search.txt",
                        "replace_file": "replace.txt",
                        "expect_count": 2
                    }
                }
            }),
        ],
    );

    assert_eq!(mcp_res.len(), 2);
    let cert = &mcp_res[1]["result"]["structuredContent"];
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["observed_cardinality"], 2);

    let written = fs::read_to_string(&target).unwrap();
    assert_eq!(written, "fn a() { verified(); verified(); }\n");
}
