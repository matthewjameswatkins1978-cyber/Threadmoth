use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;
use threadmoth::engine::compute_sha256;

fn cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("threadmoth binary executes")
}

fn cli_with_stdin(dir: &std::path::Path, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("threadmoth binary executes");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input)
        .expect("write stdin");
    child.wait_with_output().expect("wait for child")
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
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("mcp stdout utf8");
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse mcp json"))
        .collect()
}

#[test]
fn parity_simple_exact_replacement() {
    let old_dir = TempDir::new().unwrap();
    let new_dir = TempDir::new().unwrap();

    let initial = b"server_port = 8080\nenable_ssl = false\nworkers = 4\n";
    fs::write(old_dir.path().join("config.toml"), initial).unwrap();
    fs::write(new_dir.path().join("config.toml"), initial).unwrap();

    // Old route: prepare-text + apply-plan
    let prep = cli(
        old_dir.path(),
        &[
            "prepare-text",
            "config.toml",
            "--edit",
            "enable_ssl = false",
            "enable_ssl = true",
            "--output",
            "plan.json",
        ],
    );
    assert_eq!(prep.status.code(), Some(0));
    let apply = cli(old_dir.path(), &["apply-plan", "--plan", "plan.json"]);
    assert_eq!(apply.status.code(), Some(0));

    // New route: threadmoth edit
    let edit = cli(
        new_dir.path(),
        &[
            "edit",
            "config.toml",
            "--search",
            "enable_ssl = false",
            "--replace",
            "enable_ssl = true",
            "--json",
        ],
    );
    assert_eq!(edit.status.code(), Some(0));

    let old_bytes = fs::read(old_dir.path().join("config.toml")).unwrap();
    let new_bytes = fs::read(new_dir.path().join("config.toml")).unwrap();
    assert_eq!(old_bytes, new_bytes);
    assert_eq!(compute_sha256(&old_bytes), compute_sha256(&new_bytes));

    let cert: Value = serde_json::from_slice(&edit.stdout).unwrap();
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["file_path"], "config.toml");
    assert_eq!(cert["effect"]["files"], 1);
    assert_eq!(cert["effect"]["matches"], 1);
}

#[test]
fn parity_multiple_ordered_edits() {
    let old_dir = TempDir::new().unwrap();
    let new_dir = TempDir::new().unwrap();

    let initial = b"let mut x = 1;\nx += 1;\nx *= 2;\n";
    fs::write(old_dir.path().join("main.rs"), initial).unwrap();
    fs::write(new_dir.path().join("main.rs"), initial).unwrap();

    let doc = "*** File: main.rs\n<<<<<<< SEARCH\nlet mut x = 1;\n=======\nlet mut x = 10;\n>>>>>>> REPLACE\n<<<<<<< SEARCH\nx += 1;\n=======\nx += 5;\n>>>>>>> REPLACE\n";
    fs::write(old_dir.path().join("changes.tm"), doc).unwrap();
    fs::write(new_dir.path().join("changes.tm"), doc).unwrap();

    // Old route
    let prep = cli(
        old_dir.path(),
        &["prepare-edits", "changes.tm", "--output", "plan.json"],
    );
    assert_eq!(prep.status.code(), Some(0));
    let apply = cli(old_dir.path(), &["apply-plan", "--plan", "plan.json"]);
    assert_eq!(apply.status.code(), Some(0));

    // New route
    let edit = cli(new_dir.path(), &["edit", "changes.tm", "--json"]);
    assert_eq!(edit.status.code(), Some(0));

    let old_bytes = fs::read(old_dir.path().join("main.rs")).unwrap();
    let new_bytes = fs::read(new_dir.path().join("main.rs")).unwrap();
    assert_eq!(old_bytes, new_bytes);
    assert_eq!(compute_sha256(&old_bytes), compute_sha256(&new_bytes));
    assert_eq!(
        String::from_utf8(new_bytes).unwrap(),
        "let mut x = 10;\nx += 5;\nx *= 2;\n"
    );
}

