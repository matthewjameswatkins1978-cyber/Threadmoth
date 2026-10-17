use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use tempfile::TempDir;
use threadmoth::{
    engine::compute_sha256,
    pipeline::apply_prepared_plan,
    protocol::{
        Cardinality, EffectBudget, OperationPayload, Outcome, PlanApplyResult, Request,
        PROTOCOL_VERSION,
    },
    provider::text::TextOperation,
    source_prepare::{
        prepare, NewlinePolicy, SourceAwareTextRequest, SourcePreparationResult,
        TextReplacementIntent,
    },
    workspace::Workspace,
};

fn request(path: &str, edits: &[(&str, &str)]) -> SourceAwareTextRequest {
    SourceAwareTextRequest {
        file_path: path.into(),
        edits: edits
            .iter()
            .map(|(old, new)| TextReplacementIntent {
                old: (*old).into(),
                new: (*new).into(),
            })
            .collect(),
        newline_policy: NewlinePolicy::Source,
    }
}

fn prepared(
    result: SourcePreparationResult,
) -> threadmoth::source_prepare::SourceAwareTextPreparation {
    match result {
        SourcePreparationResult::Prepared(value) => *value,
        SourcePreparationResult::Refused { certificate } => {
            panic!("unexpected refusal: {certificate:?}")
        }
    }
}

#[test]
fn lf_intent_is_adapted_to_crlf_source_and_commits_through_portable_plan() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("config.txt"), b"host=one\r\nport=80\r\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let result = prepared(prepare(
        &workspace,
        &request(
            "config.txt",
            &[("host=one\nport=80\n", "host=two\nport=80\n")],
        ),
    ));
    assert_eq!(result.source_newline_profile, "crlf");
    assert_eq!(result.resolved_edits, 1);
    let cert = match apply_prepared_plan(&workspace, &result.plan) {
        PlanApplyResult::Certificate(c) => c,
        _ => panic!(),
    };
    assert_eq!(cert.outcome, Outcome::Applied);
    assert_eq!(cert.preservation.original_newline_profile, "crlf");
    assert_eq!(cert.preservation.result_newline_profile, "crlf");
    assert!(!cert.preservation.line_endings_changed);
    assert_eq!(
        fs::read(temp.path().join("config.txt")).unwrap(),
        b"host=two\r\nport=80\r\n"
    );
}

#[test]
fn ordinary_lf_and_exact_mixed_newline_sources_are_supported() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("lf.txt"), b"a=1\nb=2\n").unwrap();
    fs::write(temp.path().join("mixed.txt"), b"a=1\r\nb=2\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let lf = prepared(prepare(
        &workspace,
        &request("lf.txt", &[("a=1\nb=2\n", "a=2\nb=2\n")]),
    ));
    assert_eq!(lf.source_newline_profile, "lf");

    let mut mixed_request = request("mixed.txt", &[("a=1\r\nb=2\n", "a=2\r\nb=2\n")]);
    mixed_request.newline_policy = NewlinePolicy::Exact;
    let mixed = prepared(prepare(&workspace, &mixed_request));
    assert_eq!(mixed.source_newline_profile, "mixed");
    assert!(matches!(
        apply_prepared_plan(&workspace, &mixed.plan),
        PlanApplyResult::Certificate(certificate) if certificate.outcome == Outcome::Applied
    ));
    assert_eq!(
        fs::read(temp.path().join("mixed.txt")).unwrap(),
        b"a=2\r\nb=2\n"
    );
}

#[test]
fn mixed_newline_source_requires_explicit_exact_policy() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("mixed.txt"), b"a=1\r\nb=2\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let refused = match prepare(
        &workspace,
        &request("mixed.txt", &[("a=1\nb=2\n", "a=2\nb=2\n")]),
    ) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected mixed-line-ending refusal, got {other:?}"),
    };
    assert_eq!(
        refused.reason_code.as_deref(),
        Some("PRESERVATION_UNAVAILABLE")
    );
    let mut exact_but_wrong = request("mixed.txt", &[("a=1\nb=2\n", "a=2\nb=2\n")]);
    exact_but_wrong.newline_policy = NewlinePolicy::Exact;
    let exact_refusal = match prepare(&workspace, &exact_but_wrong) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected exact line-ending mismatch refusal, got {other:?}"),
    };
    assert_eq!(
        exact_refusal.reason_code.as_deref(),
        Some("TARGET_NOT_FOUND")
    );
    assert!(format!("{:?}", exact_refusal.refusal_reason)
        .contains("matching source region uses mixed LF and CRLF line endings"));

    assert_eq!(
        fs::read(temp.path().join("mixed.txt")).unwrap(),
        b"a=1\r\nb=2\n"
    );
}

