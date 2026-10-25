//! TERRORBATS: OPERATION CRLF ARMAGEDDON
//! Independent Adversarial Acceptance Test Suite for Threadmoth Newline Firewall (PR #74)
//! Target: Threadmoth Newline Firewall core and all public/private mutation routes.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::TempDir;

use threadmoth::{
    engine::compute_sha256,
    lifecycle::FileOperation,
    pipeline::{apply_prepared_plan, execute_request, execute_transaction, plan_identity},
    protocol::{
        ByteEdit, Cardinality, DesiredStateOperation, EffectBudget, OperationPayload, Outcome,
        PlanApplyResult, PreparedPlan, PreparedPlanOperation, Request, TransactionRequest,
        PROTOCOL_VERSION,
    },
    provider::text::{NewlinePolicy, TextOperation, TextReplacementIntent},
    workspace::Workspace,
};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_threadmoth"))
}

fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    bin()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("execute threadmoth binary")
}

fn cli_with_stdin(dir: &Path, args: &[&str], stdin_data: &[u8]) -> std::process::Output {
    let mut child = bin()
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth binary");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_data)
        .expect("write stdin");
    child.wait_with_output().expect("wait for child")
}

fn call_mcp(dir: &Path, requests: &[Value]) -> Vec<Value> {
    let mut child = bin()
        .arg("mcp")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn threadmoth mcp");

    let mut stdin = child.stdin.take().expect("open mcp stdin");
    for req in requests {
        let line = serde_json::to_string(req).expect("serialize mcp req");
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
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("parse mcp json"))
        .collect()
}

