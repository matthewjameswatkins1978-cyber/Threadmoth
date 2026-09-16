#![forbid(unsafe_code)]

//! The public knowledge base for Threadmoth.  CLI discovery surfaces are views of
//! this module; they must not grow separate hand-written descriptions of the
//! protocol.

use crate::engine::compute_sha256;
use crate::lifecycle::FileOperation;
use crate::path::{PathNamespace, PathNormalizer};
use crate::pattern::PatternOperation;
use crate::protocol::{
    CandidateGuard, Cardinality, EffectBudget, OperationPayload, Request, TransactionRequest,
    MAX_ASSERTIONS, MAX_ASSERTION_LITERAL_BYTES, MAX_FILE_BYTES, MAX_PLAN_BYTES,
    MAX_PLAN_OPERATIONS, MAX_REQUEST_BYTES, MAX_TRANSACTION_REQUESTS, PROTOCOL_VERSION,
    SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::provider::code::CodeOperation;
use crate::provider::dotenv::DotenvOperation;
use crate::provider::json::JsonOperation;
use crate::provider::patch::PatchOperation;
use crate::provider::syntax::{self, LanguageFamily};
use crate::provider::text::TextOperation;
use crate::provider::toml::{TomlOperation, TomlValueWrapper};
use crate::provider::web::WebOperation;
use crate::provider::yaml::YamlOperation;
use crate::target_registry;
use crate::workspace::Workspace;
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OperationMetadata {
    pub name: &'static str,
    pub purpose: &'static str,
    pub required_selector: &'static str,
    pub default_cardinality: &'static str,
    pub idempotent: bool,
    pub effect: &'static str,
    pub read_only: bool,
    pub previewable: bool,
    pub transactional: bool,
    pub recoverable: bool,
    pub local_only: bool,
    pub preservation: Vec<&'static str>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderMetadata {
    pub name: &'static str,
    pub version: &'static str,
    pub operations: Vec<&'static str>,
    pub selectors: Vec<&'static str>,
    pub preservation: Vec<&'static str>,
    pub encodings: Vec<&'static str>,
    pub transaction_support: &'static str,
    pub durable_anchor_support: bool,
    pub languages: Vec<&'static str>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReasonMetadata {
    pub code: &'static str,
    pub meaning: &'static str,
    pub why_refused: &'static str,
    pub recovery_category: &'static str,
    pub retry_unchanged: bool,
    pub relevant_commands: Vec<&'static str>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceLimits {
    pub max_request_bytes: usize,
    pub max_transaction_requests: usize,
    pub max_diagnostic_bytes: usize,
    pub max_pattern_bytes: usize,
    pub max_file_bytes: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TransactionCapabilities {
    pub single_file: bool,
    pub multi_file: bool,
    pub rollback: bool,
    pub crash_recovery: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FeatureCapabilities {
    pub plans: bool,
    pub plan_apply: bool,
    pub postconditions: bool,
    pub candidate_selection: bool,
    pub composite_selectors: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlanLimits {
    pub max_plan_bytes: usize,
    pub max_plan_operations: usize,
    pub max_assertions: usize,
    pub max_assertion_literal_bytes: usize,
    pub max_candidate_evidence: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CapabilityManifest {
    pub format_version: &'static str,
    pub protocol_versions: Vec<&'static str>,
    pub protocol_version: &'static str,
    pub threadmoth_version: &'static str,
    pub capability_set_id: String,
    pub providers: Vec<ProviderMetadata>,
    pub operations: Vec<OperationMetadata>,
    pub selectors: Vec<&'static str>,
    pub preservation_guarantees: Vec<&'static str>,
    pub encodings: Vec<&'static str>,
    pub path_namespaces: Vec<&'static str>,
    pub code_languages: Vec<&'static str>,
    pub web_formats: Vec<&'static str>,
    pub structural_operations: Vec<&'static str>,
    pub ast_grounded: bool,
    pub ast_typed: bool,
    pub desired_state: bool,
    pub recovery_inspection: bool,
    pub guard_modes: Vec<&'static str>,
    pub transaction_capabilities: TransactionCapabilities,
    pub features: FeatureCapabilities,
    pub supported_assertions: Vec<&'static str>,
    pub plan_limits: PlanLimits,
    pub resource_limits: ResourceLimits,
    pub effect_budget_dimensions: Vec<&'static str>,
    pub reason_codes: Vec<ReasonMetadata>,
    pub coverage_levels: Vec<&'static str>,
    pub targets: Vec<Value>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Example {
    pub topic: &'static str,
    pub intent: &'static str,
    pub request: Value,
    pub representative_response: Value,
    pub safety_property: &'static str,
}

#[derive(Serialize, Clone, Debug)]
pub struct Suggestion {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub detection_basis: String,
    pub understanding_level: String,
    pub preservation_level: String,
    pub fallback_routes: Vec<String>,
    pub goal: Option<String>,
    pub mode: String,
    pub recommended_operation: Option<String>,
    pub rationale: String,
    pub request_template: Option<Value>,
    pub guarantees: Vec<String>,
    pub budget_defaults: EffectBudget,
    pub alternatives: Vec<Value>,
    pub blocked_reasons: Vec<String>,
    pub capability_set_id: String,
}

pub fn operation_metadata() -> Vec<OperationMetadata> {
    let common = vec![
        "unrelated_bytes",
        "utf8",
        "newline_profile",
        "final_newline",
    ];
    vec![
        op(
            "replace",
            "Replace an exact target.",
            "literal",
            false,
            "mixed",
            common.clone(),
        ),
        op(
            "insert_before",
            "Insert content before an exact target.",
            "literal",
            false,
            "additive",
            common.clone(),
        ),
        op(
            "insert_after",
            "Insert content after an exact target.",
            "literal",
            false,
            "additive",
            common.clone(),
        ),
        op(
            "delete",
            "Delete an exact target.",
            "literal",
            false,
            "destructive",
            common.clone(),
        ),
        op(
            "move",
            "Move one exact target before another.",
            "literal",
            false,
            "mixed",
            common.clone(),
        ),
        op(
            "ensure_present",
            "Ensure desired content exists; replay is safe.",
            "literal",
            true,
            "additive",
            common.clone(),
        ),
        op(
            "ensure_absent",
            "Ensure a target is absent; replay is safe.",
            "literal",
            true,
            "destructive",
            common.clone(),
        ),
        op(
            "set",
            "Set a selected value or exact target.",
            "provider_selector",
            false,
            "mixed",
            common.clone(),
        ),
        op(
            "unset",
            "Remove a selected value or exact target.",
            "provider_selector",
            true,
            "destructive",
            common.clone(),
        ),
        op(
            "rename",
            "Rename a selected key or exact target.",
            "provider_selector",
            false,
            "mixed",
            common,
        ),
        op(
            "insert",
            "Insert a structured member or array item.",
            "json_pointer",
            false,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "rename_key",
            "Rename one structured key.",
            "json_pointer",
            false,
            "mixed",
            vec!["unrelated_bytes"],
        ),
        op(
            "replace_section",
            "Replace one bounded Markdown section.",
            "heading",
            false,
            "mixed",
            vec!["unrelated_bytes"],
        ),
        op(
            "ensure_section",
            "Ensure one Markdown section exists.",
            "heading",
            true,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "delete_section",
            "Delete one bounded Markdown section.",
            "heading",
            false,
            "destructive",
            vec!["unrelated_bytes"],
        ),
        op(
            "insert_after_heading",
            "Insert content after one Markdown heading.",
            "heading",
            false,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "replace_list_item",
            "Replace one bounded Markdown list item.",
            "list_item",
            false,
            "mixed",
            vec!["unrelated_bytes"],
        ),
        op(
            "ensure_list_item",
            "Ensure one Markdown list item exists.",
            "list_item",
            true,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "delete_list_item",
            "Delete one bounded Markdown list item.",
            "list_item",
            false,
            "destructive",
            vec!["unrelated_bytes"],
        ),
        op(
            "replace_fenced_block",
            "Replace one fenced Markdown block body.",
            "fenced_region",
            false,
            "mixed",
            vec!["unrelated_bytes"],
        ),
        op(
            "replace_node",
            "Replace one parsed syntax node.",
            "syntax_node_text",
            false,
            "mixed",
            vec!["unrelated_bytes"],
        ),
        op(
            "insert_before_node",
            "Insert content before one parsed syntax node.",
            "syntax_node_text",
            false,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "insert_after_node",
            "Insert content after one parsed syntax node.",
            "syntax_node_text",
            false,
            "additive",
            vec!["unrelated_bytes"],
        ),
        op(
            "remove_node",
            "Remove one parsed syntax node.",
            "syntax_node_text",
            false,
            "destructive",
            vec!["unrelated_bytes"],
        ),
        op(
            "replace_desired_state",
            "Derive bounded edits for explicitly supplied desired bytes.",
            "desired_bytes",
            true,
            "mixed",
            vec!["explicit_desired_divergence"],
        ),
        op(
            "unified_diff",
            "Apply one exact unified diff.",
            "exact_preimage",
            false,
            "mixed",
            vec!["exact_context"],
        ),
        op(
            "create_file",
            "Create a missing file without overwriting.",
            "workspace_relative_path",
            false,
            "additive",
            vec!["identity_and_confinement"],
        ),
        op(
            "delete_file",
            "Delete a file guarded by identity.",
            "workspace_relative_path",
            false,
            "destructive",
            vec!["identity_and_confinement"],
        ),
        op(
            "rename_file",
            "Rename a file with source and destination guards.",
            "workspace_relative_path",
            false,
            "mixed",
            vec!["identity_and_confinement"],
        ),
        op(
            "move_file",
            "Move a file with source and destination guards.",
            "workspace_relative_path",
            false,
            "mixed",
            vec!["identity_and_confinement"],
        ),
    ]
}

fn op(
    name: &'static str,
    purpose: &'static str,
    selector: &'static str,
    idempotent: bool,
    effect: &'static str,
    preservation: Vec<&'static str>,
) -> OperationMetadata {
    OperationMetadata {
        name,
        purpose,
        required_selector: selector,
        default_cardinality: "exactly_one",
        idempotent,
        effect,
        read_only: false,
        previewable: true,
        transactional: true,
        recoverable: true,
        local_only: true,
        preservation,
    }
}

pub fn provider_metadata() -> Vec<ProviderMetadata> {
    let text_ops = vec![
        "replace",
        "insert_before",
        "insert_after",
        "delete",
        "move",
        "ensure_present",
        "ensure_absent",
        "set",
        "unset",
        "rename",
    ];
    vec![
        provider(
            "text",
            "text-byte-v1",
            text_ops,
            vec!["literal"],
            "utf8, BOM, newline profile",
            true,
        ),
        provider(
            "json",
            "json-source-v1",
            vec![
                "set",
                "insert",
                "delete",
                "rename_key",
                "ensure_present",
                "ensure_absent",
                "unset",
                "rename",
            ],
            vec!["json_pointer"],
            "source ranges, unrelated bytes",
            true,
        ),
        provider(
            "jsonc",
            "jsonc-source-v1",
            vec![
                "set",
                "insert",
                "delete",
                "rename_key",
                "ensure_present",
                "ensure_absent",
                "unset",
                "rename",
            ],
            vec!["json_pointer"],
            "comments and source ranges",
            true,
        ),
        provider(
            "toml",
            "toml-edit-narrow-v1",
            vec![
                "set",
                "insert",
                "delete",
                "rename_key",
                "ensure_present",
                "ensure_absent",
                "unset",
                "rename",
            ],
            vec!["dotted_key"],
            "comments, ordering where supported",
            true,
        ),
        provider(
            "yaml",
            "yaml-conservative-source-v1",
            vec!["set", "ensure_present", "delete", "ensure_absent"],
            vec!["yaml_path"],
            "comments for supported scalar forms",
            true,
        ),
        provider(
            "markdown",
            "markdown-regions-v2",
            vec![
                "replace_section",
                "ensure_section",
                "delete_section",
                "insert_after_heading",
                "replace_list_item",
                "ensure_list_item",
                "delete_list_item",
                "replace_fenced_block",
            ],
            vec!["heading", "section", "fenced_region"],
            "unrelated markdown bytes",
            true,
        ),
        provider(
            "dotenv",
            "dotenv-lines-v1",
            vec!["set", "unset", "ensure_present"],
            vec!["key"],
            "comments and unrelated lines",
            true,
        ),
        provider(
            "ini",
            "ini-source-v1",
            vec![
                "set",
                "unset",
                "ensure_present",
                "ensure_absent",
                "rename_key",
                "ensure_section",
            ],
            vec!["section_key", "key"],
            "comments, ordering, whitespace and unrelated lines",
            true,
        ),
        provider(
            "pattern",
            "regex-automata-bounded-v1",
            vec!["replace", "delete", "ensure_absent"],
            vec!["bounded_pattern"],
            "unrelated bytes",
            true,
        ),
        provider(
            "patch",
            "unified-diff-strict-v1",
            vec!["unified_diff"],
            vec!["exact_preimage", "exact_context"],
            "exact patch context",
            true,
        ),
        provider(
            "code",
            "tree-sitter-node-v1",
            vec![
                "replace_node",
                "insert_before_node",
                "insert_after_node",
                "remove_node",
            ],
            vec!["syntax_node_text", "syntax_node_kind"],
            "unrelated source bytes where ranges permit",
            true,
        ),
        provider(
            "web",
            "tree-sitter-web-node-v1",
            vec![
                "replace_node",
                "insert_before_node",
                "insert_after_node",
                "remove_node",
            ],
            vec!["syntax_node_text", "syntax_node_kind"],
            "unrelated source bytes where ranges permit",
            true,
        ),
        provider(
            "desired_state",
            "strict-derived-bounded-edits-v1",
            vec!["replace_desired_state"],
            vec!["workspace_relative_path", "desired_bytes"],
            "all divergence is explicit in supplied desired bytes",
            false,
        ),
        provider(
            "filesystem",
            "lifecycle-checked-v1",
            vec!["create_file", "delete_file", "rename_file", "move_file"],
            vec!["workspace_relative_path"],
            "identity and confinement",
            true,
        ),
    ]
}

fn provider(
    name: &'static str,
    version: &'static str,
    operations: Vec<&'static str>,
    selectors: Vec<&'static str>,
    preservation: &'static str,
    durable: bool,
) -> ProviderMetadata {
    ProviderMetadata {
        name,
        version,
        operations,
        selectors,
        preservation: vec![preservation],
        encodings: vec!["utf8", "utf8_bom"],
        // Lifecycle requests are guarded and recoverable as one-shot
        // operations, but are not yet composable with the content transaction
        // journal. Do not advertise a capability that the pipeline refuses.
        transaction_support: if name == "filesystem" {
            "single_file"
        } else {
            "single_file_and_multi_file"
        },
        durable_anchor_support: durable,
        languages: match name {
            "code" => syntax::registry()
                .iter()
                .filter(|spec| spec.family == LanguageFamily::Code)
                .map(|spec| spec.id)
                .collect(),
            "web" => syntax::registry()
                .iter()
                .filter(|spec| spec.family == LanguageFamily::Web)
                .map(|spec| spec.id)
                .collect(),
            _ => Vec::new(),
        },
    }
}

pub fn reason_metadata() -> Vec<ReasonMetadata> {
    let mut out = Vec::new();
    macro_rules! reason {
        ($code:literal, $meaning:literal, $why:literal, $cat:literal, $retry:expr, [$($cmd:literal),* $(,)?]) => {
            out.push(ReasonMetadata { code: $code, meaning: $meaning, why_refused: $why, recovery_category: $cat, retry_unchanged: $retry, relevant_commands: vec![$($cmd),*] });
        };
    }
    reason!(
        "CARDINALITY_MISMATCH",
        "The observed match count did not equal the requested count.",
        "Threadmoth will not choose an unintended target.",
        "narrow_target",
        false,
        ["inspect", "suggest", "preview"]
    );
    reason!(
        "TARGET_NOT_FOUND",
        "No exact target was found.",
        "The requested precondition is absent.",
        "correct_selector",
        false,
        ["inspect", "suggest"]
    );
    reason!(
        "TARGET_AMBIGUOUS",
        "More than one plausible target was found.",
        "Choosing the first match would be unsafe.",
        "choose_candidate",
        false,
        ["suggest", "preview"]
    );
    reason!(
        "CANDIDATE_SELECTION_INVALID",
        "The supplied candidate identity did not select one reported physical occurrence.",
        "Candidate selection is bound to the exact observed file hash, span and provider.",
        "refresh_candidate_identity",
        false,
        ["inspect", "preview", "explain"]
    );
    reason!(
        "STALE_IDENTITY",
        "The accepted source or guarded region changed.",
        "The request no longer describes the state being mutated.",
        "refresh_guard",
        false,
        ["inspect", "preview"]
    );
    reason!(
        "PLAN_STALE",
        "A prepared plan no longer matches one or more files.",
        "Applying any part of a stale plan could mutate the wrong state.",
        "rebuild_plan",
        false,
        ["plan", "explain", "suggest"]
    );
    reason!(
        "WORKSPACE_BUSY",
        "Another cooperating Threadmoth process owns the mutation boundary.",
        "Concurrent Threadmoth writers must not commit conflicting observations.",
        "retry_after_lock_release",
        false,
        ["doctor", "suggest"]
    );
    reason!(
        "SCHEMA_INVALID",
        "The request used a strict schema incorrectly.",
        "Threadmoth will not reinterpret an unknown or misspelled field.",
        "correct_request_schema",
        false,
        ["schema", "examples"]
    );
    reason!(
        "EFFECT_BUDGET_EXCEEDED",
        "The prepared effect exceeds a caller limit.",
        "No bytes are written when a declared budget fails.",
        "narrow_or_confirm_budget",
        false,
        ["suggest", "preview"]
    );
    reason!(
        "RESOURCE_LIMIT_EXCEEDED",
        "The request or observed file exceeds a built-in safety limit.",
        "Threadmoth bounds parser and memory exposure before mutation.",
        "reduce_input_or_split_work",
        false,
        ["capabilities", "inspect"]
    );
    reason!(
        "WORKSPACE_ESCAPE",
        "The path or scope leaves the workspace.",
        "Threadmoth only mutates confined workspace state.",
        "correct_path",
        false,
        ["inspect", "suggest"]
    );
    reason!(
        "SYMLINK_ESCAPE",
        "A symlink or reparse path escapes confinement.",
        "Path spelling cannot override physical containment.",
        "correct_path",
        false,
        ["inspect"]
    );
    reason!(
        "PROVIDER_UNSUPPORTED",
        "No advertised provider capability supports the request.",
        "Threadmoth never silently falls back to another provider.",
        "choose_supported_provider",
        false,
        ["capabilities", "suggest"]
    );
    reason!(
        "PRESERVATION_UNAVAILABLE",
        "The requested source-preservation guarantee cannot be proved.",
        "Threadmoth refuses lossy rewriting.",
        "choose_guarantee",
        false,
        ["capabilities", "suggest"]
    );
    reason!(
        "INVALID_STRUCTURE",
        "The candidate failed provider validation.",
        "A syntactically invalid result cannot be committed.",
        "correct_request",
        false,
        ["schema", "suggest"]
    );
    reason!(
        "ENCODING_UNSUPPORTED",
        "The file encoding is not safely supported.",
        "Threadmoth never guesses a legacy encoding.",
        "convert_explicitly",
        false,
        ["inspect"]
    );
    reason!(
        "TRANSACTION_CONFLICT",
        "A transaction member could not be prepared coherently.",
        "Partial transaction application is not implicit.",
        "split_or_correct_transaction",
        false,
        ["suggest", "preview"]
    );
    reason!(
        "OVERLAPPING_EDITS",
        "Prepared edits overlap.",
        "Overlapping byte ranges do not have an unambiguous result.",
        "split_operations",
        false,
        ["suggest", "preview"]
    );
    reason!(
        "GENERATED_FILE_REQUIRES_OPT_IN",
        "The target appears generated or marked do-not-edit.",
        "Generated state is protected by default.",
        "explicit_opt_in",
        false,
        ["suggest"]
    );
    reason!(
        "DESTINATION_EXISTS",
        "The requested file destination already exists.",
        "Lifecycle operations never overwrite silently.",
        "choose_destination",
        false,
        ["inspect", "suggest"]
    );
    reason!(
        "INVALID_INPUT",
        "The request could not be parsed or contains invalid input.",
        "A malformed request cannot be interpreted safely.",
        "correct_request",
        false,
        ["schema", "examples"]
    );
    reason!(
        "LOSSY_OPERATION_REQUIRES_OPT_IN",
        "The operation could change source details that were not authorized.",
        "Threadmoth does not silently accept lossy rewriting.",
        "choose_guarantee",
        false,
        ["capabilities", "suggest"]
    );
    reason!(
        "OPERATION_UNSUPPORTED",
        "The selected provider does not implement this operation.",
        "Provider selection is explicit and never falls back silently.",
        "choose_supported_operation",
        false,
        ["capabilities", "schema"]
    );
    reason!(
        "PROTOCOL_UNSUPPORTED",
        "The request protocol version is not supported by this binary.",
        "Threadmoth will not reinterpret a different contract.",
        "upgrade_or_use_matching_binary",
        false,
        ["capabilities", "schema"]
    );
    reason!(
        "REFUSED",
        "The request was refused without a more specific public reason.",
        "The request did not meet a safe execution precondition.",
        "inspect_certificate",
        false,
        ["explain", "suggest"]
    );
    reason!(
        "BINARY_INPUT",
        "The target contains binary data and is outside content-mutation scope.",
        "Threadmoth does not guess how binary bytes should be edited.",
        "choose_text_target",
        false,
        ["inspect"]
    );
    reason!(
        "PATH_UNMAPPABLE",
        "The declared path namespace cannot be mapped to this execution environment.",
        "Threadmoth will not guess a drive, mount, or distribution mapping.",
        "correct_path_namespace",
        false,
        ["capabilities", "inspect"]
    );
    reason!(
        "COMMIT_FAILED",
        "The prepared candidate could not be committed.",
        "The certificate reports commit or recovery state explicitly.",
        "recover",
        true,
        ["recover"]
    );
    reason!(
        "POST_COMMIT_VERIFICATION_FAILED",
        "The landed bytes differ from the verified candidate.",
        "The mutation cannot be reported as successful without matching evidence.",
        "recover",
        false,
        ["recover", "inspect"]
    );
    reason!(
        "IO_ERROR",
        "The workspace could not be read or accessed.",
        "The requested state was not safely observable.",
        "repair_workspace",
        true,
        ["doctor", "recover"]
    );
    reason!(
        "INTERNAL_INVARIANT",
        "An internal Threadmoth invariant failed while preparing a candidate.",
        "Threadmoth fails closed rather than committing an unverified result.",
        "report_defect",
        false,
        ["inspect", "recover"]
    );
    reason!(
        "FAILED",
        "The operation failed without a more specific public reason.",
        "The certificate contains the failure details and recovery state.",
        "inspect_certificate",
        false,
        ["recover"]
    );
    out
}

fn registry_ids(family: LanguageFamily) -> Vec<&'static str> {
    syntax::registry()
        .iter()
        .filter(|spec| spec.family == family)
        .map(|spec| spec.id)
        .collect()
}

pub fn capabilities() -> CapabilityManifest {
    let providers = provider_metadata();
    let operations = operation_metadata();
    let reason_codes = reason_metadata();
    let value = json!({
        "format_version": "1.3",
        "protocol_versions": SUPPORTED_PROTOCOL_VERSIONS,
        "protocol_version": PROTOCOL_VERSION,
        "threadmoth_version": env!("CARGO_PKG_VERSION"),
        "providers": providers,
        "operations": operations,
        "selectors": ["literal", "json_pointer", "dotted_key", "yaml_path", "section_key", "heading", "bounded_pattern", "syntax_node_text", "syntax_node_kind", "workspace_relative_path"],
        "preservation_guarantees": ["unrelated_bytes", "utf8", "utf8_bom", "lf", "crlf", "final_newline", "comments_where_supported"],
        "encodings": ["utf8", "utf8_bom"],
        "path_namespaces": ["native", "windows", "wsl", "posix"],
        "code_languages": registry_ids(LanguageFamily::Code),
        "web_formats": registry_ids(LanguageFamily::Web),
        "structural_operations": ["replace_node", "insert_before_node", "insert_after_node", "remove_node"],
        "ast_grounded": true,
        "ast_typed": true,
        "desired_state": true,
        "recovery_inspection": true,
        "guard_modes": ["immediate", "strict_snapshot", "region_snapshot", "structural_snapshot"],
        "transaction_capabilities": {"single_file": true, "multi_file": true, "rollback": true, "crash_recovery": true},
        "features": {"plans": true, "plan_apply": true, "postconditions": true, "candidate_selection": true, "composite_selectors": false},
        "supported_assertions": ["file_exists", "file_absent", "sha256", "literal_count"],
        "plan_limits": {"max_plan_bytes": MAX_PLAN_BYTES, "max_plan_operations": MAX_PLAN_OPERATIONS, "max_assertions": MAX_ASSERTIONS, "max_assertion_literal_bytes": MAX_ASSERTION_LITERAL_BYTES, "max_candidate_evidence": 4096},
        "resource_limits": {"max_request_bytes": MAX_REQUEST_BYTES, "max_transaction_requests": MAX_TRANSACTION_REQUESTS, "max_diagnostic_bytes": 4096, "max_pattern_bytes": 8192, "max_file_bytes": MAX_FILE_BYTES},
        "effect_budget_dimensions": ["max_files", "max_matches", "max_changed_regions", "max_changed_lines", "max_changed_bytes", "allowed_path_prefixes"],
        "reason_codes": reason_codes,
        "coverage_levels": ["structured", "syntax", "region", "exact", "opaque"],
        "target_registry": target_registry::registry()
    });
    let capability_set_id = digest_without_id(&value);
    CapabilityManifest {
        format_version: "1.3",
        protocol_versions: SUPPORTED_PROTOCOL_VERSIONS.to_vec(),
        protocol_version: PROTOCOL_VERSION,
        threadmoth_version: env!("CARGO_PKG_VERSION"),
        capability_set_id,
        providers: provider_metadata(),
        operations: operation_metadata(),
        selectors: vec![
            "literal",
            "json_pointer",
            "dotted_key",
            "yaml_path",
            "section_key",
            "heading",
            "bounded_pattern",
            "syntax_node_text",
            "syntax_node_kind",
            "workspace_relative_path",
        ],
        preservation_guarantees: vec![
            "unrelated_bytes",
            "utf8",
            "utf8_bom",
            "lf",
            "crlf",
            "final_newline",
            "comments_where_supported",
        ],
        encodings: vec!["utf8", "utf8_bom"],
        path_namespaces: vec!["native", "windows", "wsl", "posix"],
        code_languages: registry_ids(LanguageFamily::Code),
        web_formats: registry_ids(LanguageFamily::Web),
        structural_operations: vec![
            "replace_node",
            "insert_before_node",
            "insert_after_node",
            "remove_node",
        ],
        ast_grounded: true,
        ast_typed: true,
        desired_state: true,
        recovery_inspection: true,
        guard_modes: vec![
            "immediate",
            "strict_snapshot",
            "region_snapshot",
            "structural_snapshot",
        ],
        transaction_capabilities: TransactionCapabilities {
            single_file: true,
            multi_file: true,
            rollback: true,
            crash_recovery: true,
        },
        features: FeatureCapabilities {
            plans: true,
            plan_apply: true,
            postconditions: true,
            candidate_selection: true,
            composite_selectors: false,
        },
        supported_assertions: vec!["file_exists", "file_absent", "sha256", "literal_count"],
        plan_limits: PlanLimits {
            max_plan_bytes: MAX_PLAN_BYTES,
            max_plan_operations: MAX_PLAN_OPERATIONS,
            max_assertions: MAX_ASSERTIONS,
            max_assertion_literal_bytes: MAX_ASSERTION_LITERAL_BYTES,
            max_candidate_evidence: 4_096,
        },
        resource_limits: ResourceLimits {
            max_request_bytes: MAX_REQUEST_BYTES,
            max_transaction_requests: MAX_TRANSACTION_REQUESTS,
            max_diagnostic_bytes: 4_096,
            max_pattern_bytes: 8_192,
            max_file_bytes: MAX_FILE_BYTES,
        },
        effect_budget_dimensions: vec![
            "max_files",
            "max_matches",
            "max_changed_regions",
            "max_changed_lines",
            "max_changed_bytes",
            "allowed_path_prefixes",
        ],
        reason_codes: reason_metadata(),
        coverage_levels: vec!["structured", "syntax", "region", "exact", "opaque"],
        targets: target_registry::registry()
            .iter()
            .map(|target| serde_json::to_value(target).expect("target registry serializes"))
            .collect(),
    }
}

pub fn capability_view(selector: Option<&str>) -> Value {
    let manifest = serde_json::to_value(capabilities()).expect("capabilities serialize");
    let Some(selector) = selector else {
        return manifest;
    };
    let mut view = manifest;
    if let Some((provider, operation)) = selector.split_once('.') {
        let provider_entries: Vec<Value> = view["providers"]
            .as_array()
            .into_iter()
            .flat_map(|providers| providers.iter())
            .filter(|entry| entry["name"] == provider)
            .cloned()
            .collect();
        let supported = provider_entries.first().is_some_and(|entry| {
            entry["operations"]
                .as_array()
                .is_some_and(|operations| operations.iter().any(|value| value == operation))
        });
        view["providers"] = provider_entries
            .into_iter()
            .map(|mut entry| {
                entry["selected_operation"] = Value::String(operation.into());
                entry["operation_supported"] = Value::Bool(supported);
                entry
            })
            .collect::<Vec<_>>()
            .into();
        if supported {
            if let Some(metadata) = operation_metadata()
                .into_iter()
                .find(|entry| entry.name == operation)
            {
                view["selected_operation"] =
                    serde_json::to_value(metadata).expect("operation serializes");
            }
        } else {
            view["selection_error"] = json!({
                "provider": provider,
                "operation": operation,
                "reason": "operation is not advertised for this provider"
            });
        }
    } else {
        view["providers"] = view["providers"]
            .as_array()
            .into_iter()
            .flat_map(|providers| providers.iter())
            .filter(|entry| entry["name"] == selector)
            .cloned()
            .collect::<Vec<_>>()
            .into();
        view["selected_provider"] = Value::String(selector.into());
    }
    view
}

pub fn capabilities_for(path: &str, bytes: Option<&[u8]>) -> Value {
    let mut value = serde_json::to_value(capabilities()).expect("capabilities serialize");
    let detection = target_registry::detect(path, bytes);
    let provider = detection.provider.as_deref().unwrap_or_else(|| {
        if detection.confidence_class == "ambiguous" {
            "ambiguous"
        } else {
            "opaque"
        }
    });
    value["target"] = json!({
        "path": path,
        "provider": provider,
        "detection_basis": detection.basis,
        "candidates": detection.alternatives,
        "detection": detection.clone(),
        "understanding_level": detection.understanding_level,
        "preservation_level": detection.preservation_level,
        "fallback_routes": detection.fallback_routes
    });
    if provider != "ambiguous" {
        value["providers"] = value["providers"]
            .as_array()
            .into_iter()
            .flat_map(|providers| providers.iter())
            .filter(|entry| entry["name"] == provider)
            .cloned()
            .collect::<Vec<_>>()
            .into();
    }
    value
}

pub const DEFAULT_EXPANSION_MAX_BYTES: usize = 8 * 1024;
pub const DEFAULT_OUTLINE_MAX_ENTRIES: usize = 64;
const MAX_OUTLINE_ENTRIES: usize = 128;
const OBSERVATION_PREFIX: &str = "threadmoth:observation:v1";

#[derive(Debug)]
struct CachedOutline {
    key: String,
    nodes: Vec<syntax::OutlineNode>,
}

/// Bounded, process-local reuse for a long-lived MCP process.
///
/// The cache never supplies identity facts: callers re-read and hash the
/// current file before looking up an outline. It is deliberately not persisted
/// and has deterministic FIFO/LRU eviction.
#[derive(Debug)]
pub struct ObservationCache {
    entries: VecDeque<CachedOutline>,
    max_entries: usize,
    max_bytes: usize,
    bytes: usize,
    hits: usize,
    misses: usize,
}

impl Default for ObservationCache {
    fn default() -> Self {
        Self::new(16, 256 * 1024)
    }
}

impl ObservationCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub fn hits(&self) -> usize {
        self.hits
    }

    pub fn misses(&self) -> usize {
        self.misses
    }

    fn get(&mut self, key: &str) -> Option<Vec<syntax::OutlineNode>> {
        let index = self.entries.iter().position(|entry| entry.key == key);
        let Some(index) = index else {
            self.misses += 1;
            return None;
        };
        let entry = self.entries.remove(index).expect("cache index exists");
        let nodes = entry.nodes.clone();
        self.entries.push_back(entry);
        self.hits += 1;
        Some(nodes)
    }

    fn insert(&mut self, key: String, nodes: Vec<syntax::OutlineNode>) {
        let size = nodes
            .iter()
            .map(|node| node.label.len() + node.kind.len() + 48)
            .sum::<usize>();
        if size > self.max_bytes {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            if let Some(old) = self.entries.remove(index) {
                self.bytes = self.bytes.saturating_sub(outline_size(&old.nodes));
            }
        }
        while self.entries.len() >= self.max_entries || self.bytes + size > self.max_bytes {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(outline_size(&old.nodes));
        }
        self.bytes += size;
        self.entries.push_back(CachedOutline { key, nodes });
    }
}

fn outline_size(nodes: &[syntax::OutlineNode]) -> usize {
    nodes
        .iter()
        .map(|node| node.label.len() + node.kind.len() + 48)
        .sum()
}

/// Return the canonical read-only identity facts used by both the CLI and
/// MCP discovery surfaces. Its shape is intentionally unchanged.
pub fn inspect(workspace: &Workspace, path: &str) -> Result<Value, String> {
    let (_, value) = read_identity(workspace, path)?;
    Ok(value)
}

/// Inspect identity, a compact structural outline, or one exact observation.
/// Handles are observations only; they never authorize mutation.
pub fn inspect_view(
    workspace: &Workspace,
    path: &str,
    view: Option<&str>,
    handle: Option<&str>,
    max_bytes: Option<usize>,
    max_entries: Option<usize>,
    cache: Option<&mut ObservationCache>,
) -> Result<Value, String> {
    match view.unwrap_or("identity") {
        "identity" => {
            if handle.is_some() {
                return Err("identity view does not accept an observation handle".into());
            }
            inspect(workspace, path)
        }
        "outline" => {
            if handle.is_some() {
                return Err("outline view does not accept an observation handle".into());
            }
            outline_view(workspace, path, max_entries, cache)
        }
        "expand" => expand_view(workspace, path, handle, max_bytes),
        other => Err(format!("unsupported inspect view: {other}")),
    }
}

fn read_identity(workspace: &Workspace, path: &str) -> Result<(Vec<u8>, Value), String> {
    let normalized = PathNormalizer::normalize(path, &PathNamespace::Native);
    let resolved = workspace
        .resolve_namespaced_path(path, &PathNamespace::Native)
        .map_err(|error| error.to_string())?;
    let bytes = workspace
        .read_file(&resolved)
        .map_err(|error| error.to_string())?;
    let detection = target_registry::detect(path, Some(&bytes));
    let newline = if bytes.windows(2).any(|window| window == b"\r\n") {
        "crlf"
    } else if bytes.contains(&b'\n') {
        "lf"
    } else {
        "none"
    };
    let detection_value = serde_json::to_value(&detection).expect("detection serializes");
    let byte_count = bytes.len();
    let hash = compute_sha256(&bytes);
    let encoding = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        "utf8_bom"
    } else {
        "utf8"
    };
    let final_newline = bytes.ends_with(b"\n");
    Ok((
        bytes,
        json!({
            "protocol_version": PROTOCOL_VERSION,
            "file_path": normalized,
            "bytes": byte_count,
            "sha256": hash,
            "encoding": encoding,
            "newline_profile": newline,
            "final_newline": final_newline
            ,"detection": detection_value
            ,"understanding_level": detection.understanding_level
            ,"preservation_level": detection.preservation_level
            ,"fallback_routes": detection.fallback_routes
        }),
    ))
}

fn outline_view(
    workspace: &Workspace,
    path: &str,
    max_entries: Option<usize>,
    mut cache: Option<&mut ObservationCache>,
) -> Result<Value, String> {
    let (bytes, identity) = read_identity(workspace, path)?;
    let detection = target_registry::detect(path, Some(&bytes));
    let Some(provider) = detection.provider.as_deref() else {
        return Ok(with_view(
            identity,
            "outline",
            json!({
                "available": false,
                "reason": "unsupported_provider"
            }),
        ));
    };
    let family = match provider {
        "code" => LanguageFamily::Code,
        "web" => LanguageFamily::Web,
        _ => {
            return Ok(with_view(
                identity,
                "outline",
                json!({
                    "available": false,
                    "reason": "unsupported_provider"
                }),
            ));
        }
    };
    let limit = max_entries
        .unwrap_or(DEFAULT_OUTLINE_MAX_ENTRIES)
        .clamp(1, MAX_OUTLINE_ENTRIES);
    let key = format!(
        "{}|{}|{}|{}|{}",
        PathNormalizer::normalize(path, &PathNamespace::Native),
        compute_sha256(&bytes),
        provider,
        detection.target_kind,
        limit
    );
    let (nodes, reuse) = if let Some(ref mut observed_cache) = cache {
        if let Some(nodes) = observed_cache.get(&key) {
            (nodes, "cache_hit")
        } else {
            let nodes = syntax::outline(&bytes, &detection.target_kind, family)
                .map_err(|error| format!("outline unavailable: {error:?}"))?;
            observed_cache.insert(key, nodes.clone());
            (nodes, "derived")
        }
    } else {
        (
            syntax::outline(&bytes, &detection.target_kind, family)
                .map_err(|error| format!("outline unavailable: {error:?}"))?,
            "derived",
        )
    };
    let truncated = nodes.len() > limit;
    let entries = nodes
        .iter()
        .take(limit)
        .map(|node| {
            json!({
                "handle": make_handle(path, &detection, provider, &bytes, node),
                "kind": node.kind,
                "label": node.label,
                "start_byte": node.start_byte,
                "end_byte": node.end_byte,
                "start_line": node.start_line,
                "end_line": node.end_line
            })
        })
        .collect::<Vec<_>>();
    Ok(with_view(
        identity,
        "outline",
        json!({
            "available": true,
            "entries": entries,
            "truncated": truncated,
            "max_entries": limit,
            "reuse": reuse
        }),
    ))
}

fn expand_view(
    workspace: &Workspace,
    path: &str,
    handle: Option<&str>,
    max_bytes: Option<usize>,
) -> Result<Value, String> {
    let handle = handle.ok_or_else(|| "expand view requires an observation handle".to_string())?;
    let observation = parse_handle(handle)?;
    let (bytes, identity) = read_identity(workspace, path)?;
    let normalized = PathNormalizer::normalize(path, &PathNamespace::Native);
    if observation.path != normalized {
        return Err("observation handle is bound to a different path".into());
    }
    let current_hash = identity["sha256"]
        .as_str()
        .ok_or_else(|| "identity hash is unavailable".to_string())?;
    if observation.sha256 != current_hash {
        return Err("stale observation handle: source identity changed".into());
    }
    if observation.start > observation.end || observation.end > bytes.len() {
        return Err("observation handle range is outside the current file".into());
    }
    let detection = target_registry::detect(path, Some(&bytes));
    let provider = detection.provider.as_deref().unwrap_or("opaque");
    if provider != observation.provider || detection.target_kind != observation.language {
        return Err("stale observation handle: provider or language changed".into());
    }
    let family = match provider {
        "code" => LanguageFamily::Code,
        "web" => LanguageFamily::Web,
        _ => return Err("observation handle has no supported structural provider".into()),
    };
    let nodes = syntax::outline(&bytes, &detection.target_kind, family)
        .map_err(|error| format!("observation expansion unavailable: {error:?}"))?;
    let node = nodes
        .iter()
        .find(|node| {
            node.kind == observation.kind
                && node.start_byte == observation.start
                && node.end_byte == observation.end
        })
        .ok_or_else(|| "stale observation handle: syntax region changed".to_string())?;
    let region = &bytes[node.start_byte..node.end_byte];
    let limit = max_bytes.unwrap_or(DEFAULT_EXPANSION_MAX_BYTES);
    if region.len() > limit {
        return Err(format!(
            "observation expansion exceeds max_bytes ({}, actual {})",
            limit,
            region.len()
        ));
    }
    let source = String::from_utf8(region.to_vec())
        .map_err(|_| "observation expansion is not valid UTF-8".to_string())?;
    let mut result = identity;
    result["view"] = Value::String("expand".into());
    result["expansion"] = json!({
        "available": true,
        "handle": handle,
        "kind": node.kind,
        "start_byte": node.start_byte,
        "end_byte": node.end_byte,
        "start_line": node.start_line,
        "end_line": node.end_line,
        "bytes": region.len(),
        "source": source
    });
    Ok(result)
}

fn with_view(mut identity: Value, view: &str, value: Value) -> Value {
    identity["view"] = Value::String(view.into());
    identity[view] = value;
    identity
}

struct Observation {
    sha256: String,
    path: String,
    provider: String,
    language: String,
    start: usize,
    end: usize,
    kind: String,
}

fn escape_handle_part(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut escaped = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/') {
            escaped.push(byte as char);
        } else {
            escaped.push('%');
            escaped.push(HEX[(byte >> 4) as usize] as char);
            escaped.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    escaped
}

fn unescape_handle_part(value: &str) -> Result<String, String> {
    let mut output = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("invalid observation handle escaping".into());
            }
            let high = (bytes[index + 1] as char)
                .to_digit(16)
                .ok_or_else(|| "invalid observation handle escaping".to_string())?;
            let low = (bytes[index + 2] as char)
                .to_digit(16)
                .ok_or_else(|| "invalid observation handle escaping".to_string())?;
            output.push(((high << 4) | low) as u8);
            index += 3;
        } else {
            if bytes[index] == b'|' {
                return Err("invalid observation handle escaping".into());
            }
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| "invalid observation handle UTF-8".into())
}

fn make_handle(
    path: &str,
    detection: &target_registry::Detection,
    provider: &str,
    bytes: &[u8],
    node: &syntax::OutlineNode,
) -> String {
    let path = PathNormalizer::normalize(path, &PathNamespace::Native);
    let sha256 = compute_sha256(bytes);
    let payload = format!(
        "{OBSERVATION_PREFIX}|{}|{}|{}|{}|{}|{}",
        sha256, path, provider, detection.target_kind, node.start_byte, node.end_byte
    );
    let payload = format!("{payload}|{}", node.kind);
    let digest = compute_sha256(payload.as_bytes());
    [
        OBSERVATION_PREFIX,
        &sha256,
        &escape_handle_part(&path),
        &escape_handle_part(provider),
        &escape_handle_part(&detection.target_kind),
        &node.start_byte.to_string(),
        &node.end_byte.to_string(),
        &escape_handle_part(&node.kind),
        &digest,
    ]
    .join("|")
}

fn parse_handle(handle: &str) -> Result<Observation, String> {
    let parts = handle.split('|').collect::<Vec<_>>();
    if parts.len() != 9 || parts[0] != OBSERVATION_PREFIX {
        return Err("invalid observation handle".into());
    }
    let path = unescape_handle_part(parts[2])?;
    let provider = unescape_handle_part(parts[3])?;
    let language = unescape_handle_part(parts[4])?;
    let kind = unescape_handle_part(parts[7])?;
    let start = parts[5]
        .parse::<usize>()
        .map_err(|_| "invalid observation handle range".to_string())?;
    let end = parts[6]
        .parse::<usize>()
        .map_err(|_| "invalid observation handle range".to_string())?;
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        OBSERVATION_PREFIX, parts[1], parts[2], parts[3], parts[4], start, end, parts[7]
    );
    if compute_sha256(payload.as_bytes()) != parts[8] {
        return Err("invalid observation handle digest".into());
    }
    Ok(Observation {
        sha256: parts[1].into(),
        path,
        provider,
        language,
        start,
        end,
        kind,
    })
}

