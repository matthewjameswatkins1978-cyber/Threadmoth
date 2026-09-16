#![forbid(unsafe_code)]

use crate::engine::{ByteEdit, EngineError};
use crate::protocol::{Candidate, Cardinality, RefusalReason};
use schemars::JsonSchema;
use serde::Serialize;
use tree_sitter::{Language, Parser};

/// The two syntax families intentionally share one parser/planner engine.
#[derive(Serialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LanguageFamily {
    Code,
    Web,
}

#[derive(Serialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuralTargeting {
    AstGrounded,
    AstTyped,
}

#[derive(Clone, Copy, Debug)]
pub struct LanguageSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub family: LanguageFamily,
    pub description: &'static str,
    pub grammar: fn() -> Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Replace,
    Before,
    After,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SyntaxPlan {
    pub edits: Vec<ByteEdit>,
    pub language: &'static str,
    pub node_kind: String,
    pub targeting: StructuralTargeting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineNode {
    pub kind: String,
    pub label: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SyntaxError {
    Refused(RefusalReason),
    Engine(EngineError),
}

pub fn registry() -> &'static [LanguageSpec] {
    static REGISTRY: &[LanguageSpec] = &[
        LanguageSpec {
            id: "javascript",
            aliases: &["js"],
            extensions: &[".js", ".mjs", ".cjs"],
            family: LanguageFamily::Code,
            description: "JavaScript source",
            grammar: || tree_sitter_javascript::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "jsx",
            aliases: &[],
            extensions: &[".jsx"],
            family: LanguageFamily::Code,
            description: "JavaScript with JSX",
            grammar: || tree_sitter_javascript::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "typescript",
            aliases: &["ts"],
            extensions: &[".ts"],
            family: LanguageFamily::Code,
            description: "TypeScript source",
            grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        },
        LanguageSpec {
            id: "tsx",
            aliases: &[],
            extensions: &[".tsx"],
            family: LanguageFamily::Code,
            description: "TypeScript with JSX",
            grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        },
        LanguageSpec {
            id: "python",
            aliases: &["py"],
            extensions: &[".py"],
            family: LanguageFamily::Code,
            description: "Python source",
            grammar: || tree_sitter_python::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "rust",
            aliases: &["rs"],
            extensions: &[".rs"],
            family: LanguageFamily::Code,
            description: "Rust source",
            grammar: || tree_sitter_rust::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "go",
            aliases: &["golang"],
            extensions: &[".go"],
            family: LanguageFamily::Code,
            description: "Go source",
            grammar: || tree_sitter_go::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "c",
            aliases: &[],
            extensions: &[".c"],
            family: LanguageFamily::Code,
            description: "C source",
            grammar: || tree_sitter_c::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "cpp",
            aliases: &["c++"],
            extensions: &[".cpp", ".cc", ".cxx", ".hpp"],
            family: LanguageFamily::Code,
            description: "C++ source",
            grammar: || tree_sitter_cpp::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "java",
            aliases: &[],
            extensions: &[".java"],
            family: LanguageFamily::Code,
            description: "Java source",
            grammar: || tree_sitter_java::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "csharp",
            aliases: &["cs", "c#"],
            extensions: &[".cs"],
            family: LanguageFamily::Code,
            description: "C# source",
            grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "php",
            aliases: &[],
            extensions: &[".php"],
            family: LanguageFamily::Code,
            description: "PHP source and embedded PHP",
            grammar: || tree_sitter_php::LANGUAGE_PHP.into(),
        },
        LanguageSpec {
            id: "hcl",
            aliases: &["terraform", "tf"],
            extensions: &[".tf", ".tfvars", ".hcl"],
            family: LanguageFamily::Code,
            description: "HCL and Terraform syntax without infrastructure semantics",
            grammar: || tree_sitter_hcl::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "bash",
            aliases: &["sh", "shell"],
            extensions: &[".sh", ".bash"],
            family: LanguageFamily::Code,
            description: "Bash and POSIX-style shell source",
            grammar: || tree_sitter_bash::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "powershell",
            aliases: &["pwsh"],
            extensions: &[".ps1", ".psm1", ".psd1"],
            family: LanguageFamily::Code,
            description: "PowerShell source",
            grammar: || tree_sitter_powershell::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "sql",
            aliases: &[],
            extensions: &[".sql"],
            family: LanguageFamily::Code,
            description: "Common SQL syntax (SQLite/PostgreSQL/MySQL envelope)",
            grammar: || tree_sitter_sequel::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "html",
            aliases: &[],
            extensions: &[".html", ".htm"],
            family: LanguageFamily::Web,
            description: "HTML structure",
            grammar: || tree_sitter_html::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "css",
            aliases: &[],
            extensions: &[".css"],
            family: LanguageFamily::Web,
            description: "CSS structure",
            grammar: || tree_sitter_css::LANGUAGE.into(),
        },
        LanguageSpec {
            id: "xml",
            aliases: &[],
            extensions: &[".xml"],
            family: LanguageFamily::Web,
            description: "XML structure",
            grammar: || tree_sitter_xml::LANGUAGE_XML.into(),
        },
    ];
    REGISTRY
}

#[cfg(test)]
fn validate_registry(specs: &[LanguageSpec]) -> Result<(), &'static str> {
    for (index, spec) in specs.iter().enumerate() {
        if spec.id.is_empty() || (spec.grammar) as usize == 0 {
            return Err("empty ID or missing grammar");
        }
        for alias in spec.aliases {
            if alias.is_empty() || *alias == spec.id {
                return Err("invalid alias");
            }
        }
        for other in &specs[index + 1..] {
            if spec.id == other.id {
                return Err("duplicate canonical ID");
            }
            if spec.aliases.contains(&other.id) || other.aliases.contains(&spec.id) {
                return Err("alias collides with canonical ID");
            }
            if spec
                .aliases
                .iter()
                .any(|alias| other.aliases.contains(alias))
            {
                return Err("duplicate alias");
            }
            for extension in spec.extensions {
                if other.extensions.contains(extension) {
                    return Err("ambiguous extension");
                }
            }
        }
    }
    Ok(())
}

