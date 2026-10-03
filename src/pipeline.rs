#![forbid(unsafe_code)]
#![allow(clippy::result_large_err, clippy::type_complexity)]

use crate::diff_planner;
use crate::engine::{apply_byte_edits, compute_sha256, ByteEdit};
use crate::lifecycle::FileOperation;
use crate::path::PathNormalizer;
use crate::pattern::{self, PatternError};
use crate::protocol::{
    candidate_selection_id, Assertion, ByteEdit as PlanByteEdit, ByteRange, CandidateGuard,
    Certificate, CommitGuarantee, DesiredStateEvidence, DesiredStateOperation, EffectBudget,
    EffectUsage, FailureReason, MutationPlan, OperationPayload, Outcome, PlanApplyResult,
    PreparedPlan, PreparedPlanOperation, PreservationFacts, RecoveryInfo, RecoveryRemedy,
    RefusalReason, Request, StructuralValidation, TransactionCertificate, TransactionRequest,
    MAX_ASSERTIONS, MAX_ASSERTION_LITERAL_BYTES, MAX_FILE_BYTES, MAX_PLAN_BYTES,
    MAX_PLAN_OPERATIONS, MAX_TRANSACTION_REQUESTS, SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::provider::code::{self, CodeError, CodeOperation};
use crate::provider::dotenv::{self, DotenvError};
use crate::provider::ini::{self, IniError};
use crate::provider::json::{JsonProvider, JsonProviderError};
use crate::provider::jsonc::JsoncProvider;
use crate::provider::markdown::{self, MarkdownError};
use crate::provider::patch::{self, PatchError};
use crate::provider::text::{TextOperation, TextProvider, TextProviderError};
use crate::provider::toml::{TomlProvider, TomlProviderError};
use crate::provider::web::{self, WebError, WebOperation};
use crate::provider::yaml::{self, YamlError};
use crate::recovery::{self, Journal, JournalEntry};
use crate::workspace::{Workspace, WorkspaceError};
use memchr::memchr_iter;

pub fn execute_pipeline(
    workspace: &Workspace,
    plan: &MutationPlan,
    op: &TextOperation,
    dry_run: bool,
) -> Certificate {
    let request = Request {
        version: plan.version.clone(),
        request_id: String::new(),
        allow_generated: false,
        file_path: plan.file_path.clone(),
        namespace: Default::default(),
        expected_pre_hash: (!plan.expected_pre_hash.is_empty())
            .then(|| plan.expected_pre_hash.clone()),
        region_guard: None,
        candidate_guard: None,
        cardinality: plan.cardinality.clone(),
        budget: EffectBudget::default(),
        operation: OperationPayload::Text(op.clone()),
    };
    execute_request(workspace, &request, dry_run)
}

fn budget_path_is_allowed(
    workspace: &Workspace,
    path: &str,
    prefixes: &[String],
) -> Result<bool, WorkspaceError> {
    if prefixes.is_empty() {
        return Ok(true);
    }
    for prefix in prefixes {
        match workspace.is_within_allowed_prefix(path, prefix) {
            Ok(true) => return Ok(true),
            Ok(false) | Err(WorkspaceError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

pub fn execute_request(workspace: &Workspace, request: &Request, dry_run: bool) -> Certificate {
    let normalized_path = PathNormalizer::normalize(&request.file_path, &request.namespace);
    let provider = provider_name(&request.operation);
    let file_path = match workspace.resolve_namespaced_path(&request.file_path, &request.namespace)
    {
        Ok(path) => path,
        Err(error) => return workspace_error(request, &normalized_path, provider, error),
    };
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&request.version.as_str()) {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::UnsupportedProtocolVersion {
                requested: request.version.clone(),
                supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
            },
            String::new(),
        );
    }
    let allowed = match budget_path_is_allowed(
        workspace,
        &file_path,
        &request.budget.allowed_path_prefixes,
    ) {
        Ok(allowed) => allowed,
        Err(error) => return workspace_error(request, &file_path, provider, error),
    };
    if !allowed {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::WorkspaceTraversal {
                path: "path is outside requested budget scope".into(),
            },
            String::new(),
        );
    }
    let _mutation_lock = if dry_run {
        None
    } else {
        match workspace.acquire_mutation_lock() {
            Ok(lock) => Some(lock),
            Err(error) => return workspace_error(request, &file_path, provider, error),
        }
    };
    if let OperationPayload::File(operation) = &request.operation {
        return execute_file_operation(workspace, request, &file_path, operation, dry_run);
    }
    let original = match workspace.read_file(&file_path) {
        Ok(b) => b,
        Err(e) => return workspace_error(request, &file_path, provider, e),
    };
    if original.len() > MAX_FILE_BYTES {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::ResourceLimitExceeded {
                dimension: "max_file_bytes".into(),
                limit: MAX_FILE_BYTES,
                actual: original.len(),
            },
            String::new(),
        );
    }
    let pre_hash = compute_sha256(&original);
    if let Some(expected) = request
        .expected_pre_hash
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
        if expected != pre_hash {
            return refusal(
                request,
                &file_path,
                provider,
                RefusalReason::StaleIdentity {
                    expected_hash: expected.into(),
                    actual_hash: pre_hash.clone(),
                },
                pre_hash,
            );
        }
    }
    if request.candidate_guard.is_some()
        && request
            .expected_pre_hash
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::MalformedInput {
                details:
                    "candidate_guard requires expected_pre_hash from the observed source state"
                        .into(),
            },
            pre_hash,
        );
    }
    if request.candidate_guard.is_some()
        && !matches!(
            request.cardinality,
            crate::protocol::Cardinality::ExactlyOne
        )
    {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::CardinalityMismatch {
                expected: "exactly_one when candidate_guard is present".into(),
                actual: 1,
            },
            pre_hash,
        );
    }
    if let Some(guard) = &request.region_guard {
        if let Err(reason) = validate_region_guard(&original, guard, request) {
            return refusal(request, &file_path, provider, reason, pre_hash);
        }
    }
    if let Some(reason) = unsupported_encoding(&original) {
        return refusal(request, &file_path, provider, reason, pre_hash);
    }
    if original.contains(&0) {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::BinaryInput,
            pre_hash,
        );
    }
    if is_generated_file(&original) && !request.allow_generated {
        return refusal(
            request,
            &file_path,
            provider,
            RefusalReason::GeneratedFileRequiresOptIn {
                marker: generated_marker(&original).into(),
            },
            pre_hash,
        );
    }
    let edits = match plan_edits(&original, request, &file_path, &pre_hash) {
        Ok(x) => x,
        Err((r, _d)) => return refusal(request, &file_path, provider, r, pre_hash),
    };
    let engine_edits: Vec<ByteEdit> = edits
        .iter()
        .map(|e| ByteEdit {
            start: e.start,
            end: e.end,
            replacement: e.replacement.clone(),
        })
        .collect();
    let candidate = match apply_byte_edits(&original, &engine_edits) {
        Ok(x) => x,
        Err(e) => {
            return failure(
                request,
                &file_path,
                provider,
                pre_hash,
                FailureReason::InternalInvariant {
                    details: e.to_string(),
                },
            )
        }
    };
    if let Some(desired) = desired_bytes(request) {
        if candidate != desired {
            return failure(
                request,
                &file_path,
                provider,
                pre_hash,
                FailureReason::InternalInvariant {
                    details: "desired-state planner produced bytes different from the requested desired state".into(),
                },
            );
        }
    }
    let ranges = changed_ranges(&engine_edits);
    let line_ranges = changed_line_ranges(&original, &engine_edits);
    let effect = effect_usage(&original, &candidate, &engine_edits, &request.budget);
    if let Some((dimension, limit, actual)) = budget_violation(&effect, &request.budget) {
        return refusal_with_effect(
            request,
            &file_path,
            provider,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
            pre_hash,
            EffectUsage {
                passed: false,
                ..effect
            },
        );
    }
    let provider_structural = match validate_candidate(request, &candidate) {
        Ok(validation) => validation,
        Err(reason) => {
            return failure(request, &file_path, provider, pre_hash, reason);
        }
    };
    if candidate == original {
        let mut certificate = completed(
            request,
            &file_path,
            provider,
            pre_hash.clone(),
            Some(pre_hash.clone()),
            Outcome::NoChange,
            ranges,
            provider_structural,
            PreservationFacts::from_bytes(&original, &candidate),
            CommitGuarantee::default(),
            String::new(),
            false,
            effect.clone(),
        );
        attach_desired_evidence(
            desired_bytes(request),
            &engine_edits,
            &effect,
            &mut certificate,
        );
        return certificate;
    }
    let post_hash = compute_sha256(&candidate);
    let (diff, diff_truncated) = bounded_diff(&original, &engine_edits);
    if !dry_run {
        match workspace.write_file_atomic_checked(&file_path, &pre_hash, &candidate) {
            Ok(()) => {}
            Err(WorkspaceError::StaleIdentity { expected, actual }) => {
                return refusal(
                    request,
                    &file_path,
                    provider,
                    RefusalReason::StaleIdentity {
                        expected_hash: expected,
                        actual_hash: actual,
                    },
                    pre_hash,
                )
            }
            Err(e) => {
                return failure(
                    request,
                    &file_path,
                    provider,
                    pre_hash,
                    FailureReason::CommitFailure {
                        message: e.to_string(),
                    },
                )
            }
        }
    }
    let commit = if dry_run {
        CommitGuarantee {
            mode: "dry_run".into(),
            ..CommitGuarantee::default()
        }
    } else {
        CommitGuarantee {
            mode: "committed_atomic_replace".into(),
            content_replacement: "atomic replacement after staged flush".into(),
            permissions: "platform-dependent; not asserted".into(),
            timestamps: "not preserved".into(),
            acl_xattr: "unknown".into(),
        }
    };
    if !dry_run {
        let landed = match workspace.read_file(&file_path) {
            Ok(b) => b,
            Err(e) => {
                return failure(
                    request,
                    &file_path,
                    provider,
                    pre_hash,
                    FailureReason::PostCommitVerificationFailure {
                        expected_hash: post_hash.clone(),
                        actual_hash: format!("read failed: {e}"),
                    },
                )
            }
        };
        if landed != candidate {
            let actual = compute_sha256(&landed);
            return failure(
                request,
                &file_path,
                provider,
                pre_hash,
                FailureReason::PostCommitVerificationFailure {
                    expected_hash: post_hash,
                    actual_hash: actual,
                },
            );
        }
    }
    let mut certificate = completed(
        request,
        &file_path,
        provider,
        pre_hash,
        Some(post_hash),
        Outcome::Applied,
        ranges,
        provider_structural,
        PreservationFacts::from_bytes(&original, &candidate),
        commit,
        diff,
        diff_truncated,
        effect.clone(),
    );
    certificate.changed_line_ranges = line_ranges;
    attach_desired_evidence(
        desired_bytes(request),
        &engine_edits,
        &effect,
        &mut certificate,
    );
    certificate
}

struct PreparedContent {
    path: String,
    original: Vec<u8>,
    candidate: Vec<u8>,
    edits: Vec<ByteEdit>,
    certificate: Certificate,
}

/// Prepare one content request exactly once and retain the observed source,
/// candidate, and certificate together. Transactions must not preview a
/// request and then reopen/replan it: that creates a certificate race in
/// which the evidence can describe different bytes from the committed plan.
#[allow(clippy::result_large_err)]
fn prepare_content_request(
    workspace: &Workspace,
    request: &Request,
) -> Result<PreparedContent, Certificate> {
    let normalized_path = PathNormalizer::normalize(&request.file_path, &request.namespace);
    let provider = provider_name(&request.operation);
    let file_path = match workspace.resolve_namespaced_path(&request.file_path, &request.namespace)
    {
        Ok(path) => path,
        Err(error) => return Err(workspace_error(request, &normalized_path, provider, error)),
    };
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&request.version.as_str()) {
        return Err(refusal(
            request,
            &file_path,
            provider,
            RefusalReason::UnsupportedProtocolVersion {
                requested: request.version.clone(),
                supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
            },
            String::new(),
        ));
    }
    let allowed = match budget_path_is_allowed(
        workspace,
        &file_path,
        &request.budget.allowed_path_prefixes,
    ) {
        Ok(allowed) => allowed,
        Err(error) => return Err(workspace_error(request, &file_path, provider, error)),
    };
    if !allowed {
        return Err(refusal(
            request,
            &file_path,
            provider,
            RefusalReason::WorkspaceTraversal {
                path: "path is outside requested budget scope".into(),
            },
            String::new(),
        ));
    }
    let original = match workspace.read_file(&file_path) {
        Ok(bytes) => bytes,
        Err(error) => return Err(workspace_error(request, &file_path, provider, error)),
    };
    if original.len() > MAX_FILE_BYTES {
        return Err(refusal(
            request,
            &file_path,
            provider,
            RefusalReason::ResourceLimitExceeded {
                dimension: "max_file_bytes".into(),
                limit: MAX_FILE_BYTES,
                actual: original.len(),
            },
            String::new(),
        ));
    }
    let pre_hash = compute_sha256(&original);
    if let Some(expected) = request
        .expected_pre_hash
        .as_deref()
        .filter(|hash| !hash.is_empty())
    {
        let expected = normalize_hash(expected);
        if expected != pre_hash {
            return Err(refusal(
                request,
                &file_path,
                provider,
                RefusalReason::StaleIdentity {
                    expected_hash: expected,
                    actual_hash: pre_hash.clone(),
                },
                pre_hash,
            ));
        }
    }
    if let Some(guard) = &request.region_guard {
        if let Err(reason) = validate_region_guard(&original, guard, request) {
            return Err(refusal(request, &file_path, provider, reason, pre_hash));
        }
    }
    if let Some(reason) = unsupported_encoding(&original) {
        return Err(refusal(request, &file_path, provider, reason, pre_hash));
    }
    if original.contains(&0) {
        return Err(refusal(
            request,
            &file_path,
            provider,
            RefusalReason::BinaryInput,
            pre_hash,
        ));
    }
    if is_generated_file(&original) && !request.allow_generated {
        return Err(refusal(
            request,
            &file_path,
            provider,
            RefusalReason::GeneratedFileRequiresOptIn {
                marker: generated_marker(&original).into(),
            },
            pre_hash,
        ));
    }
    let edits = match plan_edits(&original, request, &file_path, &pre_hash) {
        Ok(edits) => edits,
        Err((reason, _)) => return Err(refusal(request, &file_path, provider, reason, pre_hash)),
    };
    let candidate = match apply_byte_edits(&original, &edits) {
        Ok(candidate) => candidate,
        Err(error) => {
            return Err(failure(
                request,
                &file_path,
                provider,
                pre_hash,
                FailureReason::InternalInvariant {
                    details: error.to_string(),
                },
            ))
        }
    };
    if let Some(desired) = desired_bytes(request) {
        if candidate != desired {
            return Err(failure(
                request,
                &file_path,
                provider,
                pre_hash,
                FailureReason::InternalInvariant {
                    details: "desired-state planner produced bytes different from the requested desired state".into(),
                },
            ));
        }
    }
    let effect = effect_usage(&original, &candidate, &edits, &request.budget);
    if let Some((dimension, limit, actual)) = budget_violation(&effect, &request.budget) {
        return Err(refusal_with_effect(
            request,
            &file_path,
            provider,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
            pre_hash,
            EffectUsage {
                passed: false,
                ..effect
            },
        ));
    }
    let structural = match validate_candidate(request, &candidate) {
        Ok(validation) => validation,
        Err(reason) => return Err(failure(request, &file_path, provider, pre_hash, reason)),
    };
    let (diff, diff_truncated) = bounded_diff(&original, &edits);
    let mut certificate = completed(
        request,
        &file_path,
        provider,
        pre_hash.clone(),
        Some(compute_sha256(&candidate)),
        if original == candidate {
            Outcome::NoChange
        } else {
            Outcome::Applied
        },
        changed_ranges(&edits),
        structural,
        PreservationFacts::from_bytes(&original, &candidate),
        CommitGuarantee::default(),
        if original == candidate {
            String::new()
        } else {
            diff
        },
        diff_truncated,
        effect.clone(),
    );
    certificate.changed_line_ranges = changed_line_ranges(&original, &edits);
    attach_desired_evidence(desired_bytes(request), &edits, &effect, &mut certificate);
    Ok(PreparedContent {
        path: file_path,
        original,
        candidate,
        edits,
        certificate,
    })
}