fn digest_without_id(value: &Value) -> String {
    compute_sha256(
        serde_json::to_string(value)
            .expect("metadata serializes")
            .as_bytes(),
    )[..24]
        .into()
}

pub fn schema(scope: Option<&str>) -> Value {
    let mut document = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": format!("Threadmoth {} Protocol Schemas", env!("CARGO_PKG_VERSION")),
        "protocol_version": PROTOCOL_VERSION,
        "scope": scope.unwrap_or("all"),
        "request": schema_for!(Request),
        "response": schema_for!(crate::protocol::Certificate),
        "certificate": schema_for!(crate::protocol::Certificate),
        "transaction_request": schema_for!(TransactionRequest),
        "transaction_certificate": schema_for!(crate::protocol::TransactionCertificate),
        "metadata": capabilities(),
    });
    let schema_id = digest_without_id(&document);
    document["schema_id"] = Value::String(schema_id);
    document
}

pub fn examples(topic: Option<&str>) -> Vec<Example> {
    let all = vec![
        example(
            "exact-text-replacement",
            "Replace one exact token.",
            text_request(
                "src/config.txt",
                TextOperation::Replace {
                    target: "old".into(),
                    replacement: "new".into(),
                },
            ),
            "APPLIED",
            "Exactly-one cardinality prevents a wrong duplicate edit.",
        ),
        example(
            "idempotent-ensure-present",
            "Ensure a line exists and make replay safe.",
            text_request(
                "README.md",
                TextOperation::EnsurePresent {
                    content: "managed line".into(),
                },
            ),
            "NO_CHANGE or APPLIED",
            "Desired-state operations are safe to replay.",
        ),
        example(
            "json-structural-set",
            "Set a JSON value without reserializing the document.",
            json_request(
                "config.json",
                JsonOperation::Set {
                    path: "$.name".into(),
                    value: json!("new"),
                },
            ),
            "APPLIED",
            "The JSON provider targets a source range.",
        ),
        example(
            "toml-structural-set",
            "Set a TOML key while preserving supported source details.",
            toml_request(
                "Cargo.toml",
                TomlOperation::Set {
                    path: "package.name".into(),
                    value: TomlValueWrapper::String("threadmoth".into()),
                },
            ),
            "APPLIED",
            "The provider validates the candidate before commit.",
        ),
        example(
            "preview",
            "Inspect a prepared mutation without writing.",
            text_request(
                "x.txt",
                TextOperation::Replace {
                    target: "a".into(),
                    replacement: "b".into(),
                },
            ),
            "APPLIED with dry_run commit",
            "Preview produces the same certificate shape without a write.",
        ),
        example(
            "safe-file-creation",
            "Create a file only when the destination is absent.",
            file_request(
                "new.txt",
                FileOperation::CreateFile {
                    expected_absent: true,
                    content: b"hello\n".to_vec(),
                },
            ),
            "APPLIED",
            "Creation uses an explicit no-overwrite precondition.",
        ),
        example(
            "safe-deletion",
            "Delete a file whose identity is known.",
            file_request(
                "old.txt",
                FileOperation::DeleteFile {
                    expected_hash: "SHA256_OF_CURRENT_FILE".into(),
                },
            ),
            "APPLIED",
            "Deletion is guarded by content identity.",
        ),
        example(
            "multi-operation-transaction",
            "Apply coherent operations to one file.",
            transaction_request(vec![
                text_request(
                    "x.txt",
                    TextOperation::Replace {
                        target: "one".into(),
                        replacement: "first".into(),
                    },
                ),
                text_request(
                    "x.txt",
                    TextOperation::Replace {
                        target: "two".into(),
                        replacement: "second".into(),
                    },
                ),
            ]),
            "APPLIED",
            "All operations resolve against one in-memory candidate.",
        ),
        example(
            "multi-file-transaction",
            "Stage several files before commit.",
            transaction_request(vec![
                text_request(
                    "a.txt",
                    TextOperation::Replace {
                        target: "a".into(),
                        replacement: "b".into(),
                    },
                ),
                text_request(
                    "b.txt",
                    TextOperation::EnsurePresent {
                        content: "managed".into(),
                    },
                ),
            ]),
            "APPLIED",
            "Preparation completes before any member is written.",
        ),
        example(
            "ambiguous-refusal",
            "Refuse a duplicate exact target.",
            text_request(
                "x.txt",
                TextOperation::Replace {
                    target: "duplicate".into(),
                    replacement: "new".into(),
                },
            ),
            "REFUSED / TARGET_AMBIGUOUS",
            "Threadmoth returns candidates instead of choosing one.",
        ),
        example(
            "refusal-recovery",
            "Feed a refusal certificate back to discovery.",
            text_request(
                "x.txt",
                TextOperation::Replace {
                    target: "missing".into(),
                    replacement: "new".into(),
                },
            ),
            "REFUSED / TARGET_NOT_FOUND",
            "Use suggest --from-refusal to obtain corrected skeletons.",
        ),
        example(
            "effect-budget-refusal",
            "Reject a candidate that exceeds a caller limit.",
            text_request(
                "x.txt",
                TextOperation::Replace {
                    target: "old".into(),
                    replacement: "new".into(),
                },
            ),
            "REFUSED / EFFECT_BUDGET_EXCEEDED",
            "Budgets are checked before commit.",
        ),
        example(
            "strict-patch",
            "Apply an exact unified diff or refuse.",
            patch_request("x.txt"),
            "APPLIED",
            "Patch context is exact; fuzzy relocation is not used.",
        ),
    ];
    match topic {
        Some(wanted) => all
            .into_iter()
            .filter(|e| topic_matches(e.topic, wanted))
            .collect(),
        None => all,
    }
}

