//! Structural editing attachment powered by ast-grep.
//!
//! Provides AST-pattern search and rewrite proposals translated directly into
//! Threadmoth's guarded transaction and desired-state mutation pipeline.

use crate::engine::compute_sha256;
use crate::pipeline;
use crate::protocol::{
    Cardinality, DesiredStateOperation, EffectBudget, OperationPayload, Request,
    TransactionCertificate, TransactionRequest, PROTOCOL_VERSION,
};
use crate::toolbelt::AttachmentInfo;
use crate::workspace::{Workspace, WorkspaceError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "bash",
    "c",
    "cpp",
    "csharp",
    "css",
    "elixir",
    "go",
    "haskell",
    "html",
    "java",
    "javascript",
    "json",
    "kotlin",
    "lua",
    "php",
    "python",
    "ruby",
    "rust",
    "scala",
    "swift",
    "typescript",
    "tsx",
    "yaml",
];

#[derive(Error, Debug)]
pub enum StructuralError {
    #[error("ast-grep executable not found on PATH. Install via 'cargo install ast-grep --locked' or 'npm install -g @ast-grep/cli'")]
    AstGrepNotFound,

    #[error("failed to execute ast-grep: {0}")]
    Execution(std::io::Error),

    #[error("ast-grep reported an error (exit code {code:?}): {message}")]
    AstGrepReported { code: Option<i32>, message: String },

    #[error("requested path is outside workspace root: {0}")]
    WorkspaceEscape(String),

    #[error("source file {file} could not be read: {error}")]
    SourceRead { file: String, error: WorkspaceError },

    #[error("invalid byte range [{start}..{end}] for file {file} of length {len}")]
    InvalidByteRange {
        file: String,
        start: usize,
        end: usize,
        len: usize,
    },

    #[error("overlapping structural match ranges in {file}: [{start1}..{end1}] overlaps with [{start2}..{end2}]")]
    OverlappingMatches {
        file: String,
        start1: usize,
        end1: usize,
        start2: usize,
        end2: usize,
    },

    #[error("observed source bytes in {file} at [{start}..{end}] do not match ast-grep match text (expected '{expected}', observed '{observed}')")]
    MismatchedSourceText {
        file: String,
        start: usize,
        end: usize,
        expected: String,
        observed: String,
    },