/// Prepare an exact, serialisable plan without writing any bytes. The plan
/// contains the provider's resolved edits and the observed pre-image hash, so
/// a later apply operation can refuse stale or tampered input without asking a
/// provider to guess a new location.
pub fn prepare_request_plan(
    workspace: &Workspace,
    request: &Request,
    assertions: Vec<Assertion>,
) -> Result<PreparedPlan, Certificate> {
    if assertions.len() > MAX_ASSERTIONS {
        return Err(refusal(
            request,
            &request.file_path,
            provider_name(&request.operation),
            RefusalReason::PlanTooLarge {
                dimension: "assertions".into(),
                limit: MAX_ASSERTIONS,
                actual: assertions.len(),
            },
            String::new(),
        ));
    }
    if assertions.iter().any(assertion_is_too_large) {
        return Err(refusal(
            request,
            &request.file_path,
            provider_name(&request.operation),
            RefusalReason::PlanTooLarge {
                dimension: "assertion_literal_bytes".into(),
                limit: MAX_ASSERTION_LITERAL_BYTES,
                actual: MAX_ASSERTION_LITERAL_BYTES.saturating_add(1),
            },
            String::new(),
        ));
    }
    let prepared = prepare_content_request(workspace, request)?;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(prepared.path.clone(), prepared.candidate.clone());
    if let Err(failure) = evaluate_assertions(workspace, &assertions, &overrides, "prospective") {
        return Err(refusal(
            request,
            &prepared.path,
            provider_name(&request.operation),
            failure,
            prepared.certificate.pre_hash.clone(),
        ));
    }
    let mut planned_request = request.clone();
    planned_request.expected_pre_hash = Some(prepared.certificate.pre_hash.clone());
    let operation = PreparedPlanOperation {
        file_path: prepared.path.clone(),
        provider: provider_name(&request.operation).into(),
        request: planned_request,
        pre_hash: prepared.certificate.pre_hash.clone(),
        edits: prepared
            .edits
            .iter()
            .map(|edit| PlanByteEdit {
                offset: edit.start,
                delete_len: edit.end.saturating_sub(edit.start),
                replacement: edit.replacement.clone(),
            })
            .collect(),
        prospective_hash: prepared
            .certificate
            .post_hash
            .clone()
            .unwrap_or_else(|| prepared.certificate.pre_hash.clone()),
    };
    let mut plan = PreparedPlan {
        schema_version: "1.0".into(),
        protocol_version: request.version.clone(),
        plan_id: String::new(),
        request_id: request_id(request),
        transaction_id: None,
        operations: vec![operation],
        assertions,
        budget: request.budget.clone(),
    };
    plan.plan_id = plan_identity(&plan);
    if serialized_plan_size(&plan) > MAX_PLAN_BYTES {
        return Err(refusal(
            request,
            &prepared.path,
            provider_name(&request.operation),
            RefusalReason::PlanTooLarge {
                dimension: "plan_bytes".into(),
                limit: MAX_PLAN_BYTES,
                actual: serialized_plan_size(&plan),
            },
            prepared.certificate.pre_hash,
        ));
    }
    Ok(plan)
}

/// Prepare a multi-file transaction as one portable plan. Lifecycle requests
/// remain deliberately unsupported here; existing lifecycle and transaction
/// paths continue to own those semantics.
pub fn prepare_transaction_plan(
    workspace: &Workspace,
    transaction: &TransactionRequest,
    assertions: Vec<Assertion>,
) -> Result<PreparedPlan, TransactionCertificate> {
    if transaction.requests.is_empty() || transaction.requests.len() > MAX_PLAN_OPERATIONS {
        return Err(transaction_refusal(
            transaction,
            RefusalReason::PlanTooLarge {
                dimension: "operations".into(),
                limit: MAX_PLAN_OPERATIONS,
                actual: transaction.requests.len(),
            },
        ));
    }
    if assertions.len() > MAX_ASSERTIONS || assertions.iter().any(assertion_is_too_large) {
        return Err(transaction_refusal(
            transaction,
            RefusalReason::PlanTooLarge {
                dimension: "assertions".into(),
                limit: MAX_ASSERTIONS,
                actual: assertions.len(),
            },
        ));
    }
    let mut operations = Vec::with_capacity(transaction.requests.len());
    let mut overrides = std::collections::HashMap::new();
    let mut certificates = Vec::new();
    for request in &transaction.requests {
        if matches!(request.operation, OperationPayload::File(_)) {
            return Err(transaction_refusal(
                transaction,
                RefusalReason::UnsupportedOperation {
                    operation:
                        "filesystem lifecycle operations are not supported in prepared plans".into(),
                },
            ));
        }
        let prepared = match prepare_content_request(workspace, request) {
            Ok(value) => value,
            Err(certificate) => {
                return Err(TransactionCertificate {
                    protocol_version: transaction.version.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    outcome: certificate.outcome.clone(),
                    certificates: vec![certificate.clone()],
                    rollback_state: "not_started".into(),
                    transaction_guarantee: "not_committed".into(),
                    refusal_reason: certificate.refusal_reason.clone(),
                    failure_reason: certificate.failure_reason.clone(),
                    reason_code: certificate.reason_code.clone(),
                    recovery: certificate.recovery.clone(),
                    schema_diagnostic: None,
                })
            }
        };
        overrides.insert(prepared.path.clone(), prepared.candidate.clone());
        let mut planned_request = request.clone();
        planned_request.expected_pre_hash = Some(prepared.certificate.pre_hash.clone());
        operations.push(PreparedPlanOperation {
            file_path: prepared.path.clone(),
            provider: provider_name(&request.operation).into(),
            request: planned_request,
            pre_hash: prepared.certificate.pre_hash.clone(),
            edits: prepared
                .edits
                .iter()
                .map(|edit| PlanByteEdit {
                    offset: edit.start,
                    delete_len: edit.end.saturating_sub(edit.start),
                    replacement: edit.replacement.clone(),
                })
                .collect(),
            prospective_hash: prepared
                .certificate
                .post_hash
                .clone()
                .unwrap_or_else(|| prepared.certificate.pre_hash.clone()),
        });
        certificates.push(prepared.certificate);
    }
    if let Err(failure) = evaluate_assertions(workspace, &assertions, &overrides, "prospective") {
        return Err(transaction_refusal(transaction, failure));
    }
    let mut plan = PreparedPlan {
        schema_version: "1.0".into(),
        protocol_version: transaction.version.clone(),
        plan_id: String::new(),
        request_id: transaction.transaction_id.clone(),
        transaction_id: Some(transaction.transaction_id.clone()),
        operations,
        assertions,
        budget: transaction.budget.clone(),
    };
    plan.plan_id = plan_identity(&plan);
    if serialized_plan_size(&plan) > MAX_PLAN_BYTES {
        return Err(transaction_refusal(
            transaction,
            RefusalReason::PlanTooLarge {
                dimension: "plan_bytes".into(),
                limit: MAX_PLAN_BYTES,
                actual: serialized_plan_size(&plan),
            },
        ));
    }
    Ok(plan)
}

fn assertion_is_too_large(assertion: &Assertion) -> bool {
    match assertion {
        Assertion::FileExists { path } | Assertion::FileAbsent { path } => {
            path.len() > MAX_ASSERTION_LITERAL_BYTES
        }
        Assertion::Sha256 { path, equals } => {
            path.len().saturating_add(equals.len()) > MAX_ASSERTION_LITERAL_BYTES
        }
        Assertion::LiteralCount {
            path,
            literal,
            exactly: _,
            minimum: _,
            maximum: _,
        } => path.len().saturating_add(literal.len()) > MAX_ASSERTION_LITERAL_BYTES,
    }
}

fn plan_identity(plan: &PreparedPlan) -> String {
    let mut semantic = plan.clone();
    semantic.plan_id.clear();
    let bytes = serde_json::to_vec(&semantic).expect("prepared plan is serialisable");
    format!("sha256:{}", compute_sha256(&bytes))
}

fn serialized_plan_size(plan: &PreparedPlan) -> usize {
    serde_json::to_vec(plan)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

/// Apply an exact prepared plan. The operation is intentionally independent of
/// provider relocation: it rechecks the stored request guards and then applies
/// only the stored byte edits against each exact pre-image.
pub fn apply_prepared_plan(workspace: &Workspace, plan: &PreparedPlan) -> PlanApplyResult {
    if plan.schema_version != "1.0" {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::PlanInvalid {
                details: format!("unsupported plan schema {}", plan.schema_version),
            },
        );
    }
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&plan.protocol_version.as_str()) {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::UnsupportedProtocolVersion {
                requested: plan.protocol_version.clone(),
                supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
            },
        );
    }
    if plan.operations.is_empty() || plan.operations.len() > MAX_PLAN_OPERATIONS {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::PlanTooLarge {
                dimension: "operations".into(),
                limit: MAX_PLAN_OPERATIONS,
                actual: plan.operations.len(),
            },
        );
    }
    if plan_identity(plan) != plan.plan_id {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::PlanInvalid {
                details: "plan_id does not match the deterministic semantic plan identity".into(),
            },
        );
    }
    if serialized_plan_size(plan) > MAX_PLAN_BYTES {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::PlanTooLarge {
                dimension: "plan_bytes".into(),
                limit: MAX_PLAN_BYTES,
                actual: serialized_plan_size(plan),
            },
        );
    }
    let _mutation_lock = match workspace.acquire_mutation_lock() {
        Ok(lock) => lock,
        Err(error) => return plan_failure_result(workspace, plan, workspace_reason(error)),
    };
    let mut prepared = Vec::with_capacity(plan.operations.len());
    let mut overrides = std::collections::HashMap::new();
    let mut aggregate = zero_effect();
    for operation in &plan.operations {
        let path = match workspace
            .resolve_namespaced_path(&operation.file_path, &operation.request.namespace)
        {
            Ok(path) => path,
            Err(error) => return plan_failure_result(workspace, plan, workspace_reason(error)),
        };
        let allowed = match budget_path_is_allowed(
            workspace,
            &path,
            &operation.request.budget.allowed_path_prefixes,
        ) {
            Ok(allowed) => allowed,
            Err(error) => return plan_failure_result(workspace, plan, workspace_reason(error)),
        };
        if !allowed {
            return plan_failure_result(
                workspace,
                plan,
                RefusalReason::WorkspaceTraversal {
                    path: "path is outside requested budget scope".into(),
                },
            );
        }
        let original = match workspace.read_file(&path) {
            Ok(bytes) => bytes,
            Err(error) => return plan_failure_result(workspace, plan, workspace_reason(error)),
        };
        let actual_hash = compute_sha256(&original);
        if actual_hash != normalize_hash(&operation.pre_hash) {
            return plan_failure_result(
                workspace,
                plan,
                RefusalReason::PlanStale {
                    path: operation.file_path.clone(),
                    expected_hash: normalize_hash(&operation.pre_hash),
                    actual_hash,
                },
            );
        }
        let stored_edits: Vec<ByteEdit> = operation
            .edits
            .iter()
            .map(|edit| ByteEdit {
                start: edit.offset,
                end: edit.offset.saturating_add(edit.delete_len),
                replacement: edit.replacement.clone(),
            })
            .collect();
        let replanned = match plan_edits(&original, &operation.request, &path, &actual_hash) {
            Ok(edits) => edits,
            Err((reason, _)) => return plan_failure_result(workspace, plan, reason),
        };
        if replanned != stored_edits {
            return plan_failure_result(
                workspace,
                plan,
                RefusalReason::PlanInvalid {
                    details: format!(
                        "provider resolution no longer matches stored edits for {}",
                        operation.file_path
                    ),
                },
            );
        }
        let candidate = match apply_byte_edits(&original, &stored_edits) {
            Ok(bytes) => bytes,
            Err(error) => {
                return plan_failure_result(
                    workspace,
                    plan,
                    RefusalReason::PlanInvalid {
                        details: format!("stored byte edits are invalid: {error}"),
                    },
                )
            }
        };
        let effect = effect_usage(
            &original,
            &candidate,
            &stored_edits,
            &operation.request.budget,
        );
        if let Some((dimension, limit, actual)) =
            budget_violation(&effect, &operation.request.budget)
        {
            return plan_failure_result(
                workspace,
                plan,
                RefusalReason::EffectBudgetExceeded {
                    dimension,
                    limit,
                    actual,
                },
            );
        }
        aggregate.files += effect.files;
        aggregate.matches += effect.matches;
        aggregate.changed_regions += effect.changed_regions;
        aggregate.changed_lines += effect.changed_lines;
        aggregate.changed_bytes += effect.changed_bytes;
        overrides.insert(operation.file_path.clone(), candidate.clone());
        prepared.push((path, original, candidate, stored_edits));
    }
    if let Some((dimension, limit, actual)) = budget_violation(&aggregate, &plan.budget) {
        return plan_failure_result(
            workspace,
            plan,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
        );
    }
    if let Err(reason) = evaluate_assertions(workspace, &plan.assertions, &overrides, "prospective")
    {
        return plan_failure_result(workspace, plan, reason);
    }
    let transaction_id = plan
        .transaction_id
        .clone()
        .unwrap_or_else(|| format!("plan-{}", &plan.plan_id[7..23.min(plan.plan_id.len())]));
    let entries: Vec<JournalEntry> = prepared
        .iter()
        .map(|(path, original, candidate, _)| JournalEntry {
            path: path.clone(),
            pre_hash: compute_sha256(original),
            candidate_hash: compute_sha256(candidate),
            original: original.clone(),
            candidate: candidate.clone(),
        })
        .collect();
    let journal = Journal {
        protocol_version: plan.protocol_version.clone(),
        transaction_id: transaction_id.clone(),
        entries,
    };
    let transaction = TransactionRequest {
        version: plan.protocol_version.clone(),
        transaction_id,
        requests: plan
            .operations
            .iter()
            .map(|operation| operation.request.clone())
            .collect(),
        budget: plan.budget.clone(),
    };
    let aggregate_files = aggregate.files;
    let result = commit_prepared_plan(
        workspace,
        &transaction,
        &journal,
        aggregate,
        prepared,
        &plan.assertions,
    );
    match result {
        Ok(certificates) => {
            if plan.operations.len() == 1 {
                PlanApplyResult::Certificate(certificates.into_iter().next().unwrap())
            } else {
                PlanApplyResult::Transaction(TransactionCertificate {
                    protocol_version: plan.protocol_version.clone(),
                    transaction_id: plan.transaction_id.clone().unwrap_or_default(),
                    outcome: if aggregate_files == 0 {
                        Outcome::NoChange
                    } else {
                        Outcome::Applied
                    },
                    certificates,
                    rollback_state: "not_required".into(),
                    transaction_guarantee: "transactional_with_rollback".into(),
                    refusal_reason: None,
                    failure_reason: None,
                    reason_code: None,
                    recovery: None,
                    schema_diagnostic: None,
                })
            }
        }
        Err(certificate) => {
            if plan.operations.len() == 1 {
                PlanApplyResult::Certificate(certificate)
            } else {
                PlanApplyResult::Transaction(TransactionCertificate {
                    protocol_version: plan.protocol_version.clone(),
                    transaction_id: plan.transaction_id.clone().unwrap_or_default(),
                    outcome: certificate.outcome,
                    certificates: Vec::new(),
                    rollback_state: certificate.recovery_state,
                    transaction_guarantee: "transactional_with_rollback".into(),
                    refusal_reason: certificate.refusal_reason,
                    failure_reason: certificate.failure_reason,
                    reason_code: certificate.reason_code,
                    recovery: certificate.recovery,
                    schema_diagnostic: None,
                })
            }
        }
    }
}