pub fn lookup(name: &str) -> Option<&'static LanguageSpec> {
    let wanted = name.to_ascii_lowercase();
    registry()
        .iter()
        .find(|spec| spec.id == wanted || spec.aliases.iter().any(|alias| *alias == wanted))
}

pub fn suggest_extension(path: &str) -> Option<&'static str> {
    let detection = crate::target_registry::detect(path, None);
    if matches!(detection.provider.as_deref(), Some("code" | "web")) {
        crate::target_registry::lookup(&detection.target_kind).map(|spec| spec.id)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub fn plan(
    content: &[u8],
    language_name: &str,
    target: &str,
    replacement: &[u8],
    placement: Placement,
    node_kind: Option<&str>,
    family: LanguageFamily,
    cardinality: &Cardinality,
) -> Result<SyntaxPlan, SyntaxError> {
    if !matches!(cardinality, Cardinality::ExactlyOne) {
        return Err(SyntaxError::Refused(RefusalReason::CardinalityMismatch {
            expected: "exactly_one syntax node".into(),
            actual: 1,
        }));
    }
    let Some(spec) = lookup(language_name) else {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: "syntax".into(),
                capability: format!("language grammar: {language_name}"),
            },
        ));
    };
    if spec.family != family {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: match family {
                    LanguageFamily::Code => "code",
                    LanguageFamily::Web => "web",
                }
                .into(),
                capability: spec.id.into(),
            },
        ));
    }
    let tree = parse(content, spec)?;
    let mut found = Vec::new();
    collect_nodes(
        tree.root_node(),
        content,
        target.as_bytes(),
        node_kind,
        &mut found,
    );
    if node_kind.is_none() {
        found.sort_unstable_by_key(|(start, end, _)| (*start, *end));
        found.dedup_by_key(|(start, end, _)| (*start, *end));
    }
    if found.len() != 1 {
        return Err(SyntaxError::Refused(if found.is_empty() {
            RefusalReason::MissingTarget {
                target: format!("no {} syntax node matched", spec.id),
            }
        } else {
            RefusalReason::DuplicateTarget {
                target: target.into(),
                count: found.len(),
                candidates: found
                    .iter()
                    .take(8)
                    .map(|(start, end, kind)| Candidate {
                        offset: *start,
                        start: *start,
                        end: *end,
                        target: target.into(),
                        line: content[..*start].iter().filter(|b| **b == b'\n').count() + 1,
                        context: String::from_utf8_lossy(
                            &content[start.saturating_sub(24)..(*end + 24).min(content.len())],
                        )
                        .into(),
                        anchor_sha256: crate::engine::compute_sha256(&content[*start..*end]),
                        selection_id: String::new(),
                        node_kind: Some(kind.clone()),
                    })
                    .collect(),
                candidates_returned: found.len().min(8),
                truncated: found.len() > 8,
            }
        }));
    }
    let (start, end, kind) = found.remove(0);
    let replacement = match placement {
        Placement::Replace => replacement.to_vec(),
        Placement::Before => [replacement, &content[start..end]].concat(),
        Placement::After => [&content[start..end], replacement].concat(),
    };
    Ok(SyntaxPlan {
        edits: vec![ByteEdit {
            start,
            end,
            replacement,
        }],
        language: spec.id,
        node_kind: kind,
        targeting: if node_kind.is_some() {
            StructuralTargeting::AstTyped
        } else {
            StructuralTargeting::AstGrounded
        },
    })
}