#[test]
fn missing_and_duplicate_targets_refuse_without_guessing() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("targets.txt"), b"dup\ndup\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();

    let missing = match prepare(&workspace, &request("targets.txt", &[("absent", "new")])) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected missing-target refusal, got {other:?}"),
    };
    assert_eq!(missing.reason_code.as_deref(), Some("TARGET_NOT_FOUND"));
    let remedy = missing
        .recovery
        .as_ref()
        .unwrap()
        .remedies
        .iter()
        .find(|remedy| remedy.kind == "use_file_based_text_input")
        .unwrap();
    assert!(remedy
        .description
        .contains("does not establish that shell quoting caused"));
    assert!(remedy
        .description
        .contains("threadmoth prepare-text PATH --edit-files OLD_FILE NEW_FILE"));
    assert!(!remedy.description.contains("targets.txt"));

    let shell_metachar_path = "config; echo unsafe.txt";
    fs::write(temp.path().join(shell_metachar_path), b"present\n").unwrap();
    let refusal = match prepare(
        &workspace,
        &request(shell_metachar_path, &[("absent", "replacement")]),
    ) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected missing-target refusal, got {other:?}"),
    };
    let remedy = refusal
        .recovery
        .as_ref()
        .unwrap()
        .remedies
        .iter()
        .find(|remedy| remedy.kind == "use_file_based_text_input")
        .unwrap();
    assert!(!remedy.description.contains(shell_metachar_path));
    assert!(remedy
        .description
        .contains("threadmoth prepare-text PATH --edit-files OLD_FILE NEW_FILE"));

    let duplicate = match prepare(&workspace, &request("targets.txt", &[("dup", "new")])) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected duplicate-target refusal, got {other:?}"),
    };
    assert_eq!(duplicate.reason_code.as_deref(), Some("TARGET_AMBIGUOUS"));
    assert_eq!(
        fs::read(temp.path().join("targets.txt")).unwrap(),
        b"dup\ndup\n"
    );
}

#[test]
fn successive_edits_resolve_against_one_virtual_source_and_apply_atomically() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("values.txt"), b"first=1\nsecond=2\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let result = prepared(prepare(
        &workspace,
        &request(
            "values.txt",
            &[
                ("first=1", "one=2"),
                ("second=2", "two=3"),
                ("one=2", "first=1"),
            ],
        ),
    ));
    let outcome = apply_prepared_plan(&workspace, &result.plan);
    assert!(matches!(outcome, PlanApplyResult::Certificate(c) if c.outcome == Outcome::Applied));
    assert_eq!(
        fs::read(temp.path().join("values.txt")).unwrap(),
        b"first=1\ntwo=3\n"
    );
}

#[test]
fn source_change_after_preparation_is_stale_and_never_overwritten() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("stale.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let result = prepared(prepare(
        &workspace,
        &request("stale.txt", &[("old", "new")]),
    ));
    fs::write(temp.path().join("stale.txt"), b"external\n").unwrap();
    let outcome = apply_prepared_plan(&workspace, &result.plan);
    assert!(
        matches!(outcome, PlanApplyResult::Certificate(c) if c.reason_code.as_deref() == Some("PLAN_STALE"))
    );
    assert_eq!(
        fs::read(temp.path().join("stale.txt")).unwrap(),
        b"external\n"
    );
}

#[test]
fn sequence_operation_requires_protocol_14_even_when_a_plan_id_is_recomputed() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("version.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let result = prepared(prepare(
        &workspace,
        &request("version.txt", &[("old", "new")]),
    ));
    let mut old_protocol_plan = result.plan;
    old_protocol_plan.protocol_version = "1.3.1".into();
    old_protocol_plan.operations[0].request.version = "1.3.1".into();
    old_protocol_plan.plan_id.clear();
    old_protocol_plan.plan_id = format!(
        "sha256:{}",
        compute_sha256(&serde_json::to_vec(&old_protocol_plan).unwrap())
    );

    let outcome = apply_prepared_plan(&workspace, &old_protocol_plan);
    assert!(matches!(
        outcome,
        PlanApplyResult::Certificate(certificate)
            if certificate.reason_code.as_deref() == Some("OPERATION_UNSUPPORTED")
    ));
    assert_eq!(fs::read(temp.path().join("version.txt")).unwrap(), b"old\n");
}