fn commit_prepared_plan(
    workspace: &Workspace,
    transaction: &TransactionRequest,
    journal: &Journal,
    aggregate: EffectUsage,
    prepared: Vec<(String, Vec<u8>, Vec<u8>, Vec<ByteEdit>)>,
    assertions: &[Assertion],
) -> Result<Vec<Certificate>, Certificate> {
    if let Err(error) = recovery::check_journal_size(journal) {
        return Err(failure(
            &transaction.requests[0],
            &journal.entries[0].path,
            "plan",
            journal.entries[0].pre_hash.clone(),
            FailureReason::CommitFailure {
                message: error.to_string(),
            },
        ));
    }
    if let Err(error) = recovery::write_journal(workspace, journal) {
        return Err(failure(
            &transaction.requests[0],
            &journal.entries[0].path,
            "plan",
            journal.entries[0].pre_hash.clone(),
            FailureReason::CommitFailure {
                message: error.to_string(),
            },
        ));
    }
    let mut committed = Vec::new();
    for entry in &journal.entries {
        if let Err(error) =
            workspace.write_file_atomic_checked(&entry.path, &entry.pre_hash, &entry.candidate)
        {
            let rollback_ok = rollback_entries(workspace, &committed);
            return Err(failure(
                &transaction.requests[0],
                &entry.path,
                "plan",
                entry.pre_hash.clone(),
                FailureReason::CommitFailure {
                    message: format!(
                        "{} (rollback: {})",
                        error,
                        if rollback_ok {
                            "ok"
                        } else {
                            "manual recovery required"
                        }
                    ),
                },
            ));
        }
        committed.push(entry);
    }
    let mut overrides = std::collections::HashMap::new();
    for entry in &journal.entries {
        let landed = match workspace.read_file(&entry.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let rollback_ok = rollback_entries(workspace, &committed);
                return Err(failure(
                    &transaction.requests[0],
                    &entry.path,
                    "plan",
                    entry.pre_hash.clone(),
                    FailureReason::PostCommitVerificationFailure {
                        expected_hash: entry.candidate_hash.clone(),
                        actual_hash: format!("read failed: {error}; rollback={rollback_ok}"),
                    },
                ));
            }
        };
        if compute_sha256(&landed) != entry.candidate_hash {
            let actual_hash = compute_sha256(&landed);
            let rollback_ok = rollback_entries(workspace, &committed);
            return Err(failure(
                &transaction.requests[0],
                &entry.path,
                "plan",
                entry.pre_hash.clone(),
                FailureReason::PostCommitVerificationFailure {
                    expected_hash: entry.candidate_hash.clone(),
                    actual_hash: format!("{actual_hash}; rollback={rollback_ok}"),
                },
            ));
        }
        overrides.insert(entry.path.clone(), landed);
    }
    if let Err(reason) = evaluate_assertions(workspace, assertions, &overrides, "committed") {
        let rollback_ok = rollback_entries(workspace, &committed);
        return Err(failure(
            &transaction.requests[0],
            &journal.entries[0].path,
            "plan",
            journal.entries[0].pre_hash.clone(),
            FailureReason::PostCommitAssertionFailed {
                assertion: format!("{reason:?}"),
                expected: "assertion satisfied".into(),
                observed: format!("rollback={rollback_ok}"),
                path: journal.entries[0].path.clone(),
            },
        ));
    }
    let _ = recovery::remove_journal(workspace, &journal.transaction_id);
    let mut certificates = Vec::new();
    for (index, (path, original, candidate, edits)) in prepared.into_iter().enumerate() {
        let request = transaction
            .requests
            .get(index)
            .unwrap_or(&transaction.requests[0]);
        let mut certificate = completed(
            request,
            &path,
            "plan",
            compute_sha256(&original),
            Some(compute_sha256(&candidate)),
            if original == candidate {
                Outcome::NoChange
            } else {
                Outcome::Applied
            },
            changed_ranges(&edits),
            StructuralValidation::NotApplicable,
            PreservationFacts::from_bytes(&original, &candidate),
            CommitGuarantee {
                mode: "committed_atomic_replace".into(),
                content_replacement: "atomic replacement after staged flush".into(),
                permissions: "platform-dependent; not asserted".into(),
                timestamps: "not preserved".into(),
                acl_xattr: "unknown".into(),
            },
            String::new(),
            false,
            effect_usage(&original, &candidate, &edits, &request.budget),
        );
        certificate.transaction_guarantee = if aggregate.files > 1 {
            "transactional_with_rollback".into()
        } else {
            "committed_atomic_replace".into()
        };
        certificates.push(certificate);
    }
    Ok(certificates)
}

fn rollback_entries(workspace: &Workspace, committed: &[&JournalEntry]) -> bool {
    committed.iter().rev().all(|entry| {
        workspace
            .write_file_atomic_checked(&entry.path, &entry.candidate_hash, &entry.original)
            .is_ok()
    })
}

/// Read-only plan checks used by `explain --plan`. Applying a plan repeats these checks and then
/// performs the full provider, edit, budget, prospective, journal, commit, and committed-state
/// verification sequence.
pub fn check_prepared_plan(
    workspace: &Workspace,
    plan: &PreparedPlan,
) -> Result<(), RefusalReason> {
    if plan.schema_version != "1.0" {
        return Err(RefusalReason::PlanInvalid {
            details: format!("unsupported plan schema {}", plan.schema_version),
        });
    }
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&plan.protocol_version.as_str()) {
        return Err(RefusalReason::UnsupportedProtocolVersion {
            requested: plan.protocol_version.clone(),
            supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
        });
    }
    if plan.operations.is_empty() || plan.operations.len() > MAX_PLAN_OPERATIONS {
        return Err(RefusalReason::PlanTooLarge {
            dimension: "operations".into(),
            limit: MAX_PLAN_OPERATIONS,
            actual: plan.operations.len(),
        });
    }
    if plan_identity(plan) != plan.plan_id {
        return Err(RefusalReason::PlanInvalid {
            details: "plan_id does not match the deterministic semantic plan identity".into(),
        });
    }
    let size = serialized_plan_size(plan);
    if size > MAX_PLAN_BYTES {
        return Err(RefusalReason::PlanTooLarge {
            dimension: "plan_bytes".into(),
            limit: MAX_PLAN_BYTES,
            actual: size,
        });
    }
    for operation in &plan.operations {
        let path = workspace
            .resolve_namespaced_path(&operation.file_path, &operation.request.namespace)
            .map_err(workspace_reason)?;
        let original = workspace
            .read_file(&path)
            .map_err(|error| RefusalReason::PlanInvalid {
                details: format!("cannot read {}: {error}", operation.file_path),
            })?;
        let actual_hash = compute_sha256(&original);
        if actual_hash != normalize_hash(&operation.pre_hash) {
            return Err(RefusalReason::PlanStale {
                path: operation.file_path.clone(),
                expected_hash: normalize_hash(&operation.pre_hash),
                actual_hash,
            });
        }
    }
    Ok(())
}

fn plan_failure_result(
    _workspace: &Workspace,
    plan: &PreparedPlan,
    reason: RefusalReason,
) -> PlanApplyResult {
    let request = plan.operations.first().map(|operation| &operation.request);
    let certificate = if let Some(request) = request {
        refusal(
            request,
            &plan.operations[0].file_path,
            "plan",
            reason,
            String::new(),
        )
    } else {
        empty_plan_certificate(plan, reason)
    };
    if plan.operations.len() <= 1 {
        PlanApplyResult::Certificate(certificate)
    } else {
        PlanApplyResult::Transaction(TransactionCertificate {
            protocol_version: plan.protocol_version.clone(),
            transaction_id: plan.transaction_id.clone().unwrap_or_default(),
            outcome: certificate.outcome.clone(),
            certificates: vec![certificate.clone()],
            rollback_state: "not_started".into(),
            transaction_guarantee: "not_committed".into(),
            refusal_reason: certificate.refusal_reason.clone(),
            failure_reason: certificate.failure_reason.clone(),
            reason_code: certificate.reason_code.clone(),
            recovery: certificate.recovery.clone(),
            schema_diagnostic: None,
        })
    }
}

fn empty_plan_certificate(plan: &PreparedPlan, reason: RefusalReason) -> Certificate {
    Certificate {
        protocol_version: plan.protocol_version.clone(),
        request_id: plan.request_id.clone(),
        outcome: Outcome::Refused,
        file_path: String::new(),
        provider: "plan".into(),
        provider_version: "1.0".into(),
        expected_cardinality: Default::default(),
        observed_cardinality: None,
        pre_hash: String::new(),
        post_hash: None,
        changed_ranges: Vec::new(),
        changed_line_ranges: Vec::new(),
        diff_summary: None,
        diff_truncated: false,
        structural_validation: StructuralValidation::NotApplicable,
        preservation: PreservationFacts::default(),
        commit: CommitGuarantee::default(),
        refusal_reason: Some(reason.clone()),
        failure_reason: None,
        reason_code: Some(reason.code().into()),
        recovery: None,
        schema_diagnostic: None,
        diagnostics: Vec::new(),
        budget: plan.budget.clone(),
        effect: zero_effect(),
        transaction_guarantee: "not_committed".into(),
        recovery_state: "not_started".into(),
        desired_state: None,
    }
}

fn read_assertion_bytes(
    workspace: &Workspace,
    overrides: &std::collections::HashMap<String, Vec<u8>>,
    path: &str,
) -> Result<(String, Vec<u8>), String> {
    let normalized = workspace
        .resolve_namespaced_path(path, &Default::default())
        .map_err(|error| error.to_string())?;
    if let Some(bytes) = overrides.get(&normalized).or_else(|| overrides.get(path)) {
        return Ok((normalized, bytes.clone()));
    }
    workspace
        .read_file(&normalized)
        .map(|bytes| (normalized, bytes))
        .map_err(|error| error.to_string())
}