fn topic_matches(topic: &str, wanted: &str) -> bool {
    topic == wanted || topic.replace('-', "_") == wanted || topic.contains(wanted)
}

fn example<T: Serialize>(
    topic: &'static str,
    intent: &'static str,
    request: T,
    outcome: &'static str,
    safety: &'static str,
) -> Example {
    Example {
        topic,
        intent,
        request: serde_json::to_value(request).expect("example request serializes"),
        representative_response: json!({"outcome": outcome, "protocol_version": PROTOCOL_VERSION}),
        safety_property: safety,
    }
}

fn base_request(path: &str, operation: OperationPayload) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("example-{path}"),
        allow_generated: false,
        file_path: path.into(),
        namespace: PathNamespace::Native,
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            max_files: Some(1),
            max_matches: Some(1),
            ..Default::default()
        },
        operation,
    }
}
fn text_request(path: &str, operation: TextOperation) -> Request {
    base_request(path, OperationPayload::Text(operation))
}
fn json_request(path: &str, operation: JsonOperation) -> Request {
    base_request(path, OperationPayload::Json(operation))
}
fn toml_request(path: &str, operation: TomlOperation) -> Request {
    base_request(path, OperationPayload::Toml(operation))
}
fn file_request(path: &str, operation: FileOperation) -> Request {
    base_request(path, OperationPayload::File(operation))
}
fn patch_request(path: &str) -> Request {
    base_request(
        path,
        OperationPayload::Patch(PatchOperation::UnifiedDiff {
            patch: "--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-old\n+new\n".into(),
        }),
    )
}
fn transaction_request(requests: Vec<Request>) -> TransactionRequest {
    TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: "example-transaction".into(),
        requests,
        budget: EffectBudget {
            max_files: Some(2),
            max_matches: Some(2),
            ..Default::default()
        },
    }
}

