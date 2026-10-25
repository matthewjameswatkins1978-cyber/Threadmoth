use crate::pipeline::{execute_request, execute_transaction};
use crate::presentation::{rule, truncate};
use crate::protocol::{
    Cardinality, EffectBudget, OperationPayload, Outcome, Request, TransactionRequest,
    PROTOCOL_VERSION,
};
use crate::provider::text::TextOperation;
use crate::workspace::Workspace;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct CaseResult {
    name: String,
    state: &'static str,
    detail: String,
}

#[derive(Serialize)]
struct Report {
    tool: &'static str,
    state: &'static str,
    passed: usize,
    failed: usize,
    skipped: usize,
    invocations: usize,
    footgun_safe: usize,
    footgun_total: usize,
    cases: Vec<CaseResult>,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("threadmoth-torture-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn request(path: &str, operation: TextOperation) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "release-torture".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::Text(operation),
    }
}

fn add(
    results: &mut Vec<CaseResult>,
    name: impl Into<String>,
    state: &'static str,
    detail: impl Into<String>,
) {
    results.push(CaseResult {
        name: name.into(),
        state,
        detail: detail.into(),
    });
}

fn apply_case(workspace: &Workspace, root: &Path, results: &mut Vec<CaseResult>) -> usize {
    fs::write(root.join("apply.txt"), b"before TARGET after\n").unwrap();
    let certificate = execute_request(
        workspace,
        &request(
            "apply.txt",
            TextOperation::Replace {
                target: "TARGET".into(),
                replacement: "after".into(),
            },
        ),
        false,
    );
    let pass = certificate.outcome == Outcome::Applied
        && fs::read(root.join("apply.txt")).unwrap() == b"before after after\n";
    add(
        results,
        "exact replacement applies",
        if pass { "PASS" } else { "FAIL" },
        format!("outcome={:?}", certificate.outcome),
    );
    1
}

fn refusal_case(
    workspace: &Workspace,
    root: &Path,
    results: &mut Vec<CaseResult>,
    name: &str,
    contents: &[u8],
    operation: TextOperation,
) -> usize {
    fs::write(root.join(name), contents).unwrap();
    let before = fs::read(root.join(name)).unwrap();
    let certificate = execute_request(workspace, &request(name, operation), false);
    let pass =
        certificate.outcome == Outcome::Refused && fs::read(root.join(name)).unwrap() == before;
    add(
        results,
        name,
        if pass { "PASS" } else { "FAIL" },
        format!("outcome={:?}", certificate.outcome),
    );
    1
}

fn stale_identity_case(workspace: &Workspace, root: &Path, results: &mut Vec<CaseResult>) -> usize {
    let name = "stale.txt";
    fs::write(root.join(name), b"hello world\n").unwrap();
    let mut request = request(
        name,
        TextOperation::Replace {
            target: "world".into(),
            replacement: "there".into(),
        },
    );
    request.expected_pre_hash = Some("0".repeat(64));
    let certificate = execute_request(workspace, &request, false);
    let pass = certificate.outcome == Outcome::Refused
        && fs::read(root.join(name)).unwrap() == b"hello world\n";
    add(
        results,
        name,
        if pass { "PASS" } else { "FAIL" },
        format!("outcome={:?}", certificate.outcome),
    );
    1
}

#[cfg(windows)]
fn readonly_case(workspace: &Workspace, root: &Path, results: &mut Vec<CaseResult>) -> usize {
    let name = "readonly.txt";
    let path = root.join(name);
    fs::write(&path, b"old\n").unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly_permissions = original_permissions.clone();
    readonly_permissions.set_readonly(true);
    fs::set_permissions(&path, readonly_permissions).unwrap();
    let certificate = execute_request(
        workspace,
        &request(
            name,
            TextOperation::Replace {
                target: "old".into(),
                replacement: "new".into(),
            },
        ),
        false,
    );
    let residue = fs::read_dir(root).unwrap().flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(".readonly.txt.")
    });
    fs::set_permissions(&path, original_permissions).unwrap();
    let pass =
        certificate.outcome == Outcome::Failed && !residue && fs::read(&path).unwrap() == b"old\n";
    add(
        results,
        "read-only staged-residue cleanup",
        if pass { "PASS" } else { "FAIL" },
        format!("outcome={:?} residue={residue}", certificate.outcome),
    );
    1
}