#[test]
fn parity_multi_file_atomic_changes() {
    let old_dir = TempDir::new().unwrap();
    let new_dir = TempDir::new().unwrap();

    for dir in [old_dir.path(), new_dir.path()] {
        fs::write(dir.join("a.txt"), b"alpha=1\n").unwrap();
        fs::write(dir.join("b.txt"), b"beta=2\n").unwrap();
        fs::write(dir.join("c.txt"), b"gamma=3\n").unwrap();
    }

    let doc = "*** File: a.txt\n<<<<<<< SEARCH\nalpha=1\n=======\nalpha=100\n>>>>>>> REPLACE\n\n*** File: b.txt\n<<<<<<< SEARCH\nbeta=2\n=======\nbeta=200\n>>>>>>> REPLACE\n\n*** File: c.txt\n<<<<<<< SEARCH\ngamma=3\n=======\ngamma=300\n>>>>>>> REPLACE\n";
    fs::write(old_dir.path().join("changes.tm"), doc).unwrap();
    fs::write(new_dir.path().join("changes.tm"), doc).unwrap();

    // Old route
    let prep = cli(
        old_dir.path(),
        &["prepare-edits", "changes.tm", "--output", "plan.json"],
    );
    assert_eq!(prep.status.code(), Some(0));
    let apply = cli(old_dir.path(), &["apply-plan", "--plan", "plan.json"]);
    assert_eq!(apply.status.code(), Some(0));

    // New route
    let edit = cli(new_dir.path(), &["edit", "changes.tm"]);
    assert_eq!(edit.status.code(), Some(0));

    for file in ["a.txt", "b.txt", "c.txt"] {
        let old_bytes = fs::read(old_dir.path().join(file)).unwrap();
        let new_bytes = fs::read(new_dir.path().join(file)).unwrap();
        assert_eq!(old_bytes, new_bytes);
    }
}

#[test]
fn parity_lf_and_crlf_preservation() {
    let temp = TempDir::new().unwrap();

    // CRLF target
    let crlf_initial = b"first_line=1\r\nsecond_line=2\r\nthird_line=3\r\n";
    fs::write(temp.path().join("crlf.txt"), crlf_initial).unwrap();

    let edit_crlf = cli(
        temp.path(),
        &[
            "edit",
            "crlf.txt",
            "--search",
            "second_line=2",
            "--replace",
            "second_line=42",
        ],
    );
    assert_eq!(edit_crlf.status.code(), Some(0));
    let crlf_res = fs::read(temp.path().join("crlf.txt")).unwrap();
    assert_eq!(
        crlf_res,
        b"first_line=1\r\nsecond_line=42\r\nthird_line=3\r\n"
    );

    // LF target
    let lf_initial = b"line_one=1\nline_two=2\nline_three=3\n";
    fs::write(temp.path().join("lf.txt"), lf_initial).unwrap();

    let edit_lf = cli(
        temp.path(),
        &[
            "edit",
            "lf.txt",
            "--search",
            "line_two=2",
            "--replace",
            "line_two=99",
        ],
    );
    assert_eq!(edit_lf.status.code(), Some(0));
    let lf_res = fs::read(temp.path().join("lf.txt")).unwrap();
    assert_eq!(lf_res, b"line_one=1\nline_two=99\nline_three=3\n");
}

#[test]
fn parity_unicode_and_special_characters() {
    let temp = TempDir::new().unwrap();

    let initial = "fn greet() -> &'static str {\n    \"Hello, 🦀 世界!\"\n}\n";
    fs::write(temp.path().join("greet.rs"), initial.as_bytes()).unwrap();

    let edit = cli(
        temp.path(),
        &[
            "edit",
            "greet.rs",
            "--search",
            "\"Hello, 🦀 世界!\"",
            "--replace",
            "\"Hello, 🚀 全宇宙! \\n \\t \\\"quotes\\\"\"",
        ],
    );
    assert_eq!(edit.status.code(), Some(0));

    let res = fs::read_to_string(temp.path().join("greet.rs")).unwrap();
    assert_eq!(
        res,
        "fn greet() -> &'static str {\n    \"Hello, 🚀 全宇宙! \\n \\t \\\"quotes\\\"\"\n}\n"
    );
}

