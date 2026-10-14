use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_threadmoth"))
        .args(args)
        .output()
        .expect("CLI should start")
}

#[test]
fn orientation_has_compact_human_and_json_forms() {
    let json = cli(&["orient", "--json"]);
    assert!(json.status.success());
    let parsed: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        parsed["workflow"],
        serde_json::json!(["preview", "mutate", "verify"])
    );
    assert_eq!(parsed["confinement"], "workspace-and-budget");
    assert_eq!(
        parsed["text_edit"]["cli"],
        "threadmoth prepare-text PATH --edit OLD NEW --output PLAN --summary"
    );
    assert_eq!(
        parsed["text_edit"]["cli_file_input"],
        "threadmoth prepare-text PATH --edit-files OLD_FILE NEW_FILE --output PLAN --summary"
    );
    assert_eq!(
        parsed["text_edit"]["cli_multi_file"],
        "threadmoth prepare-edits CHANGES --output PLAN --review"
    );
    assert_eq!(
        parsed["text_edit"]["review"],
        "prepare-edits --review shows the bounded diff; use explain --plan PLAN --format diff --full if truncated"
    );
    assert_eq!(
        parsed["text_edit"]["apply"],
        "threadmoth apply-plan --plan PLAN --summary"
    );

    let human = cli(&["orient"]);
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("preview → mutate → verify certificate"));
    assert!(text.contains("refusal 2"));
}

#[test]
fn source_text_example_and_prepare_help_teach_the_short_route() {
    let example = cli(&["examples", "source-text"]);
    assert!(example.status.success());
    let example: Value = serde_json::from_slice(&example.stdout).unwrap();
    assert!(example["prepare"]
        .as_str()
        .unwrap()
        .contains("--edit OLD NEW"));
    assert!(example["prepare_from_files"]
        .as_str()
        .unwrap()
        .contains("--edit-files OLD_FILE NEW_FILE"));
    assert!(example["review"]
        .as_str()
        .unwrap()
        .contains("explain --plan"));
    assert!(example["apply"]
        .as_str()
        .unwrap()
        .contains("apply-plan --plan"));

    let help = cli(&["prepare-text", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--edit <OLD> <NEW>"));
    assert!(help.contains("--edit-files <OLD_FILE> <NEW_FILE>"));
    assert!(help.contains("Then review:"));
    assert!(help.contains("Then apply:"));
}

#[test]
fn schema_scope_returns_only_the_requested_contract_and_rejects_unknown_scope() {
    let request = cli(&["schema", "request", "--json"]);
    assert!(request.status.success());
    let request_schema: Value = serde_json::from_slice(&request.stdout).unwrap();
    assert_eq!(request_schema["scope"], "request");
    assert!(request_schema.get("request").is_some());
    assert!(request_schema.get("response").is_none());

    let all = cli(&["schema", "--json"]);
    assert!(all.status.success());
    let all_schema: Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(all_schema["scope"], "all");
    assert!(all_schema.get("request").is_some());
    assert!(all_schema.get("response").is_some());
    assert!(request.stdout.len() < all.stdout.len());

    let invalid = cli(&["schema", "edits", "--json"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown schema scope"));
}
#[test]
fn inspect_json_alias_matches_default_for_identity_outline_and_expansion() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("sample.rs"),
        "fn first() { let value = 1; }\nfn second() { let value = 2; }\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_threadmoth"))
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap()
    };

    for (plain, with_json) in [
        (
            vec!["inspect", "sample.rs"],
            vec!["inspect", "sample.rs", "--json"],
        ),
        (
            vec!["inspect", "sample.rs", "--outline"],
            vec!["inspect", "sample.rs", "--outline", "--json"],
        ),
    ] {
        let plain = run(&plain);
        let with_json = run(&with_json);
        assert!(plain.status.success());
        assert!(with_json.status.success());
        assert_eq!(plain.stdout, with_json.stdout);
    }

    let outline: Value =
        serde_json::from_slice(&run(&["inspect", "sample.rs", "--outline"]).stdout).unwrap();
    let handle = outline["outline"]["entries"][0]["handle"].as_str().unwrap();
    let plain = run(&["inspect", "sample.rs", "--expand", handle]);
    let with_json = run(&["inspect", "sample.rs", "--expand", handle, "--json"]);
    assert!(plain.status.success());
    assert!(with_json.status.success());
    assert_eq!(plain.stdout, with_json.stdout);
}

#[test]
fn provider_and_operation_help_are_targeted_and_machine_readable() {
    let provider = cli(&["help", "json", "--json"]);
    assert!(provider.status.success());
    let provider_json: Value = serde_json::from_slice(&provider.stdout).unwrap();
    assert_eq!(provider_json["provider"], "json");
    assert!(provider_json["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["operation"] == "set"));

    let operation = cli(&["help", "markdown:replace_section", "--json"]);
    assert!(operation.status.success());
    let operation_json: Value = serde_json::from_slice(&operation.stdout).unwrap();
    assert_eq!(operation_json["provider"], "markdown");
    assert_eq!(operation_json["operation"], "replace_section");
    assert_eq!(
        operation_json["selector_forms"],
        serde_json::json!(["heading", "section", "fenced_region"])
    );
    assert_eq!(
        operation_json["example"]["operation"]["operation"]["type"],
        "replace_section"
    );
}

#[test]
fn selector_is_canonical_and_at_remains_a_cli_compatibility_alias() {
    for selector_flag in ["--selector", "--at"] {
        let output = cli(&[
            "suggest",
            "Cargo.toml",
            "--goal",
            "set-value",
            selector_flag,
            "package.name",
        ]);
        assert!(output.status.success());
        let suggestion: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(suggestion["provider"], "toml");
        assert_eq!(suggestion["recommended_operation"], "set");
        assert_eq!(suggestion["request_template"]["file_path"], "Cargo.toml");
    }
}