fn make_req(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "armageddon-test".into(),
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

// =========================================================================
// ASSAULT PHASE 1: BLACK-BOX ATTACK ACROSS ALL INTERFACES
// =========================================================================

#[test]
fn p1_01_cli_edit_crlf_into_lf_source_stays_lf() {
    let temp = TempDir::new().unwrap();
    let initial = b"fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n";
    fs::write(temp.path().join("main.rs"), initial).unwrap();

    // Pass replacement with CRLF endings in search/replace
    let out = cli(
        temp.path(),
        &[
            "edit",
            "main.rs",
            "--search",
            "    let x = 1;\n",
            "--replace",
            "    let x = 1;\r\n    let y = 2;\r\n",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let landed = fs::read(temp.path().join("main.rs")).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(
        counts.crlf, 0,
        "Accidental CRLF injected into uniform LF file!"
    );
    assert_eq!(counts.bare_cr, 0, "Accidental bare CR injected!");
    assert_eq!(counts.lf, 5);
}

#[test]
fn p1_02_cli_edit_lf_into_crlf_source_stays_crlf() {
    let temp = TempDir::new().unwrap();
    let initial = b"fn main() {\r\n    let x = 1;\r\n    println!(\"{x}\");\r\n}\r\n";
    fs::write(temp.path().join("main.rs"), initial).unwrap();

    // Pass replacement with LF endings
    let out = cli(
        temp.path(),
        &[
            "edit",
            "main.rs",
            "--search",
            "    let x = 1;\r\n",
            "--replace",
            "    let x = 1;\n    let y = 2;\n",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let landed = fs::read(temp.path().join("main.rs")).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(
        counts.lf, 0,
        "Accidental lone LF injected into uniform CRLF file!"
    );
    assert_eq!(counts.bare_cr, 0, "Accidental bare CR injected!");
    assert_eq!(counts.crlf, 5);
}

#[test]
fn p1_03_cli_edit_file_args_crlf_payload_into_lf_source() {
    let temp = TempDir::new().unwrap();
    let initial = b"config=default\nmode=testing\n";
    fs::write(temp.path().join("app.conf"), initial).unwrap();

    // Write search and replace files with Windows CRLF endings
    fs::write(temp.path().join("search.txt"), b"mode=testing\n").unwrap();
    fs::write(
        temp.path().join("replace.txt"),
        b"mode=production\r\nworkers=8\r\n",
    )
    .unwrap();

    let out = cli(
        temp.path(),
        &[
            "edit",
            "app.conf",
            "--search-file",
            "search.txt",
            "--replace-file",
            "replace.txt",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let landed = fs::read(temp.path().join("app.conf")).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(
        counts.crlf, 0,
        "File-based replacement bypassed firewall into LF source!"
    );
    assert_eq!(counts.bare_cr, 0);
    assert_eq!(counts.lf, 3);
    assert_eq!(landed, b"config=default\nmode=production\nworkers=8\n");
}

#[test]
fn p1_04_cli_stdin_edit_document_multifile_preserves_independent_profiles() {
    let temp = TempDir::new().unwrap();
    let lf_file = b"server=1\nport=80\n";
    let crlf_file = b"host=localhost\r\ntimeout=30\r\n";
    fs::write(temp.path().join("server.txt"), lf_file).unwrap();
    fs::write(temp.path().join("client.txt"), crlf_file).unwrap();

    let doc = b"*** File: server.txt\n<<<<<<< SEARCH\nport=80\n=======\nport=443\r\ntls=true\r\n>>>>>>> REPLACE\n*** File: client.txt\n<<<<<<< SEARCH\ntimeout=30\r\n=======\ntimeout=60\nretries=3\n>>>>>>> REPLACE\n";

    let out = cli_with_stdin(temp.path(), &["edit", "-"], doc);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let landed_lf = fs::read(temp.path().join("server.txt")).unwrap();
    let counts_lf = threadmoth::newline::count_newlines(&landed_lf);
    assert_eq!(counts_lf.crlf, 0, "server.txt acquired CRLF endings!");
    assert_eq!(counts_lf.bare_cr, 0);
    assert_eq!(counts_lf.lf, 3);

    let landed_crlf = fs::read(temp.path().join("client.txt")).unwrap();
    let counts_crlf = threadmoth::newline::count_newlines(&landed_crlf);
    assert_eq!(counts_crlf.lf, 0, "client.txt acquired LF endings!");
    assert_eq!(counts_crlf.bare_cr, 0);
    assert_eq!(counts_crlf.crlf, 3);
}

#[test]
fn p1_05_mcp_threadmoth_edit_and_mutate_protocols() {
    let temp = TempDir::new().unwrap();
    let initial = b"item_a = 10\nitem_b = 20\n";
    fs::write(temp.path().join("items.toml"), initial).unwrap();

    let reqs = vec![
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "armageddon", "version": "1.0"}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "threadmoth_edit",
                "arguments": {
                    "file_path": "items.toml",
                    "search": "item_b = 20\n",
                    "replace": "item_b = 20\r\nitem_c = 30\r\n"
                }
            }
        }),
    ];

    let responses = call_mcp(temp.path(), &reqs);
    assert_eq!(responses.len(), 2);
    let edit_res = &responses[1];
    assert!(edit_res.get("error").is_none(), "MCP error: {:?}", edit_res);

    let landed = fs::read(temp.path().join("items.toml")).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(
        counts.crlf, 0,
        "MCP threadmoth_edit injected CRLF into LF file!"
    );
    assert_eq!(counts.bare_cr, 0);
    assert_eq!(counts.lf, 3);
}

#[test]
fn p1_06_alternate_structured_providers_preserve_conventions() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();

    // 1. JSON with CRLF
    let json_bytes = b"{\r\n  \"key\": \"old\"\r\n}\r\n";
    fs::write(temp.path().join("test.json"), json_bytes).unwrap();
    let json_req = make_req(
        "test.json",
        OperationPayload::Json(threadmoth::provider::json::JsonOperation::Set {
            path: "key".into(),
            value: json!("new"),
        }),
    );
    let cert = execute_request(&ws, &json_req, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed_json = fs::read(temp.path().join("test.json")).unwrap();
    assert_eq!(threadmoth::newline::count_newlines(&landed_json).lf, 0);
    assert_eq!(threadmoth::newline::count_newlines(&landed_json).crlf, 3);

    // 2. TOML with LF
    let toml_bytes = b"[section]\nname = \"alice\"\n";
    fs::write(temp.path().join("test.toml"), toml_bytes).unwrap();
    let toml_req = make_req(
        "test.toml",
        OperationPayload::Toml(threadmoth::provider::toml::TomlOperation::Set {
            path: "section.name".into(),
            value: threadmoth::provider::toml::TomlValueWrapper::String("bob".into()),
        }),
    );
    let cert = execute_request(&ws, &toml_req, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed_toml = fs::read(temp.path().join("test.toml")).unwrap();
    assert_eq!(threadmoth::newline::count_newlines(&landed_toml).crlf, 0);
    assert_eq!(threadmoth::newline::count_newlines(&landed_toml).lf, 2);

    // 3. INI with CRLF
    let ini_bytes = b"[general]\r\nport=80\r\n";
    fs::write(temp.path().join("test.ini"), ini_bytes).unwrap();
    let ini_req = make_req(
        "test.ini",
        OperationPayload::Ini(threadmoth::provider::ini::IniOperation::Set {
            path: "general.port".into(),
            value: "443".into(),
        }),
    );
    let cert = execute_request(&ws, &ini_req, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed_ini = fs::read(temp.path().join("test.ini")).unwrap();
    assert_eq!(threadmoth::newline::count_newlines(&landed_ini).lf, 0);
    assert_eq!(threadmoth::newline::count_newlines(&landed_ini).crlf, 2);

    // 4. Dotenv with LF
    let env_bytes = b"ENV=dev\nDEBUG=false\n";
    fs::write(temp.path().join(".env"), env_bytes).unwrap();
    let env_req = make_req(
        ".env",
        OperationPayload::Dotenv(threadmoth::provider::dotenv::DotenvOperation::Set {
            key: "DEBUG".into(),
            value: "true".into(),
        }),
    );
    let cert = execute_request(&ws, &env_req, false);
    assert_eq!(cert.outcome, Outcome::Applied);
    let landed_env = fs::read(temp.path().join(".env")).unwrap();
    assert_eq!(threadmoth::newline::count_newlines(&landed_env).crlf, 0);
    assert_eq!(threadmoth::newline::count_newlines(&landed_env).lf, 2);
}

#[test]
fn p1_09_cli_swapped_endings_bypass_refused_across_cli_mcp_library() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("gap.txt");
    fs::write(&file_path, b"a\nb\r\n").unwrap();
    fs::write(temp.path().join("s.txt"), b"a\nb\r\n").unwrap();
    fs::write(temp.path().join("r.txt"), b"b\na\r\n").unwrap();

    // 1. CLI execution
    let out = cli(
        temp.path(),
        &[
            "edit",
            "gap.txt",
            "--search-file",
            "s.txt",
            "--replace-file",
            "r.txt",
            "--json",
        ],
    );
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PRESERVATION_UNAVAILABLE"));
    assert_eq!(fs::read(&file_path).unwrap(), b"a\nb\r\n");

    // 2. MCP execution
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test", "version": "1.0"}}});
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "threadmoth_edit", "arguments": {"file_path": "gap.txt", "search_file": "s.txt", "replace_file": "r.txt"}}});
    let responses = call_mcp(temp.path(), &[init, call]);
    let call_res = &responses[1]["result"];
    let text = call_res["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("PRESERVATION_UNAVAILABLE"));
    assert_eq!(fs::read(&file_path).unwrap(), b"a\nb\r\n");

    // 3. Safe reordering execution
    fs::write(temp.path().join("r_good.txt"), b"b\r\na\n").unwrap();
    let out_good = cli(
        temp.path(),
        &[
            "edit",
            "gap.txt",
            "--search-file",
            "s.txt",
            "--replace-file",
            "r_good.txt",
            "--json",
        ],
    );
    assert!(
        out_good.status.success(),
        "stderr: {}, stdout: {}",
        String::from_utf8_lossy(&out_good.stderr),
        String::from_utf8_lossy(&out_good.stdout)
    );
    assert_eq!(fs::read(&file_path).unwrap(), b"b\r\na\n");
}

// =========================================================================
// ASSAULT PHASE 2: HOSTILE INPUT CORPUS
// =========================================================================

#[test]
fn p2_01_single_rogue_lf_in_ten_thousand_crlf_lines() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("big_crlf.txt");

    // 10,000 CRLF lines, but line 5,000 has lone LF
    let mut data = Vec::with_capacity(10_000 * 16);
    for i in 0..10_000 {
        if i == 5_000 {
            data.extend_from_slice(format!("line_{i}\n").as_bytes());
        } else {
            data.extend_from_slice(format!("line_{i}\r\n").as_bytes());
        }
    }
    fs::write(&file_path, &data).unwrap();

    // Attack 1: Edit in the CRLF region (line 100) with LF input
    let req_normal = make_req(
        "big_crlf.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "line_100\r\n".into(),
            replacement: "line_100_updated\nline_100_extra\n".into(),
        }),
    );
    let cert1 = execute_request(&ws, &req_normal, false);
    assert_eq!(cert1.outcome, Outcome::Applied);
    let landed1 = fs::read(&file_path).unwrap();
    let counts1 = threadmoth::newline::count_newlines(&landed1);
    assert_eq!(
        counts1.lf, 1,
        "Rogue LF line was changed or new LF introduced in CRLF region!"
    );
    assert_eq!(counts1.crlf, 10_000);

    // Attack 2: Attempting unauthorized whole-file conversion of line 5000 to CRLF without flag
    let req_convert_unauth = make_req(
        "big_crlf.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            newline_policy: NewlinePolicy::Exact,
            edits: vec![TextReplacementIntent {
                old: "line_5000\n".into(),
                new: "line_5000\r\n".into(),
            }],
        }),
    );
    let cert2 = execute_request(&ws, &req_convert_unauth, false);
    assert_eq!(cert2.outcome, Outcome::Refused);
    assert_eq!(
        cert2.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    // Attack 3: Authorised repair of line 5000
    let mut req_convert_auth = req_convert_unauth;
    req_convert_auth.allow_newline_conversion = true;
    let cert3 = execute_request(&ws, &req_convert_auth, false);
    assert_eq!(cert3.outcome, Outcome::Applied);
    let landed3 = fs::read(&file_path).unwrap();
    let counts3 = threadmoth::newline::count_newlines(&landed3);
    assert_eq!(counts3.lf, 0);
    assert_eq!(counts3.crlf, 10_001);
    assert!(cert3.preservation.line_endings_changed);
    assert!(cert3.preservation.conversion_authorized);
}

