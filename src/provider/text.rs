#![forbid(unsafe_code)]

use crate::engine::{compute_sha256, ByteEdit};
use crate::protocol::{Candidate, Cardinality, RefusalReason};
use memchr::{memchr_iter, memmem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use thiserror::Error;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextOperation {
    Replace {
        target: String,
        replacement: String,
    },
    /// Apply exact replacements in sequence against one evolving in-memory
    /// source, then expose a single exact source-to-result byte plan.
    ReplaceSequence {
        edits: Vec<TextReplacementIntent>,
        #[serde(default)]
        newline_policy: NewlinePolicy,
    },
    InsertBefore {
        target: String,
        content: String,
    },
    InsertAfter {
        target: String,
        content: String,
    },
    Delete {
        target: String,
    },
    Move {
        target: String,
        before: String,
    },
    EnsurePresent {
        content: String,
    },
    EnsureAbsent {
        target: String,
    },
    Set {
        target: String,
        replacement: String,
    },
    Unset {
        target: String,
    },
    Rename {
        target: String,
        replacement: String,
    },
    ConvertNewlines {
        to: String,
    },
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextReplacementIntent {
    pub old: String,
    pub new: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NewlinePolicy {
    /// Adapt LF or CRLF input to a single convention observed in the source.
    #[default]
    Source,
    /// Require supplied CR and LF bytes to match the source exactly.
    Exact,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TextProviderError {
    #[error("Refused: {0:?}")]
    Refused(RefusalReason),
    #[error("Error: {message}")]
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextProviderResult {
    pub edits: Vec<ByteEdit>,
    pub original_bytes: Vec<u8>,
    pub modified_bytes: Vec<u8>,
}

pub struct TextProvider;

#[derive(Clone, Copy)]
enum Placement {
    Replace,
    Before,
    After,
}

impl TextProvider {
    pub fn plan(
        content: &[u8],
        op: &TextOperation,
        cardinality: &Cardinality,
    ) -> Result<Vec<ByteEdit>, TextProviderError> {
        Self::plan_scoped(content, op, cardinality, None, None)
    }

    pub fn plan_scoped(
        content: &[u8],
        op: &TextOperation,
        cardinality: &Cardinality,
        within: Option<&str>,
        file_path: Option<&str>,
    ) -> Result<Vec<ByteEdit>, TextProviderError> {
        if within.is_some() && !matches!(op, TextOperation::ReplaceSequence { .. }) {
            return Err(TextProviderError::Refused(
                RefusalReason::ProviderCapabilityMissing {
                    provider: "text".into(),
                    capability: "scope_disambiguation requires the text replace_sequence operation"
                        .into(),
                },
            ));
        }
        if let TextOperation::ConvertNewlines { to } = op {
            let target_profile = match to.to_ascii_lowercase().as_str() {
                "lf" => crate::newline::NewlineProfile::Lf,
                "crlf" => crate::newline::NewlineProfile::Crlf,
                other => {
                    return Err(TextProviderError::Refused(RefusalReason::MalformedInput {
                        details: format!("unsupported target newline convention: '{other}'; expected 'lf' or 'crlf'"),
                    }));
                }
            };
            let (adapted, changed) =
                crate::newline::adapt_bytes_to_convention(content, target_profile);
            if !changed {
                return Ok(Vec::new());
            }
            return Ok(vec![ByteEdit {
                start: 0,
                end: content.len(),
                replacement: adapted,
            }]);
        }
        if let TextOperation::ReplaceSequence {
            edits,
            newline_policy,
        } = op
        {
            if edits.len() == 1 {
                return plan_sequence(
                    content,
                    edits,
                    *newline_policy,
                    within,
                    file_path,
                    cardinality,
                );
            }
            if !matches!(cardinality, Cardinality::ExactlyOne) {
                return Err(TextProviderError::Refused(
                    RefusalReason::CardinalityMismatch {
                        expected: "exactly_one for each ordered replacement".into(),
                        actual: edits.len(),
                    },
                ));
            }
            return plan_sequence(
                content,
                edits,
                *newline_policy,
                within,
                file_path,
                &Cardinality::ExactlyOne,
            );
        }
        match op {
            TextOperation::EnsurePresent { content: wanted } => {
                return plan_ensure_present(content, wanted)
            }
            TextOperation::Move { target, before } => {
                return plan_move(content, target, before, cardinality)
            }
            _ => {}
        }

        let (target, replacement, placement, desired_absence, set_like) = match op {
            TextOperation::Replace {
                target,
                replacement,
            } => (
                target.as_bytes(),
                replacement.as_bytes(),
                Placement::Replace,
                false,
                false,
            ),
            TextOperation::Delete { target } => {
                (target.as_bytes(), &[][..], Placement::Replace, false, false)
            }
            TextOperation::InsertBefore { target, content } => (
                target.as_bytes(),
                content.as_bytes(),
                Placement::Before,
                false,
                false,
            ),
            TextOperation::InsertAfter { target, content } => (
                target.as_bytes(),
                content.as_bytes(),
                Placement::After,
                false,
                false,
            ),
            TextOperation::EnsureAbsent { target } | TextOperation::Unset { target } => {
                (target.as_bytes(), &[][..], Placement::Replace, true, false)
            }
            TextOperation::Set {
                target,
                replacement,
            }
            | TextOperation::Rename {
                target,
                replacement,
            } => (
                target.as_bytes(),
                replacement.as_bytes(),
                Placement::Replace,
                false,
                true,
            ),
            TextOperation::Move { .. }
            | TextOperation::EnsurePresent { .. }
            | TextOperation::ConvertNewlines { .. } => unreachable!(),
            TextOperation::ReplaceSequence { .. } => unreachable!(),
        };

        if target.is_empty() {
            return Err(TextProviderError::Refused(RefusalReason::MissingTarget {
                target: String::new(),
            }));
        }

        let matches = find_matches(content, target);
        let match_count = matches.len();

        if desired_absence && match_count == 0 {
            return Ok(Vec::new());
        }

        // For desired-state set/rename operations, an already-present non-empty
        // replacement is a safe no-op. An empty replacement is not treated as
        // "present everywhere": without the target we cannot establish the
        // requested state, so normal missing-target refusal applies.
        if set_like
            && match_count == 0
            && !replacement.is_empty()
            && memmem::find(content, replacement).is_some()
        {
            return Ok(Vec::new());
        }

        enforce_cardinality(content, target, &matches, cardinality, desired_absence)?;

        let replacement = build_replacement(target, replacement, placement);
        Ok(matches
            .into_iter()
            .map(|start| ByteEdit {
                start,
                end: start + target.len(),
                replacement: replacement.clone(),
            })
            .collect())
    }

    /// Plan one already-selected physical occurrence. The caller must have
    /// verified the source identity and candidate selection before calling
    /// this method; this method still checks that the bytes at the guarded
    /// range are the requested target.
    pub fn plan_at(
        content: &[u8],
        op: &TextOperation,
        start: usize,
        end: usize,
    ) -> Result<Vec<ByteEdit>, TextProviderError> {
        let (target, replacement, placement) = match op {
            TextOperation::Replace {
                target,
                replacement,
            }
            | TextOperation::Set {
                target,
                replacement,
            }
            | TextOperation::Rename {
                target,
                replacement,
            } => (
                target.as_bytes(),
                replacement.as_bytes(),
                Placement::Replace,
            ),
            TextOperation::Delete { target }
            | TextOperation::EnsureAbsent { target }
            | TextOperation::Unset { target } => (target.as_bytes(), &[][..], Placement::Replace),
            TextOperation::InsertBefore { target, content } => {
                (target.as_bytes(), content.as_bytes(), Placement::Before)
            }
            TextOperation::InsertAfter { target, content } => {
                (target.as_bytes(), content.as_bytes(), Placement::After)
            }
            TextOperation::Move { .. }
            | TextOperation::EnsurePresent { .. }
            | TextOperation::ReplaceSequence { .. }
            | TextOperation::ConvertNewlines { .. } => {
                return Err(TextProviderError::Refused(
                    RefusalReason::UnsupportedOperation {
                        operation: "candidate guard is not available for this text operation"
                            .into(),
                    },
                ))
            }
        };
        if target.is_empty() || start > end || end > content.len() || &content[start..end] != target
        {
            return Err(TextProviderError::Refused(
                RefusalReason::CandidateSelectionInvalid {
                    offset: start,
                    selection_id: String::new(),
                    details: "guarded range does not contain the exact requested target".into(),
                },
            ));
        }
        Ok(vec![ByteEdit {
            start,
            end,
            replacement: build_replacement(target, replacement, placement),
        }])
    }
}

fn plan_sequence(
    content: &[u8],
    edits: &[TextReplacementIntent],
    newline_policy: NewlinePolicy,
    within: Option<&str>,
    file_path: Option<&str>,
    cardinality: &Cardinality,
) -> Result<Vec<ByteEdit>, TextProviderError> {
    const MAX_SEQUENCE_EDITS: usize = 64;
    if edits.is_empty() || edits.len() > MAX_SEQUENCE_EDITS {
        return Err(TextProviderError::Refused(
            RefusalReason::ResourceLimitExceeded {
                dimension: "source_text_edits".into(),
                limit: MAX_SEQUENCE_EDITS,
                actual: edits.len(),
            },
        ));
    }
    let profile = newline_profile(content);
    let is_mixed = profile == "mixed";
    let delimiter = match (newline_policy, profile) {
        (NewlinePolicy::Exact, _) => "\n",
        (NewlinePolicy::Source, "lf") => "\n",
        (NewlinePolicy::Source, "crlf") => "\r\n",
        (NewlinePolicy::Source, "none") => "\n",
        (NewlinePolicy::Source, "mixed") => {
            let any_mismatch = edits.iter().any(|edit| {
                edit.old.contains(['\r', '\n'])
                    && memmem::find(content, edit.old.as_bytes()).is_none()
            });
            if any_mismatch {
                return Err(TextProviderError::Refused(
                    RefusalReason::PreservationUnavailable {
                        details: "source newline profile is mixed; multiline targets must match exact line endings".into(),
                    },
                ));
            }
            "\n"
        }
        _ => "\n",
    };
    if edits.len() == 1 && within.is_none() && !matches!(cardinality, Cardinality::ExactlyOne) {
        let intent = &edits[0];
        let (target, replacement) = if is_mixed && newline_policy == NewlinePolicy::Source {
            resolve_edit_for_mixed(content, intent)
        } else {
            (
                adapt_newline_text(&intent.old, newline_policy, delimiter)?,
                adapt_newline_text(&intent.new, newline_policy, delimiter)?,
            )
        };
        let operation = TextOperation::Replace {
            target,
            replacement,
        };
        return TextProvider::plan(content, &operation, cardinality);
    }

    let mut candidate = content.to_vec();
    if let Some(anchor) = within {
        let (scope_start, mut scope_end) =
            crate::scope::resolve_scope(content, file_path.unwrap_or(""), anchor)
                .map_err(TextProviderError::Refused)?;

        for intent in edits {
            let (target, replacement) = if is_mixed && newline_policy == NewlinePolicy::Source {
                resolve_edit_for_mixed(&candidate[scope_start..scope_end], intent)
            } else {
                (
                    adapt_newline_text(&intent.old, newline_policy, delimiter)?,
                    adapt_newline_text(&intent.new, newline_policy, delimiter)?,
                )
            };
            if target.is_empty() {
                return Err(TextProviderError::Refused(RefusalReason::MissingTarget {
                    target: String::new(),
                }));
            }
            if scope_start > scope_end || scope_end > candidate.len() {
                return Err(TextProviderError::Error {
                    message: "resolved scope boundary invalid on candidate".into(),
                });
            }
            let scope_slice = &candidate[scope_start..scope_end];
            let matches = find_matches(scope_slice, target.as_bytes());
            enforce_cardinality(scope_slice, target.as_bytes(), &matches, cardinality, false)?;
            for &rel_offset in matches.iter().rev() {
                let abs_offset = scope_start + rel_offset;
                candidate.splice(
                    abs_offset..abs_offset + target.len(),
                    replacement.as_bytes().iter().copied(),
                );
            }
            let total_delta =
                (replacement.len() as isize - target.len() as isize) * matches.len() as isize;
            scope_end = (scope_end as isize + total_delta) as usize;
        }
    } else {
        for intent in edits {
            let (target, replacement) = if is_mixed && newline_policy == NewlinePolicy::Source {
                resolve_edit_for_mixed(&candidate, intent)
            } else {
                (
                    adapt_newline_text(&intent.old, newline_policy, delimiter)?,
                    adapt_newline_text(&intent.new, newline_policy, delimiter)?,
                )
            };
            let operation = TextOperation::Replace {
                target,
                replacement,
            };
            let byte_edits = TextProvider::plan(&candidate, &operation, cardinality)?;
            candidate =
                crate::engine::apply_byte_edits(&candidate, &byte_edits).map_err(|error| {
                    TextProviderError::Error {
                        message: error.to_string(),
                    }
                })?;
        }
    }
    crate::diff_planner::plan(content, &candidate)
        .map(|plan| plan.edits)
        .map_err(|error| match error {
            crate::diff_planner::DiffPlannerError::ResourceLimit {
                dimension,
                limit,
                actual,
            } => TextProviderError::Refused(RefusalReason::ResourceLimitExceeded {
                dimension: dimension.into(),
                limit,
                actual,
            }),
            crate::diff_planner::DiffPlannerError::Invariant(details) => {
                TextProviderError::Error { message: details }
            }
        })
}

fn newline_profile(content: &[u8]) -> &'static str {
    crate::engine::newline_profile(content)
}

fn resolve_edit_for_mixed(content: &[u8], intent: &TextReplacementIntent) -> (String, String) {
    if memmem::find(content, intent.old.as_bytes()).is_some() {
        return (intent.old.clone(), intent.new.clone());
    }
    let old_lf = intent.old.replace("\r\n", "\n");
    if memmem::find(content, old_lf.as_bytes()).is_some() {
        return (old_lf, intent.new.replace("\r\n", "\n"));
    }
    let old_crlf = old_lf.replace('\n', "\r\n");
    if memmem::find(content, old_crlf.as_bytes()).is_some() {
        return (
            old_crlf,
            intent.new.replace("\r\n", "\n").replace('\n', "\r\n"),
        );
    }
    (intent.old.clone(), intent.new.clone())
}

fn adapt_newline_text(
    value: &str,
    policy: NewlinePolicy,
    delimiter: &str,
) -> Result<String, TextProviderError> {
    if policy == NewlinePolicy::Exact {
        return Ok(value.to_owned());
    }
    let mut normalized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                normalized.push('\n');
            }
            '\r' => {
                return Err(TextProviderError::Refused(
                    RefusalReason::PreservationUnavailable {
                        details: "source newline adaptation does not accept a lone carriage return"
                            .into(),
                    },
                ));
            }
            other => normalized.push(other),
        }
    }
    if delimiter == "\r\n" {
        Ok(normalized.replace('\n', "\r\n"))
    } else {
        Ok(normalized)
    }
}

fn plan_ensure_present(content: &[u8], wanted: &str) -> Result<Vec<ByteEdit>, TextProviderError> {
    if wanted.is_empty() {
        return Err(TextProviderError::Refused(RefusalReason::MissingTarget {
            target: "empty ensure_present content".into(),
        }));
    }

    if memmem::find(content, wanted.as_bytes()).is_some() {
        return Ok(Vec::new());
    }

    let newline: &[u8] = if memmem::find(content, b"\r\n").is_some() {
        b"\r\n"
    } else {
        b"\n"
    };
    let leading_newline = !content.is_empty() && !content.ends_with(b"\n");
    let trailing_newline = content.ends_with(b"\n");
    let capacity = wanted.len()
        + usize::from(leading_newline) * newline.len()
        + usize::from(trailing_newline) * newline.len();
    let mut replacement = Vec::with_capacity(capacity);

    if leading_newline {
        replacement.extend_from_slice(newline);
    }
    replacement.extend_from_slice(wanted.as_bytes());
    if trailing_newline {
        replacement.extend_from_slice(newline);
    }

    Ok(vec![ByteEdit {
        start: content.len(),
        end: content.len(),
        replacement,
    }])
}

fn build_replacement(target: &[u8], replacement: &[u8], placement: Placement) -> Vec<u8> {
    match placement {
        Placement::Replace => replacement.to_vec(),
        Placement::Before => {
            let mut bytes = Vec::with_capacity(replacement.len() + target.len());
            bytes.extend_from_slice(replacement);
            bytes.extend_from_slice(target);
            bytes
        }
        Placement::After => {
            let mut bytes = Vec::with_capacity(target.len() + replacement.len());
            bytes.extend_from_slice(target);
            bytes.extend_from_slice(replacement);
            bytes
        }
    }
}

fn enforce_cardinality(
    content: &[u8],
    target: &[u8],
    matches: &[usize],
    cardinality: &Cardinality,
    desired_absence: bool,
) -> Result<(), TextProviderError> {
    if desired_absence {
        return Ok(());
    }

    let actual = matches.len();
    let duplicate = || {
        TextProviderError::Refused(RefusalReason::DuplicateTarget {
            target: String::from_utf8_lossy(target).into_owned(),
            count: actual,
            candidates: candidate_diagnostics(content, target, matches),
            candidates_returned: actual.min(8),
            truncated: actual > 8,
        })
    };

    match cardinality {
        Cardinality::ExactlyOne => match actual.cmp(&1) {
            Ordering::Less => Err(missing_target(content, target)),
            Ordering::Equal => Ok(()),
            Ordering::Greater => Err(duplicate()),
        },
        Cardinality::Exactly(expected) => match actual.cmp(expected) {
            Ordering::Less => Err(missing_target(content, target)),
            Ordering::Equal => Ok(()),
            Ordering::Greater => Err(duplicate()),
        },
        Cardinality::All if actual == 0 => Err(missing_target(content, target)),
        Cardinality::All => Ok(()),
    }
}

fn missing_target(content: &[u8], target: &[u8]) -> TextProviderError {
    TextProviderError::Refused(RefusalReason::MissingTarget {
        target: diagnose_near_miss(content, target),
    })
}

fn find_matches(content: &[u8], target: &[u8]) -> Vec<usize> {
    if target.is_empty() || target.len() > content.len() {
        return Vec::new();
    }
    memmem::find_iter(content, target).collect()
}

fn plan_move(
    content: &[u8],
    target: &str,
    before: &str,
    cardinality: &Cardinality,
) -> Result<Vec<ByteEdit>, TextProviderError> {
    if !matches!(cardinality, Cardinality::ExactlyOne) {
        return Err(TextProviderError::Refused(
            RefusalReason::CardinalityMismatch {
                expected: "exactly_one (move anchors are unique)".into(),
                actual: 1,
            },
        ));
    }

    let targets = find_matches(content, target.as_bytes());
    let destinations = find_matches(content, before.as_bytes());
    if targets.len() != 1 || destinations.len() != 1 || target.is_empty() || before.is_empty() {
        return Err(TextProviderError::Refused(
            RefusalReason::CardinalityMismatch {
                expected: "one target and one destination".into(),
                actual: targets.len().saturating_add(destinations.len()),
            },
        ));
    }

    let source = targets[0];
    let source_end = source + target.len();
    let destination = destinations[0];
    if source <= destination && destination <= source_end {
        return Ok(Vec::new());
    }

    let mut edits = vec![
        ByteEdit {
            start: destination,
            end: destination,
            replacement: target.as_bytes().to_vec(),
        },
        ByteEdit {
            start: source,
            end: source_end,
            replacement: Vec::new(),
        },
    ];
    edits.sort_unstable_by_key(|edit| edit.start);
    Ok(edits)
}

fn find_normalized_span<'a>(content: &'a str, normalized_target: &str) -> Option<&'a str> {
    let target_lines: Vec<&str> = normalized_target.split_terminator('\n').collect();
    if target_lines.is_empty() {
        return None;
    }
    let content_bytes = content.as_bytes();
    let mut line_spans = Vec::new();
    let mut start = 0;
    for (i, &b) in content_bytes.iter().enumerate() {
        if b == b'\n' {
            let line_raw = &content[start..i];
            let line_clean = line_raw.strip_suffix('\r').unwrap_or(line_raw);
            line_spans.push((start, i + 1, line_clean));
            start = i + 1;
        }
    }
    if start < content.len() {
        let line_raw = &content[start..];
        let line_clean = line_raw.strip_suffix('\r').unwrap_or(line_raw);
        line_spans.push((start, content.len(), line_clean));
    }

    if line_spans.len() < target_lines.len() {
        return None;
    }

    for window_start in 0..=line_spans.len() - target_lines.len() {
        let mut matches = true;
        for (offset, target_line) in target_lines.iter().enumerate() {
            if line_spans[window_start + offset].2 != *target_line {
                matches = false;
                break;
            }
        }
        if matches {
            let start_byte = line_spans[window_start].0;
            let last_idx = window_start + target_lines.len() - 1;
            let end_byte = if !normalized_target.ends_with('\n') {
                line_spans[last_idx].0 + line_spans[last_idx].2.len()
            } else {
                line_spans[last_idx].1
            };
            return Some(&content[start_byte..end_byte]);
        }
    }
    None
}

