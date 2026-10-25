#![forbid(unsafe_code)]

use crate::engine::{apply_byte_edits, ByteEdit, EngineError};
use memchr::memchr_iter;
use thiserror::Error;

/// Planning is deliberately bounded before any diff tokenisation occurs.
pub const MAX_PLANNER_INPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PLANNER_REGIONS: usize = 4096;
/// Maximum total old/new bytes given to byte-level Myers refinement. Larger
/// changed regions use one safe replacement instead of pathological allocation
/// and search cost.
pub const MAX_BYTE_REFINEMENT_INPUT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPlan {
    pub edits: Vec<ByteEdit>,
    pub derived_region_count: usize,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DiffPlannerError {
    #[error("planner resource limit: {dimension} {actual} exceeds {limit}")]
    ResourceLimit {
        dimension: &'static str,
        limit: usize,
        actual: usize,
    },
    #[error("planner invariant failure: {0}")]
    Invariant(String),
}

pub fn plan(observed: &[u8], desired: &[u8]) -> Result<DiffPlan, DiffPlannerError> {
    let input_size =
        observed
            .len()
            .checked_add(desired.len())
            .ok_or(DiffPlannerError::ResourceLimit {
                dimension: "planner_input_bytes",
                limit: MAX_PLANNER_INPUT_BYTES,
                actual: usize::MAX,
            })?;
    if input_size > MAX_PLANNER_INPUT_BYTES {
        return Err(DiffPlannerError::ResourceLimit {
            dimension: "planner_input_bytes",
            limit: MAX_PLANNER_INPUT_BYTES,
            actual: input_size,
        });
    }
    if observed == desired {
        return Ok(DiffPlan {
            edits: Vec::new(),
            derived_region_count: 0,
        });
    }

    let old_lines = split_inclusive_lines(observed);
    let new_lines = split_inclusive_lines(desired);
    let old_offsets = offsets(&old_lines);
    let new_offsets = offsets(&new_lines);
    let old_refs: Vec<&[u8]> = old_lines.to_vec();
    let new_refs: Vec<&[u8]> = new_lines.to_vec();
    let line_diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .diff_slices(&old_refs, &new_refs);

    let mut edits = Vec::new();
    for op in line_diff.ops() {
        if matches!(op, similar::DiffOp::Equal { .. }) {
            continue;
        }
        let old_len = op.old_range().len();
        let new_len = op.new_range().len();
        if old_len == new_len {
            for (old_idx, new_idx) in op.old_range().zip(op.new_range()) {
                let old_start = old_offsets[old_idx];
                let old_end = old_offsets[old_idx + 1];
                let new_start = new_offsets[new_idx];
                let new_end = new_offsets[new_idx + 1];
                refine_region(
                    observed, desired, old_start, old_end, new_start, new_end, &mut edits,
                )?;
            }
        } else {
            let old_start = old_offsets[op.old_range().start];
            let old_end = old_offsets[op.old_range().end];
            let new_start = new_offsets[op.new_range().start];
            let new_end = new_offsets[op.new_range().end];
            refine_region(
                observed, desired, old_start, old_end, new_start, new_end, &mut edits,
            )?;
        }
        if edits.len() > MAX_PLANNER_REGIONS {
            return Err(DiffPlannerError::ResourceLimit {
                dimension: "derived_regions",
                limit: MAX_PLANNER_REGIONS,
                actual: edits.len(),
            });
        }
    }

    edits.sort_by_key(|edit| (edit.start, edit.end));
    if edits.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return Err(DiffPlannerError::Invariant(
            "refinement produced overlapping edits".into(),
        ));
    }
    let candidate = apply_byte_edits(observed, &edits)
        .map_err(|error| DiffPlannerError::Invariant(format!("cannot apply plan: {error}")))?;
    if candidate != desired {
        let first_diff = candidate
            .iter()
            .zip(desired.iter())
            .position(|(a, b)| a != b);
        let detail = match first_diff {
            Some(idx) => format!(
                "derived edits do not produce desired bytes (first mismatch at byte offset {idx}: candidate byte 0x{:02x}, desired byte 0x{:02x}; candidate len {}, desired len {})",
                candidate[idx], desired[idx], candidate.len(), desired.len()
            ),
            None => format!(
                "derived edits do not produce desired bytes (length mismatch: candidate len {}, desired len {})",
                candidate.len(), desired.len()
            ),
        };
        return Err(DiffPlannerError::Invariant(detail));
    }
    Ok(DiffPlan {
        derived_region_count: edits.len(),
        edits,
    })
}

