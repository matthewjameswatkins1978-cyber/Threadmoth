use std::fs;

use serde_json::Value;
use tempfile::tempdir;
use threadmoth::{
    metadata::{inspect, inspect_view, ObservationCache},
    workspace::Workspace,
};

fn outline(workspace: &Workspace, path: &str) -> Value {
    inspect_view(workspace, path, Some("outline"), None, None, Some(64), None).unwrap()
}

#[test]
fn identity_view_keeps_the_191_shape() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("sample.rs"), "fn main() {}\n").unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let value = inspect(&workspace, "sample.rs").unwrap();
    assert_eq!(value["file_path"], "sample.rs");
    assert!(value.get("view").is_none());
    assert!(value.get("outline").is_none());
}

#[test]
fn outline_is_bounded_deterministic_and_expands_exactly() {
    let directory = tempdir().unwrap();
    let source = "fn first() {\n    let value = 1;\n}\n\nfn second() {\n    let value = 2;\n}\n";
    fs::write(directory.path().join("sample.rs"), source).unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let first = outline(&workspace, "sample.rs");
    let second = outline(&workspace, "sample.rs");
    assert_eq!(first["outline"]["entries"], second["outline"]["entries"]);
    let entries = first["outline"]["entries"].as_array().unwrap();
    assert!(entries.len() >= 2);
    for entry in entries {
        assert!(entry["handle"]
            .as_str()
            .unwrap()
            .starts_with("threadmoth:observation:v1|"));
        assert!(entry["label"].as_str().unwrap().len() <= 100);
        let handle = entry["handle"].as_str().unwrap();
        let expanded = inspect_view(
            &workspace,
            "sample.rs",
            Some("expand"),
            Some(handle),
            Some(4096),
            None,
            None,
        )
        .unwrap();
        let start = entry["start_byte"].as_u64().unwrap() as usize;
        let end = entry["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(expanded["expansion"]["source"], &source[start..end]);
    }
}

#[test]
fn stale_handles_refuse_without_mutating_the_file() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("sample.rs"), "fn first() {}\n").unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let value = outline(&workspace, "sample.rs");
    let handle = value["outline"]["entries"][0]["handle"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(directory.path().join("sample.rs"), "fn changed() {}\n").unwrap();
    let error = inspect_view(
        &workspace,
        "sample.rs",
        Some("expand"),
        Some(&handle),
        None,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("stale observation handle"));
    assert_eq!(
        fs::read_to_string(directory.path().join("sample.rs")).unwrap(),
        "fn changed() {}\n"
    );
}

#[test]
fn cache_reuses_only_an_unchanged_outline() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("sample.rs"), "fn first() {}\n").unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let mut cache = ObservationCache::new(2, 4096);
    let first = inspect_view(
        &workspace,
        "sample.rs",
        Some("outline"),
        None,
        None,
        None,
        Some(&mut cache),
    )
    .unwrap();
    let second = inspect_view(
        &workspace,
        "sample.rs",
        Some("outline"),
        None,
        None,
        None,
        Some(&mut cache),
    )
    .unwrap();
    assert_eq!(first["outline"]["reuse"], "derived");
    assert_eq!(second["outline"]["reuse"], "cache_hit");
    assert_eq!(cache.hits(), 1);
    fs::write(directory.path().join("sample.rs"), "fn second() {}\n").unwrap();
    let third = inspect_view(
        &workspace,
        "sample.rs",
        Some("outline"),
        None,
        None,
        None,
        Some(&mut cache),
    )
    .unwrap();
    assert_eq!(third["outline"]["reuse"], "derived");
}

#[test]
fn unsupported_outline_is_honest_and_malformed_source_fails_closed() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("sample.txt"), "plain text\n").unwrap();
    fs::write(directory.path().join("broken.rs"), "fn broken( {\n").unwrap();
    let workspace = Workspace::new(directory.path()).unwrap();
    let unsupported = outline(&workspace, "sample.txt");
    assert_eq!(unsupported["outline"]["available"], false);
    assert_eq!(unsupported["outline"]["reason"], "unsupported_provider");
    let error = inspect_view(
        &workspace,
        "broken.rs",
        Some("outline"),
        None,
        None,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("outline unavailable"));
}