#[test]
fn out_of_workspace_and_absolute_targets_are_refused() {
    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("target.txt"), b"old\n").unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let absolute = outside
        .path()
        .join("target.txt")
        .to_string_lossy()
        .into_owned();
    let refused = match prepare(&workspace, &request(&absolute, &[("old", "new")])) {
        SourcePreparationResult::Refused { certificate } => certificate,
        other => panic!("expected path refusal, got {other:?}"),
    };
    assert_eq!(
        refused.reason_code.as_deref(),
        Some("WORKSPACE_ROOT_MISMATCH")
    );
    assert_eq!(
        fs::read(outside.path().join("target.txt")).unwrap(),
        b"old\n"
    );
}

#[test]
fn workflow_comparison_measures_retries_calls_and_request_bytes() {
    let old_temp = TempDir::new().unwrap();
    fs::write(
        old_temp.path().join("config.txt"),
        b"key=one\r\nkey=two\r\n",
    )
    .unwrap();
    let old_workspace = Workspace::new(old_temp.path()).unwrap();
    let make_old_request = |target: &str, replacement: &str| Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "legacy-full-request".into(),
        allow_generated: false,
        file_path: "config.txt".into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            max_files: Some(1),
            max_matches: Some(1),
            max_changed_regions: Some(1),
            max_changed_bytes: Some(128),
            allowed_path_prefixes: vec!["config.txt".into()],
            ..EffectBudget::default()
        },
        operation: OperationPayload::Text(TextOperation::Replace {
            target: target.into(),
            replacement: replacement.into(),
        }),
    };
    let first = make_old_request("key=one\nkey=two\n", "key=uno\nkey=two\n");
    let old_first_bytes = serde_json::to_vec(&first).unwrap().len();
    assert_eq!(
        threadmoth::pipeline::execute_request(&old_workspace, &first, true).outcome,
        Outcome::Refused
    );
    let retry = make_old_request("key=one\r\nkey=two\r\n", "key=uno\r\nkey=two\r\n");
    let old_retry_bytes = serde_json::to_vec(&retry).unwrap().len();
    assert_eq!(
        threadmoth::pipeline::execute_request(&old_workspace, &retry, true).outcome,
        Outcome::Applied
    );
    assert_eq!(
        threadmoth::pipeline::execute_request(&old_workspace, &retry, false).outcome,
        Outcome::Applied
    );

    let new_temp = TempDir::new().unwrap();
    fs::write(
        new_temp.path().join("config.txt"),
        b"key=one\r\nkey=two\r\n",
    )
    .unwrap();
    let new_workspace = Workspace::new(new_temp.path()).unwrap();
    let intent = request(
        "config.txt",
        &[("key=one\nkey=two\n", "key=uno\nkey=two\n")],
    );
    let compact_request_bytes = serde_json::to_vec(&intent).unwrap().len();
    let result = prepared(prepare(&new_workspace, &intent));
    let apply_request_bytes = serde_json::to_vec(&result.plan).unwrap().len();
    assert!(matches!(
        apply_prepared_plan(&new_workspace, &result.plan),
        PlanApplyResult::Certificate(certificate) if certificate.outcome == Outcome::Applied
    ));

    let old_calls = 3; // initial refusal, corrected preview, commit
    let new_calls = 2; // source-aware prepare, then apply the prepared plan
    let old_retries = 1;
    let new_retries = 0;
    let old_agent_request_bytes = old_first_bytes + old_retry_bytes * 2;
    let new_agent_request_bytes = compact_request_bytes + apply_request_bytes;
    println!(
        "workflow_metrics old_success=true new_success=true old_retries={old_retries} new_retries={new_retries} old_tool_calls={old_calls} new_tool_calls={new_calls} old_agent_request_bytes={old_agent_request_bytes} new_agent_request_bytes={new_agent_request_bytes} compact_intent_bytes={compact_request_bytes}"
    );
    assert!(new_retries < old_retries);
    assert!(new_calls < old_calls);
}