fn evaluate_assertions(
    workspace: &Workspace,
    assertions: &[Assertion],
    overrides: &std::collections::HashMap<String, Vec<u8>>,
    phase: &str,
) -> Result<(), RefusalReason> {
    if assertions.len() > MAX_ASSERTIONS || assertions.iter().any(assertion_is_too_large) {
        return Err(RefusalReason::PlanTooLarge {
            dimension: "assertions".into(),
            limit: MAX_ASSERTIONS,
            actual: assertions.len(),
        });
    }
    for assertion in assertions {
        let (path, bytes, exists) = match assertion {
            Assertion::FileExists { path } | Assertion::FileAbsent { path } => {
                let normalized = workspace
                    .resolve_namespaced_path(path, &Default::default())
                    .map_err(|error| RefusalReason::PostconditionFailed {
                        assertion: format!("{assertion:?}"),
                        expected: "path contained by workspace".into(),
                        observed: error.to_string(),
                        path: path.clone(),
                        phase: phase.into(),
                    })?;
                let exists = overrides.contains_key(&normalized)
                    || overrides.contains_key(path)
                    || workspace.read_file(&normalized).is_ok();
                (normalized, Vec::new(), exists)
            }
            Assertion::Sha256 { path, .. } | Assertion::LiteralCount { path, .. } => {
                match read_assertion_bytes(workspace, overrides, path) {
                    Ok((normalized, bytes)) => (normalized, bytes, true),
                    Err(error) => {
                        return Err(RefusalReason::PostconditionFailed {
                            assertion: format!("{assertion:?}"),
                            expected: "file exists and satisfies assertion".into(),
                            observed: error,
                            path: path.clone(),
                            phase: phase.into(),
                        })
                    }
                }
            }
        };
        let (passed, expected, observed) = match assertion {
            Assertion::FileExists { .. } => (exists, "file exists".into(), exists.to_string()),
            Assertion::FileAbsent { .. } => (!exists, "file is absent".into(), exists.to_string()),
            Assertion::Sha256 { equals, .. } => {
                let observed = compute_sha256(&bytes);
                (
                    normalize_hash(equals) == observed,
                    format!("sha256:{}", normalize_hash(equals)),
                    format!("sha256:{observed}"),
                )
            }
            Assertion::LiteralCount {
                literal,
                exactly,
                minimum,
                maximum,
                ..
            } => {
                if literal.is_empty() {
                    return Err(RefusalReason::PostconditionFailed {
                        assertion: format!("{assertion:?}"),
                        expected: "non-empty literal".into(),
                        observed: "empty literal".into(),
                        path,
                        phase: phase.into(),
                    });
                }
                let needle = literal.as_bytes();
                let mut count = 0;
                let mut cursor: usize = 0;
                while cursor.saturating_add(needle.len()) <= bytes.len() {
                    let Some(relative) = bytes[cursor..]
                        .windows(needle.len())
                        .position(|window| window == needle)
                    else {
                        break;
                    };
                    count += 1;
                    cursor = cursor.saturating_add(relative + needle.len());
                }
                let passed = exactly.is_none_or(|value| count == value)
                    && minimum.is_none_or(|value| count >= value)
                    && maximum.is_none_or(|value| count <= value);
                (
                    passed,
                    format!("exactly={exactly:?},minimum={minimum:?},maximum={maximum:?}"),
                    count.to_string(),
                )
            }
        };
        if !passed {
            return Err(RefusalReason::PostconditionFailed {
                assertion: format!("{assertion:?}"),
                expected,
                observed,
                path,
                phase: phase.into(),
            });
        }
    }
    Ok(())
}

/// Prepare every member against its accepted source, then commit the complete
/// set behind one durable recovery journal. No member is written during the
/// preparation pass.
pub fn execute_transaction(
    workspace: &Workspace,
    transaction: &TransactionRequest,
    dry_run: bool,
) -> TransactionCertificate {
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&transaction.version.as_str()) {
        return transaction_refusal(
            transaction,
            RefusalReason::UnsupportedProtocolVersion {
                requested: transaction.version.clone(),
                supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
            },
        );
    }
    if transaction.requests.is_empty() {
        return transaction_refusal(
            transaction,
            RefusalReason::MalformedInput {
                details: "transaction must contain at least one request".into(),
            },
        );
    }
    if transaction.requests.len() > MAX_TRANSACTION_REQUESTS {
        return transaction_refusal(
            transaction,
            RefusalReason::ResourceLimitExceeded {
                dimension: "max_transaction_requests".into(),
                limit: MAX_TRANSACTION_REQUESTS,
                actual: transaction.requests.len(),
            },
        );
    }
    let _mutation_lock = if dry_run {
        None
    } else {
        match workspace.acquire_mutation_lock() {
            Ok(lock) => Some(lock),
            Err(error) => return transaction_refusal(transaction, workspace_reason(error)),
        }
    };
    let mut unique_paths = std::collections::HashSet::new();
    let has_duplicate_path = transaction.requests.iter().any(|request| {
        !unique_paths.insert(PathNormalizer::normalize(
            &request.file_path,
            &request.namespace,
        ))
    });
    if has_duplicate_path {
        return execute_single_file_transaction(workspace, transaction, dry_run);
    }
    let mut paths = std::collections::HashSet::new();
    for request in &transaction.requests {
        let path = match workspace.resolve_namespaced_path(&request.file_path, &request.namespace) {
            Ok(path) => path,
            Err(error) => return transaction_refusal(transaction, workspace_reason(error)),
        };
        if !paths.insert(path.clone()) {
            return transaction_refusal(
                transaction,
                RefusalReason::TransactionConflict { message: format!("multiple operations for {path} require a single-file operation batch; refusing ambiguous transaction ordering") },
            );
        }
    }
    let mut prepared = Vec::new();
    let mut certificates = Vec::new();
    let mut aggregate = EffectUsage {
        files: 0,
        matches: 0,
        changed_regions: 0,
        changed_lines: 0,
        changed_bytes: 0,
        passed: true,
    };
    for request in &transaction.requests {
        if matches!(request.operation, OperationPayload::File(_)) {
            return transaction_refusal(transaction, RefusalReason::UnsupportedOperation { operation: "filesystem lifecycle operations are not yet composable in multi-file transactions".into() });
        }
        let prepared_request = match prepare_content_request(workspace, request) {
            Ok(prepared_request) => prepared_request,
            Err(certificate) => {
                return TransactionCertificate {
                    protocol_version: transaction.version.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    outcome: certificate.outcome.clone(),
                    certificates: vec![certificate.clone()],
                    rollback_state: "not_started".into(),
                    transaction_guarantee: "not_committed".into(),
                    refusal_reason: certificate.refusal_reason.clone(),
                    failure_reason: certificate.failure_reason.clone(),
                    reason_code: certificate.reason_code.clone(),
                    recovery: certificate.recovery.clone(),
                    schema_diagnostic: None,
                }
            }
        };
        aggregate.files += usize::from(prepared_request.original != prepared_request.candidate);
        aggregate.matches += prepared_request.certificate.effect.matches;
        aggregate.changed_regions += prepared_request.certificate.effect.changed_regions;
        aggregate.changed_lines += prepared_request.certificate.effect.changed_lines;
        aggregate.changed_bytes += prepared_request.certificate.effect.changed_bytes;
        certificates.push(prepared_request.certificate);
        prepared.push((
            prepared_request.path,
            prepared_request.original,
            prepared_request.candidate,
        ));
    }
    if let Some((dimension, limit, actual)) = budget_violation(&aggregate, &transaction.budget) {
        return transaction_refusal(
            transaction,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
        );
    }
    if dry_run {
        return TransactionCertificate {
            protocol_version: transaction.version.clone(),
            transaction_id: transaction.transaction_id.clone(),
            outcome: if aggregate.files == 0 {
                Outcome::NoChange
            } else {
                Outcome::Applied
            },
            certificates,
            rollback_state: "not_required".into(),
            transaction_guarantee: "dry_run".into(),
            refusal_reason: None,
            failure_reason: None,
            reason_code: None,
            recovery: None,
            schema_diagnostic: None,
        };
    }
    let journal = Journal {
        protocol_version: transaction.version.clone(),
        transaction_id: transaction.transaction_id.clone(),
        entries: prepared
            .iter()
            .map(|(path, original, candidate)| JournalEntry {
                path: path.clone(),
                pre_hash: compute_sha256(original),
                candidate_hash: compute_sha256(candidate),
                original: original.clone(),
                candidate: candidate.clone(),
            })
            .collect(),
    };
    if let Err(error) = recovery::check_journal_size(&journal) {
        return transaction_journal_error(transaction, error);
    }
    if let Err(e) = recovery::write_journal(workspace, &journal) {
        return transaction_journal_error(transaction, e);
    }
    let mut committed = Vec::new();
    for entry in &journal.entries {
        match workspace.write_file_atomic_checked(&entry.path, &entry.pre_hash, &entry.candidate) {
            Ok(()) => committed.push(entry),
            Err(error) => {
                let mut rollback_ok = true;
                for prior in committed.iter().rev() {
                    if workspace
                        .write_file_atomic_checked(
                            &prior.path,
                            &prior.candidate_hash,
                            &prior.original,
                        )
                        .is_err()
                    {
                        rollback_ok = false;
                    }
                }
                return TransactionCertificate {
                    protocol_version: transaction.version.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    outcome: Outcome::Failed,
                    certificates,
                    rollback_state: if rollback_ok {
                        "rolled_back".into()
                    } else {
                        "manual_recovery_required".into()
                    },
                    transaction_guarantee: "transactional_with_rollback".into(),
                    refusal_reason: None,
                    failure_reason: Some(FailureReason::CommitFailure {
                        message: error.to_string(),
                    }),
                    reason_code: Some("COMMIT_FAILED".into()),
                    recovery: None,
                    schema_diagnostic: None,
                };
            }
        }
    }
    for entry in &journal.entries {
        let landed = match workspace.read_file(&entry.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let mut rollback_ok = true;
                for prior in committed.iter().rev() {
                    if workspace
                        .write_file_atomic_checked(
                            &prior.path,
                            &prior.candidate_hash,
                            &prior.original,
                        )
                        .is_err()
                    {
                        rollback_ok = false;
                    }
                }
                return TransactionCertificate {
                    protocol_version: transaction.version.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    outcome: Outcome::Failed,
                    certificates,
                    rollback_state: if rollback_ok {
                        "rolled_back".into()
                    } else {
                        "manual_recovery_required".into()
                    },
                    transaction_guarantee: "transactional_with_rollback".into(),
                    refusal_reason: None,
                    failure_reason: Some(FailureReason::PostCommitVerificationFailure {
                        expected_hash: entry.candidate_hash.clone(),
                        actual_hash: format!("read failed: {error}"),
                    }),
                    reason_code: Some("POST_COMMIT_VERIFICATION_FAILED".into()),
                    recovery: None,
                    schema_diagnostic: None,
                };
            }
        };
        if landed != entry.candidate {
            let actual_hash = compute_sha256(&landed);
            let mut rollback_ok = true;
            for prior in committed.iter().rev() {
                if workspace
                    .write_file_atomic_checked(&prior.path, &prior.candidate_hash, &prior.original)
                    .is_err()
                {
                    rollback_ok = false;
                }
            }
            return TransactionCertificate {
                protocol_version: transaction.version.clone(),
                transaction_id: transaction.transaction_id.clone(),
                outcome: Outcome::Failed,
                certificates,
                rollback_state: if rollback_ok {
                    "rolled_back".into()
                } else {
                    "manual_recovery_required".into()
                },
                transaction_guarantee: "transactional_with_rollback".into(),
                refusal_reason: None,
                failure_reason: Some(FailureReason::PostCommitVerificationFailure {
                    expected_hash: entry.candidate_hash.clone(),
                    actual_hash,
                }),
                reason_code: Some("POST_COMMIT_VERIFICATION_FAILED".into()),
                recovery: None,
                schema_diagnostic: None,
            };
        }
    }
    let cleanup = recovery::remove_journal(workspace, &transaction.transaction_id);
    let recovery_state = if cleanup.is_ok() {
        "not_required"
    } else {
        "recovery_available"
    };
    mark_transaction_certificates(&mut certificates, recovery_state);
    TransactionCertificate {
        protocol_version: transaction.version.clone(),
        transaction_id: transaction.transaction_id.clone(),
        outcome: if aggregate.files == 0 {
            Outcome::NoChange
        } else {
            Outcome::Applied
        },
        certificates,
        rollback_state: recovery_state.into(),
        transaction_guarantee: "transactional_with_rollback".into(),
        refusal_reason: None,
        failure_reason: cleanup.err().map(|error| FailureReason::CommitFailure {
            message: format!("recovery journal cleanup failed: {error}"),
        }),
        reason_code: None,
        recovery: None,
        schema_diagnostic: None,
    }
}

