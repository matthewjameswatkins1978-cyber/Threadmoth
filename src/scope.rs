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

    let detection = crate::target_registry::detect(file_path, Some(content));

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

    if detection.provider.as_deref() == Some("markdown")
        || file_path.ends_with(".md")
        || file_path.ends_with(".markdown")
    {
        return resolve_markdown_scope(content, anchor_start, anchor_end, anchor);
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
    let trimmed = trim_ascii_start(line_bytes);
    let level = trimmed.iter().take_while(|&&b| b == b'#').count();
    if level == 0 || level > 6 || trimmed.len() <= level || !matches!(trimmed[level], b' ' | b'\t')
    {
        return Err(RefusalReason::PreservationUnavailable {
            details: format!("anchor '{anchor}' is not a Markdown heading"),
        });
    }

    let mut scope_end = content.len();
    let mut scan_pos = if line_end < content.len() {
        line_end + 1
    } else {
        content.len()
    };
    let mut in_fence = false;
    while scan_pos < content.len() {
        let next_nl = match content[scan_pos..].iter().position(|&b| b == b'\n') {
            Some(pos) => scan_pos + pos,
            None => content.len(),
        };
        let line = &content[scan_pos..next_nl];
        let trimmed_line = trim_ascii_start(line);
        if trimmed_line.starts_with(b"```") || trimmed_line.starts_with(b"~~~") {
            in_fence = !in_fence;
        } else if !in_fence {
            let next_level = trimmed_line.iter().take_while(|&&b| b == b'#').count();
            if next_level > 0
                && next_level <= level
                && trimmed_line.len() > next_level
                && matches!(trimmed_line[next_level], b' ' | b'\t')
            {
                scope_end = scan_pos;
                break;
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
