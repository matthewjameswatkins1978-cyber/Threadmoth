use std::process::Command;
use tempfile::TempDir;
use threadmoth::pipeline::execute_request;
use threadmoth::protocol::{
    Cardinality, EffectBudget, OperationPayload, Outcome, RefusalReason, Request, PROTOCOL_VERSION,
};
use threadmoth::provider::json::JsonOperation;
use threadmoth::provider::text::TextOperation;
use threadmoth::workspace::Workspace;

fn request(path: String, operation: OperationPayload, prefix: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "teachback-regression".into(),
        allow_generated: false,
        file_path: path,
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            allowed_path_prefixes: vec![prefix.into()],
            ..EffectBudget::default()
        },
        operation,
    }
}

#[test]
fn workspace_root_refusal_teaches_choice_gated_recovery() {
    let bridge_root = TempDir::new().unwrap();
    std::fs::create_dir(bridge_root.path().join("allowed")).unwrap();
    let target_checkout = TempDir::new().unwrap();
    std::fs::create_dir(target_checkout.path().join("allowed")).unwrap();
    let target = target_checkout.path().join("allowed/outside.txt");
    std::fs::write(&target, b"before\n").unwrap();
    let bridge = Workspace::new(bridge_root.path()).unwrap();
    let mismatch_request = request(
        target.to_string_lossy().into_owned(),
        OperationPayload::Text(TextOperation::Replace {
            target: "before".into(),
            replacement: "after".into(),
        }),
        "allowed",
    );

    let refusal = execute_request(&bridge, &mismatch_request, false);
    assert_eq!(refusal.outcome, Outcome::Refused);
    assert!(matches!(
        refusal.refusal_reason,
        Some(RefusalReason::WorkspaceRootMismatch { .. })
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"before\n");
    let reason = threadmoth::metadata::reason("WORKSPACE_ROOT_MISMATCH").unwrap();
    assert!(reason.relevant_commands.contains(&"suggest"));
    let recovery = refusal.recovery.unwrap();
    assert!(recovery.requires_choice);
    assert_eq!(recovery.remedies[0].kind, "checkout_local_cli");
    assert!(recovery.remedies[0].description.contains(
        &bridge_root
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    ));
    assert!(recovery.remedies[0]
        .description
        .contains(&target.display().to_string()));
    assert!(recovery.remedies[0].request_patch.is_none());

    let selected_checkout = Workspace::new(target_checkout.path()).unwrap();
    let selected = execute_request(
        &selected_checkout,
        &request(
            "allowed/outside.txt".into(),
            OperationPayload::Text(TextOperation::Replace {
                target: "before".into(),
                replacement: "after".into(),
            }),
            "allowed",
        ),
        false,
    );
    assert_eq!(selected.outcome, Outcome::Applied);
    assert_eq!(std::fs::read(target).unwrap(), b"after\n");
}

#[test]
fn suggest_templates_match_detected_json_markdown_and_code_providers() {
    for (path, provider, operation) in [
        ("config.json", "json", "set"),
        (".env", "dotenv", "set"),
        ("README.md", "markdown", "replace_section"),
        ("main.rs", "code", "replace_node"),
    ] {
        let suggestion = threadmoth::metadata::suggest(path, None, None, "safe", None);
        assert_eq!(suggestion.provider, provider);
        let template = suggestion.request_template.unwrap();
        assert_eq!(template["operation"]["provider"], provider);
        assert_eq!(template["operation"]["operation"]["type"], operation);
    }
}

#[test]
fn unsupported_markdown_goals_do_not_fall_back_to_generic_text() {
    for goal in ["set-value", "rename", "add-item", "move"] {
        let suggestion =
            threadmoth::metadata::suggest("README.md", Some(goal), Some("Install"), "safe", None);
        assert_eq!(suggestion.provider, "markdown");
        assert!(suggestion.request_template.is_none(), "{goal}");
        assert!(suggestion
            .blocked_reasons
            .iter()
            .any(|reason| reason.contains(goal)));
        assert!(suggestion
            .rationale
            .contains(&format!("no safe template for goal {goal}")));
        assert!(suggestion.alternatives.iter().any(|choice| {
            choice["provider"] == "markdown" && choice["operation"] == "replace_section"
        }));
    }
}

#[test]
fn json_selector_metadata_and_runtime_diagnostic_agree() {
    let manifest = threadmoth::metadata::capabilities();
    assert!(!manifest.selectors.contains(&"json_pointer"));
    assert!(manifest.selectors.contains(&"dotted_key"));
    for provider_name in ["json", "jsonc"] {
        let provider = manifest
            .providers
            .iter()
            .find(|entry| entry.name == provider_name)
            .unwrap();
        assert!(!provider.selectors.contains(&"json_pointer"));
        assert!(provider.selectors.contains(&"dotted_key"));
    }

    let temp = TempDir::new().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let file = temp.path().join("config.json");
    std::fs::write(&file, br#"{"port":80,"service":{"port":8080}}"#).unwrap();
    let dotted = execute_request(
        &workspace,
        &request(
            "config.json".into(),
            OperationPayload::Json(JsonOperation::Set {
                path: "$.service.port".into(),
                value: serde_json::json!(9090),
            }),
            "config.json",
        ),
        false,
    );
    assert_eq!(dotted.outcome, Outcome::Applied);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        r#"{"port":80,"service":{"port":9090}}"#
    );

    let pointer = execute_request(
        &workspace,
        &request(
            "config.json".into(),
            OperationPayload::Json(JsonOperation::Set {
                path: "/port".into(),
                value: serde_json::json!(81),
            }),
            "config.json",
        ),
        false,
    );
    assert_eq!(pointer.outcome, Outcome::Refused);
    match pointer.refusal_reason.unwrap() {
        RefusalReason::MalformedInput { details } => {
            assert!(details.contains("JSON Pointer selectors are not supported"));
            assert!(details.contains("$.port"));
        }
        other => panic!("unexpected pointer diagnostic: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        r#"{"port":80,"service":{"port":9090}}"#
    );
}

#[test]
fn cli_process_exit_codes_match_the_documented_contract() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("edit.txt"), b"before\n").unwrap();
    let applied = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["replace-exact", "edit.txt", "before", "after"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));

    std::fs::write(temp.path().join("stable.txt"), b"present\n").unwrap();
    let noop = request(
        "stable.txt".into(),
        OperationPayload::Text(TextOperation::EnsurePresent {
            content: "present\n".into(),
        }),
        "stable.txt",
    );
    let request_path = temp.path().join("noop.json");
    std::fs::write(&request_path, serde_json::to_vec(&noop).unwrap()).unwrap();
    let no_change = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["mutate", "--request", request_path.to_str().unwrap()])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(no_change.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&no_change.stdout).contains("NO_CHANGE"));

    let refused = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["replace-exact", "missing.txt", "before", "after"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));

    let runtime_failure = Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(["preview", "--request", "missing-request.json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(runtime_failure.status.code(), Some(3));
}