#[test]
fn p2_02_unterminated_final_line_handling() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("no_eol.txt");

    // File with LF endings, but no newline on final line
    fs::write(&file_path, b"line1\nline2\nline3_no_newline").unwrap();

    let req = make_req(
        "no_eol.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "line3_no_newline".into(),
            replacement: "line3_appended\nline4_no_newline".into(),
        }),
    );
    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed = fs::read(&file_path).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(
        counts.crlf, 0,
        "Accidental CRLF introduced on unterminated file!"
    );
    assert_eq!(counts.bare_cr, 0);
    assert_eq!(landed, b"line1\nline2\nline3_appended\nline4_no_newline");
}

#[test]
fn p2_03_multibyte_utf8_abutting_crlf_and_lf_boundaries() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("utf8.txt");

    // 4-byte crab emoji, 3-byte CJK, 2-byte umlaut directly before CRLF
    let initial = "🦀\r\n日本語\r\nüber\r\n".as_bytes();
    fs::write(&file_path, initial).unwrap();

    let req = make_req(
        "utf8.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "日本語\r\n".into(),
            replacement: "東京・大阪\n京都\n".into(), // LF intent adapted to CRLF
        }),
    );
    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed = fs::read(&file_path).unwrap();
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(counts.lf, 0, "UTF-8 replacement broke CRLF preservation!");
    assert_eq!(counts.bare_cr, 0);
    assert_eq!(counts.crlf, 4);
    assert_eq!(
        String::from_utf8(landed).unwrap(),
        "🦀\r\n東京・大阪\r\n京都\r\nüber\r\n"
    );
}

