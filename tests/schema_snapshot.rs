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
        "8c9258e9425cdd12c48cb04494e575cae2e35bd7ac46e6b0c55edfa226e21066"
    );
}
