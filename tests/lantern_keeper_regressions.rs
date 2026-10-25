use std::fs;

use tempfile::tempdir;
use threadmoth::{
    metadata,
    pipeline::execute_request,
    protocol::{Cardinality, EffectBudget, OperationPayload, Outcome, Request, PROTOCOL_VERSION},
    provider::{patch::PatchOperation, text::TextOperation},
    workspace::Workspace,
};

fn request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "lantern-keeper-regression".into(),
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

#[test]
fn capability_name_matches_canonical_filesystem_request_provider() {
    let providers = metadata::provider_metadata();
    assert!(providers
        .iter()
        .any(|provider| provider.name == "filesystem"));
    assert!(!providers.iter().any(|provider| provider.name == "file"));

    let legacy = r#"{
        "version":"1.1.0",
        "file_path":"new.txt",
        "expected_pre_hash":null,
        "operation":{
            "provider":"file",
            "operation":{"type":"create_file","expected_absent":true,"content":[104,105,10]}
        }
    }"#;
    let parsed: Request =
        serde_json::from_str(legacy).expect("legacy file provider remains accepted");
    let rendered = serde_json::to_string(&parsed).unwrap();
    assert!(rendered.contains("\"provider\":\"filesystem\""));
}

#[test]
fn crlf_normalization_changes_only_line_endings() {
    let temp = tempdir().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let path = "crlf.rs";
    let original = b"\xEF\xBB\xBFfn one() {}\r\nfn two() {}\r\n";
    fs::write(temp.path().join(path), original).unwrap();

    let mut req = request(
        path,
        OperationPayload::Text(TextOperation::Replace {
            target: "\r\n".into(),
            replacement: "\n".into(),
        }),
    );
    req.cardinality = Cardinality::All;

    let certificate = execute_request(&workspace, &req, false);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(certificate.effect.matches, 2);
    assert_eq!(certificate.effect.changed_regions, 2);
    assert!(!certificate.preservation.unrelated_bytes_changed);
    assert!(certificate.preservation.line_endings_changed);
    assert!(!certificate.preservation.bom_changed);
    assert!(!certificate.preservation.final_newline_changed);
    assert_eq!(
        fs::read(temp.path().join(path)).unwrap(),
        b"\xEF\xBB\xBFfn one() {}\nfn two() {}\n"
    );
}

#[test]
fn multi_hunk_patch_refuses_small_budget_then_applies_without_whole_file_churn() {
    let temp = tempdir().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let path = "sample.rs";
    let original = b"fn one() {  }\n\nfn untouched() {}\n\nfn two() {  }\n";
    fs::write(temp.path().join(path), original).unwrap();

    let patch = concat!(
        "--- a/sample.rs\n",
        "+++ b/sample.rs\n",
        "@@ -1,1 +1,1 @@\n",
        "-fn one() {  }\n",
        "+fn one() {}\n",
        "@@ -5,1 +5,1 @@\n",
        "-fn two() {  }\n",
        "+fn two() {}\n",
    );
    let mut req = request(
        path,
        OperationPayload::Patch(PatchOperation::UnifiedDiff {
            patch: patch.into(),
        }),
    );
    req.budget.max_changed_regions = Some(1);

    let refused = execute_request(&workspace, &req, false);
    assert_eq!(refused.outcome, Outcome::Refused);
    assert_eq!(
        refused.reason_code.as_deref(),
        Some("EFFECT_BUDGET_EXCEEDED")
    );
    assert_eq!(refused.effect.changed_regions, 2);
    assert!(!refused.effect.passed);
    assert_eq!(fs::read(temp.path().join(path)).unwrap(), original);

    req.budget.max_changed_regions = Some(2);
    let applied = execute_request(&workspace, &req, false);
    assert_eq!(applied.outcome, Outcome::Applied);
    assert_eq!(applied.effect.changed_regions, 2);
    assert!(!applied.preservation.unrelated_bytes_changed);

    let result = fs::read_to_string(temp.path().join(path)).unwrap();
    assert_eq!(result, "fn one() {}\n\nfn untouched() {}\n\nfn two() {}\n");
    assert!(result.contains("\n\nfn untouched() {}\n\n"));
}

#[test]
fn mixed_newline_and_format_drift_can_be_repaired_as_bounded_sequential_operations() {
    let temp = tempdir().unwrap();
    let workspace = Workspace::new(temp.path()).unwrap();
    let path = "mixed.rs";
    fs::write(
        temp.path().join(path),
        b"fn one() {  }\r\n\r\nfn untouched() {}\r\n",
    )
    .unwrap();

    let mut newline = request(
        path,
        OperationPayload::Text(TextOperation::Replace {
            target: "\r\n".into(),
            replacement: "\n".into(),
        }),
    );
    newline.cardinality = Cardinality::All;
    assert_eq!(
        execute_request(&workspace, &newline, false).outcome,
        Outcome::Applied
    );

    let patch = concat!(
        "--- a/mixed.rs\n",
        "+++ b/mixed.rs\n",
        "@@ -1,1 +1,1 @@\n",
        "-fn one() {  }\n",
        "+fn one() {}\n",
    );
    let format = request(
        path,
        OperationPayload::Patch(PatchOperation::UnifiedDiff {
            patch: patch.into(),
        }),
    );
    let certificate = execute_request(&workspace, &format, false);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(certificate.effect.changed_regions, 1);
    assert_eq!(
        fs::read(temp.path().join(path)).unwrap(),
        b"fn one() {}\n\nfn untouched() {}\n"
    );
}
