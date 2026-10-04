use serde_json::json;
use tempfile::TempDir;
use threadmoth::engine::compute_sha256;
use threadmoth::lifecycle::FileOperation;
use threadmoth::pattern::PatternOperation;
use threadmoth::pipeline::execute_request;
use threadmoth::protocol::{
    Cardinality, EffectBudget, OperationPayload, Outcome, RegionGuard, RegionGuardMode, Request,
    TransactionRequest, PROTOCOL_VERSION,
};
use threadmoth::provider::code::CodeOperation;
use threadmoth::provider::json::JsonOperation;
use threadmoth::provider::jsonc::JsoncProvider;
use threadmoth::provider::markdown::MarkdownOperation;
use threadmoth::provider::patch::PatchOperation;
use threadmoth::provider::text::{TextOperation, TextProvider};
use threadmoth::provider::yaml::YamlOperation;
use threadmoth::workspace::Workspace;

#[test]
fn canonical_examples_are_current_protocol_values() {
    for example in threadmoth::metadata::examples(None) {
        if example.request.get("requests").is_some() {
            let transaction: TransactionRequest = serde_json::from_value(example.request).unwrap();
            assert_eq!(transaction.version, PROTOCOL_VERSION, "{}", example.topic);
        } else {
            let request: Request = serde_json::from_value(example.request).unwrap();
            assert_eq!(request.version, PROTOCOL_VERSION, "{}", example.topic);
        }
    }
}

#[test]
fn every_public_reason_code_is_explainable() {
    let manifest = threadmoth::metadata::capabilities();
    for reason in manifest.reason_codes {
        assert_eq!(
            threadmoth::metadata::reason(reason.code).unwrap().code,
            reason.code
        );
    }
}

#[test]
fn every_provider_operation_is_in_canonical_operation_metadata() {
    let operations: std::collections::HashSet<_> = threadmoth::metadata::operation_metadata()
        .into_iter()
        .map(|operation| operation.name)
        .collect();
    for provider in threadmoth::metadata::provider_metadata() {
        for operation in provider.operations {
            assert!(
                operations.contains(operation),
                "{}.{operation}",
                provider.name
            );
        }
    }
}

#[test]
fn targeted_help_projects_provider_selectors_and_executable_examples() {
    let help = threadmoth::metadata::operation_help("json", "set").unwrap();
    assert_eq!(help["provider"], "json");
    assert_eq!(help["operation"], "set");
    assert_eq!(help["selector"], "dotted_key");
    assert_eq!(help["selector_forms"], serde_json::json!(["dotted_key"]));
    assert_eq!(help["example"]["operation"]["provider"], "json");
    assert_eq!(help["example"]["operation"]["operation"]["type"], "set");
    assert!(threadmoth::metadata::operation_help("filesystem", "set").is_none());
    let markdown = threadmoth::metadata::operation_help("markdown", "replace_section").unwrap();
    assert_eq!(
        markdown["selector_forms"],
        serde_json::json!(["heading", "section", "fenced_region"])
    );
    assert_eq!(markdown["selector"], "heading");
    let ini = threadmoth::metadata::operation_help("ini", "ensure_section").unwrap();
    assert_eq!(ini["description"], "Ensure one INI section exists.");
    assert_eq!(ini["selector"], "section");
    assert_eq!(
        ini["example"]["operation"]["operation"]["type"],
        "ensure_section"
    );
}

#[test]
fn operation_help_cannot_diverge_from_advertised_registry_pairs() {
    for provider in threadmoth::metadata::provider_metadata() {
        for operation in &provider.operations {
            let help = threadmoth::metadata::operation_help(provider.name, operation)
                .unwrap_or_else(|| panic!("missing help for {}:{operation}", provider.name));
            assert_eq!(help["provider"], provider.name);
            assert_eq!(help["operation"], *operation);
            assert_eq!(
                help["selector_forms"],
                serde_json::to_value(&provider.selectors).unwrap()
            );
            assert_eq!(help["required_parameters"][0], "file_path");
            if let Some(example) = help["example"].as_object() {
                assert_eq!(example["operation"]["provider"], provider.name);
                assert_eq!(example["operation"]["operation"]["type"], *operation);
            }
        }
    }
}

