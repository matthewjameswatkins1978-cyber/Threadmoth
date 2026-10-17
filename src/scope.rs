#![forbid(unsafe_code)]

//! Reliable structural scope resolution for scoped anchor disambiguation.
//!
//! Enforces byte-exact structural boundaries across code ASTs (Tree-sitter),
//! TOML/INI sections, and Markdown document hierarchies.
//! Speculative indentation-based scope guessing is strictly forbidden.

use crate::protocol::RefusalReason;
use memchr::memmem;

/// Resolve the byte range `[scope_start..scope_end]` of the structural unit
/// identified by `anchor`.
///
/// Fails closed if the anchor is ambiguous, missing, inside a body, or if the
/// file format cannot establish reliable structural boundaries.
pub fn resolve_scope(
    content: &[u8],
    file_path: &str,
    anchor: &str,
) -> Result<(usize, usize), RefusalReason> {
    if anchor.is_empty() {
        return Err(RefusalReason::MalformedInput {
            details: "scope anchor must not be empty".into(),
        });
    }

    let detection = crate::target_registry::detect(file_path, Some(content));

    if detection.provider.as_deref() == Some("markdown")
        || file_path.ends_with(".md")
        || file_path.ends_with(".markdown")
    {
        let anchor_start = find_markdown_heading_anchor(content, anchor)?;
        let anchor_end = anchor_start + anchor.len();
        return resolve_markdown_scope(content, anchor_start, anchor_end, anchor);
    }

    let matches: Vec<usize> = memmem::find_iter(content, anchor.as_bytes()).collect();
    if matches.is_empty() {
        return Err(RefusalReason::MissingTarget {
            target: anchor.to_string(),
        });
    }
    if matches.len() > 1 {
        return Err(RefusalReason::DuplicateTarget {
            target: anchor.to_string(),
            count: matches.len(),
            candidates: Vec::new(),
            candidates_returned: 0,
            truncated: false,
        });
    }

    let anchor_start = matches[0];
    let anchor_end = anchor_start + anchor.len();

    if crate::provider::syntax::lookup(&detection.target_kind).is_some() {
        return crate::provider::syntax::resolve_structural_scope(
            content,
            &detection.target_kind,
            anchor_start,
            anchor_end,
        );
    }

    if detection.provider.as_deref() == Some("toml")
        || detection.provider.as_deref() == Some("ini")
        || file_path.ends_with(".toml")
        || file_path.ends_with(".ini")
    {
        return resolve_section_scope(content, anchor_start, anchor_end, anchor);
    }

    Err(RefusalReason::PreservationUnavailable {
        details: format!(
            "file '{}' ({}) does not support structural scope resolution; indentation-based guessing is forbidden",
            file_path, detection.target_kind
        ),
    })
}