pub fn commands() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "capabilities",
            "Discover providers, operations, guarantees and limits.",
        ),
        (
            "examples",
            "See small, current, validated request examples.",
        ),
        (
            "schema",
            "Inspect the exact local protocol and schema fingerprint.",
        ),
        (
            "explain",
            "Understand a stable refusal or failure reason code.",
        ),
        (
            "inspect",
            "Read identity facts, a bounded structural outline or one exact expansion without mutation.",
        ),
        ("preview", "Prepare and certify a mutation without writing."),
        (
            "mutate",
            "Prepare, verify, commit and certify one mutation.",
        ),
        ("transact", "Prepare and commit a guarded transaction."),
        (
            "recover",
            "Inspect and deterministically recover interrupted commits.",
        ),
        ("suggest", "Generate a safe request skeleton for a target."),
        (
            "benchmark",
            "Measure correctness-checked mutation performance with selectable profiles.",
        ),
        (
            "torture",
            "Run deterministic safety regressions and the FOOTGUN-100 suite.",
        ),
    ]
}

pub fn command_help(command: &str) -> Option<String> {
    let text = match command {
        "mutate" | "preview" => "Reads a JSON Request from stdin or --request FILE and emits one Certificate. preview never writes; mutate commits only after validation and budget checks.",
        "capabilities" => "Use capabilities [PROVIDER] [PROVIDER.OPERATION] or --for PATH; add --json --all for the complete machine manifest.",
        "examples" => "Use examples [TOPIC] to print current, schema-valid request patterns.",
        "schema" => "Use schema [request|response|PROVIDER|OPERATION] [--json] [--pretty] to inspect the local contract and schema_id.",
        "explain" => "Use explain REASON_CODE [--json] to get meaning, evidence interpretation and safe recovery guidance.",
        "suggest" => "Use suggest PATH [--goal GOAL] [--at SELECTOR] [--mode minimal|safe|full], or suggest --from-refusal CERTIFICATE.",
        "inspect" => "Read a workspace-relative target's identity, encoding and newline profile, or use --outline/--expand for bounded structural observations; it never mutates.",
        "transact" => "Reads a TransactionRequest and stages every member before commit; transaction-preview prepares without writing.",
        "recover" => "Inspect local recovery journals and complete or restore interrupted transactions with evidence.",
        "benchmark" => "Use benchmark [quick|standard|tough] [--json] for correctness-checked dry-run performance measurements. The tough profile adds large files, long lines, many lines and repeated small-file workloads.",
        "torture" => "Run deterministic refusal, preservation, transaction-cleanup, symlink and FOOTGUN-100 checks in a disposable workspace. SKIP is reported for capabilities unavailable on the host.",
        _ => return None,
    };
    Some(text.into())
}