fn split_inclusive_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for newline_pos in memchr_iter(b'\n', bytes) {
        lines.push(&bytes[start..=newline_pos]);
        start = newline_pos + 1;
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

fn offsets(lines: &[&[u8]]) -> Vec<usize> {
    let mut result = Vec::with_capacity(lines.len() + 1);
    result.push(0);
    let mut current = 0;
    for line in lines {
        current += line.len();
        result.push(current);
    }
    result
}

fn refine_region(
    observed: &[u8],
    desired: &[u8],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    edits: &mut Vec<ByteEdit>,
) -> Result<(), DiffPlannerError> {
    let old = &observed[old_start..old_end];
    let new = &desired[new_start..new_end];
    let common_prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(old_byte, new_byte)| old_byte == new_byte)
        .count();
    let old_after_prefix = &old[common_prefix..];
    let new_after_prefix = &new[common_prefix..];
    let mut common_suffix = old_after_prefix
        .iter()
        .rev()
        .zip(new_after_prefix.iter().rev())
        .take_while(|(old_byte, new_byte)| old_byte == new_byte)
        .count();
    if common_suffix > 0 {
        let old_rem = &old_after_prefix[..old_after_prefix.len() - common_suffix];
        let new_rem = &new_after_prefix[..new_after_prefix.len() - common_suffix];
        if ((old_rem.ends_with(b"\r") && !new_rem.ends_with(b"\r"))
            || (!old_rem.ends_with(b"\r") && new_rem.ends_with(b"\r")))
            && old_after_prefix[old_after_prefix.len() - common_suffix] == b'\n'
        {
            common_suffix -= 1;
        }
    }
    let old_changed_end = old.len() - common_suffix;
    let new_changed_end = new.len() - common_suffix;
    let old_changed = &old[common_prefix..old_changed_end];
    let new_changed = &new[common_prefix..new_changed_end];
    let changed_input = old_changed.len().checked_add(new_changed.len()).ok_or(
        DiffPlannerError::ResourceLimit {
            dimension: "byte_refinement_input_bytes",
            limit: MAX_BYTE_REFINEMENT_INPUT_BYTES,
            actual: usize::MAX,
        },
    )?;
    if changed_input > MAX_BYTE_REFINEMENT_INPUT_BYTES {
        edits.push(ByteEdit {
            start: old_start + common_prefix,
            end: old_start + old_changed_end,
            replacement: new_changed.to_vec(),
        });
        return Ok(());
    }
    let old_tokens: Vec<&[u8]> = old_changed
        .iter()
        .enumerate()
        .map(|(i, _)| &old_changed[i..i + 1])
        .collect();
    let new_tokens: Vec<&[u8]> = new_changed
        .iter()
        .enumerate()
        .map(|(i, _)| &new_changed[i..i + 1])
        .collect();
    let byte_diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .diff_slices(&old_tokens, &new_tokens);
    let region_edits_start = edits.len();
    for op in byte_diff.ops() {
        if matches!(op, similar::DiffOp::Equal { .. }) {
            continue;
        }
        let local_old_start = op.old_range().start;
        let local_old_end = op.old_range().end;
        let local_new_start = op.new_range().start;
        let local_new_end = op.new_range().end;
        let repl = &new_changed[local_new_start..local_new_end];
        edits.push(ByteEdit {
            start: old_start + common_prefix + local_old_start,
            end: old_start + common_prefix + local_old_end,
            replacement: repl.to_vec(),
        });
    }

    // Verify whether the refined byte edits actually reconstruct `new` when applied to `old`.
    // Myers diff on raw byte slices can produce non-monotonic or misaligned edits on complex
    // hunks. If verification fails, safely fall back to replacing old_changed with new_changed.
    let local_slice_edits: Vec<ByteEdit> = edits[region_edits_start..]
        .iter()
        .map(|e| ByteEdit {
            start: e.start - old_start,
            end: e.end - old_start,
            replacement: e.replacement.clone(),
        })
        .collect();

    let local_applied = apply_byte_edits(old, &local_slice_edits);
    if local_applied.as_deref() != Ok(new) {
        edits.truncate(region_edits_start);
        edits.push(ByteEdit {
            start: old_start + common_prefix,
            end: old_start + old_changed_end,
            replacement: new_changed.to_vec(),
        });
    }
    Ok(())
}

