use crate::lifecycle::FileOperation;
use crate::path::PathNamespace;
use crate::pattern::PatternOperation;
use crate::provider::code::CodeOperation;
use crate::provider::dotenv::DotenvOperation;
use crate::provider::ini::IniOperation;
use crate::provider::json::JsonOperation;
use crate::provider::markdown::MarkdownOperation;
use crate::provider::patch::PatchOperation;
use crate::provider::text::TextOperation;
use crate::provider::toml::TomlOperation;
use crate::provider::web::WebOperation;
use crate::provider::yaml::YamlOperation;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PROTOCOL_VERSION: &str = "1.3.1";
pub const LEGACY_PROTOCOL_VERSION: &str = "1.1.0";
pub const PREVIOUS_PROTOCOL_VERSION: &str = "1.2.0";
pub const PREVIOUS_CURRENT_PROTOCOL_VERSION: &str = "1.3.0";
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    PROTOCOL_VERSION,
    PREVIOUS_CURRENT_PROTOCOL_VERSION,
    PREVIOUS_PROTOCOL_VERSION,
    LEGACY_PROTOCOL_VERSION,
];
pub const CANDIDATE_SELECTION_DOMAIN: &str = "threadmoth:candidate-selection:v1";
pub const MAX_REQUEST_BYTES: usize = 1_048_576;
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TRANSACTION_REQUESTS: usize = 256;
pub const MAX_PLAN_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PLAN_OPERATIONS: usize = 256;
pub const MAX_ASSERTIONS: usize = 256;
pub const MAX_ASSERTION_LITERAL_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(
    tag = "provider",
    content = "operation",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OperationPayload {
    Text(TextOperation),
    Json(JsonOperation),
    Jsonc(JsonOperation),
    Toml(TomlOperation),
    Pattern(PatternOperation),
    Markdown(MarkdownOperation),
    Yaml(YamlOperation),
    #[serde(rename = "filesystem", alias = "file")]
    File(FileOperation),
    Code(CodeOperation),
    Dotenv(DotenvOperation),
    Ini(IniOperation),
    Patch(PatchOperation),
    Web(WebOperation),
    DesiredState(DesiredStateOperation),
}

/// A language-neutral plan constructor. The desired bytes are data supplied
/// by the caller; Threadmoth never executes the program that produced them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesiredStateOperation {
    Replace { desired_bytes: Vec<u8> },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub allow_generated: bool,
    pub file_path: String,
    #[serde(default)]
    pub namespace: PathNamespace,
    pub expected_pre_hash: Option<String>,
    #[serde(default)]
    pub region_guard: Option<RegionGuard>,
    #[serde(default)]
    pub candidate_guard: Option<CandidateGuard>,
    #[serde(default)]
    pub cardinality: Cardinality,
    #[serde(default)]
    pub budget: EffectBudget,
    pub operation: OperationPayload,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateGuard {
    pub offset: usize,
    pub selection_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegionGuard {
    pub anchor: String,
    pub target_sha256: String,
    #[serde(default)]
    pub mode: RegionGuardMode,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegionGuardMode {
    #[default]
    RegionSnapshot,
    StructuralSnapshot,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransactionRequest {
    pub version: String,
    pub transaction_id: String,
    pub requests: Vec<Request>,
    #[serde(default)]
    pub budget: EffectBudget,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assertion {
    FileExists {
        path: String,
    },
    FileAbsent {
        path: String,
    },
    Sha256 {
        path: String,
        equals: String,
    },
    LiteralCount {
        path: String,
        literal: String,
        #[serde(default)]
        exactly: Option<usize>,
        #[serde(default)]
        minimum: Option<usize>,
        #[serde(default)]
        maximum: Option<usize>,
    },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransactionCertificate {
    pub protocol_version: String,
    pub transaction_id: String,
    pub outcome: Outcome,
    pub certificates: Vec<Certificate>,
    pub rollback_state: String,
    pub transaction_guarantee: String,
    pub refusal_reason: Option<RefusalReason>,
    pub failure_reason: Option<FailureReason>,
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_diagnostic: Option<SchemaDiagnostic>,
}

/// A hard upper bound on the mutation's prepared effect. `None` means that
/// particular dimension is unbounded; Threadmoth still applies its own safety
/// limits for pathological requests.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct EffectBudget {
    pub max_files: Option<usize>,
    pub max_matches: Option<usize>,
    pub max_changed_regions: Option<usize>,
    pub max_changed_lines: Option<usize>,
    pub max_changed_bytes: Option<usize>,
    pub allowed_path_prefixes: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectUsage {
    pub files: usize,
    pub matches: usize,
    pub changed_regions: usize,
    pub changed_lines: usize,
    pub changed_bytes: usize,
    pub passed: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Applied,
    NoChange,
    Refused,
    Failed,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RefusalReason {
    CardinalityMismatch {
        expected: String,
        actual: usize,
    },
    CardinalityAmbiguous {
        path: String,
        count: usize,
    },
    StaleIdentity {
        expected_hash: String,
        actual_hash: String,
    },
    WorkspaceTraversal {
        path: String,
    },
    WorkspaceRootMismatch {
        target: String,
        workspace_root: String,
    },
    SymlinkEscape {
        path: String,
    },
    MissingTarget {
        target: String,
    },
    DuplicateTarget {
        target: String,
        count: usize,
        candidates: Vec<Candidate>,
        #[serde(default)]
        candidates_returned: usize,
        #[serde(default)]
        truncated: bool,
    },
    UnsupportedEncoding {
        details: String,
    },
    MalformedInput {
        details: String,
    },
    ProviderCapabilityMissing {
        provider: String,
        capability: String,
    },
    PreservationUnavailable {
        details: String,
    },
    LossyOperationRequiresOptIn {
        operation: String,
    },
    UnsupportedOperation {
        operation: String,
    },
    UnsupportedProtocolVersion {
        requested: String,
        supported: String,
    },
    TransactionConflict {
        message: String,
    },
    Custom {
        message: String,
    },
    EffectBudgetExceeded {
        dimension: String,
        limit: usize,
        actual: usize,
    },
    ResourceLimitExceeded {
        dimension: String,
        limit: usize,
        actual: usize,
    },
    GeneratedFileRequiresOptIn {
        marker: String,
    },
    BinaryInput,
    DestinationExists {
        path: String,
    },
    UnmappablePath {
        path: String,
    },
    CandidateSelectionInvalid {
        offset: usize,
        selection_id: String,
        details: String,
    },
    PostconditionFailed {
        assertion: String,
        expected: String,
        observed: String,
        path: String,
        phase: String,
    },
    PlanInvalid {
        details: String,
    },
    PlanStale {
        path: String,
        expected_hash: String,
        actual_hash: String,
    },
    PlanTooLarge {
        dimension: String,
        limit: usize,
        actual: usize,
    },
    WorkspaceBusy {
        lock_path: String,
    },
}

impl RefusalReason {
    /// Stable machine identifier used by certificates and `threadmoth explain`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::CardinalityMismatch { .. } => "CARDINALITY_MISMATCH",
            Self::CardinalityAmbiguous { .. } | Self::DuplicateTarget { .. } => "TARGET_AMBIGUOUS",
            Self::StaleIdentity { .. } => "STALE_IDENTITY",
            Self::WorkspaceTraversal { .. } => "WORKSPACE_ESCAPE",
            Self::WorkspaceRootMismatch { .. } => "WORKSPACE_ROOT_MISMATCH",
            Self::SymlinkEscape { .. } => "SYMLINK_ESCAPE",
            Self::MissingTarget { .. } => "TARGET_NOT_FOUND",
            Self::UnsupportedEncoding { .. } => "ENCODING_UNSUPPORTED",
            Self::MalformedInput { .. } => "INVALID_INPUT",
            Self::ProviderCapabilityMissing { .. } => "PROVIDER_UNSUPPORTED",
            Self::PreservationUnavailable { .. } => "PRESERVATION_UNAVAILABLE",
            Self::LossyOperationRequiresOptIn { .. } => "LOSSY_OPERATION_REQUIRES_OPT_IN",
            Self::UnsupportedOperation { .. } => "OPERATION_UNSUPPORTED",
            Self::UnsupportedProtocolVersion { .. } => "PROTOCOL_UNSUPPORTED",
            Self::TransactionConflict { .. } => "TRANSACTION_CONFLICT",
            Self::Custom { .. } => "REFUSED",
            Self::EffectBudgetExceeded { .. } => "EFFECT_BUDGET_EXCEEDED",
            Self::ResourceLimitExceeded { .. } => "RESOURCE_LIMIT_EXCEEDED",
            Self::GeneratedFileRequiresOptIn { .. } => "GENERATED_FILE_REQUIRES_OPT_IN",
            Self::BinaryInput => "BINARY_INPUT",
            Self::DestinationExists { .. } => "DESTINATION_EXISTS",
            Self::UnmappablePath { .. } => "PATH_UNMAPPABLE",
            Self::CandidateSelectionInvalid { .. } => "CANDIDATE_SELECTION_INVALID",
            Self::PostconditionFailed { .. } => "POSTCONDITION_FAILED",
            Self::PlanInvalid { .. } => "PLAN_INVALID",
            Self::PlanStale { .. } => "PLAN_STALE",
            Self::PlanTooLarge { .. } => "PLAN_TOO_LARGE",
            Self::WorkspaceBusy { .. } => "WORKSPACE_BUSY",
        }
    }
}

/// Deterministic, bounded advice for constructing the next legal request.
/// Threadmoth reports choices; the caller remains responsible for selecting one.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRemedy {
    pub kind: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_patch: Option<Value>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryInfo {
    pub requires_choice: bool,
    pub remedies: Vec<RecoveryRemedy>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SchemaDiagnostic {
    pub reason: String,
    pub field: String,
    pub location: String,
    pub expected_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_field: Option<String>,
    pub message: String,
}

/// Turn serde's strict parser error into bounded, agent-facing structure.
pub fn schema_diagnostic(error: &str) -> SchemaDiagnostic {
    let field = error
        .split("unknown field `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .or_else(|| {
            error
                .split("missing field `")
                .nth(1)
                .and_then(|rest| rest.split('`').next())
        })
        .unwrap_or("request")
        .to_owned();
    let pointer_typo = field == "pointer";
    SchemaDiagnostic {
        reason: "SCHEMA_INVALID".into(),
        field,
        location: if pointer_typo {
            "$.operation.operation.pointer".into()
        } else {
            "$".into()
        },
        expected_fields: if pointer_typo {
            vec!["path".into(), "value".into()]
        } else {
            Vec::new()
        },
        suggested_field: pointer_typo.then_some("path".into()),
        message: error.chars().take(512).collect(),
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub offset: usize,
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: usize,
    #[serde(default)]
    pub target: String,
    pub line: usize,
    pub context: String,
    pub anchor_sha256: String,
    #[serde(default)]
    pub selection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
}

pub fn candidate_selection_id(
    pre_hash: &str,
    provider: &str,
    start: usize,
    end: usize,
    target_sha256: &str,
) -> String {
    let canonical = format!(
        "{CANDIDATE_SELECTION_DOMAIN}\npre_hash={pre_hash}\nstart={start}\nend={end}\nprovider={provider}\ntarget_sha256={target_sha256}\n"
    );
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FailureReason {
    IoError {
        message: String,
    },
    ProviderError {
        details: String,
    },
    InternalInvariant {
        details: String,
    },
    CommitFailure {
        message: String,
    },
    PostCommitVerificationFailure {
        expected_hash: String,
        actual_hash: String,
    },
    ParseError {
        details: String,
    },
    WriteError {
        message: String,
    },
    Custom {
        message: String,
    },
    PostCommitAssertionFailed {
        assertion: String,
        expected: String,
        observed: String,
        path: String,
    },
}

impl FailureReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::IoError { .. } => "IO_ERROR",
            Self::ProviderError { .. } | Self::ParseError { .. } => "INVALID_STRUCTURE",
            Self::InternalInvariant { .. } => "INTERNAL_INVARIANT",
            Self::CommitFailure { .. } | Self::WriteError { .. } => "COMMIT_FAILED",
            Self::PostCommitVerificationFailure { .. } => "POST_COMMIT_VERIFICATION_FAILED",
            Self::Custom { .. } => "FAILED",
            Self::PostCommitAssertionFailed { .. } => "POSTCONDITION_FAILED",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Cardinality {
    #[default]
    ExactlyOne,
    #[serde(rename = "exactly")]
    Exactly(usize),
    All,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ByteEdit {
    pub offset: usize,
    pub delete_len: usize,
    pub replacement: Vec<u8>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MutationPlan {
    pub version: String,
    pub file_path: String,
    pub expected_pre_hash: String,
    pub edits: Vec<ByteEdit>,
    #[serde(default)]
    pub cardinality: Cardinality,
}

/// A portable, bounded, exact mutation prepared from a request. Applying this
/// artifact never asks a provider to relocate an edit: the pre-image hash and
/// exact byte ranges are authoritative guards.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub plan_id: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub transaction_id: Option<String>,
    pub operations: Vec<PreparedPlanOperation>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    pub budget: EffectBudget,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlanOperation {
    pub file_path: String,
    pub provider: String,
    pub request: Request,
    pub pre_hash: String,
    pub edits: Vec<ByteEdit>,
    pub prospective_hash: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum PlanApplyResult {
    Certificate(Certificate),
    Transaction(TransactionCertificate),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuralValidation {
    #[default]
    NotApplicable,
    Valid {
        format: String,
    },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreservationFacts {
    pub unrelated_bytes_changed: bool,
    pub line_endings_changed: bool,
    pub bom_changed: bool,
    pub final_newline_changed: bool,
    pub comments_preserved: Option<bool>,
    pub metadata: String,
    pub original_newline_profile: String,
    pub result_newline_profile: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommitGuarantee {
    pub mode: String,
    pub content_replacement: String,
    pub permissions: String,
    pub timestamps: String,
    pub acl_xattr: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    pub protocol_version: String,
    pub request_id: String,
    pub outcome: Outcome,
    pub file_path: String,
    pub provider: String,
    pub provider_version: String,
    pub expected_cardinality: Cardinality,
    pub observed_cardinality: Option<usize>,
    pub pre_hash: String,
    pub post_hash: Option<String>,
    pub changed_ranges: Vec<ByteRange>,
    pub changed_line_ranges: Vec<ByteRange>,
    pub diff_summary: Option<String>,
    pub diff_truncated: bool,
    pub structural_validation: StructuralValidation,
    pub preservation: PreservationFacts,
    pub commit: CommitGuarantee,
    pub refusal_reason: Option<RefusalReason>,
    pub failure_reason: Option<FailureReason>,
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_diagnostic: Option<SchemaDiagnostic>,
    pub diagnostics: Vec<String>,
    pub budget: EffectBudget,
    pub effect: EffectUsage,
    pub transaction_guarantee: String,
    pub recovery_state: String,
    #[serde(default)]
    pub desired_state: Option<DesiredStateEvidence>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesiredStateEvidence {
    pub mode: String,
    pub desired_hash: String,
    pub derived_region_count: usize,
    pub changed_lines: usize,
    pub changed_bytes: usize,
    pub verification: String,
}

impl Default for PreservationFacts {
    fn default() -> Self {
        Self {
            unrelated_bytes_changed: false,
            line_endings_changed: false,
            bom_changed: false,
            final_newline_changed: false,
            comments_preserved: None,
            metadata: "not_verified".into(),
            original_newline_profile: "unknown".into(),
            result_newline_profile: "unknown".into(),
        }
    }
}
impl Default for CommitGuarantee {
    fn default() -> Self {
        Self {
            mode: "not_committed".into(),
            content_replacement: "not_applicable".into(),
            permissions: "not_verified".into(),
            timestamps: "not_preserved_by_replacement".into(),
            acl_xattr: "unknown".into(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    #[test]
    fn outcome_serializes() {
        assert_eq!(
            serde_json::to_string(&Outcome::Applied).unwrap(),
            "\"APPLIED\""
        );
    }

    #[test]
    fn cardinality_serializes() {
        assert_eq!(
            serde_json::to_string(&Cardinality::ExactlyOne).unwrap(),
            "{\"type\":\"exactly_one\"}"
        );
        assert_eq!(
            serde_json::to_string(&Cardinality::Exactly(5)).unwrap(),
            "{\"type\":\"exactly\",\"value\":5}"
        );
    }

    #[test]
    fn filesystem_is_canonical_and_file_remains_a_compatibility_alias() {
        let legacy = r#"{"provider":"file","operation":{"type":"create_file","expected_absent":true,"content":[104,105]}}"#;
        let operation: OperationPayload = serde_json::from_str(legacy).unwrap();
        let rendered = serde_json::to_string(&operation).unwrap();
        assert!(rendered.contains("\"provider\":\"filesystem\""));

        let canonical = r#"{"provider":"filesystem","operation":{"type":"create_file","expected_absent":true,"content":[104,105]}}"#;
        let canonical_operation: OperationPayload = serde_json::from_str(canonical).unwrap();
        assert_eq!(operation, canonical_operation);
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        let json = r#"{"version":"0.1.0","file_path":"a","expected_pre_hash":null,"operation":{"type":"text","bogus":1}}"#;
        assert!(serde_json::from_str::<Request>(json).is_err());
    }

    #[test]
    fn unknown_nested_request_fields_are_rejected() {
        let cardinality = r#"{"type":"exactly_one","future":true}"#;
        assert!(serde_json::from_str::<Cardinality>(cardinality).is_err());
        let namespace = r#"{"type":"native","future":true}"#;
        assert!(serde_json::from_str::<PathNamespace>(namespace).is_err());
    }

    #[test]
    fn schema_generation_runs() {
        assert!(!serde_json::to_string(&schema_for!(Certificate))
            .unwrap()
            .is_empty());
    }
}