pub fn find_help(term: &str) -> Vec<(&'static str, &'static str)> {
    let term = term.to_ascii_lowercase();
    commands()
        .into_iter()
        .filter(|(name, description)| {
            name.contains(&term) || description.to_ascii_lowercase().contains(&term)
        })
        .collect()
}

pub fn reason(code: &str) -> Option<ReasonMetadata> {
    reason_metadata()
        .into_iter()
        .find(|entry| entry.code.eq_ignore_ascii_case(code))
}

pub fn detect_provider(path: &str, bytes: Option<&[u8]>) -> (String, String, Vec<String>) {
    let detection = target_registry::detect(path, bytes);
    let provider = detection.provider.clone().unwrap_or_else(|| {
        if detection.confidence_class == "ambiguous" {
            "ambiguous".into()
        } else {
            "opaque".into()
        }
    });
    (provider, detection.basis, detection.alternatives)
}

pub fn suggest(
    path: &str,
    goal: Option<&str>,
    at: Option<&str>,
    mode: &str,
    bytes: Option<&[u8]>,
) -> Suggestion {
    let detection = target_registry::detect(path, bytes);
    let detected = detection.provider.clone().unwrap_or_else(|| {
        if detection.confidence_class == "ambiguous" {
            "ambiguous".into()
        } else {
            "opaque".into()
        }
    });
    let basis = detection.basis.clone();
    let candidates = detection.alternatives.clone();
    // An ambiguous content-based detection is evidence, not permission to
    // choose the first provider. Keep the request template empty until the
    // caller explicitly selects a provider through the path/operation it
    // submits.
    let selected = if candidates.is_empty() && detected != "opaque" {
        Some(detected.as_str())
    } else {
        None
    };
    let requested_goal = goal.map(str::to_ascii_lowercase);
    let goal_name = requested_goal.as_deref().unwrap_or("replace-text");
    let allowed_goal = matches!(
        goal_name,
        "replace-text"
            | "set-value"
            | "add-item"
            | "remove-item"
            | "rename"
            | "ensure-present"
            | "ensure-absent"
            | "move"
            | "create-file"
            | "delete-file"
            | "apply-patch"
            | "transact"
    );
    let mode = match mode {
        "minimal" | "safe" | "full" => mode,
        _ => "safe",
    };
    let budget = if mode == "safe" {
        EffectBudget {
            max_files: Some(1),
            max_matches: Some(1),
            ..Default::default()
        }
    } else {
        EffectBudget::default()
    };
    let template = allowed_goal
        .then(|| selected.and_then(|provider| template_for(provider, path, goal_name, at, &budget)))
        .flatten();
    let mut alternatives = Vec::new();
    if selected == Some("text") {
        alternatives.push(
            serde_json::to_value(base_request(
                path,
                OperationPayload::Pattern(PatternOperation::Replace {
                    pattern: "BOUNDED_PATTERN".into(),
                    replacement: "NEW_VALUE".into(),
                }),
            ))
            .expect("suggestion serializes"),
        );
    }
    if !candidates.is_empty() {
        alternatives.extend(candidates.iter().map(|provider| json!({"provider": provider, "path": path, "next": format!("threadmoth suggest {path} --goal {goal_name}")})));
    }
    Suggestion {
        provider: detected.clone(),
        language: syntax::lookup(&detection.target_kind).map(|spec| spec.id.to_owned()),
        detection_basis: basis,
        understanding_level: format!("{:?}", detection.understanding_level).to_ascii_lowercase(),
        preservation_level: format!("{:?}", detection.preservation_level).to_ascii_lowercase(),
        fallback_routes: detection.fallback_routes.clone(),
        goal: goal.map(str::to_owned),
        mode: mode.into(),
        recommended_operation: template
            .as_ref()
            .and_then(|value| value.get("operation"))
            .and_then(|value| value.get("operation").or_else(|| value.get("type")))
            .and_then(|value| value.get("type").or(Some(value)))
            .and_then(Value::as_str)
            .map(str::to_owned),
        rationale: if !allowed_goal {
            "The requested goal is outside the controlled 1.1 goal set; choose one of the advertised goals.".into()
        } else if candidates.is_empty() {
            format!("Use the most specific advertised provider for {}; preview before committing when the target is unfamiliar.", selected.unwrap_or("the target"))
        } else {
            "Provider detection is ambiguous; choose one of the candidate providers explicitly."
                .into()
        },
        request_template: template,
        guarantees: vec![
            "exact cardinality is explicit".into(),
            "max_files=1 and max_matches=1 are conservative defaults".into(),
            "preview is available before commit".into(),
        ],
        budget_defaults: budget,
        alternatives,
        blocked_reasons: candidates
            .into_iter()
            .map(|candidate| format!("provider detection remains ambiguous: {candidate}"))
            .chain((!allowed_goal).then_some(format!("unsupported controlled goal: {goal_name}")))
            .collect(),
        capability_set_id: capabilities().capability_set_id,
    }
}

