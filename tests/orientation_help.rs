use serde_json::Value;
use std::process::Command;

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

    let human = cli(&["orient"]);
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("preview → mutate → verify certificate"));
    assert!(text.contains("refusal 2"));
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