    #[error("failed to prepare plan: {0:?}")]
    PlanPreparation(Box<TransactionCertificate>),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuralRequest {
    /// AST pattern to search (e.g. 'foo($A, $B)')
    pub pattern: String,

    /// Replacement template with matched metavariables (e.g. 'bar($B, $A)')
    pub rewrite: String,

    /// Language of the syntax tree (e.g. 'python', 'rust'). Inferred from file extension if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,

    /// Target files or directories relative to the workspace root
    #[serde(default)]
    pub paths: Vec<String>,

    /// Maximum number of files permitted to be modified
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_files: Option<usize>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StructuralOutcome {
    NoChange {
        message: String,
    },
    PlanPrepared {
        plan_path: String,
        transaction_id: String,
        files_affected: usize,
    },
    Preview {
        certificate: TransactionCertificate,
    },
    Applied {
        certificate: TransactionCertificate,
    },
}

#[derive(Deserialize, Debug)]
struct AstGrepJsonRecord {
    text: String,
    range: AstGrepRange,
    file: String,
    replacement: Option<String>,
    #[serde(rename = "replacementOffsets")]
    replacement_offsets: Option<AstGrepOffsets>,
}

#[derive(Deserialize, Debug)]
struct AstGrepRange {
    #[serde(rename = "byteOffset")]
    byte_offset: AstGrepOffsets,
}

#[derive(Deserialize, Debug, Clone, Copy)]
struct AstGrepOffsets {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct MatchProposal {
    start: usize,
    end: usize,
    match_start: usize,
    match_end: usize,
    matched_text: String,
    replacement: String,
}

/// Check if ast-grep is installed and query its version.
pub fn detect_ast_grep() -> Option<String> {
    let mut cmd = Command::new("ast-grep");
    cmd.arg("--version");
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Typical output: "ast-grep 0.45.3"
    let version = stdout
        .lines()
        .next()?
        .trim()
        .strip_prefix("ast-grep ")
        .unwrap_or(stdout.trim())
        .to_string();
    Some(version)
}

/// Infer ast-grep language from file extension.
pub fn infer_language_from_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some("rust"),
        Some("py") => Some("python"),
        Some("js" | "mjs" | "cjs") => Some("javascript"),
        Some("ts" | "mts" | "cts") => Some("typescript"),
        Some("tsx") => Some("tsx"),
        Some("jsx") => Some("javascript"),
        Some("go") => Some("go"),
        Some("c" | "h") => Some("c"),
        Some("cpp" | "cc" | "cxx" | "hpp") => Some("cpp"),
        Some("cs") => Some("csharp"),
        Some("java") => Some("java"),
        Some("json") => Some("json"),
        Some("yaml" | "yml") => Some("yaml"),
        Some("html" | "htm") => Some("html"),
        Some("css") => Some("css"),
        Some("sh" | "bash") => Some("bash"),
        Some("php") => Some("php"),
        Some("rb") => Some("ruby"),
        Some("kt" | "kts") => Some("kotlin"),
        Some("swift") => Some("swift"),
        Some("lua") => Some("lua"),
        Some("scala") => Some("scala"),
        _ => None,
    }
}

/// Advertise ast-grep structural attachment info.
pub fn discover(_workspace: &Workspace) -> AttachmentInfo {
    let version = detect_ast_grep();
    let available = version.is_some();
    AttachmentInfo {
        name: "structural".into(),
        engine: "ast-grep".into(),
        version,
        available,
        description: "Structural AST pattern search and rewrite for AI coding agents".into(),
        supported_languages: SUPPORTED_LANGUAGES.iter().map(|&s| s.into()).collect(),
        prerequisites: "ast-grep CLI on PATH (install via 'cargo install ast-grep --locked' or 'npm install -g @ast-grep/cli')".into(),
        limitations: vec![
            "Proposals are extracted against observed source snapshots without modifying live files".into(),
            "Overlapping match ranges within the same file are refused for safety".into(),
            "Native custom parser execution is disabled (--custom-languages ignore)".into(),
            "Mutations are verified and committed through Threadmoth's guarded rollback pipeline".into(),
        ],
    }
}

/// Run ast-grep in proposal mode and collect validated match proposals grouped by workspace file.
fn collect_proposals(
    workspace: &Workspace,
    request: &StructuralRequest,
) -> Result<BTreeMap<String, Vec<MatchProposal>>, StructuralError> {
    let version_str = detect_ast_grep().ok_or(StructuralError::AstGrepNotFound)?;
    let parsed_version = semver::Version::parse(&version_str).ok();

    let mut cmd = Command::new("ast-grep");
    cmd.arg("run");
    cmd.arg("--pattern").arg(&request.pattern);
    cmd.arg("--rewrite").arg(&request.rewrite);

    if let Some(lang) = &request.lang {
        cmd.arg("--lang").arg(lang);
    }

    cmd.arg("--json=stream");

    // Security addendum: disable untrusted native custom parsers if version >= 0.50
    if let Some(v) = parsed_version {
        if v >= semver::Version::new(0, 50, 0) {
            cmd.arg("--custom-languages").arg("ignore");
        }
    }

    cmd.current_dir(workspace.root());

    // Resolve target paths
    let paths = if request.paths.is_empty() {
        vec![".".to_string()]
    } else {
        request.paths.clone()
    };

    for target in &paths {
        let abs_path = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            workspace.root().join(target)
        };
        // Normalize and verify workspace containment
        let rel = match abs_path.strip_prefix(workspace.root()) {
            Ok(r) => r,
            Err(_) => return Err(StructuralError::WorkspaceEscape(target.clone())),
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            cmd.arg(".");
        } else {
            cmd.arg(&rel_str);
        }
    }

    let output = cmd.output().map_err(StructuralError::Execution)?;
    if !output.status.success() && output.stdout.is_empty() {
        // ast-grep exits with code 1 when no matches are found (standard grep convention)
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(BTreeMap::new());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(StructuralError::AstGrepReported {
            code: output.status.code(),
            message: stderr,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut raw_by_file: BTreeMap<String, Vec<MatchProposal>> = BTreeMap::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: AstGrepJsonRecord = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => continue, // ignore non-record lines (e.g. diagnostics)
        };

        // Determine workspace relative path
        let record_path = Path::new(&record.file);
        let rel_path = if record_path.is_absolute() {
            match record_path.strip_prefix(workspace.root()) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => return Err(StructuralError::WorkspaceEscape(record.file)),
            }
        } else {
            record.file.replace('\\', "/")
        };

        let (start, end) = if let Some(offsets) = record.replacement_offsets {
            (offsets.start, offsets.end)
        } else {
            (record.range.byte_offset.start, record.range.byte_offset.end)
        };

        let match_start = record.range.byte_offset.start;
        let match_end = record.range.byte_offset.end;
        let replacement = record.replacement.unwrap_or_default();
        raw_by_file
            .entry(rel_path)
            .or_default()
            .push(MatchProposal {
                start,
                end,
                match_start,
                match_end,
                matched_text: record.text,
                replacement,
            });
    }

    // Sort and validate against observed source bytes
    let mut validated_by_file: BTreeMap<String, Vec<MatchProposal>> = BTreeMap::new();