#[test]
fn compact_cli_prepares_repeated_crlf_edits_and_guides_review_then_apply() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("settings.conf");
    let plan = temp.path().join("prepared-plan.json");
    fs::write(
        &source,
        b"first=one\r\nsecond=two\r\n# keep this comment\r\n",
    )
    .unwrap();
    let original = fs::read(&source).unwrap();

    let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "settings.conf", "--edit"])
        .arg("first=one\n")
        .arg("first=uno\n")
        .arg("--edit")
        .arg("second=two\n")
        .arg("second=dos\n")
        .arg("--output")
        .arg(&plan)
        .arg("--summary")
        .output()
        .unwrap();
    assert!(preparation.status.success());
    let preparation = String::from_utf8_lossy(&preparation.stdout);
    assert!(preparation.contains("Newlines       crlf"));
    assert!(preparation.contains("Exact edits    2"));
    assert!(preparation.contains("threadmoth explain --plan"));
    assert!(preparation.contains("threadmoth apply-plan --plan"));
    assert_eq!(fs::read(&source).unwrap(), original);
    let prepared_plan: serde_json::Value =
        serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(
        prepared_plan["operations"][0]["request"]["operation"]["operation"]["edits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let review = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["explain", "--plan"])
        .arg(&plan)
        .args(["--format", "diff"])
        .output()
        .unwrap();
    assert!(review.status.success());
    let review = String::from_utf8_lossy(&review.stdout);
    assert!(review.contains("state: FRESH"));
    assert!(review.contains("Next: threadmoth apply-plan --plan"));

    let apply = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan"])
        .arg(&plan)
        .args(["--summary"])
        .output()
        .unwrap();
    assert!(apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stdout).contains("APPLIED"));
    assert_eq!(
        fs::read(&source).unwrap(),
        b"first=uno\r\nsecond=dos\r\n# keep this comment\r\n"
    );
}

#[cfg(windows)]
fn round_trip_powershell_command(
    command: &str,
    working_directory: &std::path::Path,
) -> Vec<String> {
    let script = r#"
function threadmoth { $global:captured = @($args) }
Invoke-Expression $env:THREADMOTH_RENDERED_COMMAND
[Console]::Out.Write((ConvertTo-Json -InputObject @($global:captured) -Compress))
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(script)
        .env("THREADMOTH_RENDERED_COMMAND", command)
        .current_dir(working_directory)
        .output()
        .expect("PowerShell is available on Windows");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("PowerShell returns the captured argument array")
}

#[cfg(windows)]
#[test]
fn summary_review_and_apply_commands_round_trip_plan_path_in_powershell() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    let plan = temp
        .path()
        .join("plan-$(Set-Content -NoNewline marker.txt injected)-'quoted'.json");

    let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit",
            "old",
            "new",
            "--output",
        ])
        .arg(&plan)
        .args(["--summary"])
        .output()
        .unwrap();
    assert!(
        preparation.status.success(),
        "{}",
        String::from_utf8_lossy(&preparation.stderr)
    );
    let summary = String::from_utf8_lossy(&preparation.stdout);
    let review = summary
        .lines()
        .find_map(|line| line.strip_prefix("Review (PowerShell): "))
        .expect("summary contains a PowerShell Review command");
    let apply = summary
        .lines()
        .find_map(|line| line.strip_prefix("Apply (PowerShell): "))
        .expect("summary contains a PowerShell Apply command");
    let expected_plan = plan.to_string_lossy().into_owned();

    assert_eq!(
        round_trip_powershell_command(review, temp.path()),
        vec![
            "explain",
            "--plan",
            expected_plan.as_str(),
            "--format",
            "diff"
        ]
    );
    assert!(!temp.path().join("marker.txt").exists());
    assert_eq!(
        round_trip_powershell_command(apply, temp.path()),
        vec!["apply-plan", "--plan", expected_plan.as_str()]
    );
    assert!(!temp.path().join("marker.txt").exists());
}

#[cfg(unix)]
fn round_trip_posix_command(command: &str, working_directory: &std::path::Path) -> Vec<Vec<u8>> {
    let script = r#"
threadmoth() { printf '%s\n' "$@"; }
eval "$THREADMOTH_RENDERED_COMMAND"
"#;
    let output = Command::new("sh")
        .args(["-c", script])
        .env("THREADMOTH_RENDERED_COMMAND", command)
        .current_dir(working_directory)
        .output()
        .expect("a POSIX shell is available on Unix");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|argument| !argument.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

#[cfg(unix)]
#[test]
fn summary_review_and_apply_commands_round_trip_plan_paths_in_posix_shell() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    let plan_names = [
        "plan.json",
        "plan with spaces.json",
        "plan-O'Brien.json",
        "plan-$HOME-$(touch marker-substitution)-`touch marker-backtick`;&|*?.json",
    ];

    for plan_name in plan_names {
        let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
            .current_dir(temp.path())
            .args([
                "prepare-text",
                "source.txt",
                "--edit",
                "old",
                "new",
                "--output",
            ])
            .arg(plan_name)
            .args(["--summary"])
            .output()
            .unwrap();
        assert!(
            preparation.status.success(),
            "{}",
            String::from_utf8_lossy(&preparation.stderr)
        );
        let summary = String::from_utf8(preparation.stdout).unwrap();
        let review = summary
            .lines()
            .find_map(|line| line.strip_prefix("Review (POSIX shell): "))
            .expect("summary contains a POSIX Review command");
        let apply = summary
            .lines()
            .find_map(|line| line.strip_prefix("Apply (POSIX shell): "))
            .expect("summary contains a POSIX Apply command");
        let expected_path = plan_name.as_bytes();

        assert_eq!(
            round_trip_posix_command(review, temp.path()),
            vec![
                b"explain".to_vec(),
                b"--plan".to_vec(),
                expected_path.to_vec(),
                b"--format".to_vec(),
                b"diff".to_vec(),
            ],
            "Review command changed plan path {plan_name:?}"
        );
        assert_eq!(
            round_trip_posix_command(apply, temp.path()),
            vec![
                b"apply-plan".to_vec(),
                b"--plan".to_vec(),
                expected_path.to_vec()
            ],
            "Apply command changed plan path {plan_name:?}"
        );
        assert!(!temp.path().join("marker-substitution").exists());
        assert!(!temp.path().join("marker-backtick").exists());
    }
}