fn execute_single_file_transaction(
    workspace: &Workspace,
    transaction: &TransactionRequest,
    dry_run: bool,
) -> TransactionCertificate {
    let first = &transaction.requests[0];
    if transaction
        .requests
        .iter()
        .any(|request| matches!(request.operation, OperationPayload::File(_)))
    {
        return transaction_refusal(
            transaction,
            RefusalReason::UnsupportedOperation {
                operation: "filesystem lifecycle cannot be mixed into a content transaction".into(),
            },
        );
    }
    let path = match workspace.resolve_namespaced_path(&first.file_path, &first.namespace) {
        Ok(path) => path,
        Err(error) => return transaction_refusal(transaction, workspace_reason(error)),
    };
    let original = match workspace.read_file(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return transaction_failure(
                transaction,
                FailureReason::IoError {
                    message: error.to_string(),
                },
            )
        }
    };
    if original.len() > MAX_FILE_BYTES {
        return transaction_refusal(
            transaction,
            RefusalReason::ResourceLimitExceeded {
                dimension: "max_file_bytes".into(),
                limit: MAX_FILE_BYTES,
                actual: original.len(),
            },
        );
    }
    let mut current = original.clone();
    let mut certificates = Vec::new();
    let mut aggregate = zero_effect();
    for request in &transaction.requests {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&request.version.as_str()) {
            return transaction_refusal(
                transaction,
                RefusalReason::UnsupportedProtocolVersion {
                    requested: request.version.clone(),
                    supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
                },
            );
        }
        let allowed =
            match budget_path_is_allowed(workspace, &path, &request.budget.allowed_path_prefixes) {
                Ok(allowed) => allowed,
                Err(error) => return transaction_refusal(transaction, workspace_reason(error)),
            };
        if !allowed {
            return transaction_refusal(
                transaction,
                RefusalReason::WorkspaceTraversal {
                    path: "path is outside requested budget scope".into(),
                },
            );
        }
        if let Some(expected) = request
            .expected_pre_hash
            .as_deref()
            .filter(|hash| !hash.is_empty())
        {
            let expected = normalize_hash(expected);
            let actual = compute_sha256(&current);
            if expected != actual {
                return transaction_refusal(
                    transaction,
                    RefusalReason::StaleIdentity {
                        expected_hash: expected,
                        actual_hash: actual,
                    },
                );
            }
        }
        if let Some(reason) = unsupported_encoding(&current) {
            return transaction_refusal(transaction, reason);
        }
        if current.contains(&0) {
            return transaction_refusal(transaction, RefusalReason::BinaryInput);
        }
        if is_generated_file(&current) && !request.allow_generated {
            return transaction_refusal(
                transaction,
                RefusalReason::GeneratedFileRequiresOptIn {
                    marker: generated_marker(&current).into(),
                },
            );
        }
        if let Some(guard) = &request.region_guard {
            if let Err(reason) = validate_region_guard(&current, guard, request) {
                return transaction_refusal(transaction, reason);
            }
        }
        let pre_hash = compute_sha256(&current);
        let edits = match plan_edits(&current, request, &path, &pre_hash) {
            Ok(edits) => edits,
            Err((reason, _)) => return transaction_refusal(transaction, reason),
        };
        let candidate = match apply_byte_edits(&current, &edits) {
            Ok(candidate) => candidate,
            Err(error) => {
                return transaction_failure(
                    transaction,
                    FailureReason::InternalInvariant {
                        details: error.to_string(),
                    },
                )
            }
        };
        if let Some(desired) = desired_bytes(request) {
            if candidate != desired {
                return transaction_failure(
                    transaction,
                    FailureReason::InternalInvariant {
                        details: "desired-state planner produced bytes different from the requested desired state".into(),
                    },
                );
            }
        }
        if candidate.len() > MAX_FILE_BYTES {
            return transaction_refusal(
                transaction,
                RefusalReason::ResourceLimitExceeded {
                    dimension: "max_file_bytes".into(),
                    limit: MAX_FILE_BYTES,
                    actual: candidate.len(),
                },
            );
        }
        let structural = match validate_candidate(request, &candidate) {
            Ok(validation) => validation,
            Err(reason) => return transaction_failure(transaction, reason),
        };
        let effect = effect_usage(&current, &candidate, &edits, &request.budget);
        if let Some((dimension, limit, actual)) = budget_violation(&effect, &request.budget) {
            return transaction_refusal(
                transaction,
                RefusalReason::EffectBudgetExceeded {
                    dimension,
                    limit,
                    actual,
                },
            );
        }
        aggregate.matches += effect.matches;
        aggregate.changed_regions += effect.changed_regions;
        aggregate.changed_lines += effect.changed_lines;
        aggregate.changed_bytes += effect.changed_bytes;
        let mut certificate = completed(
            request,
            &path,
            provider_name(&request.operation),
            pre_hash,
            Some(compute_sha256(&candidate)),
            if candidate == current {
                Outcome::NoChange
            } else {
                Outcome::Applied
            },
            changed_ranges(&edits),
            structural,
            PreservationFacts::from_bytes(&current, &candidate),
            CommitGuarantee {
                mode: "dry_run".into(),
                ..CommitGuarantee::default()
            },
            bounded_diff(&current, &edits).0,
            false,
            effect.clone(),
        );
        certificate.changed_line_ranges = changed_line_ranges(&current, &edits);
        attach_desired_evidence(desired_bytes(request), &edits, &effect, &mut certificate);
        certificates.push(certificate);
        current = candidate;
    }
    aggregate.files = usize::from(original != current);
    aggregate.passed = budget_violation(&aggregate, &transaction.budget).is_none();
    if let Some((dimension, limit, actual)) = budget_violation(&aggregate, &transaction.budget) {
        return transaction_refusal(
            transaction,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
        );
    }
    if dry_run {
        return TransactionCertificate {
            protocol_version: transaction.version.clone(),
            transaction_id: transaction.transaction_id.clone(),
            outcome: if original == current {
                Outcome::NoChange
            } else {
                Outcome::Applied
            },
            certificates,
            rollback_state: "not_required".into(),
            transaction_guarantee: "dry_run".into(),
            refusal_reason: None,
            failure_reason: None,
            reason_code: None,
            recovery: None,
            schema_diagnostic: None,
        };
    }
    if original == current {
        return TransactionCertificate {
            protocol_version: transaction.version.clone(),
            transaction_id: transaction.transaction_id.clone(),
            outcome: Outcome::NoChange,
            certificates,
            rollback_state: "not_required".into(),
            transaction_guarantee: "transactional_with_rollback".into(),
            refusal_reason: None,
            failure_reason: None,
            reason_code: None,
            recovery: None,
            schema_diagnostic: None,
        };
    }
    let journal = Journal {
        protocol_version: transaction.version.clone(),
        transaction_id: transaction.transaction_id.clone(),
        entries: vec![JournalEntry {
            path: path.clone(),
            pre_hash: compute_sha256(&original),
            candidate_hash: compute_sha256(&current),
            original,
            candidate: current,
        }],
    };
    if let Err(error) = recovery::check_journal_size(&journal) {
        return transaction_journal_error(transaction, error);
    }
    if let Err(error) = recovery::write_journal(workspace, &journal) {
        return transaction_journal_error(transaction, error);
    }
    match workspace.write_file_atomic_checked(
        &path,
        &journal.entries[0].pre_hash,
        &journal.entries[0].candidate,
    ) {
        Ok(()) => {
            let landed = match workspace.read_file(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return TransactionCertificate {
                        protocol_version: transaction.version.clone(),
                        transaction_id: transaction.transaction_id.clone(),
                        outcome: Outcome::Failed,
                        certificates,
                        rollback_state: "recovery_available".into(),
                        transaction_guarantee: "transactional_with_rollback".into(),
                        refusal_reason: None,
                        failure_reason: Some(FailureReason::PostCommitVerificationFailure {
                            expected_hash: journal.entries[0].candidate_hash.clone(),
                            actual_hash: format!("read failed: {error}"),
                        }),
                        reason_code: Some("POST_COMMIT_VERIFICATION_FAILED".into()),
                        recovery: None,
                        schema_diagnostic: None,
                    };
                }
            };
            if landed != journal.entries[0].candidate {
                let actual_hash = compute_sha256(&landed);
                return TransactionCertificate {
                    protocol_version: transaction.version.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    outcome: Outcome::Failed,
                    certificates,
                    rollback_state: "recovery_available".into(),
                    transaction_guarantee: "transactional_with_rollback".into(),
                    refusal_reason: None,
                    failure_reason: Some(FailureReason::PostCommitVerificationFailure {
                        expected_hash: journal.entries[0].candidate_hash.clone(),
                        actual_hash,
                    }),
                    reason_code: Some("POST_COMMIT_VERIFICATION_FAILED".into()),
                    recovery: None,
                    schema_diagnostic: None,
                };
            }
            let cleanup = recovery::remove_journal(workspace, &transaction.transaction_id);
            let recovery_state = if cleanup.is_ok() {
                "not_required"
            } else {
                "recovery_available"
            };
            mark_transaction_certificates(&mut certificates, recovery_state);
            TransactionCertificate {
                protocol_version: transaction.version.clone(),
                transaction_id: transaction.transaction_id.clone(),
                outcome: Outcome::Applied,
                certificates,
                rollback_state: recovery_state.into(),
                transaction_guarantee: "transactional_with_rollback".into(),
                refusal_reason: None,
                failure_reason: cleanup.err().map(|error| FailureReason::CommitFailure {
                    message: format!("recovery journal cleanup failed: {error}"),
                }),
                reason_code: None,
                recovery: None,
                schema_diagnostic: None,
            }
        }
        Err(error) => TransactionCertificate {
            protocol_version: transaction.version.clone(),
            transaction_id: transaction.transaction_id.clone(),
            outcome: Outcome::Failed,
            certificates,
            rollback_state: "recovery_available".into(),
            transaction_guarantee: "transactional_with_rollback".into(),
            refusal_reason: None,
            failure_reason: Some(FailureReason::CommitFailure {
                message: error.to_string(),
            }),
            reason_code: Some("COMMIT_FAILED".into()),
            recovery: None,
            schema_diagnostic: None,
        },
    }
}

fn transaction_refusal(
    transaction: &TransactionRequest,
    reason: RefusalReason,
) -> TransactionCertificate {
    let reason_code = reason.code().into();
    let recovery = transaction
        .requests
        .first()
        .and_then(|request| recovery_for(request, &reason, ""));
    TransactionCertificate {
        protocol_version: transaction.version.clone(),
        transaction_id: transaction.transaction_id.clone(),
        outcome: Outcome::Refused,
        certificates: Vec::new(),
        rollback_state: "not_started".into(),
        transaction_guarantee: "not_committed".into(),
        refusal_reason: Some(reason),
        failure_reason: None,
        reason_code: Some(reason_code),
        recovery,
        schema_diagnostic: None,
    }
}
fn transaction_failure(
    transaction: &TransactionRequest,
    reason: FailureReason,
) -> TransactionCertificate {
    let reason_code = reason.code().into();
    TransactionCertificate {
        protocol_version: transaction.version.clone(),
        transaction_id: transaction.transaction_id.clone(),
        outcome: Outcome::Failed,
        certificates: Vec::new(),
        rollback_state: "not_started".into(),
        transaction_guarantee: "not_committed".into(),
        refusal_reason: None,
        failure_reason: Some(reason),
        reason_code: Some(reason_code),
        recovery: None,
        schema_diagnostic: None,
    }
}

fn transaction_journal_error(
    transaction: &TransactionRequest,
    error: WorkspaceError,
) -> TransactionCertificate {
    match error {
        WorkspaceError::ResourceLimit { .. } => {
            transaction_refusal(transaction, workspace_reason(error))
        }
        error => transaction_failure(
            transaction,
            FailureReason::CommitFailure {
                message: error.to_string(),
            },
        ),
    }
}

fn plan_edits(
    original: &[u8],
    request: &Request,
    file_path: &str,
    pre_hash: &str,
) -> Result<Vec<ByteEdit>, (RefusalReason, String)> {
    if let Some(guard) = request.candidate_guard.as_ref() {
        if request
            .expected_pre_hash
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err((
                RefusalReason::MalformedInput {
                    details:
                        "candidate_guard requires expected_pre_hash from the observed source state"
                            .into(),
                },
                "candidate_guard requires expected_pre_hash".into(),
            ));
        }
        if !matches!(
            request.cardinality,
            crate::protocol::Cardinality::ExactlyOne
        ) {
            return Err((
                RefusalReason::CardinalityMismatch {
                    expected: "exactly_one when candidate_guard is present".into(),
                    actual: 1,
                },
                "candidate_guard cannot override the requested cardinality".into(),
            ));
        }
        if guard.selection_id.is_empty() {
            return Err((
                RefusalReason::CandidateSelectionInvalid {
                    offset: guard.offset,
                    selection_id: String::new(),
                    details: "selection_id must not be empty".into(),
                },
                "candidate selection ID is empty".into(),
            ));
        }
    }
    let planned = plan_edits_unchecked(original, request, file_path);
    let Err((mut reason, detail)) = planned else {
        return planned;
    };
    enrich_candidates(&mut reason, pre_hash, provider_name(&request.operation));
    let Some(guard) = request.candidate_guard.as_ref() else {
        return Err((reason, detail));
    };
    let Some(candidate) = candidate_for_guard(&reason, guard) else {
        return Err((
            RefusalReason::CandidateSelectionInvalid {
                offset: guard.offset,
                selection_id: guard.selection_id.clone(),
                details: "selection_id and offset do not identify one reported candidate".into(),
            },
            "candidate selection did not match the refusal certificate".into(),
        ));
    };
    guarded_plan(original, request, candidate)
}

fn enrich_candidates(reason: &mut RefusalReason, pre_hash: &str, provider: &str) {
    if let RefusalReason::DuplicateTarget { candidates, .. } = reason {
        for candidate in candidates {
            if candidate.end < candidate.start {
                continue;
            }
            candidate.selection_id = candidate_selection_id(
                pre_hash,
                provider,
                candidate.start,
                candidate.end,
                &candidate.anchor_sha256,
            );
        }
    }
}

fn candidate_for_guard<'a>(
    reason: &'a RefusalReason,
    guard: &CandidateGuard,
) -> Option<&'a crate::protocol::Candidate> {
    let RefusalReason::DuplicateTarget { candidates, .. } = reason else {
        return None;
    };
    let mut matches = candidates.iter().filter(|candidate| {
        candidate.offset == guard.offset && candidate.selection_id == guard.selection_id
    });
    let candidate = matches.next()?;
    matches.next().is_none().then_some(candidate)
}

fn guarded_plan(
    original: &[u8],
    request: &Request,
    candidate: &crate::protocol::Candidate,
) -> Result<Vec<ByteEdit>, (RefusalReason, String)> {
    match &request.operation {
        OperationPayload::Text(operation) => {
            TextProvider::plan_at(original, operation, candidate.start, candidate.end).map_err(
                |error| match error {
                    TextProviderError::Refused(reason) => {
                        let detail = reason.code().to_string();
                        (reason, detail)
                    }
                    TextProviderError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                },
            )
        }
        OperationPayload::Code(operation) => {
            code::plan_at(original, operation, candidate.start, candidate.end).map_err(|error| {
                match error {
                    CodeError::Refused(reason) => {
                        let detail = reason.code().to_string();
                        (reason, detail)
                    }
                }
            })
        }
        _ => Err((
            RefusalReason::CandidateSelectionInvalid {
                offset: candidate.offset,
                selection_id: candidate.selection_id.clone(),
                details: format!(
                    "candidate selection is not implemented for provider {}",
                    provider_name(&request.operation)
                ),
            },
            "no safe automatic candidate-selected plan is available for this provider".into(),
        )),
    }
}