    for (file, mut matches) in raw_by_file {
        let original = workspace
            .read_file(&file)
            .map_err(|e| StructuralError::SourceRead {
                file: file.clone(),
                error: e,
            })?;

        // Sort by start offset
        matches.sort_by_key(|m| m.start);

        // Validate byte bounds, exact match text, and non-overlapping invariant
        for i in 0..matches.len() {
            let m = &matches[i];
            if m.start > original.len() || m.end > original.len() || m.start > m.end {
                return Err(StructuralError::InvalidByteRange {
                    file: file.clone(),
                    start: m.start,
                    end: m.end,
                    len: original.len(),
                });
            }
            if m.match_start <= original.len()
                && m.match_end <= original.len()
                && m.match_start <= m.match_end
            {
                let observed = &original[m.match_start..m.match_end];
                if observed != m.matched_text.as_bytes() {
                    return Err(StructuralError::MismatchedSourceText {
                        file: file.clone(),
                        start: m.match_start,
                        end: m.match_end,
                        expected: m.matched_text.clone(),
                        observed: String::from_utf8_lossy(observed).to_string(),
                    });
                }
            }
            if i + 1 < matches.len() {
                let next = &matches[i + 1];
                if m.end > next.start {
                    return Err(StructuralError::OverlappingMatches {
                        file: file.clone(),
                        start1: m.start,
                        end1: m.end,
                        start2: next.start,
                        end2: next.end,
                    });
                }
            }
        }

        validated_by_file.insert(file, matches);
    }

    Ok(validated_by_file)
}

/// Build a Threadmoth TransactionRequest from validated match proposals.
pub fn build_transaction(
    workspace: &Workspace,
    request: &StructuralRequest,
) -> Result<Option<TransactionRequest>, StructuralError> {
    let proposals_by_file = collect_proposals(workspace, request)?;
    if proposals_by_file.is_empty() {
        return Ok(None);
    }

    let mut requests = Vec::new();

    for (file, matches) in proposals_by_file {
        let original = workspace
            .read_file(&file)
            .map_err(|e| StructuralError::SourceRead {
                file: file.clone(),
                error: e,
            })?;

        // Construct prospective candidate bytes
        let mut candidate = Vec::with_capacity(original.len());
        let mut cursor = 0;
        for m in matches {
            candidate.extend_from_slice(&original[cursor..m.start]);
            candidate.extend_from_slice(m.replacement.as_bytes());
            cursor = m.end;
        }
        candidate.extend_from_slice(&original[cursor..]);

        if candidate == original {
            // No-op for this file
            continue;
        }

        let pre_hash = compute_sha256(&original);
        let req_id = format!("structural-{}", safe_identifier(&file));

        requests.push(Request {
            version: PROTOCOL_VERSION.into(),
            request_id: req_id,
            allow_generated: false,
            file_path: file,
            namespace: Default::default(),
            expected_pre_hash: Some(pre_hash),
            region_guard: None,
            candidate_guard: None,
            cardinality: Cardinality::ExactlyOne,
            budget: EffectBudget {
                max_files: request.max_files,
                ..Default::default()
            },
            operation: OperationPayload::DesiredState(DesiredStateOperation::Replace {
                desired_bytes: candidate,
            }),
        });
    }

    if requests.is_empty() {
        return Ok(None);
    }

    let tx = TransactionRequest {
        version: PROTOCOL_VERSION.into(),
        transaction_id: format!("tx-structural-{}", safe_identifier(&request.pattern)),
        requests,
        budget: EffectBudget {
            max_files: request.max_files,
            ..Default::default()
        },
    };

    Ok(Some(tx))
}

/// Execute a structural transformation: either preparing a plan, previewing diff, or applying atomically.
pub fn execute_structural(
    workspace: &Workspace,
    request: &StructuralRequest,
    plan_output: Option<&Path>,
    preview_only: bool,
) -> Result<StructuralOutcome, StructuralError> {
    let tx = match build_transaction(workspace, request)? {
        Some(t) => t,
        None => {
            return Ok(StructuralOutcome::NoChange {
                message: "No matching structural patterns found".into(),
            });
        }
    };

    if let Some(output_path) = plan_output {
        let prepared_plan = pipeline::prepare_transaction_plan(workspace, &tx, vec![])
            .map_err(|cert| StructuralError::PlanPreparation(Box::new(cert)))?;

        let bytes = serde_json::to_vec_pretty(&prepared_plan)?;
        let abs_output = if output_path.is_absolute() {
            output_path.to_path_buf()
        } else {
            workspace.root().join(output_path)
        };
        std::fs::write(&abs_output, bytes)?;

        return Ok(StructuralOutcome::PlanPrepared {
            plan_path: output_path.to_string_lossy().to_string(),
            transaction_id: tx.transaction_id,
            files_affected: tx.requests.len(),
        });
    }

    if preview_only {
        let certificate = pipeline::execute_transaction(workspace, &tx, true);
        return Ok(StructuralOutcome::Preview { certificate });
    }

    let certificate = pipeline::execute_transaction(workspace, &tx, false);
    Ok(StructuralOutcome::Applied { certificate })
}

fn safe_identifier(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push('_');
        }
    }
    if out.len() > 40 {
        out.truncate(40);
    }
    out
}