#[cfg(unix)]
#[test]
fn summary_omits_commands_for_newline_and_non_utf8_plan_paths() {
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    let plan_paths = [
        std::ffi::OsString::from("plan\nwith-newline.json"),
        std::ffi::OsString::from("plan\u{2028}line-separator.json"),
        std::ffi::OsString::from_vec(b"plan-\xff-non-utf8.json".to_vec()),
    ];

    for plan_path in plan_paths {
        let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
            .current_dir(temp.path())
            .args([
                "prepare-text",
                "source.txt",
                "--edit",
                "old",
                "new",
                "--output",
            ])
            .arg(&plan_path)
            .args(["--summary"])
            .output()
            .unwrap();
        assert!(
            preparation.status.success(),
            "{}",
            String::from_utf8_lossy(&preparation.stderr)
        );
        let summary = String::from_utf8_lossy(&preparation.stdout);
        assert!(summary.contains("Review (POSIX shell): unavailable (not executable;"));
        assert!(summary.contains("Apply (POSIX shell): unavailable (not executable;"));
        assert!(!summary.contains("threadmoth explain --plan"));
        assert!(!summary.contains("threadmoth apply-plan --plan"));
    }
}

#[test]
fn prepare_edits_review_shows_diff_and_keeps_apply_separate() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("settings.txt");
    fs::write(&source, b"mode=old\nkeep=exact\n").unwrap();
    fs::write(
        temp.path().join("changes.tm"),
        "*** File: settings.txt\n<<<<<<< SEARCH\nmode=old\n=======\nmode=new\n>>>>>>> REPLACE\n",
    )
    .unwrap();

    let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "plan.json",
            "--review",
        ])
        .output()
        .unwrap();
    assert!(
        preparation.status.success(),
        "{}",
        String::from_utf8_lossy(&preparation.stderr)
    );
    let review = String::from_utf8_lossy(&preparation.stdout);
    assert!(review.contains("state: FRESH"), "{review}");
    assert!(review.contains("-mode=old"), "{review}");
    assert!(review.contains("+mode=new"), "{review}");
    let apply_label = if cfg!(windows) {
        "Apply (PowerShell):"
    } else {
        "Apply (POSIX shell):"
    };
    assert!(review.contains(apply_label), "{review}");
    assert_eq!(fs::read(&source).unwrap(), b"mode=old\nkeep=exact\n");
    assert!(temp.path().join("plan.json").exists());

    let applied = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan", "plan.json"])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stdout)
    );
    let certificate: threadmoth::protocol::Certificate =
        serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(fs::read(&source).unwrap(), b"mode=new\nkeep=exact\n");
}

#[test]
fn prepare_edits_review_marks_truncation_and_offers_full_review() {
    let temp = TempDir::new().unwrap();
    let replacement = "x".repeat(5_000);
    fs::write(temp.path().join("large.txt"), b"old\n").unwrap();
    fs::write(
        temp.path().join("changes.tm"),
        format!(
            "*** File: large.txt\n<<<<<<< SEARCH\nold\n=======\n{replacement}\n>>>>>>> REPLACE\n"
        ),
    )
    .unwrap();
    let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "plan.json",
            "--review",
        ])
        .output()
        .unwrap();
    assert!(
        preparation.status.success(),
        "{}",
        String::from_utf8_lossy(&preparation.stderr)
    );
    let review = String::from_utf8_lossy(&preparation.stdout);
    assert!(review.contains("review output truncated"), "{review}");
    assert!(review.contains("Full review ("), "{review}");
    assert!(review.contains("--format diff --full"), "{review}");

    let full = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "explain",
            "--plan",
            "plan.json",
            "--format",
            "diff",
            "--full",
        ])
        .output()
        .unwrap();
    assert!(
        full.status.success(),
        "{}",
        String::from_utf8_lossy(&full.stderr)
    );
    let full = String::from_utf8_lossy(&full.stdout);
    assert!(!full.contains("review output truncated"));
    assert!(full.contains(&format!("+{replacement}")));
    assert_eq!(fs::read(temp.path().join("large.txt")).unwrap(), b"old\n");
}