fn plan_edits_unchecked(
    original: &[u8],
    request: &Request,
    file_path: &str,
) -> Result<Vec<ByteEdit>, (RefusalReason, String)> {
    match &request.operation {
        OperationPayload::Text(o) => {
            TextProvider::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    TextProviderError::Refused(r) => (r, detail),
                    TextProviderError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                }
            })
        }
        OperationPayload::Json(o) => {
            JsonProvider::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    JsonProviderError::Refused(r) => (r, detail),
                    JsonProviderError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                }
            })
        }
        OperationPayload::Jsonc(o) => JsoncProvider::plan(original, o, &request.cardinality)
            .map_err(|e| {
                let detail = e.to_string();
                match e {
                    JsonProviderError::Refused(r) => (r, detail),
                    JsonProviderError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                }
            }),
        OperationPayload::Toml(o) => {
            TomlProvider::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    TomlProviderError::Refused(r) => (r, detail),
                    TomlProviderError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                }
            })
        }
        OperationPayload::Pattern(o) => {
            pattern::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    PatternError::Refused(r) => (r, detail),
                    PatternError::Error { message } => (
                        RefusalReason::Custom {
                            message: message.clone(),
                        },
                        message,
                    ),
                }
            })
        }
        OperationPayload::Markdown(o) => {
            markdown::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    MarkdownError::Refused(r) => (r, detail),
                }
            })
        }
        OperationPayload::Yaml(o) => yaml::plan(original, o, &request.cardinality).map_err(|e| {
            let detail = e.to_string();
            match e {
                YamlError::Refused(r) => (r, detail),
            }
        }),
        OperationPayload::File(_) => Err((
            RefusalReason::UnsupportedOperation {
                operation: "filesystem lifecycle is handled before content planning".into(),
            },
            "unreachable lifecycle operation".into(),
        )),
        OperationPayload::Code(o) => code::plan(original, o, &request.cardinality).map_err(|e| {
            let detail = e.to_string();
            match e {
                CodeError::Refused(r) => (r, detail),
            }
        }),
        OperationPayload::Dotenv(o) => {
            dotenv::plan(original, o, &request.cardinality).map_err(|e| {
                let detail = e.to_string();
                match e {
                    DotenvError::Refused(r) => (r, detail),
                }
            })
        }
        OperationPayload::Ini(o) => ini::plan(original, o, &request.cardinality).map_err(|e| {
            let detail = e.to_string();
            match e {
                IniError::Refused(reason) => (reason, detail),
            }
        }),
        OperationPayload::Patch(o) => {
            patch::plan_with_path(original, o, &request.cardinality, file_path).map_err(|e| {
                let detail = e.to_string();
                match e {
                    PatchError::Refused(r) => (r, detail),
                }
            })
        }
        OperationPayload::Web(o) => web::plan(original, o, &request.cardinality).map_err(|e| {
            let detail = e.to_string();
            match e {
                WebError::Refused(r) => (r, detail),
            }
        }),
        OperationPayload::DesiredState(DesiredStateOperation::Replace { desired_bytes }) => {
            diff_planner::plan(original, desired_bytes)
                .map(|plan| plan.edits)
                .map_err(|error| {
                    let detail = error.to_string();
                    let reason = match error {
                        diff_planner::DiffPlannerError::ResourceLimit {
                            dimension,
                            limit,
                            actual,
                        } => RefusalReason::ResourceLimitExceeded {
                            dimension: dimension.into(),
                            limit,
                            actual,
                        },
                        diff_planner::DiffPlannerError::Invariant(details) => {
                            RefusalReason::Custom {
                                message: format!("INTERNAL PLANNER INVARIANT FAILURE: {details}"),
                            }
                        }
                    };
                    (reason, detail)
                })
        }
    }
}

fn validate_candidate(
    request: &Request,
    candidate: &[u8],
) -> Result<StructuralValidation, FailureReason> {
    match &request.operation {
        OperationPayload::Json(_) => {
            JsonProvider::validate(candidate).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: "strict_json".into(),
            })
        }
        OperationPayload::Jsonc(_) => {
            JsoncProvider::validate(candidate).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: "strict_json".into(),
            })
        }
        OperationPayload::Toml(_) => {
            let valid = std::str::from_utf8(candidate)
                .ok()
                .and_then(|source| source.parse::<toml_edit::DocumentMut>().ok())
                .is_some();
            if !valid {
                return Err(FailureReason::ProviderError {
                    details: "candidate TOML failed validation".into(),
                });
            }
            Ok(StructuralValidation::Valid {
                format: "toml".into(),
            })
        }
        OperationPayload::Text(_) | OperationPayload::Pattern(_) => {
            Ok(StructuralValidation::NotApplicable)
        }
        OperationPayload::Markdown(_) => Ok(StructuralValidation::Valid {
            format: "markdown_regions".into(),
        }),
        OperationPayload::Yaml(_) => {
            yaml::validate(candidate).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: "yaml_conservative_source".into(),
            })
        }
        OperationPayload::File(_) => Ok(StructuralValidation::NotApplicable),
        OperationPayload::Code(operation) => {
            let language = match operation {
                CodeOperation::ReplaceNode { language, .. }
                | CodeOperation::InsertBeforeNode { language, .. }
                | CodeOperation::InsertAfterNode { language, .. }
                | CodeOperation::RemoveNode { language, .. } => language,
            };
            code::validate(candidate, language).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: format!("tree_sitter:{language}"),
            })
        }
        OperationPayload::Dotenv(_) => Ok(StructuralValidation::Valid {
            format: "dotenv_lines".into(),
        }),
        OperationPayload::Ini(_) => {
            ini::validate(candidate).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: "ini_source".into(),
            })
        }
        OperationPayload::Patch(_) => Ok(StructuralValidation::NotApplicable),
        OperationPayload::Web(operation) => {
            let language = web_language(operation);
            web::validate(candidate, language).map_err(|error| FailureReason::ProviderError {
                details: error.to_string(),
            })?;
            Ok(StructuralValidation::Valid {
                format: format!("tree_sitter:{language}"),
            })
        }
        OperationPayload::DesiredState(_) => Ok(StructuralValidation::NotApplicable),
    }
}

fn execute_file_operation(
    workspace: &Workspace,
    request: &Request,
    file_path: &str,
    operation: &FileOperation,
    dry_run: bool,
) -> Certificate {
    let provider = "filesystem";
    if !matches!(
        request.cardinality,
        crate::protocol::Cardinality::ExactlyOne
    ) {
        return refusal(
            request,
            file_path,
            provider,
            RefusalReason::CardinalityMismatch {
                expected: "exactly_one filesystem target".into(),
                actual: 1,
            },
            String::new(),
        );
    }
    let source = workspace.resolve_path(file_path);
    let mut expected_rename_content = None;
    let (pre_hash, post_hash, effect) = match operation {
        FileOperation::CreateFile {
            expected_absent,
            content,
        } => {
            if content.len() > MAX_FILE_BYTES {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::ResourceLimitExceeded {
                        dimension: "max_file_bytes".into(),
                        limit: MAX_FILE_BYTES,
                        actual: content.len(),
                    },
                    String::new(),
                );
            }
            if !expected_absent {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::Custom {
                        message: "create_file requires expected_absent=true".into(),
                    },
                    String::new(),
                );
            }
            match source {
                Ok(path) if path.exists() => {
                    return refusal(
                        request,
                        file_path,
                        provider,
                        RefusalReason::DestinationExists {
                            path: file_path.into(),
                        },
                        String::new(),
                    )
                }
                Err(e) => return workspace_error(request, file_path, provider, e),
                _ => {}
            }
            let effect = EffectUsage {
                files: 1,
                matches: 1,
                changed_regions: 1,
                changed_lines: content.split(|b| *b == b'\n').count(),
                changed_bytes: content.len(),
                passed: true,
            };
            (String::new(), Some(compute_sha256(content)), effect)
        }
        FileOperation::DeleteFile { expected_hash } => {
            let original = match workspace.read_file(file_path) {
                Ok(x) => x,
                Err(e) => return workspace_error(request, file_path, provider, e),
            };
            if original.len() > MAX_FILE_BYTES {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::ResourceLimitExceeded {
                        dimension: "max_file_bytes".into(),
                        limit: MAX_FILE_BYTES,
                        actual: original.len(),
                    },
                    String::new(),
                );
            }
            let pre = compute_sha256(&original);
            if normalize_hash(expected_hash) != pre {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::StaleIdentity {
                        expected_hash: normalize_hash(expected_hash),
                        actual_hash: pre.clone(),
                    },
                    pre,
                );
            }
            (
                pre,
                None,
                EffectUsage {
                    files: 1,
                    matches: 1,
                    changed_regions: 1,
                    changed_lines: original.split(|b| *b == b'\n').count(),
                    changed_bytes: original.len(),
                    passed: true,
                },
            )
        }
        FileOperation::RenameFile {
            destination,
            expected_source_hash,
            destination_absent,
        }
        | FileOperation::MoveFile {
            destination,
            expected_source_hash,
            destination_absent,
        } => {
            let original = match workspace.read_file(file_path) {
                Ok(x) => x,
                Err(e) => return workspace_error(request, file_path, provider, e),
            };
            if original.len() > MAX_FILE_BYTES {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::ResourceLimitExceeded {
                        dimension: "max_file_bytes".into(),
                        limit: MAX_FILE_BYTES,
                        actual: original.len(),
                    },
                    String::new(),
                );
            }
            let pre = compute_sha256(&original);
            if normalize_hash(expected_source_hash) != pre {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::StaleIdentity {
                        expected_hash: normalize_hash(expected_source_hash),
                        actual_hash: pre.clone(),
                    },
                    pre,
                );
            }
            let dest = match workspace
                .resolve_namespaced_path(destination, &request.namespace)
                .and_then(|path| workspace.resolve_destination_path(path))
            {
                Ok(x) => x,
                Err(e) => return workspace_error(request, file_path, provider, e),
            };
            let destination_relative =
                match workspace.resolve_namespaced_path(destination, &request.namespace) {
                    Ok(path) => path,
                    Err(error) => return workspace_error(request, file_path, provider, error),
                };
            let allowed = match budget_path_is_allowed(
                workspace,
                &destination_relative,
                &request.budget.allowed_path_prefixes,
            ) {
                Ok(allowed) => allowed,
                Err(error) => return workspace_error(request, file_path, provider, error),
            };
            if !allowed {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::WorkspaceTraversal {
                        path: "destination is outside requested budget scope".into(),
                    },
                    pre,
                );
            }
            let same_source = std::fs::canonicalize(&dest)
                .map(|path| source.as_ref().is_ok_and(|source| path == *source))
                .unwrap_or(false);
            if *destination_absent && dest.exists() && !same_source {
                return refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::DestinationExists {
                        path: destination.clone(),
                    },
                    pre,
                );
            }
            expected_rename_content = Some(original.clone());
            (
                pre,
                Some(compute_sha256(&original)),
                EffectUsage {
                    files: 1,
                    matches: 1,
                    changed_regions: 1,
                    changed_lines: 0,
                    changed_bytes: 0,
                    passed: true,
                },
            )
        }
    };
    if let Some((dimension, limit, actual)) = budget_violation(&effect, &request.budget) {
        return refusal_with_effect(
            request,
            file_path,
            provider,
            RefusalReason::EffectBudgetExceeded {
                dimension,
                limit,
                actual,
            },
            pre_hash,
            EffectUsage {
                passed: false,
                ..effect
            },
        );
    }
    if !dry_run {
        let result = match operation {
            FileOperation::CreateFile { content, .. } => {
                workspace.create_file_new(file_path, content)
            }
            FileOperation::DeleteFile { expected_hash } => {
                workspace.delete_file_checked(file_path, &normalize_hash(expected_hash))
            }
            FileOperation::RenameFile {
                destination,
                expected_source_hash,
                destination_absent,
            }
            | FileOperation::MoveFile {
                destination,
                expected_source_hash,
                destination_absent,
            } => {
                let destination_path =
                    match workspace.resolve_namespaced_path(destination, &request.namespace) {
                        Ok(path) => path,
                        Err(error) => return workspace_error(request, file_path, provider, error),
                    };
                let allowed = match budget_path_is_allowed(
                    workspace,
                    &destination_path,
                    &request.budget.allowed_path_prefixes,
                ) {
                    Ok(allowed) => allowed,
                    Err(error) => return workspace_error(request, file_path, provider, error),
                };
                if !allowed {
                    return refusal(
                        request,
                        file_path,
                        provider,
                        RefusalReason::WorkspaceTraversal {
                            path: "destination is outside requested budget scope".into(),
                        },
                        pre_hash,
                    );
                }
                workspace.rename_file_checked(
                    file_path,
                    destination_path,
                    &normalize_hash(expected_source_hash),
                    *destination_absent,
                )
            }
        };
        if let Err(e) = result {
            return match e {
                WorkspaceError::AlreadyExists(path) => refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::DestinationExists { path },
                    pre_hash,
                ),
                WorkspaceError::StaleIdentity { expected, actual } => refusal(
                    request,
                    file_path,
                    provider,
                    RefusalReason::StaleIdentity {
                        expected_hash: expected,
                        actual_hash: actual,
                    },
                    pre_hash,
                ),
                other => failure(
                    request,
                    file_path,
                    provider,
                    pre_hash,
                    FailureReason::CommitFailure {
                        message: other.to_string(),
                    },
                ),
            };
        }
    }
    if !dry_run {
        let verification = match operation {
            FileOperation::CreateFile { content, .. } => match workspace.read_file(file_path) {
                Ok(landed) if landed == *content => Ok(()),
                Ok(landed) => Err(compute_sha256(&landed)),
                Err(error) => Err(format!("read failed: {error}")),
            },
            FileOperation::DeleteFile { .. } => match workspace.read_file(file_path) {
                Err(WorkspaceError::NotFound(_)) => Ok(()),
                Ok(landed) => Err(compute_sha256(&landed)),
                Err(error) => Err(format!("delete verification failed: {error}")),
            },
            FileOperation::RenameFile { destination, .. }
            | FileOperation::MoveFile { destination, .. } => {
                let destination_path =
                    match workspace.resolve_namespaced_path(destination, &request.namespace) {
                        Ok(path) => path,
                        Err(error) => {
                            return failure(
                                request,
                                file_path,
                                provider,
                                pre_hash,
                                FailureReason::PostCommitVerificationFailure {
                                    expected_hash: post_hash.clone().unwrap_or_default(),
                                    actual_hash: format!("destination resolution failed: {error}"),
                                },
                            )
                        }
                    };
                match workspace.read_file(destination_path) {
                    Ok(landed) if expected_rename_content.as_deref() == Some(landed.as_slice()) => {
                        Ok(())
                    }
                    Ok(landed) => Err(compute_sha256(&landed)),
                    Err(error) => Err(format!("read failed: {error}")),
                }
            }
        };
        if let Err(actual_hash) = verification {
            return failure(
                request,
                file_path,
                provider,
                pre_hash,
                FailureReason::PostCommitVerificationFailure {
                    expected_hash: post_hash.unwrap_or_else(|| "absent".into()),
                    actual_hash,
                },
            );
        }
    }
    let commit = if dry_run {
        CommitGuarantee {
            mode: "dry_run".into(),
            ..CommitGuarantee::default()
        }
    } else {
        CommitGuarantee {
            mode: "committed_checked_lifecycle".into(),
            content_replacement: "checked filesystem operation".into(),
            ..CommitGuarantee::default()
        }
    };
    completed(
        request,
        file_path,
        provider,
        pre_hash,
        post_hash,
        Outcome::Applied,
        Vec::new(),
        StructuralValidation::NotApplicable,
        PreservationFacts::default(),
        commit,
        String::new(),
        false,
        effect,
    )
}