fn diagnose_near_miss(content: &[u8], target: &[u8]) -> String {
    let target_str = String::from_utf8_lossy(target);
    let content_str = String::from_utf8_lossy(content);

    if target_str.contains('\n') {
        let normalized_target = target_str.replace("\r\n", "\n");
        if let Some(slice) = find_normalized_span(&content_str, &normalized_target) {
            let slice_bytes = slice.as_bytes();
            let has_crlf_in_match = slice_bytes.windows(2).any(|w| w == b"\r\n");
            let has_lone_lf_in_match = slice_bytes
                .iter()
                .enumerate()
                .any(|(i, &b)| b == b'\n' && (i == 0 || slice_bytes[i - 1] != b'\r'));

            if has_crlf_in_match && has_lone_lf_in_match {
                return "Target not found exact; the matching source region uses mixed LF and CRLF line endings. Set newline_policy to exact and copy each line ending from the observed source.".into();
            } else if has_crlf_in_match && !target_str.contains("\r\n") {
                return "Target not found exact; detected CRLF vs LF line ending discrepancy: the matching source region uses CRLF line endings while the target used LF. Set newline_policy to exact or use CRLF line endings.".into();
            } else if has_lone_lf_in_match && target_str.contains("\r\n") {
                return "Target not found exact; detected LF vs CRLF line ending discrepancy: the matching source region uses LF line endings while the target used CRLF. Set newline_policy to exact or use LF line endings.".into();
            }
        }
    }

    if target_str.contains(' ') && content_str.contains('\u{00A0}') {
        let alt = target_str.replace(' ', "\u{00A0}");
        if content_str.contains(&alt) {
            return "Target not found exact; detected Non-Breaking Space (U+00A0) discrepancy. Try replacing spaces with NBSP.".to_string();
        }
    }
    if target_str.contains('\u{00A0}') && content_str.contains(' ') {
        let alt = target_str.replace('\u{00A0}', " ");
        if content_str.contains(&alt) {
            return "Target not found exact; detected space vs Non-Breaking Space (U+00A0) discrepancy.".to_string();
        }
    }

    if target_str.contains('\n') && !target_str.contains("\r\n") && content_str.contains("\r\n") {
        let alt = target_str.replace('\n', "\r\n");
        if content_str.contains(&alt) {
            return "Target not found exact; detected CRLF vs LF line ending discrepancy."
                .to_string();
        }
    }
    if target_str.contains("\r\n") && content_str.contains('\n') && !content_str.contains("\r\n") {
        let alt = target_str.replace("\r\n", "\n");
        if content_str.contains(&alt) {
            return "Target not found exact; detected LF vs CRLF line ending discrepancy."
                .to_string();
        }
    }

    if target_str.contains('\t') && content_str.contains("    ") {
        let alt = target_str.replace('\t', "    ");
        if content_str.contains(&alt) {
            return "Target not found exact; detected Tab vs Spaces indentation discrepancy."
                .to_string();
        }
    }

    if content_str.contains('\u{200B}') {
        return "Target not found exact; content contains Zero-Width Space (U+200B).".to_string();
    }

    if content.starts_with(&[0xEF, 0xBB, 0xBF]) && !target.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return "Target not found exact; file starts with UTF-8 BOM (EF BB BF) while target does not."
            .to_string();
    }

    format!("Target not found: '{target_str}'")
}

