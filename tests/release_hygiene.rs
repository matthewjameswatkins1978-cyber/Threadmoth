use threadmoth::{metadata, protocol::PROTOCOL_VERSION, PACKAGE_VERSION};

#[test]
fn package_and_runtime_version_are_authoritative_and_protocol_is_separate() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.13.0");
    assert_eq!(PACKAGE_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata::capabilities().threadmoth_version, PACKAGE_VERSION);
    assert_eq!(PROTOCOL_VERSION, "1.3.1");
}

#[test]
fn shipped_agent_manifests_match_the_package_version() {
    let package_version = env!("CARGO_PKG_VERSION");
    let antigravity: serde_json::Value =
        serde_json::from_str(include_str!("../plugin.json")).unwrap();
    let gemini: serde_json::Value =
        serde_json::from_str(include_str!("../gemini-extension.json")).unwrap();
    assert_eq!(antigravity["version"], package_version);
    assert_eq!(gemini["version"], package_version);
    assert!(include_str!("../skills/threadmoth/SKILL.md")
        .contains(&format!("version: \"{package_version}\"")));
}