#[test]
fn prepare_edits_review_requires_plan_output_and_conflicts_with_summary() {
    let temp = TempDir::new().unwrap();
    let missing_output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-edits", "changes.tm", "--review"])
        .output()
        .unwrap();
    assert_eq!(missing_output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_output.stderr).contains("--output"));

    let conflict = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-edits",
            "changes.tm",
            "--output",
            "plan.json",
            "--review",
            "--summary",
        ])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(2));
}
#[test]
fn stale_plan_review_explains_repreparation_and_apply_refuses_without_overwrite() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("stale.txt");
    let plan = temp.path().join("stale-plan.json");
    fs::write(&source, b"old\n").unwrap();
    let preparation = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "stale.txt",
            "--edit",
            "old",
            "new",
            "--output",
        ])
        .arg(&plan)
        .args(["--summary"])
        .output()
        .unwrap();
    assert!(preparation.status.success());
    fs::write(&source, b"external update\n").unwrap();

    let review = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["explain", "--plan"])
        .arg(&plan)
        .args(["--format", "diff"])
        .output()
        .unwrap();
    assert!(review.status.success());
    let review = String::from_utf8_lossy(&review.stdout);
    assert!(review.contains("state: STALE"));
    assert!(review.contains("prepare a new plan from the current source"));

    let apply = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan"])
        .arg(&plan)
        .args(["--summary"])
        .output()
        .unwrap();
    assert_eq!(apply.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&apply.stdout).contains("PLAN_STALE"));
    assert_eq!(fs::read(&source).unwrap(), b"external update\n");
}

#[test]
fn compact_cli_requires_plan_output_for_summary_and_rejects_outside_path() {
    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(temp.path().join("inside.txt"), b"old\n").unwrap();
    fs::write(outside.path().join("outside.txt"), b"old\n").unwrap();

    let missing_output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "inside.txt",
            "--edit",
            "old",
            "new",
            "--summary",
        ])
        .output()
        .unwrap();
    assert_eq!(missing_output.status.code(), Some(2));
    let explicit_json_policy = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "--newline-policy", "exact"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            let request =
                serde_json::json!({"file_path":"inside.txt","edits":[{"old":"old","new":"new"}]})
                    .to_string();
            child.stdin.take().unwrap().write_all(request.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert_eq!(explicit_json_policy.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&explicit_json_policy.stderr)
        .contains("set newline_policy in the JSON request"));

    fs::write(temp.path().join("mixed.txt"), b"first\nsecond\r\n").unwrap();
    let exact_policy = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "mixed.txt", "--edit"])
        .arg("first\n")
        .arg("one\n")
        .arg("--edit")
        .arg("second\r\n")
        .arg("two\r\n")
        .args(["--newline-policy", "exact", "--output", "mixed-plan.json"])
        .output()
        .unwrap();
    assert!(exact_policy.status.success());
    assert_eq!(
        fs::read(temp.path().join("mixed.txt")).unwrap(),
        b"first\nsecond\r\n"
    );

    let output_conflict = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "inside.txt",
            "--edit",
            "old",
            "new",
            "--output",
            "inside.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(output_conflict.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output_conflict.stderr)
        .contains("must not overwrite the edited source file"));
    let source = temp.path().join("inside.txt");
    assert_eq!(fs::read(&source).unwrap(), b"old\n");

    let hard_link = temp.path().join("inside-hard-link.txt");
    fs::hard_link(&source, &hard_link).unwrap();
    let hard_link_output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "inside.txt",
            "--edit",
            "old",
            "new",
            "--output",
        ])
        .arg("inside-hard-link.txt")
        .output()
        .unwrap();
    assert_eq!(hard_link_output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&hard_link_output.stderr).contains("plan output already exists")
    );
    assert_eq!(fs::read(&source).unwrap(), b"old\n");
    assert_eq!(fs::read(&hard_link).unwrap(), b"old\n");

    let outside = outside.path().join("outside.txt");
    let refused = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text"])
        .arg(&outside)
        .args(["--edit", "old", "new"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    let certificate: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(certificate["reason_code"], "WORKSPACE_ROOT_MISMATCH");

    let summary = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text"])
        .arg(&outside)
        .args([
            "--edit",
            "old",
            "new",
            "--output",
            "refused-plan.json",
            "--summary",
        ])
        .output()
        .unwrap();
    assert_eq!(summary.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&summary.stdout).contains("Result       REFUSED"));
    assert!(!temp.path().join("refused-plan.json").exists());
    assert_eq!(fs::read(outside).unwrap(), b"old\n");
}