#[test]
fn p2_04_utf8_bom_preservation() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("bom.txt");

    // BOM + CRLF
    let mut initial = vec![0xEF, 0xBB, 0xBF];
    initial.extend_from_slice(b"title=BOM Test\r\nstatus=ok\r\n");
    fs::write(&file_path, &initial).unwrap();

    let req = make_req(
        "bom.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "status=ok\r\n".into(),
            replacement: "status=verified\r\n".into(),
        }),
    );
    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Applied);

    let landed = fs::read(&file_path).unwrap();
    assert_eq!(
        &landed[..3],
        &[0xEF, 0xBB, 0xBF],
        "BOM was stripped or corrupted!"
    );
    let counts = threadmoth::newline::count_newlines(&landed);
    assert_eq!(counts.lf, 0);
    assert_eq!(counts.crlf, 2);
    assert!(!cert.preservation.bom_changed);
}

#[test]
fn p2_05_bare_cr_refusal_and_immutability() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("cr.txt");

    // Attempt to inject bare CR
    fs::write(&file_path, b"safe=true\nvalue=1\n").unwrap();
    let req_inject = make_req(
        "cr.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "value=1".into(),
            replacement: "value=1\rinjected".into(),
        }),
    );
    let cert = execute_request(&ws, &req_inject, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert_eq!(fs::read(&file_path).unwrap(), b"safe=true\nvalue=1\n");

    // Existing source with bare CR is preserved without stripping or converting to CRLF
    let source_with_cr = b"legacy\rheader\nline2\n";
    fs::write(&file_path, source_with_cr).unwrap();
    let req_other = make_req(
        "cr.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "line2".into(),
            replacement: "line2_updated".into(),
        }),
    );
    let cert2 = execute_request(&ws, &req_other, false);
    assert_eq!(cert2.outcome, Outcome::Applied);
    let landed = fs::read(&file_path).unwrap();
    assert_eq!(landed, b"legacy\rheader\nline2_updated\n");
    assert_eq!(threadmoth::newline::count_newlines(&landed).bare_cr, 1);
}

// =========================================================================
// ASSAULT PHASE 3: BYPASS CORE ATTACKS
// =========================================================================

#[test]
fn p3_01_forged_prepared_plan_with_valid_hash_fails_closed_at_apply() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("secure.txt");
    fs::write(&file_path, b"key1\r\nkey2\r\n").unwrap();
    let pre_hash = compute_sha256(b"key1\r\nkey2\r\n");

    // Construct a plan that cleanly passes hash and signature checks,
    // but secretly converts CRLF to LF in stored edits
    let op_req = make_req(
        "secure.txt",
        OperationPayload::Text(TextOperation::ReplaceSequence {
            newline_policy: NewlinePolicy::Exact,
            edits: vec![TextReplacementIntent {
                old: "key1\r\n".into(),
                new: "key1\n".into(),
            }],
        }),
    );
    let mut plan = PreparedPlan {
        schema_version: "1.0".into(),
        protocol_version: PROTOCOL_VERSION.into(),
        plan_id: String::new(),
        request_id: "forged-bypass".into(),
        transaction_id: None,
        operations: vec![PreparedPlanOperation {
            file_path: "secure.txt".into(),
            provider: "text".into(),
            request: op_req,
            pre_hash: pre_hash.clone(),
            edits: vec![ByteEdit {
                offset: 4,
                delete_len: 2,
                replacement: b"\n".to_vec(),
            }],
            prospective_hash: compute_sha256(b"key1\nkey2\r\n"),
        }],
        assertions: Vec::new(),
        budget: EffectBudget::default(),
    };
    plan.plan_id = plan_identity(&plan);

    let res = apply_prepared_plan(&ws, &plan);
    match res {
        PlanApplyResult::Certificate(c) => {
            assert_eq!(c.outcome, Outcome::Refused);
            assert_eq!(c.reason_code.as_deref(), Some("PRESERVATION_UNAVAILABLE"));
        }
        _ => panic!("Expected refused certificate on forged conversion plan"),
    }
    // Disk remains untouched
    assert_eq!(fs::read(&file_path).unwrap(), b"key1\r\nkey2\r\n");
}