/// Plan one physical syntax-node occurrence selected from a refusal
/// certificate. The core performs the file-identity and selection-ID checks;
/// this function re-parses the unchanged bytes and re-validates the exact
/// node span before returning an edit.
#[allow(clippy::too_many_arguments)]
pub fn plan_at(
    content: &[u8],
    language_name: &str,
    target: &str,
    replacement: &[u8],
    placement: Placement,
    node_kind: Option<&str>,
    family: LanguageFamily,
    start: usize,
    end: usize,
) -> Result<SyntaxPlan, SyntaxError> {
    let Some(spec) = lookup(language_name) else {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: "syntax".into(),
                capability: format!("language grammar: {language_name}"),
            },
        ));
    };
    if spec.family != family || start > end || end > content.len() {
        return Err(SyntaxError::Refused(
            RefusalReason::CandidateSelectionInvalid {
                offset: start,
                selection_id: String::new(),
                details: "candidate span is outside the selected syntax provider".into(),
            },
        ));
    }
    let tree = parse(content, spec)?;
    let mut found = Vec::new();
    collect_nodes(
        tree.root_node(),
        content,
        target.as_bytes(),
        node_kind,
        &mut found,
    );
    found.sort_unstable_by_key(|(node_start, node_end, _)| (*node_start, *node_end));
    found.dedup_by_key(|(node_start, node_end, _)| (*node_start, *node_end));
    let Some((_, _, kind)) = found
        .iter()
        .find(|(node_start, node_end, _)| *node_start == start && *node_end == end)
        .cloned()
    else {
        return Err(SyntaxError::Refused(
            RefusalReason::CandidateSelectionInvalid {
                offset: start,
                selection_id: String::new(),
                details: "candidate span no longer identifies the requested syntax node".into(),
            },
        ));
    };
    let replacement = match placement {
        Placement::Replace => replacement.to_vec(),
        Placement::Before => [replacement, &content[start..end]].concat(),
        Placement::After => [&content[start..end], replacement].concat(),
    };
    Ok(SyntaxPlan {
        edits: vec![ByteEdit {
            start,
            end,
            replacement,
        }],
        language: spec.id,
        node_kind: kind,
        targeting: if node_kind.is_some() {
            StructuralTargeting::AstTyped
        } else {
            StructuralTargeting::AstGrounded
        },
    })
}

pub fn validate(content: &[u8], language_name: &str) -> Result<(), SyntaxError> {
    let Some(spec) = lookup(language_name) else {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: "syntax".into(),
                capability: format!("language grammar: {language_name}"),
            },
        ));
    };
    parse(content, spec).map(|_| ())
}