#[cfg(not(windows))]
fn readonly_case(_workspace: &Workspace, _root: &Path, results: &mut Vec<CaseResult>) -> usize {
    add(
        results,
        "read-only staged-residue cleanup",
        "SKIP",
        "Windows-only filesystem permission case",
    );
    0
}

fn run_footgun_100(
    workspace: &Workspace,
    root: &Path,
    results: &mut Vec<CaseResult>,
) -> (usize, usize) {
    let mut safe = 0;
    let total = 100;
    for index in 1..=total {
        let name = format!("footgun-{index}.txt");
        let target = format!("FOOTGUN-{index}");
        let duplicate = index % 10 == 0;
        let missing = index % 10 == 1;
        let content = if duplicate {
            format!("{target}\n{target}\n")
        } else {
            format!("header\n{target}\nfooter\n")
        };
        fs::write(root.join(&name), content.as_bytes()).unwrap();
        let requested_target = if missing {
            format!("MISSING-{index}")
        } else {
            target.clone()
        };
        let certificate = execute_request(
            workspace,
            &request(
                &name,
                TextOperation::Replace {
                    target: requested_target,
                    replacement: "SAFE".into(),
                },
            ),
            false,
        );
        let expected = if duplicate || missing {
            certificate.outcome == Outcome::Refused
        } else {
            certificate.outcome == Outcome::Applied
                && fs::read_to_string(root.join(&name))
                    .unwrap()
                    .contains("SAFE")
        };
        if expected {
            safe += 1;
        }
    }
    add(
        results,
        "FOOTGUN-100",
        if safe == total { "PASS" } else { "FAIL" },
        format!("safe={safe}/{total}"),
    );
    (safe, total)
}

