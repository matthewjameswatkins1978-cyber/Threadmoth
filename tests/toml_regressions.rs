use threadmoth::engine::apply_byte_edits;
use threadmoth::protocol::{Cardinality, RefusalReason};
use threadmoth::provider::toml::{
    TomlOperation, TomlProvider, TomlProviderError, TomlValueWrapper,
};

#[test]
fn toml_occupied_key_insertion_refuses_without_overwriting() {
    let content = br#"[package]
name = "threadmoth"
version = "1.0.0"
"#;
    let op = TomlOperation::Insert {
        path: "package".to_string(),
        key: "version".to_string(),
        value: TomlValueWrapper::String("2.0.0".to_string()),
    };
    let res = TomlProvider::plan(content, &op, &Cardinality::ExactlyOne);
    assert!(matches!(
        res,
        Err(TomlProviderError::Refused(RefusalReason::DestinationExists { path })) if path == "package.version"
    ));
}

#[test]
fn toml_rename_collision_refuses_without_overwriting() {
    let content = br#"[package]
old_name = "legacy"
name = "occupied"
"#;
    let op = TomlOperation::RenameKey {
        path: "package.old_name".to_string(),
        new_key: "name".to_string(),
    };
    let res = TomlProvider::plan(content, &op, &Cardinality::ExactlyOne);
    assert!(matches!(
        res,
        Err(TomlProviderError::Refused(RefusalReason::DestinationExists { path })) if path == "name"
    ));
}

#[test]
fn toml_inline_comment_and_spacing_preservation() {
    let content = br#"# Configuration file
[package]
name    =    "threadmoth"
version = "1.0.0" # release version comment
"#;
    let op = TomlOperation::Set {
        path: "package.version".to_string(),
        value: TomlValueWrapper::String("1.0.1".to_string()),
    };
    let edits = TomlProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
    let modified = apply_byte_edits(content, &edits).unwrap();
    let expected = b"# Configuration file
[package]
name    =    \"threadmoth\"
version = \"1.0.1\" # release version comment
";
    assert_eq!(modified, expected);
}

#[test]
fn toml_crlf_preservation_and_exact_byte_edit() {
    let content = b"[package]\r\nname = \"threadmoth\"\r\nversion = \"1.0.0\"\r\n";
    let op = TomlOperation::Set {
        path: "package.version".to_string(),
        value: TomlValueWrapper::String("1.0.1".to_string()),
    };
    let edits = TomlProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
    let modified = apply_byte_edits(content, &edits).unwrap();
    let expected = b"[package]\r\nname = \"threadmoth\"\r\nversion = \"1.0.1\"\r\n";
    assert_eq!(modified, expected);
    assert!(modified.windows(2).any(|w| w == b"\r\n"));
}