#[allow(dead_code)]
fn _engine_error_is_exhaustive(error: EngineError) -> DiffPlannerError {
    DiffPlannerError::Invariant(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_has_no_regions() {
        let plan = plan(b"same\r\n", b"same\r\n").unwrap();
        assert!(plan.edits.is_empty());
        assert_eq!(plan.derived_region_count, 0);
    }

    #[test]
    fn long_line_distant_changes_stay_narrow_and_separate() {
        let observed = format!(
            "{}LEFT{}MIDDLE{}RIGHT",
            "a".repeat(400),
            "b".repeat(400),
            "c".repeat(400)
        );
        let desired = observed.replace("LEFT", "LFT").replace("RIGHT", "RITE");
        let plan = plan(observed.as_bytes(), desired.as_bytes()).unwrap();
        assert!(plan.derived_region_count >= 2);
        assert!(plan.edits.iter().all(|edit| edit.end - edit.start <= 5));
    }

    #[test]
    fn repeated_bytes_are_deterministic() {
        let observed = b"aaXXaaYYaa";
        let desired = b"aa11aa22aa";
        assert_eq!(plan(observed, desired), plan(observed, desired));
        assert_eq!(plan(observed, desired).unwrap().derived_region_count, 2);
    }

    #[test]
    fn empty_and_crlf_states_are_exact() {
        let inserted = plan(b"", b"a\r\nb\r\n").unwrap();
        assert_eq!(
            apply_byte_edits(b"", &inserted.edits).unwrap(),
            b"a\r\nb\r\n"
        );
        let removed = plan(b"a\r\nb\r\n", b"").unwrap();
        assert_eq!(
            apply_byte_edits(b"a\r\nb\r\n", &removed.edits).unwrap(),
            b""
        );
    }

    #[test]
    fn large_changed_region_uses_one_bounded_replacement() {
        let observed = format!("prefix{}suffix", "a".repeat(20 * 1024));
        let desired = format!("prefix{}suffix", "b".repeat(20 * 1024));
        let planned = plan(observed.as_bytes(), desired.as_bytes()).unwrap();

        assert_eq!(planned.edits.len(), 1);
        assert_eq!(planned.edits[0].start, "prefix".len());
        assert_eq!(planned.edits[0].end, "prefix".len() + 20 * 1024);
        assert_eq!(planned.edits[0].replacement.len(), 20 * 1024);
        assert_eq!(
            apply_byte_edits(observed.as_bytes(), &planned.edits).unwrap(),
            desired.as_bytes()
        );
    }

    #[test]
    fn common_prefix_and_suffix_are_excluded_from_refinement() {
        let observed = format!("{}old{}", "x".repeat(100_000), "y".repeat(100_000));
        let desired = format!("{}new{}", "x".repeat(100_000), "y".repeat(100_000));
        let planned = plan(observed.as_bytes(), desired.as_bytes()).unwrap();

        assert_eq!(
            planned.edits,
            vec![ByteEdit {
                start: 100_000,
                end: 100_003,
                replacement: b"new".to_vec(),
            }]
        );
    }

    #[test]
    fn long_line_tiny_edits_at_each_position_remain_narrow() {
        for (old, desired, expected_start) in [
            (
                format!("a{}", "x".repeat(100_000)),
                format!("b{}", "x".repeat(100_000)),
                0,
            ),
            (
                format!("{}a", "x".repeat(100_000)),
                format!("{}b", "x".repeat(100_000)),
                100_000,
            ),
            (
                format!("{}a{}", "x".repeat(50_000), "y".repeat(50_000)),
                format!("{}b{}", "x".repeat(50_000), "y".repeat(50_000)),
                50_000,
            ),
        ] {
            let planned = plan(old.as_bytes(), desired.as_bytes()).unwrap();
            assert_eq!(planned.edits.len(), 1);
            assert_eq!(planned.edits[0].start, expected_start);
            assert_eq!(
                apply_byte_edits(old.as_bytes(), &planned.edits).unwrap(),
                desired.as_bytes()
            );
        }
    }

    #[test]
    fn minified_json_javascript_crlf_and_bom_are_exact() {
        let json_old = br#"{"a":1,"b":2}"#;
        let json_new = br#"{"a":1,"b":3}"#;
        let json_plan = plan(json_old, json_new).unwrap();
        assert_eq!(
            apply_byte_edits(json_old, &json_plan.edits).unwrap(),
            json_new
        );

        let js_old = b"\xef\xbb\xbfconst x=1;\r\nconst y=2;\r\n";
        let js_new = b"\xef\xbb\xbfconst x=1;\r\nconst y=3;\r\n";
        let js_plan = plan(js_old, js_new).unwrap();
        assert_eq!(apply_byte_edits(js_old, &js_plan.edits).unwrap(), js_new);
    }
}
