//! Compact, exact, source-aware multi-file edit documents.
//!
//! The parser only translates author intent into existing `ReplaceSequence`
//! requests. Preparation and application remain owned by Threadmoth's normal
//! transaction planner and portable prepared-plan pipeline.

use crate::{
    engine::compute_sha256,
    pipeline::prepare_transaction_plan,
    protocol::{
        Cardinality, EffectBudget, OperationPayload, PreparedPlan, Request, TransactionCertificate,
        TransactionRequest, MAX_PLAN_BYTES, MAX_PLAN_OPERATIONS, MAX_REQUEST_BYTES,
        PROTOCOL_VERSION,
    },
    provider::text::{NewlinePolicy, TextOperation, TextProvider, TextReplacementIntent},
    workspace::Workspace,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_EDIT_DOCUMENT_FILES: usize = 64;
pub const MAX_EDIT_DOCUMENT_EDITS: usize = MAX_PLAN_OPERATIONS;
pub const MAX_EDIT_DOCUMENT_EDITS_PER_FILE: usize = crate::source_prepare::MAX_SOURCE_EDITS;

const FILE_PREFIX: &str = "*** File: ";
const SEARCH: &str = "<<<<<<< SEARCH";
const SEPARATOR: &str = "=======";
const REPLACE: &str = ">>>>>>> REPLACE";
const MARKERS: [&str; 3] = [SEARCH, SEPARATOR, REPLACE];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditDocumentFile {
    pub path: String,
    pub edits: Vec<TextReplacementIntent>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditDocumentDiagnostic {
    pub reason_code: String,
    pub line: usize,
    pub message: String,
}

#[derive(Debug)]
pub enum EditDocumentFailure {
    Invalid(EditDocumentDiagnostic),
    Refused(Box<TransactionCertificate>),
}

#[derive(Debug)]
pub struct EditDocumentPreparation {
    pub plan: PreparedPlan,
    pub file_count: usize,
    pub edit_count: usize,
    pub input_bytes: usize,
}

/// Parse a UTF-8 edit document. CRLF in the document itself is normalized to
/// LF; replacement bodies are then adapted to each uniformly-newline source by
/// the existing source-aware exact text provider.
pub fn parse(document: &str) -> Result<Vec<EditDocumentFile>, EditDocumentDiagnostic> {
    if document.len() > MAX_REQUEST_BYTES {
        return Err(diagnostic(
            1,
            format!(
                "edit document is {} bytes; the limit is {MAX_REQUEST_BYTES}",
                document.len()
            ),
        ));
    }
    if document.contains('\0') {
        return Err(diagnostic(
            1,
            "NUL bytes are not allowed in an edit document",
        ));
    }
    if document.starts_with('\u{feff}') {
        return Err(diagnostic(
            1,
            "remove the UTF-8 byte-order mark before the first file header",
        ));
    }

    let normalized = document.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split_terminator('\n').collect();
    if lines.is_empty() {
        return Err(diagnostic(1, "expected a `*** File: PATH` header"));
    }

    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut total_edits = 0usize;
    let mut index = 0usize;
    while index < lines.len() {
        let header_line = index + 1;
        let Some(path) = lines[index].strip_prefix(FILE_PREFIX) else {
            return Err(diagnostic(
                header_line,
                "expected a `*** File: WORKSPACE/RELATIVE/PATH` header",
            ));
        };
        if path.is_empty() || path.trim() != path {
            return Err(diagnostic(
                header_line,
                "file path must be non-empty and have no surrounding whitespace",
            ));
        }
        let path_key = if cfg!(windows) {
            path.replace('\\', "/").to_lowercase()
        } else {
            path.replace('\\', "/")
        };
        if !seen_paths.insert(path_key) {
            return Err(diagnostic(
                header_line,
                "each file may have one section; put its edits in that section in execution order",
            ));
        }
        index += 1;
        let mut edits = Vec::new();
        while index < lines.len() && !lines[index].starts_with(FILE_PREFIX) {
            let edit_line = index + 1;
            if lines[index] != SEARCH {
                return Err(diagnostic(
                    edit_line,
                    "expected `<<<<<<< SEARCH` or the next `*** File:` header",
                ));
            }
            index += 1;
            let old = read_body(&lines, &mut index, SEPARATOR, edit_line)?;
            if index >= lines.len() || lines[index] != SEPARATOR {
                return Err(diagnostic(edit_line, "search block is missing `=======`"));
            }
            index += 1;
            let new = read_body(&lines, &mut index, REPLACE, edit_line)?;
            if index >= lines.len() || lines[index] != REPLACE {
                return Err(diagnostic(
                    edit_line,
                    "replacement block is missing `>>>>>>> REPLACE`",
                ));
            }
            index += 1;
            edits.push(TextReplacementIntent { old, new });
            total_edits += 1;
            if edits.len() > MAX_EDIT_DOCUMENT_EDITS_PER_FILE {
                return Err(diagnostic(
                    edit_line,
                    format!(
                        "file section exceeds the per-file edit limit of {MAX_EDIT_DOCUMENT_EDITS_PER_FILE}"
                    ),
                ));
            }
            if total_edits > MAX_EDIT_DOCUMENT_EDITS {
                return Err(diagnostic(
                    edit_line,
                    format!("edit count exceeds the limit of {MAX_EDIT_DOCUMENT_EDITS}"),
                ));
            }
        }
        if edits.is_empty() {
            return Err(diagnostic(
                header_line,
                "file section must contain at least one SEARCH/REPLACE block",
            ));
        }
        files.push(EditDocumentFile {
            path: path.into(),
            edits,
        });
        if files.len() > MAX_EDIT_DOCUMENT_FILES {
            return Err(diagnostic(
                header_line,
                format!("file count exceeds the limit of {MAX_EDIT_DOCUMENT_FILES}"),
            ));
        }
    }
    Ok(files)
}

fn read_body(
    lines: &[&str],
    index: &mut usize,
    terminator: &str,
    edit_line: usize,
) -> Result<String, EditDocumentDiagnostic> {
    let mut body = String::new();
    while *index < lines.len() {
        let line = lines[*index];
        if line == terminator {
            // The line break required to put a delimiter on its own line is
            // syntax. An extra blank line represents a body ending in LF.
            if body.ends_with('\n') {
                body.pop();
            }
            return Ok(body);
        }
        if MARKERS.contains(&line) {
            return Err(diagnostic(
                *index + 1,
                format!(
                    "unescaped `{line}` inside a block; prefix a literal delimiter line with `\\`"
                ),
            ));
        }
        if let Some(unescaped) = unescape_delimiter(line) {
            body.push_str(&unescaped);
        } else {
            body.push_str(line);
        }
        body.push('\n');
        *index += 1;
    }
    Err(diagnostic(
        edit_line,
        format!("block is missing `{terminator}`"),
    ))
}

fn unescape_delimiter(line: &str) -> Option<String> {
    let slash_count = line
        .chars()
        .take_while(|character| *character == '\\')
        .count();
    if slash_count == 0 || !MARKERS.contains(&&line[slash_count..]) {
        return None;
    }
    Some(line[1..].to_owned())
}

fn workspace_scope(path: &str) -> String {
    std::path::Path::new(path)
        .components()
        .find_map(|component| match component {
            std::path::Component::Normal(component) => {
                Some(component.to_string_lossy().into_owned())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn path_scope_refusal(
    workspace: &Workspace,
    request: &Request,
    authority_root: &str,
) -> EditDocumentFailure {
    let mut scoped_request = request.clone();
    scoped_request.budget.allowed_path_prefixes = vec![authority_root.into()];
    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: format!("{}-scope-check", request.request_id),
        requests: vec![scoped_request],
        budget: EffectBudget {
            max_files: Some(1),
            allowed_path_prefixes: vec![authority_root.into()],
            ..EffectBudget::default()
        },
    };
    match prepare_transaction_plan(workspace, &transaction, Vec::new()) {
        Err(certificate) => EditDocumentFailure::Refused(Box::new(certificate)),
        Ok(_) => EditDocumentFailure::Invalid(diagnostic(
            1,
            "target resolves outside its validated file-section authority root",
        )),
    }
}

fn diagnostic(line: usize, message: impl Into<String>) -> EditDocumentDiagnostic {
    EditDocumentDiagnostic {
        reason_code: "INVALID_INPUT".into(),
        line,
        message: message.into(),
    }
}

/// Prepare all file edits as one guarded plan. Each file is represented by one
/// ordered exact replacement sequence; the existing transaction planner owns
/// authority checks, source hashes, budgets and rollback-aware apply semantics.
pub fn prepare(
    workspace: &Workspace,
    document: &str,
) -> Result<EditDocumentPreparation, EditDocumentFailure> {
    let files = parse(document).map_err(EditDocumentFailure::Invalid)?;
    let edit_count = files.iter().map(|file| file.edits.len()).sum::<usize>();
    let transaction_id = format!(
        "edit-document-{}",
        &compute_sha256(document.as_bytes())[..16]
    );
    let mut requests = Vec::with_capacity(files.len());
    let mut canonical_paths = HashSet::new();
    let mut allowed_paths = Vec::with_capacity(files.len());

    for (file_index, file) in files.iter().enumerate() {
        let path = match workspace
            .resolve_namespaced_path(&file.path, &crate::path::PathNamespace::Native)
        {
            Ok(path) => path,
            // Let the canonical planner produce the established path refusal
            // certificate instead of inventing a parallel path policy here.
            Err(_) => file.path.clone(),
        };
        let canonical_key = workspace
            .resolve_path(&path)
            .map(|resolved| resolved.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.clone());
        let duplicate_key = if cfg!(windows) {
            canonical_key.to_lowercase()
        } else {
            canonical_key
        };
        if !canonical_paths.insert(duplicate_key) {
            return Err(EditDocumentFailure::Invalid(diagnostic(
                1,
                format!(
                    "multiple file sections resolve to the same workspace target: {}",
                    file.path
                ),
            )));
        }
        let authority_root = workspace_scope(&path);
        let source = workspace.read_file(&path).ok();
        let expected_pre_hash = source.as_deref().map(compute_sha256);
        let changed_bytes = source
            .as_deref()
            .and_then(|source| {
                TextProvider::plan(
                    source,
                    &TextOperation::ReplaceSequence {
                        edits: file.edits.clone(),
                        newline_policy: NewlinePolicy::Source,
                    },
                    &Cardinality::ExactlyOne,
                )
                .ok()
            })
            .map(|edits| crate::pipeline::changed_byte_effect(&edits))
            .unwrap_or_default();
        let changed_lines = file
            .edits
            .iter()
            .map(|edit| {
                edit.old.bytes().filter(|byte| *byte == b'\n').count()
                    + edit.new.bytes().filter(|byte| *byte == b'\n').count()
                    + 2
            })
            .sum::<usize>();
        let per_file_budget = EffectBudget {
            max_files: Some(1),
            max_matches: Some(changed_lines.max(file.edits.len())),
            max_changed_regions: Some(changed_lines.max(file.edits.len())),
            max_changed_lines: Some(changed_lines),
            max_changed_bytes: Some(changed_bytes),
            allowed_path_prefixes: vec![path.clone()],
        };
        let request = Request {
            version: PROTOCOL_VERSION.into(),
            request_id: format!("{transaction_id}-{}", file_index + 1),
            allow_generated: false,
            file_path: path.clone(),
            namespace: crate::path::PathNamespace::Native,
            expected_pre_hash,
            region_guard: None,
            candidate_guard: None,
            cardinality: Cardinality::ExactlyOne,
            budget: per_file_budget,
            operation: OperationPayload::Text(TextOperation::ReplaceSequence {
                edits: file.edits.clone(),
                newline_policy: NewlinePolicy::Source,
            }),
        };
        if !matches!(
            workspace.is_within_allowed_prefix(&path, &authority_root),
            Ok(true)
        ) {
            return Err(path_scope_refusal(workspace, &request, &authority_root));
        }
        allowed_paths.push(path);
        requests.push(request);
    }

    let changed_bytes = requests
        .iter()
        .map(|request| request.budget.max_changed_bytes.unwrap_or_default())
        .sum::<usize>();
    let changed_lines = requests
        .iter()
        .map(|request| request.budget.max_changed_lines.unwrap_or_default())
        .sum::<usize>();
    let transaction = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: transaction_id.clone(),
        requests,
        budget: EffectBudget {
            max_files: Some(files.len()),
            max_matches: Some(changed_lines.max(edit_count)),
            max_changed_regions: Some(changed_lines.max(edit_count)),
            max_changed_lines: Some(changed_lines),
            max_changed_bytes: Some(changed_bytes),
            allowed_path_prefixes: allowed_paths,
        },
    };
    let plan = prepare_transaction_plan(workspace, &transaction, Vec::new())
        .map_err(|certificate| EditDocumentFailure::Refused(Box::new(certificate)))?;
    let serialized_plan_bytes = serde_json::to_vec(&plan)
        .map_err(|error| EditDocumentFailure::Invalid(diagnostic(1, error.to_string())))?
        .len();
    if serialized_plan_bytes > MAX_PLAN_BYTES {
        return Err(EditDocumentFailure::Invalid(EditDocumentDiagnostic {
            reason_code: "RESOURCE_LIMIT_EXCEEDED".into(),
            line: 1,
            message: format!(
                "prepared plan is {serialized_plan_bytes} bytes; the limit is {MAX_PLAN_BYTES}"
            ),
        }));
    }
    Ok(EditDocumentPreparation {
        plan,
        file_count: files.len(),
        edit_count,
        input_bytes: document.len(),
    })
}