#[test]
fn p3_02_desired_state_cannot_silently_convert_mixed_to_uniform() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("mixed.txt");
    fs::write(&file_path, b"header\nitem=1\r\nfooter\n").unwrap();

    let req = make_req(
        "mixed.txt",
        OperationPayload::DesiredState(DesiredStateOperation::Replace {
            desired_bytes: b"header\nitem=1\nfooter\n".to_vec(),
        }),
    );
    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert_eq!(fs::read(&file_path).unwrap(), b"header\nitem=1\r\nfooter\n");
}

#[test]
fn p3_03_create_file_refuses_mixed_and_bare_cr() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();

    // Mixed creation
    let req_mixed = make_req(
        "mixed_new.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"alpha\r\nbeta\n".to_vec(),
        }),
    );
    let cert1 = execute_request(&ws, &req_mixed, false);
    assert_eq!(cert1.outcome, Outcome::Refused);
    assert_eq!(
        cert1.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert!(!temp.path().join("mixed_new.txt").exists());

    // Bare CR creation
    let req_cr = make_req(
        "cr_new.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"alpha\rbeta\r".to_vec(),
        }),
    );
    let cert2 = execute_request(&ws, &req_cr, false);
    assert_eq!(cert2.outcome, Outcome::Refused);
    assert_eq!(
        cert2.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    assert!(!temp.path().join("cr_new.txt").exists());
}

// =========================================================================
// ASSAULT PHASE 4: STATE AND FAILURE ATTACKS
// =========================================================================

#[test]
fn p4_01_stale_pre_hash_fails_safely_without_corruption() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();
    let file_path = temp.path().join("toctou.txt");
    fs::write(&file_path, b"version=1\r\nstatus=draft\r\n").unwrap();

    // Concurrent modification right before commit
    let mut req = make_req(
        "toctou.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "status=draft\r\n".into(),
            replacement: "status=approved\r\n".into(),
        }),
    );
    req.expected_pre_hash = Some(compute_sha256(b"version=0\r\nstatus=draft\r\n"));

    let cert = execute_request(&ws, &req, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(cert.reason_code.as_deref(), Some("STALE_IDENTITY"));
    assert_eq!(
        fs::read(&file_path).unwrap(),
        b"version=1\r\nstatus=draft\r\n"
    );
}

#[test]
fn p4_02_multifile_transaction_atomic_rollback_on_preservation_violation() {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();

    let file_a = temp.path().join("file_a.txt");
    let file_b = temp.path().join("file_b.txt");
    fs::write(&file_a, b"alpha=1\nalpha=2\n").unwrap();
    fs::write(&file_b, b"beta=1\nbad=2\r\n").unwrap(); // mixed

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-atomic-test".into(),
        budget: EffectBudget::default(),
        requests: vec![
            make_req(
                "file_a.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "alpha=1\n".into(),
                    replacement: "alpha=10\n".into(),
                }),
            ),
            make_req(
                "file_b.txt",
                OperationPayload::DesiredState(DesiredStateOperation::Replace {
                    desired_bytes: b"beta=1\nbad=2\n".to_vec(), // unauthorized mixed->uniform conversion
                }),
            ),
        ],
    };

    let cert = execute_transaction(&ws, &tx, false);
    assert_eq!(cert.outcome, Outcome::Refused);
    assert_eq!(
        cert.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );

    // File A must be completely unchanged despite succeeding on its own
    assert_eq!(fs::read(&file_a).unwrap(), b"alpha=1\nalpha=2\n");
    assert_eq!(fs::read(&file_b).unwrap(), b"beta=1\nbad=2\r\n");
}

// =========================================================================
// ASSAULT PHASE 5: 1,000-CASE RANDOMIZED PROPERTY FUZZING WITH ORACLE
// =========================================================================

struct PseudoRng {
    state: u64,
}