fn mark_transaction_certificates(certificates: &mut [Certificate], recovery_state: &str) {
    for certificate in certificates {
        if certificate.outcome == Outcome::Applied {
            certificate.commit = CommitGuarantee {
                mode: "committed_atomic_replace".into(),
                content_replacement: "atomic replacement after staged flush".into(),
                permissions: "platform-dependent; not asserted".into(),
                timestamps: "not preserved".into(),
                acl_xattr: "unknown".into(),
            };
        }
        certificate.transaction_guarantee = "transactional_with_rollback".into();
        certificate.recovery_state = recovery_state.into();
    }
}

fn validate_region_guard(
    content: &[u8],
    guard: &crate::protocol::RegionGuard,
    request: &Request,
) -> Result<(), RefusalReason> {
    if guard.anchor.is_empty() {
        return Err(RefusalReason::MalformedInput {
            details: "durable region anchor must not be empty".into(),
        });
    }
    let matches = content
        .windows(guard.anchor.len())
        .filter(|bytes| *bytes == guard.anchor.as_bytes())
        .count();
    if matches != 1 {
        return Err(RefusalReason::CardinalityMismatch {
            expected: "one durable region anchor".into(),
            actual: matches,
        });
    }
    let actual = compute_sha256(guard.anchor.as_bytes());
    if actual != normalize_hash(&guard.target_sha256) {
        return Err(RefusalReason::StaleIdentity {
            expected_hash: normalize_hash(&guard.target_sha256),
            actual_hash: actual,
        });
    }
    if guard.mode == crate::protocol::RegionGuardMode::StructuralSnapshot {
        let OperationPayload::Code(operation) = &request.operation else {
            return Err(RefusalReason::ProviderCapabilityMissing {
                provider: provider_name(&request.operation).into(),
                capability: "structural_snapshot requires the code provider".into(),
            });
        };
        let target = match operation {
            CodeOperation::ReplaceNode { target, .. }
            | CodeOperation::InsertBeforeNode { target, .. }
            | CodeOperation::InsertAfterNode { target, .. }
            | CodeOperation::RemoveNode { target, .. } => target,
        };
        if target != &guard.anchor {
            return Err(RefusalReason::StaleIdentity {
                expected_hash: normalize_hash(&guard.target_sha256),
                actual_hash: compute_sha256(target.as_bytes()),
            });
        }
        if let Err(CodeError::Refused(reason)) = code::plan(
            content,
            operation,
            &crate::protocol::Cardinality::ExactlyOne,
        ) {
            return Err(reason);
        }
    }
    Ok(())
}

fn normalize_hash(value: &str) -> String {
    value.strip_prefix("sha256:").unwrap_or(value).into()
}
fn provider_name(op: &OperationPayload) -> &'static str {
    match op {
        OperationPayload::Text(_) => "text",
        OperationPayload::Json(_) => "json",
        OperationPayload::Jsonc(_) => "jsonc",
        OperationPayload::Toml(_) => "toml",
        OperationPayload::Pattern(_) => "pattern",
        OperationPayload::Markdown(_) => "markdown",
        OperationPayload::Yaml(_) => "yaml",
        OperationPayload::File(_) => "filesystem",
        OperationPayload::Code(_) => "code",
        OperationPayload::Dotenv(_) => "dotenv",
        OperationPayload::Ini(_) => "ini",
        OperationPayload::Patch(_) => "patch",
        OperationPayload::Web(_) => "web",
        OperationPayload::DesiredState(_) => "desired_state",
    }
}
fn provider_version(op: &OperationPayload) -> &'static str {
    match op {
        OperationPayload::Text(_) => "text-byte-v1",
        OperationPayload::Json(_) => "json-source-v1",
        OperationPayload::Jsonc(_) => "jsonc-source-v1",
        OperationPayload::Toml(_) => "toml-edit-narrow-v1",
        OperationPayload::Pattern(_) => "regex-automata-bounded-v1",
        OperationPayload::Markdown(_) => "markdown-regions-v2",
        OperationPayload::Yaml(_) => "yaml-conservative-source-v1",
        OperationPayload::File(_) => "lifecycle-checked-v1",
        OperationPayload::Code(_) => "tree-sitter-node-v1",
        OperationPayload::Dotenv(_) => "dotenv-lines-v1",
        OperationPayload::Ini(_) => "ini-source-v1",
        OperationPayload::Patch(_) => "unified-diff-strict-v1",
        OperationPayload::Web(_) => "tree-sitter-web-node-v1",
        OperationPayload::DesiredState(_) => "strict-derived-bounded-edits-v1",
    }
}
fn unsupported_encoding(bytes: &[u8]) -> Option<RefusalReason> {
    if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[0xff, 0xfe, 0, 0])
        || bytes.starts_with(&[0, 0, 0xfe, 0xff])
    {
        Some(RefusalReason::UnsupportedEncoding {
            details: "UTF-16/UTF-32 is not supported by this protocol version".into(),
        })
    } else if std::str::from_utf8(bytes).is_err() {
        Some(RefusalReason::UnsupportedEncoding {
            details: "input is not valid UTF-8".into(),
        })
    } else {
        None
    }
}

fn generated_marker(bytes: &[u8]) -> &'static str {
    let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    if sample.contains("code generated") {
        "code generated"
    } else if sample.contains("machine-generated") {
        "machine-generated"
    } else if sample.contains("do not edit") {
        "do not edit"
    } else {
        "generated file"
    }
}

fn is_generated_file(bytes: &[u8]) -> bool {
    let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    [
        "code generated",
        "machine-generated",
        "do not edit",
        "generated file",
    ]
    .iter()
    .any(|marker| sample.contains(marker))
}
fn changed_ranges(edits: &[ByteEdit]) -> Vec<ByteRange> {
    edits
        .iter()
        .filter(|e| e.start != e.end || !e.replacement.is_empty())
        .map(|e| ByteRange {
            start: e.start,
            end: e.end,
        })
        .collect()
}

fn changed_line_ranges(original: &[u8], edits: &[ByteEdit]) -> Vec<ByteRange> {
    // Index newlines once. The previous implementation rescanned every prefix
    // for every edit, which made many-edit plans unnecessarily quadratic in the
    // source size. `partition_point` preserves the old exclusive-prefix count:
    // a newline at the edit boundary belongs after the boundary.
    let newline_positions: Vec<usize> = memchr_iter(b'\n', original).collect();
    edits
        .iter()
        .map(|edit| {
            let start_offset = edit.start.min(original.len());
            let end_offset = edit.end.min(original.len());
            let start = newline_positions.partition_point(|&position| position < start_offset) + 1;
            let end = newline_positions.partition_point(|&position| position < end_offset) + 1;
            ByteRange { start, end }
        })
        .collect()
}
fn bounded_diff(original: &[u8], edits: &[ByteEdit]) -> (String, bool) {
    const LIMIT: usize = 4096;
    let mut d = String::from("byte-range diff (unchanged bytes omitted):\n");
    for edit in edits {
        let old = String::from_utf8_lossy(&original[edit.start..edit.end]);
        let new = String::from_utf8_lossy(&edit.replacement);
        d.push_str(&format!(
            "@@ {}..{} @@\n-{}\n+{}\n",
            edit.start, edit.end, old, new
        ));
    }
    if d.len() > LIMIT {
        let end = d
            .char_indices()
            .take_while(|(i, _)| *i < LIMIT)
            .map(|(i, _)| i)
            .last()
            .unwrap_or(0);
        (
            format!(
                "{}\n[diff truncated; total characters: {}]",
                &d[..end],
                d.len()
            ),
            true,
        )
    } else {
        (d, false)
    }
}
fn refusal(
    r: &Request,
    path: &str,
    provider: &str,
    reason: RefusalReason,
    pre: String,
) -> Certificate {
    let mut certificate = completed(
        r,
        path,
        provider,
        pre.clone(),
        None,
        Outcome::Refused,
        Vec::new(),
        StructuralValidation::NotApplicable,
        PreservationFacts::default(),
        CommitGuarantee::default(),
        format!("{reason:?}"),
        false,
        zero_effect(),
    );
    certificate.refusal_reason = Some(reason.clone());
    certificate.reason_code = Some(reason.code().into());
    certificate.recovery = recovery_for(r, &reason, &pre);
    certificate
}

fn refusal_with_effect(
    r: &Request,
    path: &str,
    provider: &str,
    reason: RefusalReason,
    pre: String,
    effect: EffectUsage,
) -> Certificate {
    let mut certificate = completed(
        r,
        path,
        provider,
        pre.clone(),
        None,
        Outcome::Refused,
        Vec::new(),
        StructuralValidation::NotApplicable,
        PreservationFacts::default(),
        CommitGuarantee::default(),
        format!("{reason:?}"),
        false,
        effect,
    );
    certificate.refusal_reason = Some(reason.clone());
    certificate.reason_code = certificate
        .refusal_reason
        .as_ref()
        .map(|value| value.code().into());
    certificate.recovery = recovery_for(r, &reason, &pre);
    certificate
}

fn recovery_for(request: &Request, reason: &RefusalReason, pre_hash: &str) -> Option<RecoveryInfo> {
    let mut remedies = Vec::new();
    let request_patch = |mut patched: Request| {
        serde_json::to_value({
            patched.version = request.version.clone();
            patched
        })
        .ok()
    };
    match reason {
        RefusalReason::DuplicateTarget { candidates, .. } => {
            let mut candidates = candidates.clone();
            candidates.sort_by(|left, right| {
                (left.offset, left.end, &left.selection_id).cmp(&(
                    right.offset,
                    right.end,
                    &right.selection_id,
                ))
            });
            for candidate in candidates {
                let mut patched = request.clone();
                patched.expected_pre_hash = (!pre_hash.is_empty()).then(|| pre_hash.to_owned());
                patched.candidate_guard = Some(CandidateGuard {
                    offset: candidate.offset,
                    selection_id: candidate.selection_id.clone(),
                });
                remedies.push(RecoveryRemedy {
                    kind: "candidate_selection".into(),
                    description: format!("candidate at line {}", candidate.line),
                    request_patch: request_patch(patched),
                });
            }
            Some(RecoveryInfo {
                requires_choice: true,
                remedies,
            })
        }
        RefusalReason::WorkspaceRootMismatch { target, workspace_root } => Some(RecoveryInfo {
            requires_choice: true,
            remedies: vec![RecoveryRemedy {
                kind: "checkout_local_cli".into(),
                description: format!("target {target} is outside configured workspace {workspace_root}; if this is the intended authorized checkout, run the checkout-local Threadmoth CLI with that checkout as its working directory and preview again"),
                request_patch: None,
            }],
        }),
        RefusalReason::StaleIdentity { actual_hash, .. } => {
            let mut patched = request.clone();
            patched.expected_pre_hash = Some(actual_hash.clone());
            remedies.push(RecoveryRemedy {
                kind: "refresh_pre_hash".into(),
                description: "refresh the observed file hash, then preview again".into(),
                request_patch: request_patch(patched),
            });
            Some(RecoveryInfo {
                requires_choice: false,
                remedies,
            })
        }
        RefusalReason::EffectBudgetExceeded {
            dimension, actual, ..
        } => {
            let mut patched = request.clone();
            match dimension.as_str() {
                "max_files" => patched.budget.max_files = Some(*actual),
                "max_matches" => patched.budget.max_matches = Some(*actual),
                "max_changed_regions" => patched.budget.max_changed_regions = Some(*actual),
                "max_changed_lines" => patched.budget.max_changed_lines = Some(*actual),
                "max_changed_bytes" => patched.budget.max_changed_bytes = Some(*actual),
                _ => return None,
            }
            remedies.push(RecoveryRemedy {
                kind: "increase_exact_budget".into(),
                description: format!("raise {dimension} to the observed bounded effect"),
                request_patch: request_patch(patched),
            });
            Some(RecoveryInfo {
                requires_choice: true,
                remedies,
            })
        }
        RefusalReason::MissingTarget { .. } => Some(RecoveryInfo {
            requires_choice: true,
            remedies: vec![RecoveryRemedy {
                kind: "narrow_target".into(),
                description: "inspect the current file and provide a supported exact target".into(),
                request_patch: None,
            }],
        }),
        RefusalReason::LossyOperationRequiresOptIn { operation } => Some(RecoveryInfo {
            requires_choice: true,
            remedies: vec![RecoveryRemedy {
                kind: "explicit_lossy_opt_in".into(),
                description: format!("explicitly authorize the lossy operation {operation}"),
                request_patch: None,
            }],
        }),
        RefusalReason::WorkspaceBusy { .. } => Some(RecoveryInfo {
            requires_choice: false,
            remedies: vec![RecoveryRemedy {
                kind: "retry_after_lock_release".into(),
                description: "retry after the cooperating Threadmoth writer exits".into(),
                request_patch: None,
            }],
        }),
        RefusalReason::PlanStale { .. } | RefusalReason::PlanInvalid { .. } => Some(RecoveryInfo {
            requires_choice: false,
            remedies: vec![RecoveryRemedy {
                kind: "rebuild_plan".into(),
                description: "re-prepare the plan from the current workspace state".into(),
                request_patch: None,
            }],
        }),
        _ => None,
    }
}
fn failure(
    r: &Request,
    path: &str,
    provider: &str,
    pre: String,
    reason: FailureReason,
) -> Certificate {
    let mut c = completed(
        r,
        path,
        provider,
        pre,
        None,
        Outcome::Failed,
        Vec::new(),
        StructuralValidation::NotApplicable,
        PreservationFacts::default(),
        CommitGuarantee::default(),
        String::new(),
        false,
        zero_effect(),
    );
    c.reason_code = Some(reason.code().into());
    c.failure_reason = Some(reason);
    c
}
fn workspace_error(r: &Request, path: &str, provider: &str, e: WorkspaceError) -> Certificate {
    match e {
        WorkspaceError::WorkspaceRootMismatch {
            target,
            workspace_root,
        } => refusal(
            r,
            path,
            provider,
            RefusalReason::WorkspaceRootMismatch {
                target,
                workspace_root,
            },
            String::new(),
        ),
        WorkspaceError::Traversal(p) => refusal(
            r,
            path,
            provider,
            RefusalReason::WorkspaceTraversal { path: p.clone() },
            String::new(),
        ),
        WorkspaceError::SymlinkEscape(p) => refusal(
            r,
            path,
            provider,
            RefusalReason::SymlinkEscape { path: p.clone() },
            String::new(),
        ),
        WorkspaceError::NotFound(p) => refusal(
            r,
            path,
            provider,
            RefusalReason::MissingTarget { target: p.clone() },
            String::new(),
        ),
        WorkspaceError::StaleIdentity { expected, actual } => refusal(
            r,
            path,
            provider,
            RefusalReason::StaleIdentity {
                expected_hash: expected,
                actual_hash: actual,
            },
            String::new(),
        ),
        WorkspaceError::Io(e) => failure(
            r,
            path,
            provider,
            String::new(),
            FailureReason::IoError {
                message: e.to_string(),
            },
        ),
        WorkspaceError::AlreadyExists(path) => refusal(
            r,
            &path.clone(),
            provider,
            RefusalReason::DestinationExists { path },
            String::new(),
        ),
        WorkspaceError::UnmappablePath(path) => refusal(
            r,
            &path,
            provider,
            RefusalReason::UnmappablePath { path: path.clone() },
            String::new(),
        ),
        WorkspaceError::ResourceLimit {
            dimension,
            limit,
            actual,
        } => refusal(
            r,
            path,
            provider,
            RefusalReason::ResourceLimitExceeded {
                dimension,
                limit,
                actual,
            },
            String::new(),
        ),
        WorkspaceError::Busy { lock_path } => refusal(
            r,
            path,
            provider,
            RefusalReason::WorkspaceBusy { lock_path },
            String::new(),
        ),
    }
}

