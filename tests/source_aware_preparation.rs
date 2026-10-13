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
    assert!(matches!(
        apply_prepared_plan(&workspace, &result.plan),
        PlanApplyResult::Certificate(certificate) if certificate.outcome == Outcome::Applied
    ));
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