#[test]
fn orientation_is_compact_versioned_and_describes_the_existing_authority_boundary() {
    let orientation = threadmoth::metadata::orientation("C:/checkout");
    assert_eq!(orientation["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(orientation["workspace"], "C:/checkout");
    assert_eq!(orientation["confinement"], "workspace-and-budget");
    assert_eq!(
        orientation["workflow"],
        serde_json::json!(["preview", "mutate", "verify"])
    );
    assert!(orientation["providers"]
        .as_array()
        .unwrap()
        .contains(&json!("markdown")));
}

#[test]
fn capability_operation_selection_refuses_unadvertised_pair() {
    let view = threadmoth::metadata::capability_view(Some("filesystem.set"));
    assert_eq!(view["providers"].as_array().unwrap().len(), 1);
    assert_eq!(view["providers"][0]["operation_supported"], false);
    assert_eq!(view["selection_error"]["operation"], "set");
    assert!(view.get("selected_operation").is_none());
}

#[test]
fn ambiguous_suggestion_never_selects_the_first_provider() {
    let suggestion = threadmoth::metadata::suggest(
        "config.unknown",
        Some("set-value"),
        Some("$.name"),
        "safe",
        Some(br#"{"name":"old"}"#),
    );
    assert_eq!(suggestion.provider, "ambiguous");
    assert!(suggestion.request_template.is_none());
    assert!(suggestion
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("ambiguous")));
    assert_eq!(suggestion.alternatives.len(), 3);
}

#[test]
fn capabilities_do_not_advertise_unsupported_lifecycle_transactions() {
    let filesystem = threadmoth::metadata::provider_metadata()
        .into_iter()
        .find(|provider| provider.name == "filesystem")
        .unwrap();
    assert_eq!(filesystem.transaction_support, "single_file");
}

#[test]
fn suggestion_is_schema_valid_and_refusal_recovery_is_machine_readable() {
    let suggestion = threadmoth::metadata::suggest(
        "config.json",
        Some("set-value"),
        Some("$.name"),
        "safe",
        None,
    );
    let suggested_request: Request =
        serde_json::from_value(suggestion.request_template.unwrap()).unwrap();
    assert_eq!(suggested_request.version, PROTOCOL_VERSION);
    let temp = TempDir::new().unwrap();
    let refusal = execute_request(
        &Workspace::new(temp.path()).unwrap(),
        &request(
            "missing.txt",
            OperationPayload::Text(TextOperation::Replace {
                target: "x".into(),
                replacement: "y".into(),
            }),
        ),
        true,
    );
    let recovery = threadmoth::metadata::refusal_recovery(&refusal);
    assert_eq!(recovery["reason_code"], "TARGET_NOT_FOUND");
    assert!(recovery["suggestions"].as_array().unwrap().len() == 1);
}

fn request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "test-v1".into(),
        allow_generated: false,
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
fn capabilities_are_versioned_and_advertise_budgets() {
    let caps = serde_json::to_value(threadmoth::capabilities::current()).unwrap();
    assert_eq!(caps["protocol_versions"][0], PROTOCOL_VERSION);
    assert!(caps["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "jsonc"));
    assert!(
        caps["resource_limits"]["max_request_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        caps["guard_modes"],
        serde_json::json!([
            "immediate",
            "strict_snapshot",
            "region_snapshot",
            "structural_snapshot"
        ])
    );
}

#[test]
fn effect_budget_refuses_before_write() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"one two\n").unwrap();
    let mut r = request(
        "x.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "one".into(),
            replacement: "a much longer replacement".into(),
        }),
    );
    r.budget.max_changed_bytes = Some(2);
    let c = execute_request(&w, &r, false);
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(c.reason_code.as_deref(), Some("EFFECT_BUDGET_EXCEEDED"));
    assert!(!c.effect.passed);
    assert_eq!(w.read_file("x.txt").unwrap(), b"one two\n");
}

#[test]
fn built_in_pattern_resource_limit_is_explicit() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"text\n").unwrap();
    let c = execute_request(
        &w,
        &request(
            "x.txt",
            OperationPayload::Pattern(PatternOperation::Replace {
                pattern: "a".repeat(8_193),
                replacement: "b".into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(c.reason_code.as_deref(), Some("RESOURCE_LIMIT_EXCEEDED"));
    assert_eq!(w.read_file("x.txt").unwrap(), b"text\n");
}

#[test]
fn capability_and_schema_fingerprints_are_deterministic_and_scoped() {
    let first = threadmoth::metadata::capabilities();
    let second = threadmoth::metadata::capabilities();
    assert_eq!(first.capability_set_id, second.capability_set_id);
    let view = threadmoth::metadata::capability_view(Some("json.set"));
    assert_eq!(view["selected_operation"]["name"], "set");
    let schema = threadmoth::metadata::schema(Some("json"));
    assert_eq!(schema["protocol_version"], PROTOCOL_VERSION);
    assert!(schema["schema_id"].as_str().is_some());
}

#[test]
fn desired_state_replay_is_no_change() {
    let first = TextProvider::plan(
        b"a\n",
        &TextOperation::EnsurePresent {
            content: "b".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    let after = threadmoth::engine::apply_byte_edits(b"a\n", &first).unwrap();
    let second = TextProvider::plan(
        &after,
        &TextOperation::EnsurePresent {
            content: "b".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert!(second.is_empty());
    let absent = TextProvider::plan(
        b"a",
        &TextOperation::EnsureAbsent {
            target: "missing".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert!(absent.is_empty());
}

#[test]
fn desired_state_providers_insert_missing_values_and_preserve_newlines() {
    let yaml = b"name: old\r\n";
    let edits = threadmoth::provider::yaml::plan(
        yaml,
        &YamlOperation::EnsurePresent {
            path: "count".into(),
            value: serde_json::json!(1),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert_eq!(
        threadmoth::engine::apply_byte_edits(yaml, &edits).unwrap(),
        b"name: old\r\ncount: 1\r\n"
    );

    let dotenv = b"A=1\r\n";
    let edits = threadmoth::provider::dotenv::plan(
        dotenv,
        &threadmoth::provider::dotenv::DotenvOperation::EnsurePresent {
            key: "B".into(),
            value: "2".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert_eq!(
        threadmoth::engine::apply_byte_edits(dotenv, &edits).unwrap(),
        b"A=1\r\nB=2\r\n"
    );

    let toml = b"name = \"threadmoth\"\n";
    let edits = threadmoth::provider::toml::TomlProvider::plan(
        toml,
        &threadmoth::provider::toml::TomlOperation::EnsurePresent {
            path: "version".into(),
            value: threadmoth::provider::toml::TomlValueWrapper::String("1".into()),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    let result = threadmoth::engine::apply_byte_edits(toml, &edits).unwrap();
    assert!(String::from_utf8_lossy(&result).contains("version = \"1\""));
}

#[test]
fn dotenv_rejects_multiline_values() {
    let result = threadmoth::provider::dotenv::plan(
        b"A=1\n",
        &threadmoth::provider::dotenv::DotenvOperation::Set {
            key: "A".into(),
            value: "bad\nvalue".into(),
        },
        &Cardinality::ExactlyOne,
    );
    assert!(matches!(
        result,
        Err(threadmoth::provider::dotenv::DotenvError::Refused(
            threadmoth::protocol::RefusalReason::MalformedInput { .. }
        ))
    ));
}

#[test]
fn jsonc_comments_are_not_in_edit_ranges() {
    let original = br#"{
  // keep this comment
  "name": "old"
}
"#;
    let edits = JsoncProvider::plan(
        original,
        &JsonOperation::Set {
            path: "$.name".into(),
            value: serde_json::json!("new"),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    let changed = threadmoth::engine::apply_byte_edits(original, &edits).unwrap();
    assert!(String::from_utf8_lossy(&changed).contains("keep this comment"));
}

#[test]
fn markdown_yaml_and_lifecycle_are_guarded() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("doc.md"), b"# A\nold\n\n# B\nkeep\n").unwrap();
    let c = execute_request(
        &w,
        &request(
            "doc.md",
            OperationPayload::Markdown(MarkdownOperation::ReplaceSection {
                heading: "A".into(),
                content: "new".into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    std::fs::write(
        t.path().join("config.yaml"),
        b"name: old # comment\ncount: 1\n",
    )
    .unwrap();
    let c = execute_request(
        &w,
        &request(
            "config.yaml",
            OperationPayload::Yaml(YamlOperation::Set {
                path: "name".into(),
                value: serde_json::json!("new"),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    let content = b"created\n".to_vec();
    let c = execute_request(
        &w,
        &request(
            "new.txt",
            OperationPayload::File(FileOperation::CreateFile {
                expected_absent: true,
                content: content.clone(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    let c = execute_request(
        &w,
        &request(
            "new.txt",
            OperationPayload::File(FileOperation::DeleteFile {
                expected_hash: compute_sha256(&content),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    assert!(!t.path().join("new.txt").exists());
}

#[test]
fn markdown_ensure_and_insert_do_not_shift_or_duplicate_headings() {
    let missing = b"intro\n";
    let ensure = MarkdownOperation::EnsureSection {
        heading: "Added".into(),
        content: "body".into(),
    };
    let edits =
        threadmoth::provider::markdown::plan(missing, &ensure, &Cardinality::ExactlyOne).unwrap();
    let ensured = threadmoth::engine::apply_byte_edits(missing, &edits).unwrap();
    assert_eq!(ensured, b"intro\n# Added\nbody\n");

    let existing = b"# A\nold\n# B\nkeep\n";
    let insert = MarkdownOperation::InsertAfterHeading {
        heading: "A".into(),
        content: "new".into(),
    };
    let edits =
        threadmoth::provider::markdown::plan(existing, &insert, &Cardinality::ExactlyOne).unwrap();
    let inserted = threadmoth::engine::apply_byte_edits(existing, &edits).unwrap();
    assert_eq!(inserted, b"# A\nnew\nold\n# B\nkeep\n");
}

#[test]
fn markdown_list_items_and_fenced_blocks_are_bounded() {
    let list = b"- first\r\n- second\r\n";
    let edits = threadmoth::provider::markdown::plan(
        list,
        &MarkdownOperation::ReplaceListItem {
            target: "second".into(),
            replacement: "updated".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert_eq!(
        threadmoth::engine::apply_byte_edits(list, &edits).unwrap(),
        b"- first\r\n- updated\r\n"
    );

    let fenced = b"before\n```rust\nold\n```\nafter\n";
    let edits = threadmoth::provider::markdown::plan(
        fenced,
        &MarkdownOperation::ReplaceFencedBlock {
            info: "rust".into(),
            content: "new".into(),
        },
        &Cardinality::ExactlyOne,
    )
    .unwrap();
    assert_eq!(
        threadmoth::engine::apply_byte_edits(fenced, &edits).unwrap(),
        b"before\n```rust\nnew\n```\nafter\n"
    );
}

#[test]
fn multi_file_transaction_prepares_then_commits_and_cleans_journal() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("a.txt"), b"old-a").unwrap();
    std::fs::write(t.path().join("b.txt"), b"old-b").unwrap();
    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-v1-test".into(),
        requests: vec![
            request(
                "a.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "old-a".into(),
                    replacement: "new-a".into(),
                }),
            ),
            request(
                "b.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "old-b".into(),
                    replacement: "new-b".into(),
                }),
            ),
        ],
        budget: EffectBudget {
            max_files: Some(2),
            ..Default::default()
        },
    };
    let c = threadmoth::pipeline::execute_transaction(&w, &tx, false);
    assert_eq!(c.outcome, Outcome::Applied);
    assert!(c
        .certificates
        .iter()
        .filter(|certificate| certificate.outcome == Outcome::Applied)
        .all(
            |certificate| certificate.commit.mode == "committed_atomic_replace"
                && certificate.transaction_guarantee == "transactional_with_rollback"
        ));
    let a_certificate = c
        .certificates
        .iter()
        .find(|certificate| certificate.file_path == "a.txt")
        .unwrap();
    assert_eq!(a_certificate.pre_hash, compute_sha256(b"old-a"));
    let expected_post_hash = compute_sha256(b"new-a");
    assert_eq!(
        a_certificate.post_hash.as_deref(),
        Some(expected_post_hash.as_str())
    );
    assert_eq!(w.read_file("a.txt").unwrap(), b"new-a");
    assert_eq!(w.read_file("b.txt").unwrap(), b"new-b");
    assert!(!t
        .path()
        .join(".threadmoth-recovery/tx-v1-test.json")
        .exists());
}

#[test]
fn code_provider_targets_validated_syntax_nodes() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("main.py"), b"value = 1\n").unwrap();
    let c = execute_request(
        &w,
        &request(
            "main.py",
            OperationPayload::Code(CodeOperation::ReplaceNode {
                language: "python".into(),
                target: "1".into(),
                replacement: "2".into(),
                node_kind: Some("integer".into()),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(w.read_file("main.py").unwrap(), b"value = 2\n");
}

#[test]
fn typescript_provider_uses_typescript_grammar() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("main.ts"), b"const value: number = 1;\n").unwrap();
    let c = execute_request(
        &w,
        &request(
            "main.ts",
            OperationPayload::Code(CodeOperation::ReplaceNode {
                language: "typescript".into(),
                target: "1".into(),
                replacement: "2".into(),
                node_kind: Some("number".into()),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(
        w.read_file("main.ts").unwrap(),
        b"const value: number = 2;\n"
    );
}

#[test]
fn generated_files_fail_closed_without_explicit_opt_in() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(
        t.path().join("generated.rs"),
        b"// Code generated by tool; DO NOT EDIT.\nfn old() {}\n",
    )
    .unwrap();
    let c = execute_request(
        &w,
        &request(
            "generated.rs",
            OperationPayload::Text(TextOperation::Replace {
                target: "old".into(),
                replacement: "new".into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Refused);
}

#[test]
fn strict_patch_requires_exact_context() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"one\ntwo\n").unwrap();
    let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    let c = execute_request(
        &w,
        &request(
            "x.txt",
            OperationPayload::Patch(PatchOperation::UnifiedDiff {
                patch: patch.into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(w.read_file("x.txt").unwrap(), b"one\nTWO\n");
}

#[test]
fn strict_patch_rejects_wrong_path_and_ignored_garbage() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"one\ntwo\n").unwrap();
    let wrong_path = "--- a/other.txt\n+++ b/other.txt\n@@ -1,1 +1,1 @@\n-one\n+ONE\n";
    let c = execute_request(
        &w,
        &request(
            "x.txt",
            OperationPayload::Patch(PatchOperation::UnifiedDiff {
                patch: wrong_path.into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(c.reason_code.as_deref(), Some("INVALID_INPUT"));
    assert_eq!(w.read_file("x.txt").unwrap(), b"one\ntwo\n");

    let garbage = "--- a/x.txt\n+++ b/x.txt\nnoise\n@@ -1,1 +1,1 @@\n-one\n+ONE\n";
    let c = execute_request(
        &w,
        &request(
            "x.txt",
            OperationPayload::Patch(PatchOperation::UnifiedDiff {
                patch: garbage.into(),
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(w.read_file("x.txt").unwrap(), b"one\ntwo\n");
}

#[test]
fn durable_region_guard_survives_unrelated_edit() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"target\n").unwrap();
    let mut r = request(
        "x.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "target".into(),
            replacement: "changed".into(),
        }),
    );
    r.region_guard = Some(RegionGuard {
        anchor: "target".into(),
        target_sha256: compute_sha256(b"target"),
        mode: RegionGuardMode::RegionSnapshot,
    });
    std::fs::write(t.path().join("x.txt"), b"unrelated\ntarget\n").unwrap();
    let c = execute_request(&w, &r, false);
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(w.read_file("x.txt").unwrap(), b"unrelated\nchanged\n");
}

#[test]
fn structural_region_guard_checks_the_code_target() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("main.py"), b"value = 1\n").unwrap();
    let mut r = request(
        "main.py",
        OperationPayload::Code(CodeOperation::ReplaceNode {
            language: "python".into(),
            target: "1".into(),
            replacement: "2".into(),
            node_kind: Some("integer".into()),
        }),
    );
    r.region_guard = Some(RegionGuard {
        anchor: "1".into(),
        target_sha256: compute_sha256(b"1"),
        mode: RegionGuardMode::StructuralSnapshot,
    });
    let certificate = execute_request(&w, &r, false);
    assert_eq!(certificate.outcome, Outcome::Applied);
    assert_eq!(w.read_file("main.py").unwrap(), b"value = 2\n");
}

#[test]
fn empty_region_anchor_is_refused_without_panicking() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"target\n").unwrap();
    let mut r = request(
        "x.txt",
        OperationPayload::Text(TextOperation::Replace {
            target: "target".into(),
            replacement: "changed".into(),
        }),
    );
    r.region_guard = Some(RegionGuard {
        anchor: String::new(),
        target_sha256: String::new(),
        mode: RegionGuardMode::RegionSnapshot,
    });
    let c = execute_request(&w, &r, false);
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(c.reason_code.as_deref(), Some("INVALID_INPUT"));
}

#[cfg(windows)]
#[test]
fn case_only_rename_preserves_the_requested_destination_spelling() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("name.txt"), b"content").unwrap();
    let c = execute_request(
        &w,
        &request(
            "name.txt",
            OperationPayload::File(FileOperation::RenameFile {
                destination: "NAME.TXT".into(),
                expected_source_hash: compute_sha256(b"content"),
                destination_absent: true,
            }),
        ),
        false,
    );
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(w.read_file("NAME.TXT").unwrap(), b"content");
}

#[test]
fn lifecycle_operations_require_exactly_one_target() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    let mut r = request(
        "new.txt",
        OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: b"content".to_vec(),
        }),
    );
    r.cardinality = Cardinality::All;
    let c = execute_request(&w, &r, false);
    assert_eq!(c.outcome, Outcome::Refused);
    assert_eq!(c.reason_code.as_deref(), Some("CARDINALITY_MISMATCH"));
    assert!(!t.path().join("new.txt").exists());
}

#[test]
fn single_file_transaction_resolves_operations_on_coherent_candidate() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    std::fs::write(t.path().join("x.txt"), b"one two\n").unwrap();
    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-single-file".into(),
        requests: vec![
            request(
                "x.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "one".into(),
                    replacement: "first".into(),
                }),
            ),
            request(
                "x.txt",
                OperationPayload::Text(TextOperation::Replace {
                    target: "two".into(),
                    replacement: "second".into(),
                }),
            ),
        ],
        budget: EffectBudget::default(),
    };
    let c = threadmoth::pipeline::execute_transaction(&w, &tx, false);
    assert_eq!(c.outcome, Outcome::Applied);
    assert_eq!(w.read_file("x.txt").unwrap(), b"first second\n");
}

#[test]
fn single_file_transaction_validates_candidates_before_commit() {
    let t = TempDir::new().unwrap();
    let w = Workspace::new(t.path()).unwrap();
    let original = b"const value = 1;\n";
    std::fs::write(t.path().join("main.js"), original).unwrap();
    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "tx-invalid-candidate".into(),
        requests: vec![
            request(
                "main.js",
                OperationPayload::Code(CodeOperation::ReplaceNode {
                    language: "javascript".into(),
                    target: "1".into(),
                    replacement: "}".into(),
                    node_kind: Some("number".into()),
                }),
            ),
            request(
                "main.js",
                OperationPayload::Text(TextOperation::Replace {
                    target: "value".into(),
                    replacement: "other".into(),
                }),
            ),
        ],
        budget: EffectBudget::default(),
    };
    let certificate = threadmoth::pipeline::execute_transaction(&w, &tx, false);
    assert_eq!(certificate.outcome, Outcome::Failed);
    assert_eq!(
        certificate.reason_code.as_deref(),
        Some("INVALID_STRUCTURE")
    );
    assert_eq!(w.read_file("main.js").unwrap(), original);
}