fn template_for(
    provider: &str,
    path: &str,
    goal: &str,
    at: Option<&str>,
    budget: &EffectBudget,
) -> Option<Value> {
    let operation = match goal {
        "create-file" => OperationPayload::File(FileOperation::CreateFile {
            expected_absent: true,
            content: Vec::new(),
        }),
        "delete-file" => OperationPayload::File(FileOperation::DeleteFile {
            expected_hash: "SHA256_OF_CURRENT_FILE".into(),
        }),
        "apply-patch" => OperationPayload::Patch(PatchOperation::UnifiedDiff {
            patch: "--- a/PATH\n+++ b/PATH\n@@ -1 +1 @@\n-OLD\n+NEW\n".into(),
        }),
        "ensure-present" => match provider {
            "json" | "jsonc" => OperationPayload::Json(JsonOperation::EnsurePresent {
                path: at.unwrap_or("$.KEY").into(),
                value: json!("VALUE"),
            }),
            "toml" => OperationPayload::Toml(TomlOperation::EnsurePresent {
                path: at.unwrap_or("key").into(),
                value: TomlValueWrapper::String("VALUE".into()),
            }),
            "yaml" => OperationPayload::Yaml(YamlOperation::EnsurePresent {
                path: at.unwrap_or("key").into(),
                value: json!("VALUE"),
            }),
            "dotenv" => OperationPayload::Dotenv(DotenvOperation::EnsurePresent {
                key: at.unwrap_or("KEY").into(),
                value: "VALUE".into(),
            }),
            "ini" => OperationPayload::Ini(crate::provider::ini::IniOperation::EnsurePresent {
                path: at.unwrap_or("SECTION.KEY").into(),
                value: "VALUE".into(),
            }),
            _ => OperationPayload::Text(TextOperation::EnsurePresent {
                content: "CONTENT_TO_ENSURE".into(),
            }),
        },
        "ensure-absent" | "remove-item" => match provider {
            "json" | "jsonc" => OperationPayload::Json(if goal == "remove-item" {
                JsonOperation::Delete {
                    path: at.unwrap_or("$.KEY").into(),
                }
            } else {
                JsonOperation::EnsureAbsent {
                    path: at.unwrap_or("$.KEY").into(),
                }
            }),
            "toml" => OperationPayload::Toml(if goal == "remove-item" {
                TomlOperation::Delete {
                    path: at.unwrap_or("key").into(),
                }
            } else {
                TomlOperation::EnsureAbsent {
                    path: at.unwrap_or("key").into(),
                }
            }),
            "yaml" => OperationPayload::Yaml(if goal == "remove-item" {
                YamlOperation::Delete {
                    path: at.unwrap_or("key").into(),
                }
            } else {
                YamlOperation::EnsureAbsent {
                    path: at.unwrap_or("key").into(),
                }
            }),
            "dotenv" => OperationPayload::Dotenv(DotenvOperation::Unset {
                key: at.unwrap_or("KEY").into(),
            }),
            "ini" => OperationPayload::Ini(if goal == "remove-item" {
                crate::provider::ini::IniOperation::Unset {
                    path: at.unwrap_or("SECTION.KEY").into(),
                }
            } else {
                crate::provider::ini::IniOperation::EnsureAbsent {
                    path: at.unwrap_or("SECTION.KEY").into(),
                }
            }),
            _ => OperationPayload::Text(TextOperation::EnsureAbsent {
                target: "EXACT_TARGET".into(),
            }),
        },
        "rename" => match provider {
            "json" | "jsonc" => OperationPayload::Json(JsonOperation::RenameKey {
                path: at.unwrap_or("$.OLD_KEY").into(),
                new_key: "NEW_KEY".into(),
            }),
            "toml" => OperationPayload::Toml(TomlOperation::RenameKey {
                path: at.unwrap_or("old.key").into(),
                new_key: "new_key".into(),
            }),
            "ini" => OperationPayload::Ini(crate::provider::ini::IniOperation::RenameKey {
                path: at.unwrap_or("SECTION.OLD_KEY").into(),
                new_key: "NEW_KEY".into(),
            }),
            "code" => OperationPayload::Code(CodeOperation::ReplaceNode {
                language: language_for_path(path),
                target: "OLD_NODE".into(),
                replacement: "NEW_NODE".into(),
                node_kind: None,
            }),
            "web" => OperationPayload::Web(WebOperation::ReplaceNode {
                language: language_for_path(path),
                target: "OLD_NODE".into(),
                replacement: "NEW_NODE".into(),
                node_kind: None,
            }),
            _ => OperationPayload::Text(TextOperation::Rename {
                target: "OLD_TEXT".into(),
                replacement: "NEW_TEXT".into(),
            }),
        },
        "set-value" => match provider {
            "json" | "jsonc" => OperationPayload::Json(JsonOperation::Set {
                path: at.unwrap_or("$.KEY").into(),
                value: json!("NEW_VALUE"),
            }),
            "toml" => OperationPayload::Toml(TomlOperation::Set {
                path: at.unwrap_or("key").into(),
                value: TomlValueWrapper::String("NEW_VALUE".into()),
            }),
            "yaml" => OperationPayload::Yaml(YamlOperation::Set {
                path: at.unwrap_or("key").into(),
                value: json!("NEW_VALUE"),
            }),
            "ini" => OperationPayload::Ini(crate::provider::ini::IniOperation::Set {
                path: at.unwrap_or("SECTION.KEY").into(),
                value: "NEW_VALUE".into(),
            }),
            "code" => OperationPayload::Code(CodeOperation::ReplaceNode {
                language: language_for_path(path),
                target: "OLD_LITERAL".into(),
                replacement: "NEW_LITERAL".into(),
                node_kind: None,
            }),
            "web" => OperationPayload::Web(WebOperation::ReplaceNode {
                language: language_for_path(path),
                target: "OLD_NODE".into(),
                replacement: "NEW_NODE".into(),
                node_kind: None,
            }),
            _ => OperationPayload::Text(TextOperation::Set {
                target: "OLD_VALUE".into(),
                replacement: "NEW_VALUE".into(),
            }),
        },
        "add-item" => match provider {
            "json" | "jsonc" => OperationPayload::Json(JsonOperation::Insert {
                path: at.unwrap_or("$").into(),
                key_or_index: "KEY_OR_INDEX".into(),
                value: json!("VALUE"),
            }),
            "toml" => OperationPayload::Toml(TomlOperation::Insert {
                path: at.unwrap_or("").into(),
                key: "KEY".into(),
                value: TomlValueWrapper::String("VALUE".into()),
            }),
            "dotenv" => OperationPayload::Dotenv(DotenvOperation::Set {
                key: at.unwrap_or("KEY").into(),
                value: "VALUE".into(),
            }),
            _ => OperationPayload::Text(TextOperation::EnsurePresent {
                content: "ITEM_TO_ADD".into(),
            }),
        },
        "move" => OperationPayload::Text(TextOperation::Move {
            target: "EXACT_TARGET".into(),
            before: "EXACT_DESTINATION".into(),
        }),
        "transact" => {
            return Some(
                serde_json::to_value(transaction_request(vec![text_request(
                    path,
                    TextOperation::Replace {
                        target: "OLD".into(),
                        replacement: "NEW".into(),
                    },
                )]))
                .expect("suggestion serializes"),
            )
        }
        "replace-text" => match provider {
            "code" => OperationPayload::Code(CodeOperation::ReplaceNode {
                language: language_for_path(path),
                target: "OLD_NODE".into(),
                replacement: "NEW_NODE".into(),
                node_kind: None,
            }),
            _ => OperationPayload::Text(TextOperation::Replace {
                target: "EXACT_TARGET".into(),
                replacement: "NEW_VALUE".into(),
            }),
        },
        _ => OperationPayload::Text(TextOperation::Replace {
            target: "EXACT_TARGET".into(),
            replacement: "NEW_VALUE".into(),
        }),
    };
    let mut request = match operation {
        OperationPayload::Json(operation) if provider == "jsonc" => {
            base_request(path, OperationPayload::Jsonc(operation))
        }
        operation => base_request(path, operation),
    };
    request.budget = budget.clone();
    let value = serde_json::to_value(request).expect("suggestion serializes");
    Some(value)
}