/// Return deterministic, bounded structural observations using the existing
/// Tree-sitter registry and parser. This is read-only; mutation still requires
/// a normal request, cardinality check and certificate.
pub fn outline(
    content: &[u8],
    language_name: &str,
    family: LanguageFamily,
) -> Result<Vec<OutlineNode>, SyntaxError> {
    let Some(spec) = lookup(language_name) else {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: "syntax".into(),
                capability: format!("language grammar: {language_name}"),
            },
        ));
    };
    if spec.family != family {
        return Err(SyntaxError::Refused(
            RefusalReason::ProviderCapabilityMissing {
                provider: match family {
                    LanguageFamily::Code => "code",
                    LanguageFamily::Web => "web",
                }
                .into(),
                capability: spec.id.into(),
            },
        ));
    }
    let tree = parse(content, spec)?;
    let mut found = Vec::new();
    collect_outline_nodes(tree.root_node(), content, family, &mut found);
    found.sort_unstable_by_key(|node| (node.start_byte, node.end_byte, node.kind.clone()));
    found.dedup_by(|left, right| {
        left.start_byte == right.start_byte
            && left.end_byte == right.end_byte
            && left.kind == right.kind
    });
    Ok(found)
}

fn collect_outline_nodes(
    node: tree_sitter::Node<'_>,
    content: &[u8],
    family: LanguageFamily,
    out: &mut Vec<OutlineNode>,
) {
    if is_outline_kind(node.kind(), family) {
        let start_byte = node.start_byte();
        let end_byte = node.end_byte();
        out.push(OutlineNode {
            kind: node.kind().into(),
            label: bounded_label(content, start_byte, end_byte),
            start_byte,
            end_byte,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_outline_nodes(child, content, family, out);
    }
}

fn bounded_label(content: &[u8], start_byte: usize, end_byte: usize) -> String {
    const MAX_LABEL_BYTES: usize = 96;
    let preview_end = (start_byte + MAX_LABEL_BYTES).min(end_byte);
    let mut label = String::from_utf8_lossy(&content[start_byte..preview_end])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if preview_end < end_byte {
        label.push_str(" ...");
    }
    label
}

fn is_outline_kind(kind: &str, family: LanguageFamily) -> bool {
    match family {
        LanguageFamily::Code => matches!(
            kind,
            "function_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "impl_item"
                | "mod_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "function_definition"
                | "async_function_definition"
                | "class_definition"
                | "function_declaration"
                | "generator_function_declaration"
                | "class_declaration"
                | "method_definition"
                | "method_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "namespace_declaration"
                | "type_declaration"
                | "struct_specifier"
                | "create_function_statement"
                | "create_table_statement"
        ),
        LanguageFamily::Web => matches!(
            kind,
            "element"
                | "script_element"
                | "style_element"
                | "style_rule"
                | "media_statement"
                | "keyframes_statement"
                | "rule_set"
                | "qualified_rule"
                | "template_element"
        ),
    }
}

fn parse(content: &[u8], spec: &LanguageSpec) -> Result<tree_sitter::Tree, SyntaxError> {
    let language = (spec.grammar)();
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|_| {
        SyntaxError::Refused(RefusalReason::ProviderCapabilityMissing {
            provider: "syntax".into(),
            capability: spec.id.into(),
        })
    })?;
    let tree = parser.parse(content, None).ok_or_else(|| {
        SyntaxError::Refused(RefusalReason::MalformedInput {
            details: "parser returned no syntax tree".into(),
        })
    })?;
    if tree.root_node().has_error() {
        return Err(SyntaxError::Refused(RefusalReason::MalformedInput {
            details: format!("{} source contains syntax errors", spec.id),
        }));
    }
    Ok(tree)
}

fn collect_nodes(
    node: tree_sitter::Node<'_>,
    content: &[u8],
    target: &[u8],
    kind: Option<&str>,
    out: &mut Vec<(usize, usize, String)>,
) {
    let start = node.start_byte();
    let end = node.end_byte();
    if &content[start..end] == target && kind.is_none_or(|wanted| wanted == node.kind()) {
        out.push((start, end, node.kind().into()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_nodes(child, content, target, kind, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_unique_canonical_ids_and_aliases() {
        let specs = registry();
        assert!(validate_registry(specs).is_ok());
        for (index, spec) in specs.iter().enumerate() {
            assert!(!spec.id.is_empty());
            assert!(spec.description.len() > 3);
            for other in &specs[index + 1..] {
                assert_ne!(spec.id, other.id);
                assert!(!other.aliases.contains(&spec.id));
                assert!(!spec
                    .aliases
                    .iter()
                    .any(|alias| other.aliases.contains(alias)));
            }
        }
    }

    #[test]
    fn registry_validation_rejects_alias_and_extension_collisions() {
        const GRAMMAR: fn() -> Language = || tree_sitter_javascript::LANGUAGE.into();
        let duplicate_alias = [
            LanguageSpec {
                id: "one",
                aliases: &["same"],
                extensions: &[".one"],
                family: LanguageFamily::Code,
                description: "one",
                grammar: GRAMMAR,
            },
            LanguageSpec {
                id: "two",
                aliases: &["same"],
                extensions: &[".two"],
                family: LanguageFamily::Code,
                description: "two",
                grammar: GRAMMAR,
            },
        ];
        assert_eq!(validate_registry(&duplicate_alias), Err("duplicate alias"));

        let ambiguous_extension = [
            LanguageSpec {
                id: "one",
                aliases: &[],
                extensions: &[".same"],
                family: LanguageFamily::Code,
                description: "one",
                grammar: GRAMMAR,
            },
            LanguageSpec {
                id: "two",
                aliases: &[],
                extensions: &[".same"],
                family: LanguageFamily::Web,
                description: "two",
                grammar: GRAMMAR,
            },
        ];
        assert_eq!(
            validate_registry(&ambiguous_extension),
            Err("ambiguous extension")
        );
    }

    #[test]
    fn python_same_span_wrappers_are_one_physical_candidate() {
        let plan = plan(
            b"value = 1\n",
            "python",
            "value = 1",
            b"value = 2",
            Placement::Replace,
            None,
            LanguageFamily::Code,
            &Cardinality::ExactlyOne,
        )
        .expect("same source span must not be false ambiguity");

        assert_eq!(plan.edits.len(), 1);
        assert_eq!(plan.edits[0].start, 0);
        assert_eq!(plan.edits[0].end, 9);
    }

    #[test]
    fn distinct_identical_python_targets_remain_ambiguous() {
        let error = plan(
            b"value = 1\nvalue = 1\n",
            "python",
            "value = 1",
            b"value = 2",
            Placement::Replace,
            None,
            LanguageFamily::Code,
            &Cardinality::ExactlyOne,
        )
        .unwrap_err();

        match error {
            SyntaxError::Refused(RefusalReason::DuplicateTarget {
                count, candidates, ..
            }) => {
                assert_eq!(count, 2);
                assert_eq!(candidates.len(), 2);
                assert_ne!(candidates[0].offset, candidates[1].offset);
            }
            other => panic!("expected real ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn extension_suggestions_are_unambiguous() {
        assert_eq!(suggest_extension("foo.cpp"), Some("cpp"));
        assert_eq!(suggest_extension("foo.ps1"), Some("powershell"));
        assert_eq!(suggest_extension("Example.java"), Some("java"));
        assert_eq!(suggest_extension("Example.cs"), Some("csharp"));
        assert_eq!(suggest_extension("index.php"), Some("php"));
        assert_eq!(suggest_extension("main.tf"), Some("hcl"));
        assert_eq!(suggest_extension("index.html"), Some("html"));
        assert_eq!(suggest_extension("unknown"), None);
    }

    #[test]
    fn outlines_are_deterministic_and_include_bounded_code_items() {
        let source = b"fn load() {}\n\nstruct Service {\n    name: String,\n}\n";
        let first = outline(source, "rust", LanguageFamily::Code).unwrap();
        let second = outline(source, "rust", LanguageFamily::Code).unwrap();
        assert_eq!(first, second);
        assert!(first.iter().any(|node| node.kind == "function_item"));
        assert!(first.iter().any(|node| node.kind == "struct_item"));
        assert!(first.iter().all(|node| node.label.len() <= 100));
        assert!(first
            .windows(2)
            .all(|nodes| (nodes[0].start_byte, nodes[0].end_byte)
                <= (nodes[1].start_byte, nodes[1].end_byte)));
    }

    #[test]
    fn admitted_language_grammars_parse_representative_source() {
        let cases = [
            ("java", b"class Example { int value; }".as_slice()),
            (
                "csharp",
                b"class Example { public int Value { get; set; } }".as_slice(),
            ),
            (
                "php",
                b"<?php function value(): int { return 1; }".as_slice(),
            ),
            ("php", b"<html><?php echo \"ok\"; ?></html>".as_slice()),
            (
                "hcl",
                b"resource \"example\" \"one\" { value = \"ok\" }".as_slice(),
            ),
        ];
        for (language, source) in cases {
            validate(source, language).unwrap_or_else(|error| {
                panic!("{language} admission source must parse: {error:?}")
            });
        }
    }

    #[test]
    fn admitted_language_grammars_refuse_malformed_source() {
        for (language, source) in [
            ("java", b"class {".as_slice()),
            ("csharp", b"class {".as_slice()),
            ("php", b"<?php function {".as_slice()),
            ("hcl", b"resource {".as_slice()),
        ] {
            let error = validate(source, language).unwrap_err();
            assert!(matches!(
                error,
                SyntaxError::Refused(RefusalReason::MalformedInput { .. })
            ));
        }
    }

    #[test]
    fn syntax_plan_distinguishes_grounded_and_typed() {
        let grounded = plan(
            b"let x = 1;",
            "javascript",
            "x",
            b"y",
            Placement::Replace,
            None,
            LanguageFamily::Code,
            &Cardinality::ExactlyOne,
        )
        .unwrap();
        assert_eq!(grounded.targeting, StructuralTargeting::AstGrounded);
        let typed = plan(
            b"let x = 1;",
            "javascript",
            "x",
            b"y",
            Placement::Replace,
            Some("identifier"),
            LanguageFamily::Code,
            &Cardinality::ExactlyOne,
        )
        .unwrap();
        assert_eq!(typed.targeting, StructuralTargeting::AstTyped);
    }

    #[test]
    fn built_in_grammars_parse_representative_sources() {
        let fixtures = [
            ("javascript", b"const answer = 42;".as_slice()),
            ("jsx", b"const view = <div>ok</div>;".as_slice()),
            ("typescript", b"const answer: number = 42;".as_slice()),
            ("tsx", b"const view = <div>ok</div>;".as_slice()),
            ("python", b"def answer():\n    return 42\n".as_slice()),
            ("rust", b"fn answer() -> i32 { 42 }".as_slice()),
            (
                "go",
                b"package main\nfunc answer() int { return 42 }".as_slice(),
            ),
            ("c", b"int answer(void) { return 42; }".as_slice()),
            ("cpp", b"int answer() { return 42; }".as_slice()),
            ("bash", b"answer() { printf '%s\\n' 42; }".as_slice()),
            ("powershell", b"$answer = 42".as_slice()),
            ("sql", b"SELECT id FROM users WHERE id = 42;".as_slice()),
            ("html", b"<main><p>ok</p></main>".as_slice()),
            ("css", b".answer { color: red; }".as_slice()),
            (
                "xml",
                b"<?xml version=\"1.0\"?><root><value>42</value></root>".as_slice(),
            ),
        ];
        for (language, source) in fixtures {
            let spec = lookup(language).expect(language);
            assert!(
                parse(source, spec).is_ok(),
                "{language} fixture did not parse"
            );
        }
    }

    #[test]
    fn invalid_source_is_refused_by_the_shared_engine() {
        let spec = lookup("sql").unwrap();
        assert!(parse(b"SELECT (", spec).is_err());
    }

    #[test]
    fn sql_common_dialect_envelope_is_accepted() {
        let spec = lookup("sql").unwrap();
        let fixtures = [
            b"CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);".as_slice(),
            b"CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT);".as_slice(),
            b"CREATE TABLE `users` (`id` INT AUTO_INCREMENT PRIMARY KEY);".as_slice(),
        ];
        for fixture in fixtures {
            assert!(parse(fixture, spec).is_ok(), "SQL dialect fixture rejected");
        }
    }
}