fn workspace_reason(error: WorkspaceError) -> RefusalReason {
    match error {
        WorkspaceError::WorkspaceRootMismatch {
            target,
            workspace_root,
        } => RefusalReason::WorkspaceRootMismatch {
            target,
            workspace_root,
        },
        WorkspaceError::Traversal(path) => RefusalReason::WorkspaceTraversal { path },
        WorkspaceError::SymlinkEscape(path) => RefusalReason::SymlinkEscape { path },
        WorkspaceError::NotFound(path) => RefusalReason::MissingTarget { target: path },
        WorkspaceError::StaleIdentity { expected, actual } => RefusalReason::StaleIdentity {
            expected_hash: expected,
            actual_hash: actual,
        },
        WorkspaceError::AlreadyExists(path) => RefusalReason::DestinationExists { path },
        WorkspaceError::UnmappablePath(path) => RefusalReason::UnmappablePath { path },
        WorkspaceError::ResourceLimit {
            dimension,
            limit,
            actual,
        } => RefusalReason::ResourceLimitExceeded {
            dimension,
            limit,
            actual,
        },
        WorkspaceError::Busy { lock_path } => RefusalReason::WorkspaceBusy { lock_path },
        WorkspaceError::Io(error) => RefusalReason::Custom {
            message: format!("workspace I/O error: {error}"),
        },
    }
}
#[allow(clippy::too_many_arguments)]
fn completed(
    r: &Request,
    path: &str,
    provider: &str,
    pre: String,
    post: Option<String>,
    outcome: Outcome,
    ranges: Vec<ByteRange>,
    structural: StructuralValidation,
    preservation: PreservationFacts,
    commit: CommitGuarantee,
    diff: String,
    truncated: bool,
    effect: EffectUsage,
) -> Certificate {
    let transaction_guarantee = commit.mode.clone();
    Certificate {
        protocol_version: r.version.clone(),
        request_id: request_id(r),
        outcome,
        file_path: path.into(),
        provider: provider.into(),
        provider_version: provider_version(&r.operation).into(),
        expected_cardinality: r.cardinality.clone(),
        observed_cardinality: Some(1),
        pre_hash: pre,
        post_hash: post,
        changed_ranges: ranges,
        changed_line_ranges: Vec::new(),
        diff_summary: Some(diff),
        diff_truncated: truncated,
        structural_validation: structural,
        preservation,
        commit,
        refusal_reason: None,
        failure_reason: None,
        reason_code: None,
        recovery: None,
        diagnostics: Vec::new(),
        schema_diagnostic: None,
        budget: r.budget.clone(),
        effect,
        transaction_guarantee,
        recovery_state: "not_required".into(),
        desired_state: None,
    }
}

fn desired_bytes(request: &Request) -> Option<&[u8]> {
    match &request.operation {
        OperationPayload::DesiredState(DesiredStateOperation::Replace { desired_bytes }) => {
            Some(desired_bytes)
        }
        _ => None,
    }
}

fn attach_desired_evidence(
    desired: Option<&[u8]>,
    edits: &[ByteEdit],
    effect: &EffectUsage,
    certificate: &mut Certificate,
) {
    let Some(desired) = desired else { return };
    certificate.desired_state = Some(DesiredStateEvidence {
        mode: "desired_state".into(),
        desired_hash: compute_sha256(desired),
        derived_region_count: edits.len(),
        changed_lines: effect.changed_lines,
        changed_bytes: effect.changed_bytes,
        verification: if certificate.outcome == Outcome::NoChange {
            "pre_hash_equals_desired_hash".into()
        } else if certificate.outcome == Outcome::Applied {
            "post_hash_equals_desired_hash".into()
        } else {
            "plan_candidate_verified".into()
        },
    });
}

fn web_language(operation: &WebOperation) -> &str {
    match operation {
        WebOperation::ReplaceNode { language, .. }
        | WebOperation::InsertBeforeNode { language, .. }
        | WebOperation::InsertAfterNode { language, .. }
        | WebOperation::RemoveNode { language, .. } => language,
    }
}

fn zero_effect() -> EffectUsage {
    EffectUsage {
        files: 0,
        matches: 0,
        changed_regions: 0,
        changed_lines: 0,
        changed_bytes: 0,
        passed: true,
    }
}

fn request_id(r: &Request) -> String {
    if r.request_id.is_empty() {
        format!(
            "threadmoth-{}",
            &compute_sha256(r.file_path.as_bytes())[..16]
        )
    } else {
        r.request_id.clone()
    }
}

fn effect_usage(
    original: &[u8],
    candidate: &[u8],
    edits: &[ByteEdit],
    _budget: &EffectBudget,
) -> EffectUsage {
    let changed_bytes = edits
        .iter()
        .map(|edit| {
            edit.end
                .saturating_sub(edit.start)
                .max(edit.replacement.len())
        })
        .sum();
    let changed_lines = changed_line_count(original, candidate);
    EffectUsage {
        files: usize::from(original != candidate),
        matches: edits.len(),
        changed_regions: edits.len(),
        changed_lines,
        changed_bytes,
        passed: true,
    }
}

fn budget_violation(effect: &EffectUsage, budget: &EffectBudget) -> Option<(String, usize, usize)> {
    [
        ("max_files", budget.max_files, effect.files),
        ("max_matches", budget.max_matches, effect.matches),
        (
            "max_changed_regions",
            budget.max_changed_regions,
            effect.changed_regions,
        ),
        (
            "max_changed_lines",
            budget.max_changed_lines,
            effect.changed_lines,
        ),
        (
            "max_changed_bytes",
            budget.max_changed_bytes,
            effect.changed_bytes,
        ),
    ]
    .into_iter()
    .find_map(|(name, limit, actual)| {
        limit
            .filter(|limit| actual > *limit)
            .map(|limit| (name.into(), limit, actual))
    })
}

fn changed_line_count(original: &[u8], candidate: &[u8]) -> usize {
    // Keep the general-purpose diff here deliberately. Its old/new line
    // contribution is part of the effect-budget contract, and an edit-derived
    // approximation is not generally equivalent when similar aligns equal
    // lines outside the edited byte ranges. Replace this only with
    // differential/property evidence covering those alignment cases.
    let old = String::from_utf8_lossy(original);
    let new = String::from_utf8_lossy(candidate);
    similar::TextDiff::from_lines(&old, &new)
        .ops()
        .iter()
        .map(|op| match op {
            similar::DiffOp::Delete { old_len, .. } => *old_len,
            similar::DiffOp::Insert { new_len, .. } => *new_len,
            similar::DiffOp::Replace {
                old_len, new_len, ..
            } => (*old_len).max(*new_len),
            similar::DiffOp::Equal { .. } => 0,
        })
        .sum()
}
impl PreservationFacts {
    fn from_bytes(a: &[u8], b: &[u8]) -> Self {
        Self {
            unrelated_bytes_changed: false,
            line_endings_changed: newline_profile(a) != newline_profile(b),
            bom_changed: a.starts_with(&[0xef, 0xbb, 0xbf]) != b.starts_with(&[0xef, 0xbb, 0xbf]),
            final_newline_changed: a.ends_with(b"\n") != b.ends_with(b"\n"),
            comments_preserved: Some(comment_count(a) == comment_count(b)),
            metadata: "content-only verification; replacement metadata not asserted".into(),
            original_newline_profile: newline_profile(a),
            result_newline_profile: newline_profile(b),
        }
    }
}

fn newline_profile(bytes: &[u8]) -> String {
    let crlf = bytes.windows(2).filter(|w| w == b"\r\n").count();
    let lf = bytes
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        .saturating_sub(crlf);
    let bare_cr = bytes
        .iter()
        .filter(|b| **b == b'\r')
        .count()
        .saturating_sub(crlf);
    match (crlf > 0, lf > 0, bare_cr > 0) {
        (false, false, false) => "none",
        (true, false, false) => "crlf",
        (false, true, false) => "lf",
        (true, true, false) => "mixed",
        _ => "mixed_with_bare_cr",
    }
    .into()
}
fn comment_count(b: &[u8]) -> usize {
    b.split(|x| *x == b'\n')
        .filter(|l| {
            l.iter()
                .position(|x| *x != b' ' && *x != b'\t' && *x != b'\r')
                .map(|i| l[i..].starts_with(b"#"))
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Cardinality, OperationPayload, PROTOCOL_VERSION};
    use crate::provider::text::TextOperation;
    use tempfile::TempDir;
    #[test]
    fn apply_and_certify_landed_bytes() {
        let t = TempDir::new().unwrap();
        let w = Workspace::new(t.path()).unwrap();
        std::fs::write(t.path().join("x.txt"), b"a b\n").unwrap();
        let r = Request {
            version: PROTOCOL_VERSION.into(),
            request_id: String::new(),
            allow_generated: false,
            file_path: "x.txt".into(),
            namespace: Default::default(),
            expected_pre_hash: None,
            region_guard: None,
            candidate_guard: None,
            cardinality: Cardinality::ExactlyOne,
            budget: Default::default(),
            operation: OperationPayload::Text(TextOperation::Replace {
                target: "b".into(),
                replacement: "c".into(),
            }),
        };
        let c = execute_request(&w, &r, false);
        let expected = compute_sha256(b"a c\n");
        assert_eq!(c.outcome, Outcome::Applied);
        assert_eq!(c.post_hash.as_deref(), Some(expected.as_str()));
        assert_eq!(w.read_file("x.txt").unwrap(), b"a c\n");
    }
    #[test]
    fn dry_run_is_non_mutating() {
        let t = TempDir::new().unwrap();
        let w = Workspace::new(t.path()).unwrap();
        std::fs::write(t.path().join("x.txt"), b"a b").unwrap();
        let r = Request {
            version: PROTOCOL_VERSION.into(),
            request_id: String::new(),
            allow_generated: false,
            file_path: "x.txt".into(),
            namespace: Default::default(),
            expected_pre_hash: None,
            region_guard: None,
            candidate_guard: None,
            cardinality: Cardinality::ExactlyOne,
            budget: Default::default(),
            operation: OperationPayload::Text(TextOperation::Replace {
                target: "b".into(),
                replacement: "c".into(),
            }),
        };
        let c = execute_request(&w, &r, true);
        assert_eq!(c.outcome, Outcome::Applied);
        assert_eq!(w.read_file("x.txt").unwrap(), b"a b");
    }

    fn reference_changed_line_ranges(original: &[u8], edits: &[ByteEdit]) -> Vec<ByteRange> {
        edits
            .iter()
            .map(|edit| ByteRange {
                start: original[..edit.start.min(original.len())]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count()
                    + 1,
                end: original[..edit.end.min(original.len())]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count()
                    + 1,
            })
            .collect()
    }

    #[test]
    fn changed_line_ranges_match_the_previous_prefix_scan() {
        let original = b"first\nsecond\n\nfourth\nlast";
        let mut edits = Vec::new();
        for start in 0..=original.len() {
            for end in start..=original.len() {
                edits.push(ByteEdit {
                    start,
                    end,
                    replacement: Vec::new(),
                });
            }
        }
        edits.push(ByteEdit {
            start: original.len() + 10,
            end: original.len() + 20,
            replacement: Vec::new(),
        });

        assert_eq!(
            changed_line_ranges(original, &edits),
            reference_changed_line_ranges(original, &edits)
        );
    }
}