fn language_for_path(path: &str) -> String {
    let detection = target_registry::detect(path, None);
    if matches!(detection.provider.as_deref(), Some("code" | "web")) {
        detection.target_kind
    } else {
        "javascript".into()
    }
}

pub fn refusal_recovery(certificate: &crate::protocol::Certificate) -> Value {
    let reason = certificate
        .refusal_reason
        .as_ref()
        .map(|value| value.code())
        .unwrap_or("UNKNOWN");
    let mut suggestions = Vec::new();
    if let Some(crate::protocol::RefusalReason::DuplicateTarget { candidates, .. }) =
        certificate.refusal_reason.as_ref()
    {
        let mut candidates = candidates.clone();
        candidates.sort_by(|left, right| {
            (left.offset, left.end, &left.selection_id).cmp(&(
                right.offset,
                right.end,
                &right.selection_id,
            ))
        });
        for candidate in &candidates {
            let target = if candidate.target.is_empty() {
                candidate.context.clone()
            } else {
                candidate.target.clone()
            };
            let request_template = recovery_request(certificate, candidate, &target);
            suggestions.push(json!({
                "file_path": certificate.file_path,
                "provider": certificate.provider,
                "selector": target,
                "candidate_line": candidate.line,
                "candidate_start": candidate.start,
                "candidate_end": candidate.end,
                "candidate_fingerprint": candidate.anchor_sha256,
                "selection_id": candidate.selection_id,
                "request_template": request_template,
                "next": if request_template.is_some() { "preview this provider-preserving skeleton and confirm the intended candidate" } else { "retain the original provider request, add this candidate_guard and preview again" }
            }));
        }
    }
    if suggestions.is_empty() {
        suggestions.push(json!({
            "file_path": certificate.file_path,
            "provider": certificate.provider,
            "request_template": recovery_request_without_candidate(certificate),
            "next": "inspect the target, narrow the selector or explicitly correct the guard/budget",
            "no_safe_automatic_retry_template": !matches!(certificate.provider.as_str(), "text" | "pattern" | "markdown" | "code" | "web")
        }));
    }
    json!({"reason_code": reason, "capability_set_id": capabilities().capability_set_id, "source_certificate": certificate.request_id, "recovery": certificate.recovery, "suggestions": suggestions, "blocked_reasons": [format!("{reason} must be corrected before retry")], "safe_retry": false})
}

