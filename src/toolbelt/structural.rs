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
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
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

pub const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_STDOUT_BYTES: usize = 16 * 1024 * 1024; // 16 MB
pub const MAX_STDERR_BYTES: usize = 1024 * 1024; // 1 MB
pub const MAX_TOTAL_MATCHES: usize = 5000;
pub const MAX_FILES_AFFECTED: usize = 256;
pub const MAX_PRE_SCAN_FILES: usize = 10_000;
pub const MAX_TRAVERSAL_DEPTH: usize = 32;

#[cfg(feature = "test-fault-injection")]
pub mod fault_injection {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static SPAWN_HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
        static FORCE_UNAVAILABLE: Cell<bool> = const { Cell::new(false) };
    }

    pub fn set_on_subprocess_spawned<F: Fn() + 'static>(f: F) {
        SPAWN_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }

    pub fn clear_on_subprocess_spawned() {
        SPAWN_HOOK.with(|h| *h.borrow_mut() = None);
    }

    pub fn run_spawn_hook() {
        SPAWN_HOOK.with(|h| {
            if let Some(hook) = h.borrow().as_ref() {
                hook();
            }
        });
    }

    pub fn is_ast_grep_unavailable_forced() -> bool {
        FORCE_UNAVAILABLE.with(|c| c.get())
    }

    pub fn with_forced_unavailable<R, F: FnOnce() -> R>(f: F) -> R {
        FORCE_UNAVAILABLE.with(|c| c.set(true));
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FORCE_UNAVAILABLE.with(|c| c.set(false));
            }
        }
        let _reset = Reset;
        f()
    }
}

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

    #[error("security violation: {0}")]
    SecurityViolation(String),

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

    #[error("unsupported language '{0}'; supported languages: {}", SUPPORTED_LANGUAGES.join(", "))]
    UnsupportedLanguage(String),

    #[error("unverifiable or invalid ast-grep version: '{0}'")]
    UnverifiableAstGrepVersion(String),

    #[error("ast-grep execution timed out after {0} seconds")]
    SubprocessTimeout(u64),

    #[error("ast-grep output limit exceeded for {stream} ({limit} bytes)")]
    OutputLimitExceeded { stream: &'static str, limit: usize },

    #[error("ast-grep produced non-utf8 {stream} at byte {valid_up_to}")]
    NonUtf8Output {
        stream: &'static str,
        valid_up_to: usize,
    },

    #[error("concurrent modification detected for {file}: {message}")]
    ConcurrentModification { file: String, message: String },

    #[error("limit exceeded for {dimension}: limit is {limit}, observed {actual}")]
    LimitExceeded {
        dimension: &'static str,
        limit: usize,
        actual: usize,
    },

    #[error("malformed JSON record from ast-grep on line {line}: {message} (raw: '{raw}')")]
    MalformedRecord {
        line: usize,
        message: String,
        raw: String,
    },

    #[error("missing replacement in rewrite match record for file {file}")]
    MissingReplacement { file: String },

    #[error("stale source detected for {file}: file drifted between ast-grep observation ({expected}) and transaction construction ({actual})")]
    StaleSource {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("plan output path collides with target source file: {0}")]
    PlanOutputCollision(String),

    #[error("plan output file already exists: '{0}' (Threadmoth will not overwrite an existing file with a plan)")]
    PlanOutputAlreadyExists(String),

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
pub struct AstGrepJsonRecord {
    pub text: String,
    pub range: AstGrepRange,
    pub file: String,
    pub replacement: Option<String>,
    #[serde(rename = "replacementOffsets")]
    pub replacement_offsets: Option<AstGrepOffsets>,
}

#[derive(Deserialize, Debug)]
pub struct AstGrepRange {
    #[serde(rename = "byteOffset")]
    pub byte_offset: AstGrepOffsets,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct AstGrepOffsets {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct MatchProposal {
    pub start: usize,
    pub end: usize,
    pub match_start: usize,
    pub match_end: usize,
    pub matched_text: String,
    pub replacement: String,
}

#[derive(Debug, Clone)]
pub struct FileProposalGroup {
    pub observed_hash: String,
    pub matches: Vec<MatchProposal>,
}

/// Check if ast-grep is installed and query its version.
pub fn detect_ast_grep() -> Option<String> {
    #[cfg(feature = "test-fault-injection")]
    if fault_injection::is_ast_grep_unavailable_forced() {
        return None;
    }

    let mut cmd = Command::new("ast-grep");
    cmd.arg("--version");
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
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

fn normalize_relative_path_string(rel: &str) -> String {
    let mut p = rel.replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    if p.is_empty() {
        ".".to_string()
    } else {
        p
    }
}

/// Validate and canonicalise a target path requested by the agent before invoking ast-grep.
pub fn validate_and_canonicalize_target_path(
    workspace: &Workspace,
    target: &str,
) -> Result<String, StructuralError> {
    if target.is_empty() || target == "." {
        return Ok(".".to_string());
    }

    // Reject raw flag injection directly
    if target.starts_with("--") && !target.contains('/') && !target.contains('\\') {
        let trimmed = target.trim_start_matches('-');
        if trimmed.is_empty() {
            return Err(StructuralError::SecurityViolation(
                "invalid bare option separator in paths".into(),
            ));
        }
    }

    let rel = workspace
        .resolve_namespaced_path(target, &Default::default())
        .map_err(|e| match e {
            WorkspaceError::Traversal(t) => {
                StructuralError::WorkspaceEscape(format!("path traversal not allowed: {t}"))
            }
            WorkspaceError::SymlinkEscape(s) => {
                StructuralError::SecurityViolation(format!("escaping symlink: {s}"))
            }
            WorkspaceError::WorkspaceRootMismatch { target, .. } => {
                StructuralError::WorkspaceEscape(format!(
                    "requested path is outside workspace root: {target}"
                ))
            }
            WorkspaceError::UnmappablePath(p) => {
                StructuralError::SecurityViolation(format!("unmappable path: {p}"))
            }
            other => StructuralError::SourceRead {
                file: target.to_string(),
                error: other,
            },
        })?;
    Ok(normalize_relative_path_string(&rel))
}

/// Validate returned file paths from ast-grep against workspace boundaries.
pub fn validate_returned_file_path(
    workspace: &Workspace,
    raw_file: &str,
) -> Result<String, StructuralError> {
    let rel = workspace
        .resolve_namespaced_path(raw_file, &Default::default())
        .map_err(|e| match e {
            WorkspaceError::Traversal(t) => {
                StructuralError::WorkspaceEscape(format!("returned file traversal: {t}"))
            }
            WorkspaceError::SymlinkEscape(s) => StructuralError::SecurityViolation(format!(
                "returned file escapes via symlink: {s}"
            )),
            WorkspaceError::WorkspaceRootMismatch { target, .. } => {
                StructuralError::WorkspaceEscape(format!(
                    "returned path is outside workspace root: {target}"
                ))
            }
            other => StructuralError::SourceRead {
                file: raw_file.to_string(),
                error: other,
            },
        })?;
    Ok(normalize_relative_path_string(&rel))
}

/// Collect pre-scan source hashes across target paths to detect concurrent modifications during scanning.
pub fn collect_pre_scan_hashes(
    workspace: &Workspace,
    paths: &[String],
    lang: Option<&str>,
) -> Result<BTreeMap<String, String>, StructuralError> {
    let mut hashes = BTreeMap::new();
    let mut visited_dirs = HashSet::new();

    for target in paths {
        let canonical = validate_and_canonicalize_target_path(workspace, target)?;
        let resolved = if canonical == "." {
            workspace.root().to_path_buf()
        } else {
            workspace.root().join(&canonical)
        };

        let sym_meta =
            std::fs::symlink_metadata(&resolved).map_err(|e| StructuralError::SourceRead {
                file: canonical.clone(),
                error: WorkspaceError::Io(e),
            })?;

        if sym_meta.file_type().is_symlink() {
            return Err(StructuralError::SecurityViolation(format!(
                "target path '{canonical}' is a symlink or junction, which is not permitted"
            )));
        }

        if resolved.is_file() {
            let bytes =
                workspace
                    .read_file(&canonical)
                    .map_err(|e| StructuralError::SourceRead {
                        file: canonical.clone(),
                        error: e,
                    })?;
            hashes.insert(canonical, compute_sha256(&bytes));
        } else if resolved.is_dir() {
            collect_dir_hashes(
                workspace,
                &resolved,
                lang,
                &mut hashes,
                &mut visited_dirs,
                0,
            )?;
        }
    }
    Ok(hashes)
}

fn collect_dir_hashes(
    workspace: &Workspace,
    dir: &Path,
    lang: Option<&str>,
    hashes: &mut BTreeMap<String, String>,
    visited_dirs: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<(), StructuralError> {
    if depth > MAX_TRAVERSAL_DEPTH {
        return Err(StructuralError::LimitExceeded {
            dimension: "traversal_depth",
            limit: MAX_TRAVERSAL_DEPTH,
            actual: depth,
        });
    }

    let canonical_dir = dir
        .canonicalize()
        .map_err(|e| StructuralError::SourceRead {
            file: dir
                .strip_prefix(workspace.root())
                .unwrap_or(dir)
                .to_string_lossy()
                .to_string(),
            error: WorkspaceError::Io(e),
        })?;

    if !visited_dirs.insert(canonical_dir) {
        // Directory already visited; skip cycle
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|e| StructuralError::SourceRead {
        file: dir
            .strip_prefix(workspace.root())
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string(),
        error: WorkspaceError::Io(e),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| StructuralError::SourceRead {
            file: dir
                .strip_prefix(workspace.root())
                .unwrap_or(dir)
                .to_string_lossy()
                .to_string(),
            error: WorkspaceError::Io(e),
        })?;

        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        let is_symlink = match entry.file_type() {
            Ok(ft) => ft.is_symlink(),
            Err(e) => {
                return Err(StructuralError::SourceRead {
                    file: path
                        .strip_prefix(workspace.root())
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string(),
                    error: WorkspaceError::Io(e),
                });
            }
        };

        if is_symlink {
            // Do not follow directory symlinks or junctions
            continue;
        }

        let meta = entry.metadata().map_err(|e| StructuralError::SourceRead {
            file: path
                .strip_prefix(workspace.root())
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string(),
            error: WorkspaceError::Io(e),
        })?;

        if meta.is_dir() {
            if name_str.starts_with('.') || name_str == "target" || name_str == "node_modules" {
                continue;
            }
            collect_dir_hashes(workspace, &path, lang, hashes, visited_dirs, depth + 1)?;
            continue;
        }

        if !meta.is_file() {
            continue;
        }

        let file_lang = infer_language_from_path(&path);
        let matches_lang = match (lang, file_lang) {
            (Some(l), Some(fl)) => l == fl,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => false,
        };

        if !matches_lang {
            continue;
        }

        if hashes.len() >= MAX_PRE_SCAN_FILES {
            return Err(StructuralError::LimitExceeded {
                dimension: "max_pre_scan_files",
                limit: MAX_PRE_SCAN_FILES,
                actual: hashes.len(),
            });
        }

        let rel = path.strip_prefix(workspace.root()).map_err(|_| {
            StructuralError::WorkspaceEscape(format!(
                "path outside workspace root: {}",
                path.display()
            ))
        })?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let bytes = workspace
            .read_file(&rel_str)
            .map_err(|e| StructuralError::SourceRead {
                file: rel_str.clone(),
                error: e,
            })?;
        hashes.insert(rel_str, compute_sha256(&bytes));
    }
    Ok(())
}

/// Run a subprocess with execution timeout and strict stdout/stderr output caps.
fn run_subprocess_bounded(
    mut cmd: Command,
    timeout: Duration,
    max_stdout: usize,
    max_stderr: usize,
) -> Result<std::process::Output, StructuralError> {
    use std::io::Read;

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(StructuralError::Execution)?;

    #[cfg(feature = "test-fault-injection")]
    fault_injection::run_spawn_hook();

    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");

    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();

    let h_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut exceeded = false;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > max_stdout {
                        exceeded = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
        let _ = tx_out.send((buf, exceeded));
    });

    let h_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut exceeded = false;
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > max_stderr {
                        exceeded = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
        let _ = tx_err.send((buf, exceeded));
    });

    let start = std::time::Instant::now();
    let mut status = None;
    while start.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(StructuralError::Execution(e));
            }
        }
    }

    let status = match status {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(StructuralError::SubprocessTimeout(timeout.as_secs()));
        }
    };

    let (out_bytes, out_exceeded) = rx_out.recv().unwrap_or_default();
    let (err_bytes, err_exceeded) = rx_err.recv().unwrap_or_default();
    let _ = h_out.join();
    let _ = h_err.join();

    if out_exceeded {
        return Err(StructuralError::OutputLimitExceeded {
            stream: "stdout",
            limit: max_stdout,
        });
    }

    if err_exceeded {
        return Err(StructuralError::OutputLimitExceeded {
            stream: "stderr",
            limit: max_stderr,
        });
    }

    Ok(std::process::Output {
        status,
        stdout: out_bytes,
        stderr: err_bytes,
    })
}