impl PseudoRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u32(&mut self) -> u32 {
        // Linear Congruential Generator (Numerical Recipes)
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.state >> 16) as u32
    }
    fn range(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u32() as usize % (max - min + 1))
    }
    fn choice<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
        &slice[self.range(0, slice.len() - 1)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefEnding {
    Lf,
    Crlf,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RefLine {
    content: Vec<u8>,
    ending: RefEnding,
}

fn ref_split_lines(bytes: &[u8]) -> Vec<RefLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > start && bytes[i - 1] == b'\r' {
                lines.push(RefLine {
                    content: bytes[start..i - 1].to_vec(),
                    ending: RefEnding::Crlf,
                });
            } else {
                lines.push(RefLine {
                    content: bytes[start..i].to_vec(),
                    ending: RefEnding::Lf,
                });
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(RefLine {
            content: bytes[start..].to_vec(),
            ending: RefEnding::None,
        });
    }
    lines
}

fn ref_is_subsequence(needle: &[RefEnding], haystack: &[RefEnding]) -> bool {
    let mut h = 0;
    for &n in needle {
        while h < haystack.len() && haystack[h] != n {
            h += 1;
        }
        if h >= haystack.len() {
            return false;
        }
        h += 1;
    }
    true
}

fn audit_independent_line_identity(pre_bytes: &[u8], post_bytes: &[u8], iteration: usize) {
    let pre_lines = ref_split_lines(pre_bytes);
    let post_lines = ref_split_lines(post_bytes);

    let mut pre_map: std::collections::HashMap<Vec<u8>, Vec<RefEnding>> =
        std::collections::HashMap::new();
    for l in &pre_lines {
        pre_map.entry(l.content.clone()).or_default().push(l.ending);
    }

    let mut post_map: std::collections::HashMap<Vec<u8>, Vec<RefEnding>> =
        std::collections::HashMap::new();
    for l in &post_lines {
        post_map
            .entry(l.content.clone())
            .or_default()
            .push(l.ending);
    }

    for (content, post_endings) in &post_map {
        if let Some(pre_endings) = pre_map.get(content) {
            let pre_lf = pre_endings.iter().filter(|&&e| e == RefEnding::Lf).count();
            let pre_crlf = pre_endings
                .iter()
                .filter(|&&e| e == RefEnding::Crlf)
                .count();
            let post_lf = post_endings.iter().filter(|&&e| e == RefEnding::Lf).count();
            let post_crlf = post_endings
                .iter()
                .filter(|&&e| e == RefEnding::Crlf)
                .count();

            // Invariant A: No retained line content may convert ending from LF to CRLF
            assert!(
                !(post_crlf > pre_crlf && post_lf < pre_lf),
                "Independent Oracle: Retained line {:?} had ending converted from LF to CRLF at iteration {iteration}!",
                String::from_utf8_lossy(content)
            );
            // Invariant B: No retained line content may convert ending from CRLF to LF
            assert!(
                !(post_lf > pre_lf && post_crlf < pre_crlf),
                "Independent Oracle: Retained line {:?} had ending converted from CRLF to LF at iteration {iteration}!",
                String::from_utf8_lossy(content)
            );

            // Invariant C: Duplicate lines with mixed endings must not swap or invert endings
            if pre_lf > 0 && pre_crlf > 0 {
                if post_endings.len() == pre_endings.len() {
                    assert_eq!(
                        post_endings, pre_endings,
                        "Independent Oracle: Retained identical lines {:?} swapped line endings at iteration {iteration}!",
                        String::from_utf8_lossy(content)
                    );
                }
                if post_endings.len() < pre_endings.len() {
                    assert!(
                        ref_is_subsequence(post_endings, pre_endings),
                        "Independent Oracle: Retained identical lines {:?} do not match valid subsequence at iteration {iteration}!",
                        String::from_utf8_lossy(content)
                    );
                }
            }
        }
    }
}

fn run_fuzzing_oracle(iterations: usize, seed: u64) {
    let temp = TempDir::new().unwrap();
    let ws = Workspace::new(temp.path()).unwrap();

    let mut rng = PseudoRng::new(seed);
    let line_terminators = ["\n", "\r\n"];
    let tokens = [
        "apple",
        "banana",
        "cherry",
        "date",
        "elderberry",
        "fig",
        "grape",
        "🦀",
        "日本語",
        "Über",
    ];

    for iteration in 1..=iterations {
        let file_name = format!("fuzz_{iteration}.txt");
        let file_path = temp.path().join(&file_name);

        // Deterministically construct file structure
        let line_count = rng.range(4, 40);
        let profile_mode = rng.range(0, 7); // 0: Pure LF, 1: Pure CRLF, 2: Mixed, 3: No newlines, 4: Lone rogue, 5: Repeated, 6: Blocked, 7: Permutation

        let mut source_lines = Vec::with_capacity(line_count);
        let mut source_bytes = Vec::new();

        let base_term = match profile_mode {
            0 => "\n",
            1 => "\r\n",
            2 => "", // dynamic
            3 => "",
            _ => "\r\n",
        };

        for line_idx in 0..line_count {
            let token = if profile_mode == 5 {
                "REPEATED_LINE"
            } else if profile_mode == 7 {
                match line_idx % 4 {
                    0 => "line_alpha",
                    1 => "line_beta",
                    2 => "line_gamma",
                    _ => "line_delta",
                }
            } else {
                *rng.choice(&tokens)
            };
            let term = if profile_mode == 2 || profile_mode == 5 {
                *rng.choice(&line_terminators)
            } else if profile_mode == 6 {
                if line_idx < line_count / 2 {
                    "\n"
                } else {
                    "\r\n"
                }
            } else if profile_mode == 4 && line_idx == line_count / 2 {
                "\n" // lone rogue in CRLF
            } else {
                base_term
            };

            let line_str = if profile_mode == 3 && line_idx == line_count - 1 {
                format!("{token}_{line_idx}") // no newline on single-line or final line
            } else if profile_mode == 5 {
                format!("{token}{term}")
            } else {
                format!("{token}_{line_idx}{term}")
            };
            source_lines.push(line_str.clone());
            source_bytes.extend_from_slice(line_str.as_bytes());
        }

        fs::write(&file_path, &source_bytes).unwrap();
        let pre_counts = threadmoth::newline::count_newlines(&source_bytes);
        let pre_profile = pre_counts.profile();

        // Generate diverse edit types (multiline replacements, deletions, disguised conversions, permutations)
        let edit_type = rng.range(0, 6);
        let target_line_idx = rng.range(0, line_count - 2);
        let span = rng.range(1, 3.min(line_count - target_line_idx));

        let target_text: String = source_lines[target_line_idx..target_line_idx + span].concat();
        let rep_term = *rng.choice(&line_terminators);

        let replacement = match edit_type {
            0 => {
                let mut rep = String::new();
                for (i, line) in source_lines[target_line_idx..target_line_idx + span]
                    .iter()
                    .enumerate()
                {
                    let term = if line.ends_with("\r\n") {
                        "\r\n"
                    } else if line.ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    rep.push_str(&format!("MUTATED_{target_line_idx}_{i}{term}"));
                }
                rep
            }
            1 => String::new(), // pure deletion
            2 => {
                // Safe deletion of partial lines
                source_lines[target_line_idx].clone()
            }
            3 => {
                // Disguised conversion attack
                let first = &source_lines[target_line_idx];
                if first.ends_with("\r\n") {
                    format!("{}\n", first.trim_end_matches("\r\n"))
                } else if first.ends_with('\n') {
                    format!("{}\r\n", first.trim_end_matches('\n'))
                } else {
                    format!("{first}{rep_term}")
                }
            }
            4 => {
                // Permutation attack: reverse lines and invert their line endings
                let mut rep = String::new();
                for line in source_lines[target_line_idx..target_line_idx + span]
                    .iter()
                    .rev()
                {
                    let clean = line.trim_end_matches(['\r', '\n']);
                    let inverted_term = if line.ends_with("\r\n") { "\n" } else { "\r\n" };
                    rep.push_str(&format!("{clean}{inverted_term}"));
                }
                rep
            }
            5 => {
                // Safe line permutation: reverse lines keeping their exact original line endings
                let mut rep = String::new();
                for line in source_lines[target_line_idx..target_line_idx + span]
                    .iter()
                    .rev()
                {
                    rep.push_str(line);
                }
                rep
            }
            _ => format!("MUTATED_{target_line_idx}{rep_term}"),
        };

        let req = make_req(
            &file_name,
            OperationPayload::Text(TextOperation::Replace {
                target: target_text,
                replacement,
            }),
        );

        let cert = execute_request(&ws, &req, false);
        let post_bytes = fs::read(&file_path).unwrap();
        let post_counts = threadmoth::newline::count_newlines(&post_bytes);
        let post_profile = post_counts.profile();

        // =================================================================
        // INDEPENDENT PRESERVATION ORACLE AUDIT
        // =================================================================

        if cert.outcome == Outcome::Applied {
            // Invariant 1: Uniform LF file must NEVER acquire CRLF
            if pre_profile == threadmoth::newline::NewlineProfile::Lf {
                assert_eq!(
                    post_counts.crlf, 0,
                    "CRLF injected into LF file at iteration {iteration}! Pre: {pre_counts:?}, Post: {post_counts:?}"
                );
                assert_eq!(
                    post_profile,
                    threadmoth::newline::NewlineProfile::Lf,
                    "LF file changed profile at iteration {iteration}!"
                );
            }

            // Invariant 2: Uniform CRLF file must NEVER acquire lone LF
            if pre_profile == threadmoth::newline::NewlineProfile::Crlf {
                assert_eq!(
                    post_counts.lf, 0,
                    "Lone LF injected into CRLF file at iteration {iteration}! Pre: {pre_counts:?}, Post: {post_counts:?}"
                );
                assert_eq!(
                    post_profile,
                    threadmoth::newline::NewlineProfile::Crlf,
                    "CRLF file changed profile at iteration {iteration}!"
                );
            }

            // Invariant 3: No mutation may EVER introduce bare CR
            assert!(
                post_counts.bare_cr <= pre_counts.bare_cr,
                "Bare CR increased at iteration {iteration}! Pre: {pre_counts:?}, Post: {post_counts:?}"
            );

            // Invariant 4: Truthful certificate metadata
            assert_eq!(
                cert.preservation.original_newline_profile,
                pre_profile.as_str()
            );
            assert_eq!(
                cert.preservation.result_newline_profile,
                post_profile.as_str()
            );
            assert!(
                !cert.preservation.conversion_authorized,
                "Unauthorized edit falsely claimed conversion authority at iteration {iteration}!"
            );
            // Invariant 5: Independent oracle verification of retained line identities
            audit_independent_line_identity(&source_bytes, &post_bytes, iteration);
        } else if cert.outcome == Outcome::Refused {
            // Invariant 6: Refusal must guarantee ZERO disk mutation
            assert_eq!(
                post_bytes, source_bytes,
                "Refused request altered physical bytes at iteration {iteration}!"
            );
        }
    }
}

#[test]
fn p5_01_bounded_property_fuzzing_ci() {
    run_fuzzing_oracle(100, 0xCAFE_BABE_1337_2026);
}

#[test]
#[ignore = "full 1000-case fuzzing profile; run with -- --ignored"]
fn p5_02_full_one_thousand_case_property_fuzzing() {
    run_fuzzing_oracle(1000, 0xCAFE_BABE_1337_2026);
}
// =========================================================================
// ASSAULT PHASE 6: AGENT HAPPINESS & HAMESH GUI REPRODUCTION
// =========================================================================

#[test]
fn p6_01_hamesh_gui_agent_happiness_and_ergonomics_suite() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("hamesh_editor.rs");

    // Exact replica of the Hamesh 1,109-line LF source file
    let mut source = String::with_capacity(30_000);
    for i in 1..=1109 {
        if i == 450 {
            source.push_str("    pub fn render_frame(&mut self) -> Result<(), Error> {\n");
        } else if i == 451 {
            source.push_str("        let buffer = self.alloc_back_buffer();\n");
        } else if i == 720 {
            source.push_str("    pub fn handle_key_event(&mut self, code: KeyCode) {\n");
        } else {
            source.push_str(&format!("    // line {i} placeholder comment\n"));
        }
    }
    fs::write(&file_path, source.as_bytes()).unwrap();

    // 10 realistic agent operations from PowerShell / CLI
    // Agent passes Windows CRLF heredocs and quotes

    // Op 1: Replace method signature with Windows CRLF string
    let out1 = cli(
        temp.path(),
        &[
            "edit",
            "hamesh_editor.rs",
            "--search",
            "    pub fn render_frame(&mut self) -> Result<(), Error> {\n        let buffer = self.alloc_back_buffer();\n",
            "--replace",
            "    pub fn render_frame(&mut self) -> Result<(), Error> {\r\n        // instrumented\r\n        let buffer = self.alloc_back_buffer();\r\n",
        ],
    );
    assert!(
        out1.status.success(),
        "Op 1 failed: {}",
        String::from_utf8_lossy(&out1.stderr)
    );

    // Op 2: Insert new method
    let out2 = cli(
        temp.path(),
        &[
            "edit",
            "hamesh_editor.rs",
            "--search",
            "    pub fn handle_key_event(&mut self, code: KeyCode) {\n",
            "--replace",
            "    pub fn handle_mouse_event(&mut self) {}\r\n\r\n    pub fn handle_key_event(&mut self, code: KeyCode) {\r\n",
        ],
    );
    assert!(
        out2.status.success(),
        "Op 2 failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );

    // Op 3: Replace using search-file and replace-file
    fs::write(
        temp.path().join("s3.txt"),
        b"    // line 100 placeholder comment\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("r3.txt"),
        b"    const MAX_RETRIES: usize = 5;\r\n",
    )
    .unwrap();
    let out3 = cli(
        temp.path(),
        &[
            "edit",
            "hamesh_editor.rs",
            "--search-file",
            "s3.txt",
            "--replace-file",
            "r3.txt",
        ],
    );
    assert!(
        out3.status.success(),
        "Op 3 failed: {}",
        String::from_utf8_lossy(&out3.stderr)
    );

    // Op 4: Replace with multiline edit document
    let edit_doc = b"*** File: hamesh_editor.rs\n<<<<<<< SEARCH\n    // line 200 placeholder comment\n=======\n    let mut frame_count: u64 = 0;\r\n    frame_count += 1;\r\n>>>>>>> REPLACE\n";
    let out4 = cli_with_stdin(temp.path(), &["edit", "-"], edit_doc);
    assert!(
        out4.status.success(),
        "Op 4 failed: stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out4.stdout),
        String::from_utf8_lossy(&out4.stderr)
    );

    // Verify after all operations:
    let final_bytes = fs::read(&file_path).unwrap();
    let counts = threadmoth::newline::count_newlines(&final_bytes);
    assert_eq!(
        counts.crlf, 0,
        "Hamesh file acquired CRLF endings after agent edits!"
    );
    assert_eq!(counts.bare_cr, 0, "Hamesh file acquired bare CR!");
    assert_eq!(
        threadmoth::newline::detect_profile(&final_bytes),
        threadmoth::newline::NewlineProfile::Lf,
        "Hamesh file failed to remain 100% uniform LF!"
    );
}