fn recovery_request(
    certificate: &crate::protocol::Certificate,
    candidate: &crate::protocol::Candidate,
    target: &str,
) -> Option<Value> {
    let operation = match certificate.provider.as_str() {
        "text" => OperationPayload::Text(TextOperation::Replace {
            target: target.into(),
            replacement: "REPLACEMENT".into(),
        }),
        "pattern" => OperationPayload::Pattern(PatternOperation::Replace {
            pattern: target.into(),
            replacement: "REPLACEMENT".into(),
        }),
        "code" => OperationPayload::Code(CodeOperation::ReplaceNode {
            language: "LANGUAGE_REQUIRED".into(),
            target: target.into(),
            replacement: "REPLACEMENT".into(),
            node_kind: candidate.node_kind.clone(),
        }),
        "web" => OperationPayload::Web(WebOperation::ReplaceNode {
            language: "LANGUAGE_REQUIRED".into(),
            target: target.into(),
            replacement: "REPLACEMENT".into(),
            node_kind: candidate.node_kind.clone(),
        }),
        "markdown" => OperationPayload::Markdown(
            crate::provider::markdown::MarkdownOperation::ReplaceListItem {
                target: target.into(),
                replacement: "REPLACEMENT".into(),
            },
        ),
        _ => return None,
    };
    let mut request = base_request(&certificate.file_path, operation);
    request.expected_pre_hash =
        (!certificate.pre_hash.is_empty()).then(|| certificate.pre_hash.clone());
    request.candidate_guard = (!candidate.selection_id.is_empty()).then(|| CandidateGuard {
        offset: candidate.offset,
        selection_id: candidate.selection_id.clone(),
    });
    Some(serde_json::to_value(request).expect("recovery request serializes"))
}

fn recovery_request_without_candidate(certificate: &crate::protocol::Certificate) -> Option<Value> {
    let operation = match certificate.provider.as_str() {
        "text" => OperationPayload::Text(TextOperation::Replace {
            target: "EXACT_TARGET".into(),
            replacement: "REPLACEMENT".into(),
        }),
        "pattern" => OperationPayload::Pattern(PatternOperation::Replace {
            pattern: "BOUNDED_PATTERN".into(),
            replacement: "REPLACEMENT".into(),
        }),
        "code" => OperationPayload::Code(CodeOperation::ReplaceNode {
            language: "LANGUAGE_REQUIRED".into(),
            target: "EXACT_NODE_TEXT".into(),
            replacement: "REPLACEMENT".into(),
            node_kind: None,
        }),
        "web" => OperationPayload::Web(WebOperation::ReplaceNode {
            language: "LANGUAGE_REQUIRED".into(),
            target: "EXACT_NODE_TEXT".into(),
            replacement: "REPLACEMENT".into(),
            node_kind: None,
        }),
        _ => return None,
    };
    Some(
        serde_json::to_value(base_request(&certificate.file_path, operation))
            .expect("recovery request serializes"),
    )
}
