use threadmoth::{metadata, protocol::PROTOCOL_VERSION, PACKAGE_VERSION};

#[test]
fn package_and_runtime_version_are_authoritative_and_protocol_is_separate() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.10.0");
    assert_eq!(PACKAGE_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata::capabilities().threadmoth_version, PACKAGE_VERSION);
    assert_eq!(PROTOCOL_VERSION, "1.3.1");
}