#[test]
fn parity_missing_and_duplicate_targets() {
    let temp = TempDir::new().unwrap();
    let initial = b"duplicate = true\nduplicate = true\n";
    fs::write(temp.path().join("target.conf"), initial).unwrap();

    // Missing target: refused with exit 2, TARGET_NOT_FOUND, source unchanged
    let missing = cli(
        temp.path(),
        &[
            "edit",
            "target.conf",
            "--search",
            "nonexistent_key = 1",
            "--replace",
            "new_key = 2",
            "--json",
        ],
    );
    assert_eq!(missing.status.code(), Some(2));
    let missing_cert: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing_cert["outcome"], "REFUSED");
    assert_eq!(missing_cert["reason_code"], "TARGET_NOT_FOUND");
    assert_eq!(fs::read(temp.path().join("target.conf")).unwrap(), initial);

    // Duplicate target: refused with exit 2, TARGET_AMBIGUOUS, source unchanged
    let duplicate = cli(
        temp.path(),
        &[
            "edit",
            "target.conf",
            "--search",
            "duplicate = true",
            "--replace",
            "duplicate = false",
            "--json",
        ],
    );
    assert_eq!(duplicate.status.code(), Some(2));
    let dup_cert: Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(dup_cert["outcome"], "REFUSED");
    assert_eq!(dup_cert["reason_code"], "TARGET_AMBIGUOUS");
    assert_eq!(fs::read(temp.path().join("target.conf")).unwrap(), initial);
}

#[test]
fn parity_stale_source_changes() {
    let temp = TempDir::new().unwrap();
    let initial = b"v = 1\n";
    fs::write(temp.path().join("stale.txt"), initial).unwrap();

    // Prepare plan with --plan
    let plan = cli(
        temp.path(),
        &[
            "edit",
            "stale.txt",
            "--search",
            "v = 1",
            "--replace",
            "v = 2",
            "--plan",
            "--output",
            "plan.json",
        ],
    );
    assert_eq!(plan.status.code(), Some(0));

    // Concurrently modify file
    fs::write(temp.path().join("stale.txt"), b"v = 999\n").unwrap();

    // Apply plan should refuse stale
    let apply = cli(
        temp.path(),
        &["apply-plan", "--plan", "plan.json", "--summary"],
    );
    assert_eq!(apply.status.code(), Some(2));
    assert_eq!(
        fs::read(temp.path().join("stale.txt")).unwrap(),
        b"v = 999\n"
    );
}

#[test]
fn parity_workspace_and_path_violations() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("file.txt"), b"content").unwrap();

    let outside = cli(
        temp.path(),
        &[
            "edit",
            "../outside.txt",
            "--search",
            "old",
            "--replace",
            "new",
        ],
    );
    assert_eq!(outside.status.code(), Some(2));
}

#[test]
fn parity_non_mutating_plan_mode() {
    let temp = TempDir::new().unwrap();
    let initial = b"mode = strict\n";
    fs::write(temp.path().join("mode.txt"), initial).unwrap();

    let plan = cli(
        temp.path(),
        &[
            "edit",
            "mode.txt",
            "--search",
            "mode = strict",
            "--replace",
            "mode = permissive",
            "--plan",
            "--json",
        ],
    );
    assert_eq!(plan.status.code(), Some(0));

    // Source must be completely unchanged
    assert_eq!(fs::read(temp.path().join("mode.txt")).unwrap(), initial);

    // Output must be a valid prepared plan
    let plan_json: Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan_json["schema_version"], "1.0");
    assert_eq!(plan_json["operations"].as_array().unwrap().len(), 1);
}