#[cfg(unix)]
#[test]
fn cli_edit_files_rejects_non_unicode_input_paths() {
    use std::os::unix::ffi::OsStrExt;

    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    let non_unicode_path = std::ffi::OsStr::from_bytes(b"old-\xFF.txt");
    fs::write(temp.path().join(non_unicode_path), b"old").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .arg("prepare-text")
        .arg("source.txt")
        .arg("--edit-files")
        .arg(non_unicode_path)
        .arg("new.txt")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("input path is not valid Unicode"));
    assert_eq!(fs::read(temp.path().join("source.txt")).unwrap(), b"old\n");
}

#[test]
fn cli_edit_files_preserves_powershell_text_and_orders_repeated_pairs() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source.txt");
    let old_first = temp.path().join("old-first.txt");
    let new_first = temp.path().join("new-first.txt");
    let old_second = temp.path().join("old-second.txt");
    let new_second = temp.path().join("new-second.txt");
    let original = b"item=old\r\nquoted=\"value ${TOKEN} C:\\tmp\"\r\n";
    fs::write(&source, original).unwrap();
    fs::write(&old_first, "item=old\n").unwrap();
    fs::write(&new_first, "item=new\n").unwrap();
    fs::write(&old_second, r#"quoted="value ${TOKEN} C:\tmp""#).unwrap();
    fs::write(&new_second, r#"quoted="next ${TOKEN} D:\data""#).unwrap();

    let prepared = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit-files",
            "old-first.txt",
            "new-first.txt",
            "--edit-files",
            "old-second.txt",
            "new-second.txt",
            "--output",
            "plan.json",
            "--summary",
        ])
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert!(String::from_utf8_lossy(&prepared.stdout).contains("Exact edits    2"));
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&old_first).unwrap(), b"item=old\n");
    assert_eq!(
        fs::read(&new_second).unwrap(),
        r#"quoted="next ${TOKEN} D:\data""#.as_bytes()
    );

    let applied = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan", "plan.json", "--summary"])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        b"item=new\r\nquoted=\"next ${TOKEN} D:\\data\"\r\n"
    );
}

#[test]
fn cli_edit_files_refuses_invalid_utf8_oversize_and_outside_inputs() {
    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    fs::write(temp.path().join("old.txt"), [0xff]).unwrap();
    fs::write(temp.path().join("new.txt"), b"new").unwrap();

    let invalid_utf8 = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit-files",
            "old.txt",
            "new.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(invalid_utf8.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid_utf8.stderr).contains("not valid UTF-8"));

    fs::write(outside.path().join("old.txt"), b"old").unwrap();
    let outside_input = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "source.txt", "--edit-files"])
        .arg(outside.path().join("old.txt"))
        .arg("new.txt")
        .output()
        .unwrap();
    assert_eq!(outside_input.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&outside_input.stderr).contains("input path refused"));
    assert_eq!(fs::read(temp.path().join("source.txt")).unwrap(), b"old\n");

    let oversize = vec![b'x'; threadmoth::protocol::MAX_REQUEST_BYTES + 1];
    fs::write(temp.path().join("large.txt"), oversize).unwrap();
    let too_large = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit-files",
            "large.txt",
            "new.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(too_large.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&too_large.stderr).contains("exceeds"));
}

#[test]
fn cli_edit_files_supports_explicit_exact_mixed_newlines() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"alpha\r\nbeta\n").unwrap();
    fs::write(temp.path().join("old.txt"), b"alpha\r\nbeta\n").unwrap();
    fs::write(temp.path().join("new.txt"), b"gamma\r\nbeta\n").unwrap();
    let prepared = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit-files",
            "old.txt",
            "new.txt",
            "--newline-policy",
            "exact",
            "--output",
            "plan.json",
        ])
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert_eq!(
        fs::read(temp.path().join("source.txt")).unwrap(),
        b"alpha\r\nbeta\n"
    );
    let applied = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["apply-plan", "--plan", "plan.json"])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        fs::read(temp.path().join("source.txt")).unwrap(),
        b"gamma\r\nbeta\n"
    );
}