/// Parse and validate ast-grep stream records into validated proposals bound to source snapshots.
pub fn parse_ast_grep_output(
    workspace: &Workspace,
    stdout: &str,
    pre_scan_hashes: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, FileProposalGroup>, StructuralError> {
    let mut raw_by_file: BTreeMap<String, Vec<MatchProposal>> = BTreeMap::new();
    let mut total_matches = 0;

    for (line_idx, line) in stdout.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: AstGrepJsonRecord =
            serde_json::from_str(trimmed).map_err(|e| StructuralError::MalformedRecord {
                line: line_idx + 1,
                message: e.to_string(),
                raw: if trimmed.len() > 100 {
                    format!("{}...", &trimmed[..100])
                } else {
                    trimmed.to_string()
                },
            })?;

        total_matches += 1;
        if total_matches > MAX_TOTAL_MATCHES {
            return Err(StructuralError::LimitExceeded {
                dimension: "matches",
                limit: MAX_TOTAL_MATCHES,
                actual: total_matches,
            });
        }

        let rel_path = validate_returned_file_path(workspace, &record.file)?;

        let replacement =
            record
                .replacement
                .ok_or_else(|| StructuralError::MissingReplacement {
                    file: rel_path.clone(),
                })?;

        let (start, end) = if let Some(offsets) = record.replacement_offsets {
            (offsets.start, offsets.end)
        } else {
            (record.range.byte_offset.start, record.range.byte_offset.end)
        };

        let match_start = record.range.byte_offset.start;
        let match_end = record.range.byte_offset.end;

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

    if raw_by_file.len() > MAX_FILES_AFFECTED {
        return Err(StructuralError::LimitExceeded {
            dimension: "max_files",
            limit: MAX_FILES_AFFECTED,
            actual: raw_by_file.len(),
        });
    }

    // Sort and validate against observed source bytes
    let mut validated_by_file: BTreeMap<String, FileProposalGroup> = BTreeMap::new();

    for (file, mut matches) in raw_by_file {
        let original = workspace
            .read_file(&file)
            .map_err(|e| StructuralError::SourceRead {
                file: file.clone(),
                error: e,
            })?;

        let post_hash = compute_sha256(&original);

        // Snapshot validation: verify file hasn't been created or modified concurrently during the scan
        if let Some(pre_map) = pre_scan_hashes {
            match pre_map.get(&file) {
                Some(pre_hash) => {
                    if *pre_hash != post_hash {
                        return Err(StructuralError::ConcurrentModification {
                            file: file.clone(),
                            message: format!(
                                "file was modified concurrently during ast-grep scan (pre-scan: {pre_hash}, post-scan: {post_hash})"
                            ),
                        });
                    }
                }
                None => {
                    return Err(StructuralError::ConcurrentModification {
                        file: file.clone(),
                        message: "file was created concurrently during ast-grep scan".to_string(),
                    });
                }
            }
        }

        // Sort by start offset
        matches.sort_by_key(|m| m.start);

        // Validate byte bounds, exact match text, UTF-8 char boundaries, and non-overlapping invariant
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

            // UTF-8 char boundary check if original is valid UTF-8
            if let Ok(s) = std::str::from_utf8(&original) {
                if !s.is_char_boundary(m.start) || !s.is_char_boundary(m.end) {
                    return Err(StructuralError::InvalidByteRange {
                        file: file.clone(),
                        start: m.start,
                        end: m.end,
                        len: original.len(),
                    });
                }
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
            } else {
                return Err(StructuralError::InvalidByteRange {
                    file: file.clone(),
                    start: m.match_start,
                    end: m.match_end,
                    len: original.len(),
                });
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

        validated_by_file.insert(
            file,
            FileProposalGroup {
                observed_hash: post_hash,
                matches,
            },
        );
    }

    Ok(validated_by_file)
}

/// Run ast-grep in proposal mode and collect validated match proposals grouped by workspace file.
fn collect_proposals(
    workspace: &Workspace,
    request: &StructuralRequest,
) -> Result<BTreeMap<String, FileProposalGroup>, StructuralError> {
    // 1. Language validation
    if let Some(lang) = &request.lang {
        if !SUPPORTED_LANGUAGES.contains(&lang.as_str()) {
            return Err(StructuralError::UnsupportedLanguage(lang.clone()));
        }
    }

    // 2. Version detection & capability checks
    let version_str = detect_ast_grep().ok_or(StructuralError::AstGrepNotFound)?;
    let parsed_version = semver::Version::parse(&version_str)
        .map_err(|_| StructuralError::UnverifiableAstGrepVersion(version_str.clone()))?;

    let mut cmd = Command::new("ast-grep");
    cmd.arg("run");
    cmd.arg("--pattern").arg(&request.pattern);
    cmd.arg("--rewrite").arg(&request.rewrite);

    if let Some(lang) = &request.lang {
        cmd.arg("--lang").arg(lang);
    }

    cmd.arg("--json=stream");

    // Security addendum: disable untrusted native custom parsers if version >= 0.50
    if parsed_version >= semver::Version::new(0, 50, 0) {
        cmd.arg("--custom-languages").arg("ignore");
    }

    cmd.current_dir(workspace.root());

    // 3. Resolve and validate target paths
    let paths = if request.paths.is_empty() {
        vec![".".to_string()]
    } else {
        request.paths.clone()
    };

    // Pre-scan source hashing to close any gap between ast-grep's observation and Threadmoth's capture
    let pre_scan_hashes = collect_pre_scan_hashes(workspace, &paths, request.lang.as_deref())?;

    // Option-argument boundary separator: ensure target paths cannot be parsed as flags
    cmd.arg("--");

    for target in &paths {
        let canonical_target = validate_and_canonicalize_target_path(workspace, target)?;
        let safe_target = if canonical_target.starts_with('-') {
            format!("./{canonical_target}")
        } else {
            canonical_target
        };
        cmd.arg(&safe_target);
    }

    // 4. Execute with bounded timeout and size caps
    let output =
        run_subprocess_bounded(cmd, SUBPROCESS_TIMEOUT, MAX_STDOUT_BYTES, MAX_STDERR_BYTES)?;

    if !output.status.success() {
        // ast-grep exits with code 1 when no matches are found (standard grep convention)
        if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(BTreeMap::new());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(StructuralError::AstGrepReported {
            code: output.status.code(),
            message: stderr,
        });
    }

    // Strict UTF-8 decoding for structured stdout
    let stdout_str =
        String::from_utf8(output.stdout).map_err(|e| StructuralError::NonUtf8Output {
            stream: "stdout",
            valid_up_to: e.utf8_error().valid_up_to(),
        })?;

    parse_ast_grep_output(workspace, &stdout_str, Some(&pre_scan_hashes))
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

    if let Some(max_f) = request.max_files {
        if proposals_by_file.len() > max_f {
            return Err(StructuralError::LimitExceeded {
                dimension: "max_files",
                limit: max_f,
                actual: proposals_by_file.len(),
            });
        }
    }

    let mut requests = Vec::new();

    for (file, group) in proposals_by_file {
        let live_bytes = workspace
            .read_file(&file)
            .map_err(|e| StructuralError::SourceRead {
                file: file.clone(),
                error: e,
            })?;

        let live_hash = compute_sha256(&live_bytes);
        // Stale detection: verify live bytes match observed snapshot
        if live_hash != group.observed_hash {
            return Err(StructuralError::StaleSource {
                file: file.clone(),
                expected: group.observed_hash,
                actual: live_hash,
            });
        }

        // Construct prospective candidate bytes
        let mut candidate = Vec::with_capacity(live_bytes.len());
        let mut cursor = 0;
        for m in group.matches {
            candidate.extend_from_slice(&live_bytes[cursor..m.start]);
            candidate.extend_from_slice(m.replacement.as_bytes());
            cursor = m.end;
        }
        candidate.extend_from_slice(&live_bytes[cursor..]);

        if candidate == live_bytes {
            // No-op for this file
            continue;
        }

        let req_id = format!("structural-{}", safe_identifier(&file));

        requests.push(Request {
            version: PROTOCOL_VERSION.into(),
            request_id: req_id,
            allow_generated: false,
            file_path: file,
            namespace: Default::default(),
            expected_pre_hash: Some(live_hash),
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
        let output_str = output_path.to_string_lossy().to_string();
        let rel_output_str = workspace
            .resolve_namespaced_path(&output_str, &Default::default())
            .map_err(|e| match e {
                WorkspaceError::Traversal(t) => {
                    StructuralError::WorkspaceEscape(format!("plan output traversal: {t}"))
                }
                WorkspaceError::SymlinkEscape(s) => StructuralError::SecurityViolation(format!(
                    "plan output escapes via symlink: {s}"
                )),
                WorkspaceError::WorkspaceRootMismatch { target, .. } => {
                    StructuralError::WorkspaceEscape(format!(
                        "plan output outside workspace root: {target}"
                    ))
                }
                other => StructuralError::SecurityViolation(other.to_string()),
            })?;

        if rel_output_str.is_empty() || rel_output_str == "." {
            return Err(StructuralError::SecurityViolation(
                "plan output cannot target workspace root directory".into(),
            ));
        }

        // Guard against overwriting an existing file with a plan
        let resolved = workspace
            .resolve_path(&rel_output_str)
            .map_err(|e| match e {
                WorkspaceError::Traversal(t) => {
                    StructuralError::WorkspaceEscape(format!("plan output traversal: {t}"))
                }
                WorkspaceError::SymlinkEscape(s) => StructuralError::SecurityViolation(format!(
                    "plan output escapes via symlink: {s}"
                )),
                WorkspaceError::WorkspaceRootMismatch { target, .. } => {
                    StructuralError::WorkspaceEscape(format!(
                        "plan output outside workspace root: {target}"
                    ))
                }
                other => StructuralError::SecurityViolation(other.to_string()),
            })?;

        if resolved.exists() {
            return Err(StructuralError::PlanOutputAlreadyExists(rel_output_str));
        }

        // Guard against overwriting any file targeted by the transaction
        if tx.requests.iter().any(|r| r.file_path == rel_output_str) {
            return Err(StructuralError::PlanOutputCollision(rel_output_str));
        }

        let prepared_plan = pipeline::prepare_transaction_plan(workspace, &tx, vec![])
            .map_err(|cert| StructuralError::PlanPreparation(Box::new(cert)))?;

        let bytes = serde_json::to_vec_pretty(&prepared_plan)?;
        workspace
            .create_file_new(&rel_output_str, &bytes)
            .map_err(|e| match e {
                WorkspaceError::AlreadyExists(p) => StructuralError::PlanOutputAlreadyExists(p),
                WorkspaceError::Io(io_err) => StructuralError::Io(io_err),
                other => StructuralError::SecurityViolation(other.to_string()),
            })?;

        return Ok(StructuralOutcome::PlanPrepared {
            plan_path: rel_output_str,
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
