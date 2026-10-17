//! Compact source-aware text preparation. The returned plan is the existing
//! portable exact plan; preparation itself never writes.

use crate::{
    engine::compute_sha256,
    pipeline::prepare_request_plan,
    protocol::{
        Cardinality, Certificate, EffectBudget, OperationPayload, PreparedPlan, Request,
        PROTOCOL_VERSION,
    },
    provider::text::TextOperation,
    workspace::Workspace,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::provider::text::{NewlinePolicy, TextReplacementIntent};

pub const MAX_SOURCE_EDITS: usize = 64;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceAwareTextRequest {
    pub file_path: String,
    pub edits: Vec<TextReplacementIntent>,
    #[serde(default)]
    pub newline_policy: NewlinePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceAwareTextPreparation {
    pub outcome: String,
    pub source_sha256: String,
    pub source_newline_profile: String,
    pub resolved_edits: usize,
    pub plan: PreparedPlan,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourcePreparationResult {
    Prepared(Box<SourceAwareTextPreparation>),
    Refused { certificate: Box<Certificate> },
}

/// Resolve a compact replacement sequence against one observed source state.
/// Every failure is returned through Threadmoth's normal refusal certificate.
pub fn prepare(workspace: &Workspace, request: &SourceAwareTextRequest) -> SourcePreparationResult {
    let resolved = match workspace
        .resolve_namespaced_path(&request.file_path, &crate::path::PathNamespace::Native)
    {
        Ok(path) => path,
        Err(_) => {
            let failed = Request {
                version: PROTOCOL_VERSION.into(),
                request_id: "source-text-prepare".into(),
                allow_generated: false,
                file_path: request.file_path.clone(),
                namespace: crate::path::PathNamespace::Native,
                expected_pre_hash: None,
                region_guard: None,
                candidate_guard: None,
                cardinality: Cardinality::ExactlyOne,
                budget: EffectBudget::default(),
                operation: OperationPayload::Text(TextOperation::ReplaceSequence {
                    edits: request.edits.clone(),
                    newline_policy: request.newline_policy,
                }),
            };
            // Core is the authority for path semantics and certificate codes.
            return SourcePreparationResult::Refused {
                certificate: Box::new(crate::pipeline::execute_request(workspace, &failed, true)),
            };
        }
    };
    let source = match workspace.read_file(&resolved) {
        Ok(source) => source,
        Err(_) => {
            let failed = Request {
                version: PROTOCOL_VERSION.into(),
                request_id: "source-text-prepare".into(),
                allow_generated: false,
                file_path: resolved.clone(),
                namespace: crate::path::PathNamespace::Native,
                expected_pre_hash: None,
                region_guard: None,
                candidate_guard: None,
                cardinality: Cardinality::ExactlyOne,
                budget: EffectBudget::default(),
                operation: OperationPayload::Text(TextOperation::ReplaceSequence {
                    edits: request.edits.clone(),
                    newline_policy: request.newline_policy,
                }),
            };
            return SourcePreparationResult::Refused {
                certificate: Box::new(crate::pipeline::execute_request(workspace, &failed, true)),
            };
        }
    };
    let source_hash = compute_sha256(&source);
    let profile = newline_profile(&source);
    let plan_request = Request {
        version: PROTOCOL_VERSION.into(),
        request_id: format!("source-text-{}", &source_hash[..16]),
        allow_generated: false,
        file_path: resolved.clone(),
        namespace: crate::path::PathNamespace::Native,
        expected_pre_hash: Some(source_hash.clone()),
        region_guard: request
            .within
            .as_ref()
            .map(|anchor| crate::protocol::RegionGuard {
                anchor: anchor.clone(),
                target_sha256: compute_sha256(anchor.as_bytes()),
                mode: crate::protocol::RegionGuardMode::RegionSnapshot,
            }),
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget {
            max_files: Some(1),
            allowed_path_prefixes: vec![resolved.clone()],
            ..EffectBudget::default()
        },
        operation: OperationPayload::Text(TextOperation::ReplaceSequence {
            edits: request.edits.clone(),
            newline_policy: request.newline_policy,
        }),
    };
    match prepare_request_plan(workspace, &plan_request, Vec::new()) {
        Ok(plan) => SourcePreparationResult::Prepared(Box::new(SourceAwareTextPreparation {
            outcome: "PREPARED".into(),
            source_sha256: source_hash,
            source_newline_profile: profile.into(),
            resolved_edits: request.edits.len(),
            plan,
        })),
        Err(mut certificate) => {
            if certificate.reason_code.as_deref() == Some("TARGET_NOT_FOUND") {
                let recovery =
                    certificate
                        .recovery
                        .get_or_insert_with(|| crate::protocol::RecoveryInfo {
                            requires_choice: true,
                            remedies: Vec::new(),
                        });
                recovery.remedies.push(crate::protocol::RecoveryRemedy {
                    kind: "use_file_based_text_input".into(),
                    description: "If multiline CLI arguments may have been altered by shell quoting, place the exact UTF-8 old and new text in workspace files and retry with: threadmoth prepare-text PATH --edit-files OLD_FILE NEW_FILE --output PLAN --summary. This is an alternate input route, not a fuzzy target search; the refusal does not establish that shell quoting caused the mismatch.".into(),
                    request_patch: None,
                });
            }
            if certificate.reason_code.as_deref() == Some("TARGET_AMBIGUOUS") {
                let recovery =
                    certificate
                        .recovery
                        .get_or_insert_with(|| crate::protocol::RecoveryInfo {
                            requires_choice: true,
                            remedies: Vec::new(),
                        });
                recovery.remedies.push(crate::protocol::RecoveryRemedy {
                    kind: "use_scope_anchor".into(),
                    description: "If the target exists in multiple functions or sections, scope the edit to a specific declaration with --within <ANCHOR>.".into(),
                    request_patch: None,
                });
            }
            SourcePreparationResult::Refused {
                certificate: Box::new(certificate),
            }
        }
    }
}

fn newline_profile(content: &[u8]) -> &'static str {
    crate::engine::newline_profile(content)
}
