use schemars::schema_for;
use threadmoth::protocol::{Certificate, Request, PROTOCOL_VERSION};

#[test]
fn exported_protocol_schema_has_a_golden_digest() {
    let value = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("Threadmoth {} Protocol Schemas", env!("CARGO_PKG_VERSION")),
        "protocol_version": PROTOCOL_VERSION,
        "request": schema_for!(Request),
        "certificate": schema_for!(Certificate)
    });
    let rendered = serde_json::to_string_pretty(&value).unwrap();
    let digest = threadmoth::engine::compute_sha256(format!("{rendered}\n").as_bytes());
    assert_eq!(
        digest,
        "c80efea63d2d4293465d7ba7d54e1835c697c21e3733d2579f03b8f3a2bfcb63"
    );
}