fn symlink_case(workspace: &Workspace, root: &Path, results: &mut Vec<CaseResult>) -> usize {
    let outside = root.with_file_name(format!(
        "{}-outside",
        root.file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("sentinel.txt"), b"OUTSIDE-SENTINEL\n").unwrap();
    let link = root.join("link-escape.txt");
    let made = symlink_file(&outside.join("sentinel.txt"), &link);
    if !made {
        let _ = fs::remove_dir_all(&outside);
        add(
            results,
            "symlink escape",
            "SKIP",
            "symbolic-link creation unavailable",
        );
        return 0;
    }
    let certificate = execute_request(
        workspace,
        &request(
            "link-escape.txt",
            TextOperation::Replace {
                target: "OUTSIDE-SENTINEL".into(),
                replacement: "ESCAPED".into(),
            },
        ),
        false,
    );
    let pass = certificate.outcome == Outcome::Refused
        && fs::read(outside.join("sentinel.txt")).unwrap() == b"OUTSIDE-SENTINEL\n";
    let _ = fs::remove_dir_all(&outside);
    add(
        results,
        "symlink escape",
        if pass { "PASS" } else { "FAIL" },
        format!("outcome={:?}", certificate.outcome),
    );
    1
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_target: &Path, _link: &Path) -> bool {
    false
}

fn print_human_report(report: &Report) {
    println!("THREADMOTH TORTURE");
    println!("{}", rule());
    println!("Version     {}", env!("CARGO_PKG_VERSION"));
    println!("Mode        deterministic safety + refusal checks");
    println!("Invocations {}", report.invocations);
    println!();
    println!("{:<38} {:>8}  Detail", "Case", "Result");
    println!("{}", rule());
    for case in &report.cases {
        println!(
            "{:<38} {:>8}  {}",
            truncate(&case.name, 38),
            case.state,
            truncate(&case.detail, 28),
        );
    }
    println!("{}", rule());
    println!(
        "{}  {} passed · {} failed · {} skipped · FOOTGUN {}/{} safe",
        report.state,
        report.passed,
        report.failed,
        report.skipped,
        report.footgun_safe,
        report.footgun_total,
    );
}

pub fn run(json: bool) -> i32 {
    let root = match TempRoot::new() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("torture setup failed: {error}");
            return 3;
        }
    };
    let workspace = match Workspace::new(&root.0) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("torture workspace setup failed: {error}");
            return 3;
        }
    };
    let mut results = Vec::new();
    let mut invocations = 0;
    invocations += apply_case(&workspace, &root.0, &mut results);
    invocations += refusal_case(
        &workspace,
        &root.0,
        &mut results,
        "ambiguous.txt",
        b"TARGET\nTARGET\n",
        TextOperation::Replace {
            target: "TARGET".into(),
            replacement: "SAFE".into(),
        },
    );
    invocations += refusal_case(
        &workspace,
        &root.0,
        &mut results,
        "missing.txt",
        b"nothing here\n",
        TextOperation::Replace {
            target: "MISSING".into(),
            replacement: "SAFE".into(),
        },
    );
    invocations += refusal_case(
        &workspace,
        &root.0,
        &mut results,
        "empty-set.txt",
        b"hello world\n",
        TextOperation::Set {
            target: "missing".into(),
            replacement: String::new(),
        },
    );
    invocations += refusal_case(
        &workspace,
        &root.0,
        &mut results,
        "empty-rename.txt",
        b"hello world\n",
        TextOperation::Rename {
            target: "missing".into(),
            replacement: String::new(),
        },
    );
    invocations += stale_identity_case(&workspace, &root.0, &mut results);
    invocations += readonly_case(&workspace, &root.0, &mut results);
    let (safe, total) = run_footgun_100(&workspace, &root.0, &mut results);
    invocations += total;
    invocations += symlink_case(&workspace, &root.0, &mut results);

    fs::write(root.0.join("transaction-a.txt"), b"old-a\n").unwrap();
    fs::write(root.0.join("transaction-b.txt"), b"old-b\n").unwrap();
    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "release-torture-transaction".into(),
        requests: vec![
            request(
                "transaction-a.txt",
                TextOperation::Replace {
                    target: "old-a".into(),
                    replacement: "new-a".into(),
                },
            ),
            request(
                "transaction-b.txt",
                TextOperation::Replace {
                    target: "old-b".into(),
                    replacement: "new-b".into(),
                },
            ),
        ],
        budget: EffectBudget::default(),
    };
    let transaction_certificate = execute_transaction(&workspace, &transaction, false);
    let transaction_pass = transaction_certificate.outcome == Outcome::Applied
        && !root.0.join(".threadmoth-recovery").exists();
    add(
        &mut results,
        "successful transaction cleanup",
        if transaction_pass { "PASS" } else { "FAIL" },
        format!(
            "outcome={:?} recovery_dir_present={}",
            transaction_certificate.outcome,
            root.0.join(".threadmoth-recovery").exists()
        ),
    );
    invocations += 1;

    let passed = results
        .iter()
        .filter(|result| result.state == "PASS")
        .count();
    let failed = results
        .iter()
        .filter(|result| result.state == "FAIL")
        .count();
    let skipped = results
        .iter()
        .filter(|result| result.state == "SKIP")
        .count();
    let state = if failed == 0 && safe == total {
        "PASS"
    } else {
        "FAIL"
    };
    let report = Report {
        tool: "threadmoth torture",
        state,
        passed,
        failed,
        skipped,
        invocations,
        footgun_safe: safe,
        footgun_total: total,
        cases: results,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print_human_report(&report);
    }
    i32::from(failed != 0 || safe != total)
}