fn resolve_section_scope(
    content: &[u8],
    anchor_start: usize,
    anchor_end: usize,
    anchor: &str,
) -> Result<(usize, usize), RefusalReason> {
    let line_start = match content[..anchor_start].iter().rposition(|&b| b == b'\n') {
        Some(pos) => pos + 1,
        None => 0,
    };
    let line_end = match content[anchor_end..].iter().position(|&b| b == b'\n') {
        Some(pos) => anchor_end + pos,
        None => content.len(),
    };
    let line_bytes = &content[line_start..line_end];
    let trimmed = trim_ascii_start(line_bytes);
    if !trimmed.starts_with(b"[") {
        return Err(RefusalReason::PreservationUnavailable {
            details: format!("anchor '{anchor}' is not a section header in TOML/INI file"),
        });
    }

    let mut scope_end = content.len();
    let mut scan_pos = if line_end < content.len() {
        line_end + 1
    } else {
        content.len()
    };
    while scan_pos < content.len() {
        let next_nl = match content[scan_pos..].iter().position(|&b| b == b'\n') {
            Some(pos) => scan_pos + pos,
            None => content.len(),
        };
        let line = &content[scan_pos..next_nl];
        let trimmed_line = trim_ascii_start(line);
        if trimmed_line.starts_with(b"[") {
            scope_end = scan_pos;
            break;
        }
        scan_pos = if next_nl < content.len() {
            next_nl + 1
        } else {
            content.len()
        };
    }

    Ok((line_start, scope_end))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodeFenceState {
    fence_char: u8,
    min_len: usize,
}

fn parse_markdown_heading(line: &[u8]) -> Option<usize> {
    let indent = line.iter().take_while(|&&b| b == b' ').count();
    if indent > 3 || (line.len() > indent && line[indent] == b'\t') {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.iter().take_while(|&&b| b == b'#').count();
    if (1..=6).contains(&level) && rest.len() > level && matches!(rest[level], b' ' | b'\t') {
        Some(level)
    } else {
        None
    }
}

/// Helper that checks if a line opens, continues, or closes a Markdown fenced code block.
///
/// In Markdown (CommonMark), a code fence can have up to 3 spaces of indentation.
/// 4 or more spaces of indentation makes the line an indented code block, NOT a fence.
///
/// Returns `(new_state, is_fenced)`.
/// `is_fenced` is true if the line is an opening fence, a closing fence, or inside a fence.
fn update_code_fence(state: Option<CodeFenceState>, line: &[u8]) -> (Option<CodeFenceState>, bool) {
    let indent = line.iter().take_while(|&&b| b == b' ').count();
    if indent > 3 || (line.len() > indent && line[indent] == b'\t') {
        // Four or more spaces, or tab indentation: cannot open or close a code fence.
        // If already inside a fence, it remains content inside the fence.
        return (state, state.is_some());
    }
    let rest = &line[indent..];
    match state {
        None => {
            if rest.starts_with(b"```") {
                let count = rest.iter().take_while(|&&b| b == b'`').count();
                (
                    Some(CodeFenceState {
                        fence_char: b'`',
                        min_len: count,
                    }),
                    true,
                )
            } else if rest.starts_with(b"~~~") {
                let count = rest.iter().take_while(|&&b| b == b'~').count();
                (
                    Some(CodeFenceState {
                        fence_char: b'~',
                        min_len: count,
                    }),
                    true,
                )
            } else {
                (None, false)
            }
        }
        Some(active) => {
            let count = rest.iter().take_while(|&&b| b == active.fence_char).count();
            if count >= active.min_len && trim_ascii(&rest[count..]).is_empty() {
                // Closing fence delimiter line
                (None, true)
            } else {
                // Inside fence
                (Some(active), true)
            }
        }
    }
}

fn find_markdown_heading_anchor(content: &[u8], anchor: &str) -> Result<usize, RefusalReason> {
    let all_matches: Vec<usize> = memmem::find_iter(content, anchor.as_bytes()).collect();
    if all_matches.is_empty() {
        return Err(RefusalReason::MissingTarget {
            target: anchor.to_string(),
        });
    }

    let mut fence_state = None;
    let mut scan_pos = 0;
    let mut valid_heading_starts = Vec::new();
    let mut inside_fence_matches = Vec::new();

    while scan_pos < content.len() {
        let line_end = match content[scan_pos..].iter().position(|&b| b == b'\n') {
            Some(pos) => scan_pos + pos,
            None => content.len(),
        };
        let line_bytes = &content[scan_pos..line_end];
        let (next_fence_state, is_fenced) = update_code_fence(fence_state, line_bytes);
        fence_state = next_fence_state;

        for &m in &all_matches {
            if m >= scan_pos && m < line_end {
                if is_fenced {
                    inside_fence_matches.push(m);
                } else if parse_markdown_heading(line_bytes).is_some() {
                    valid_heading_starts.push(m);
                }
            }
        }

        scan_pos = if line_end < content.len() {
            line_end + 1
        } else {
            content.len()
        };
    }

    if valid_heading_starts.len() == 1 {
        Ok(valid_heading_starts[0])
    } else if valid_heading_starts.len() > 1 {
        Err(RefusalReason::DuplicateTarget {
            target: anchor.to_string(),
            count: valid_heading_starts.len(),
            candidates: Vec::new(),
            candidates_returned: 0,
            truncated: false,
        })
    } else if !inside_fence_matches.is_empty() {
        Err(RefusalReason::PreservationUnavailable {
            details: format!(
                "anchor '{anchor}' is inside a fenced code block and cannot be selected as a Markdown structural heading"
            ),
        })
    } else {
        Err(RefusalReason::PreservationUnavailable {
            details: format!("anchor '{anchor}' is not a Markdown heading"),
        })
    }
}

fn resolve_markdown_scope(
    content: &[u8],
    anchor_start: usize,
    anchor_end: usize,
    anchor: &str,
) -> Result<(usize, usize), RefusalReason> {
    let line_start = match content[..anchor_start].iter().rposition(|&b| b == b'\n') {
        Some(pos) => pos + 1,
        None => 0,
    };
    let line_end = match content[anchor_end..].iter().position(|&b| b == b'\n') {
        Some(pos) => anchor_end + pos,
        None => content.len(),
    };
    let line_bytes = &content[line_start..line_end];
    let level = match parse_markdown_heading(line_bytes) {
        Some(l) => l,
        None => {
            return Err(RefusalReason::PreservationUnavailable {
                details: format!("anchor '{anchor}' is not a Markdown heading"),
            });
        }
    };

    let mut scope_end = content.len();
    let mut scan_pos = if line_end < content.len() {
        line_end + 1
    } else {
        content.len()
    };
    let mut fence_state = None;
    while scan_pos < content.len() {
        let next_nl = match content[scan_pos..].iter().position(|&b| b == b'\n') {
            Some(pos) => scan_pos + pos,
            None => content.len(),
        };
        let line = &content[scan_pos..next_nl];
        let (next_state, is_fenced) = update_code_fence(fence_state, line);
        fence_state = next_state;
        if !is_fenced {
            if let Some(next_level) = parse_markdown_heading(line) {
                if next_level <= level {
                    scope_end = scan_pos;
                    break;
                }
            }
        }
        scan_pos = if next_nl < content.len() {
            next_nl + 1
        } else {
            content.len()
        };
    }

    Ok((line_start, scope_end))
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r') {
        i += 1;
    }
    &bytes[i..]
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && matches!(bytes[start], b' ' | b'\t' | b'\r' | b'\n') {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    &bytes[start..end]
}