fn candidate_diagnostics(content: &[u8], target: &[u8], matches: &[usize]) -> Vec<Candidate> {
    matches
        .iter()
        .take(8)
        .map(|&offset| {
            let context_start = offset.saturating_sub(24);
            let context_end = (offset + target.len() + 24).min(content.len());
            Candidate {
                offset,
                start: offset,
                end: offset + target.len(),
                target: String::from_utf8_lossy(target).into_owned(),
                line: memchr_iter(b'\n', &content[..offset]).count() + 1,
                context: String::from_utf8_lossy(&content[context_start..context_end]).into_owned(),
                anchor_sha256: compute_sha256(&content[offset..offset + target.len()]),
                selection_id: String::new(),
                node_kind: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::apply_byte_edits;

    #[test]
    fn test_replace_single_unique() {
        let content = b"Hello world, welcome to Threadmoth.";
        let op = TextOperation::Replace {
            target: "world".to_string(),
            replacement: "universe".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"Hello universe, welcome to Threadmoth.");
    }

    #[test]
    fn test_insert_before() {
        let content = b"target line\n";
        let op = TextOperation::InsertBefore {
            target: "target line".to_string(),
            content: "prefix: ".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"prefix: target line\n");
    }

    #[test]
    fn test_insert_after() {
        let content = b"target line\n";
        let op = TextOperation::InsertAfter {
            target: "target line".to_string(),
            content: " suffix".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"target line suffix\n");
    }

    #[test]
    fn test_delete() {
        let content = b"remove this part carefully.\n";
        let op = TextOperation::Delete {
            target: "this part ".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"remove carefully.\n");
    }

    #[test]
    fn test_duplicate_target_refusal() {
        let content = b"foo and foo and foo";
        let op = TextOperation::Replace {
            target: "foo".to_string(),
            replacement: "bar".to_string(),
        };
        let res = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        match res {
            Err(TextProviderError::Refused(RefusalReason::DuplicateTarget {
                count,
                candidates,
                candidates_returned,
                truncated,
                ..
            })) => {
                assert_eq!(count, 3);
                assert_eq!(candidates.len(), 3);
                assert_eq!(candidates_returned, 3);
                assert!(!truncated);
            }
            other => panic!("expected bounded ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn test_missing_target_refusal() {
        let content = b"hello world";
        let op = TextOperation::Replace {
            target: "missing".to_string(),
            replacement: "bar".to_string(),
        };
        let res = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        assert!(matches!(
            res,
            Err(TextProviderError::Refused(
                RefusalReason::MissingTarget { .. }
            ))
        ));
    }

    #[test]
    fn empty_set_replacement_does_not_panic_or_fake_a_noop() {
        let content = b"hello world";
        let op = TextOperation::Set {
            target: "missing".to_string(),
            replacement: String::new(),
        };
        let result = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        assert!(matches!(
            result,
            Err(TextProviderError::Refused(
                RefusalReason::MissingTarget { .. }
            ))
        ));
    }

    #[test]
    fn exactly_n_shortfall_remains_a_missing_target_refusal() {
        let content = b"foo foo";
        let op = TextOperation::Replace {
            target: "foo".into(),
            replacement: "bar".into(),
        };
        let result = TextProvider::plan(content, &op, &Cardinality::Exactly(3));
        assert!(matches!(
            result,
            Err(TextProviderError::Refused(
                RefusalReason::MissingTarget { .. }
            ))
        ));
    }

    #[test]
    fn test_near_miss_nbsp() {
        let content = "hello\u{00A0}world".as_bytes();
        let op = TextOperation::Replace {
            target: "hello world".to_string(),
            replacement: "hi".to_string(),
        };
        let res = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        match res {
            Err(TextProviderError::Refused(RefusalReason::MissingTarget { target })) => {
                assert!(target.contains("Non-Breaking Space"));
            }
            _ => panic!("Expected near-miss NBSP refusal"),
        }
    }

    #[test]
    fn test_near_miss_multibyte_unicode_before_newline_does_not_panic() {
        let content = "café\r\nline2\r\n".as_bytes();
        let op = TextOperation::Replace {
            target: "café\nline2".to_string(),
            replacement: "replaced".to_string(),
        };
        let res = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        match res {
            Err(TextProviderError::Refused(RefusalReason::MissingTarget { target })) => {
                assert!(target.contains("CRLF"));
            }
            _ => panic!("Expected near-miss CRLF refusal without panic"),
        }
    }

    #[test]
    fn test_near_miss_crlf() {
        let content = b"line1\r\nline2\r\n";
        let op = TextOperation::Replace {
            target: "line1\nline2".to_string(),
            replacement: "replaced".to_string(),
        };
        let res = TextProvider::plan(content, &op, &Cardinality::ExactlyOne);
        match res {
            Err(TextProviderError::Refused(RefusalReason::MissingTarget { target })) => {
                assert!(target.contains("CRLF"));
            }
            _ => panic!("Expected near-miss CRLF refusal"),
        }
    }

    #[test]
    fn test_cardinality_all_bulk_replace() {
        let content = b"apple banana apple orange apple";
        let op = TextOperation::Replace {
            target: "apple".to_string(),
            replacement: "fruit".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::All).unwrap();
        assert_eq!(edits.len(), 3);
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"fruit banana fruit orange fruit");
    }

    #[test]
    fn exact_matching_remains_non_overlapping() {
        let content = b"aaaa";
        let op = TextOperation::Replace {
            target: "aa".into(),
            replacement: "b".into(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::All).unwrap();
        assert_eq!(
            edits.iter().map(|edit| edit.start).collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(apply_byte_edits(content, &edits).unwrap(), b"bb");
    }

    #[test]
    fn test_bom_and_crlf_preservation() {
        let content = b"\xEF\xBB\xBFline1\r\nline2\r\n";
        let op = TextOperation::Replace {
            target: "line2".to_string(),
            replacement: "modified2".to_string(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        let modified = apply_byte_edits(content, &edits).unwrap();
        assert_eq!(modified, b"\xEF\xBB\xBFline1\r\nmodified2\r\n");
    }

    #[test]
    fn moving_an_already_adjacent_target_is_no_change() {
        let content = b"A B C";
        let op = TextOperation::Move {
            target: "B ".into(),
            before: "C".into(),
        };
        let edits = TextProvider::plan(content, &op, &Cardinality::ExactlyOne).unwrap();
        assert!(edits.is_empty());
    }
}