#[test]
fn stdin_multi_file_editing() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("stdin_a.txt"), b"apple=red\n").unwrap();
    fs::write(temp.path().join("stdin_b.txt"), b"banana=yellow\n").unwrap();

    let doc = "*** File: stdin_a.txt\n<<<<<<< SEARCH\napple=red\n=======\napple=green\n>>>>>>> REPLACE\n\n*** File: stdin_b.txt\n<<<<<<< SEARCH\nbanana=yellow\n=======\nbanana=ripe\n>>>>>>> REPLACE\n";

    let edit = cli_with_stdin(temp.path(), &["edit", "-"], doc.as_bytes());
    assert_eq!(edit.status.code(), Some(0));

    assert_eq!(
        fs::read(temp.path().join("stdin_a.txt")).unwrap(),
        b"apple=green\n"
    );
    assert_eq!(
        fs::read(temp.path().join("stdin_b.txt")).unwrap(),
        b"banana=ripe\n"
    );
}

#[test]
fn mcp_threadmoth_edit_parity() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("mcp_single.txt"), b"color = blue\n").unwrap();
    fs::write(temp.path().join("mcp_multi.txt"), b"count = 10\n").unwrap();

    // 1. Single-file inline edit via MCP
    let response = call_mcp(
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
                        "file_path": "mcp_single.txt",
                        "search": "color = blue",
                        "replace": "color = cyan"
                    }
                }
            }),
        ],
    );
    assert_eq!(
        fs::read(temp.path().join("mcp_single.txt")).unwrap(),
        b"color = cyan\n"
    );
    let cert = &response[1]["result"]["structuredContent"];
    assert_eq!(cert["outcome"], "APPLIED");
    assert_eq!(cert["file_path"], "mcp_single.txt");

    // 2. Multi-file edit via MCP with document content
    let doc = "*** File: mcp_multi.txt\n<<<<<<< SEARCH\ncount = 10\n=======\ncount = 100\n>>>>>>> REPLACE\n";
    let response2 = call_mcp(
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
                        "document": doc
                    }
                }
            }),
        ],
    );
    assert_eq!(
        fs::read(temp.path().join("mcp_multi.txt")).unwrap(),
        b"count = 100\n"
    );
    let tx_cert = &response2[1]["result"]["structuredContent"];
    assert_eq!(tx_cert["outcome"], "APPLIED");

    // 3. Plan mode via MCP does not mutate
    let response3 = call_mcp(
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
                        "file_path": "mcp_single.txt",
                        "search": "color = cyan",
                        "replace": "color = magenta",
                        "plan": true
                    }
                }
            }),
        ],
    );
    // Must remain color = cyan
    assert_eq!(
        fs::read(temp.path().join("mcp_single.txt")).unwrap(),
        b"color = cyan\n"
    );
    let plan = &response3[1]["result"]["structuredContent"];
    assert_eq!(plan["schema_version"], "1.0");
}

#[test]
fn parity_budget_refusals() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("limits.txt"), b"x=1\n").unwrap();

    let mut doc = String::from("*** File: limits.txt\n");
    for i in 0..65 {
        doc.push_str(&format!(
            "<<<<<<< SEARCH\nx={i}\n=======\nx={}\n>>>>>>> REPLACE\n",
            i + 1
        ));
    }
    fs::write(temp.path().join("too_many.tm"), doc).unwrap();

    let edit = cli(temp.path(), &["edit", "too_many.tm", "--json"]);
    assert_eq!(edit.status.code(), Some(2));
    let val: Value = serde_json::from_slice(&edit.stdout).unwrap();
    assert_eq!(val["outcome"], "REFUSED");
    assert_eq!(val["reason_code"], "INVALID_INPUT");
    assert!(val["message"]
        .as_str()
        .unwrap()
        .contains("exceeds the per-file edit limit"));
    assert_eq!(fs::read(temp.path().join("limits.txt")).unwrap(), b"x=1\n");
}