#[test]
fn cli_prepare_text_rejects_mixed_inline_and_file_input_modes() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\n").unwrap();
    fs::write(temp.path().join("old.txt"), b"old").unwrap();
    fs::write(temp.path().join("new.txt"), b"new").unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args([
            "prepare-text",
            "source.txt",
            "--edit",
            "old",
            "new",
            "--edit-files",
            "old.txt",
            "new.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("cannot be mixed"));
    assert_eq!(fs::read(temp.path().join("source.txt")).unwrap(), b"old\n");
}

#[test]
fn cli_refusal_reports_proven_newline_evidence_and_file_input_recovery() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("source.txt"), b"old\r\n").unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "source.txt", "--edit"])
        .arg("old\n")
        .arg("new")
        .args(["--newline-policy", "exact"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    let certificate: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(certificate["reason_code"], "TARGET_NOT_FOUND");
    assert!(certificate["diff_summary"]
        .as_str()
        .unwrap()
        .contains("line ending discrepancy"));
    assert!(certificate["recovery"]["remedies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|remedy| remedy["kind"] == "use_file_based_text_input"));
    assert_eq!(
        fs::read(temp.path().join("source.txt")).unwrap(),
        b"old\r\n"
    );

    let summary = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .args(["prepare-text", "source.txt", "--edit"])
        .arg("old\n")
        .arg("new")
        .args([
            "--newline-policy",
            "exact",
            "--output",
            "refused-plan.json",
            "--summary",
        ])
        .output()
        .unwrap();
    assert_eq!(summary.status.code(), Some(2));
    let summary = String::from_utf8_lossy(&summary.stdout);
    assert!(summary.contains("line ending discrepancy"));
    assert!(summary.contains("--edit-files OLD_FILE NEW_FILE"));
    assert!(!temp.path().join("refused-plan.json").exists());
    assert_eq!(
        fs::read(temp.path().join("source.txt")).unwrap(),
        b"old\r\n"
    );
}

#[test]
fn cli_preparation_reads_compact_json_and_does_not_write_the_source() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("cli.txt"), b"old\r\n").unwrap();
    let input = serde_json::json!({
        "file_path":"cli.txt",
        "edits":[{"old":"old\n","new":"new\n"}]
    })
    .to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .current_dir(temp.path())
        .arg("prepare-text")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["outcome"], "PREPARED");
    assert_eq!(response["source_newline_profile"], "crlf");
    assert_eq!(fs::read(temp.path().join("cli.txt")).unwrap(), b"old\r\n");
}

#[test]
fn test_rendermono_reproduction() {
    let pre_bytes = include_bytes!("fixtures/rendermono/mono-preblob.cs");
    let req_str = include_str!("fixtures/rendermono/gui-b1r-mono.json");
    let req: serde_json::Value = serde_json::from_str(req_str).unwrap();

    // Canonicalize to pure LF bytes first regardless of checkout line endings
    let mut lf_bytes = Vec::with_capacity(pre_bytes.len());
    let mut i = 0;
    while i < pre_bytes.len() {
        if pre_bytes[i] == b'\r' && pre_bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
        } else {
            lf_bytes.push(pre_bytes[i]);
            i += 1;
        }
    }

    // 1. Test against LF bytes
    let temp_lf = TempDir::new().unwrap();
    fs::create_dir_all(temp_lf.path().join("src/AgentX.App")).unwrap();
    fs::write(
        temp_lf.path().join("src/AgentX.App/MarkdownView.cs"),
        &lf_bytes,
    )
    .unwrap();
    let ws_lf = Workspace::new(temp_lf.path()).unwrap();
    let req_obj: Request = serde_json::from_value(req).unwrap();
    let cert_lf = threadmoth::pipeline::execute_request(&ws_lf, &req_obj, true);

    // 2. Test against CRLF bytes
    let mut crlf_bytes = Vec::with_capacity(lf_bytes.len() * 2);
    for &b in &lf_bytes {
        if b == b'\n' {
            crlf_bytes.push(b'\r');
        }
        crlf_bytes.push(b);
    }
    let temp_crlf = TempDir::new().unwrap();
    fs::create_dir_all(temp_crlf.path().join("src/AgentX.App")).unwrap();
    fs::write(
        temp_crlf.path().join("src/AgentX.App/MarkdownView.cs"),
        &crlf_bytes,
    )
    .unwrap();
    let ws_crlf = Workspace::new(temp_crlf.path()).unwrap();
    let cert_crlf = threadmoth::pipeline::execute_request(&ws_crlf, &req_obj, true);

    assert_eq!(cert_lf.outcome, Outcome::Applied);
    assert_eq!(cert_crlf.outcome, Outcome::Applied);
}
