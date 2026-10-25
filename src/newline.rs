use crate::engine::ByteEdit;
use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NewlineProfile {
    Lf,
    Crlf,
    Mixed,
    None,
}

impl NewlineProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::Crlf => "crlf",
            Self::Mixed => "mixed",
            Self::None => "none",
        }
    }

    pub fn parse_profile(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

impl std::str::FromStr for NewlineProfile {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lf" | "LF" => Ok(Self::Lf),
            "crlf" | "CRLF" => Ok(Self::Crlf),
            "mixed" => Ok(Self::Mixed),
            "none" => Ok(Self::None),
            _ => Err(()),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct NewlineCounts {
    pub lf: usize,
    pub crlf: usize,
    pub bare_cr: usize,
}

impl NewlineCounts {
    pub fn is_empty(&self) -> bool {
        self.lf == 0 && self.crlf == 0 && self.bare_cr == 0
    }

    pub fn profile(&self) -> NewlineProfile {
        let kinds =
            usize::from(self.crlf > 0) + usize::from(self.lf > 0) + usize::from(self.bare_cr > 0);
        match (kinds, self.crlf, self.lf, self.bare_cr) {
            (0, _, _, _) => NewlineProfile::None,
            (1, count, 0, 0) if count > 0 => NewlineProfile::Crlf,
            (1, 0, count, 0) if count > 0 => NewlineProfile::Lf,
            _ => NewlineProfile::Mixed,
        }
    }
}

/// Count LF, CRLF, and bare CR terminators across arbitrary bytes using memchr2.
pub fn count_newlines(bytes: &[u8]) -> NewlineCounts {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut bare_cr = 0usize;
    let mut index = 0usize;
    while let Some(rel) = memchr::memchr2(b'\r', b'\n', &bytes[index..]) {
        let pos = index + rel;
        if bytes[pos] == b'\r' {
            if bytes.get(pos + 1) == Some(&b'\n') {
                crlf += 1;
                index = pos + 2;
            } else {
                bare_cr += 1;
                index = pos + 1;
            }
        } else {
            lf += 1;
            index = pos + 1;
        }
    }
    NewlineCounts { lf, crlf, bare_cr }
}

/// Detect the newline profile of arbitrary bytes.
pub fn detect_profile(bytes: &[u8]) -> NewlineProfile {
    count_newlines(bytes).profile()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalConvention {
    Lf,
    Crlf,
    Ambiguous { details: String },
    None,
}

/// Deterministically infer the local newline convention of a region in a file.
/// If the region or surrounding context uniformly uses LF or CRLF, that convention is returned.
/// If immediately adjacent boundaries conflict, Ambiguous is returned rather than guessing.
pub fn infer_local_convention(bytes: &[u8], start: usize, end: usize) -> LocalConvention {
    if start < end && end <= bytes.len() {
        let inside = count_newlines(&bytes[start..end]);
        if inside.crlf > 0 && inside.lf == 0 && inside.bare_cr == 0 {
            return LocalConvention::Crlf;
        }
        if inside.lf > 0 && inside.crlf == 0 && inside.bare_cr == 0 {
            return LocalConvention::Lf;
        }
    }

    // Inspect preceding terminator
    let mut preceding = None;
    if start > 0 {
        let mut idx = start;
        while idx > 0 {
            idx -= 1;
            if bytes[idx] == b'\n' {
                if idx > 0 && bytes[idx - 1] == b'\r' {
                    preceding = Some(NewlineProfile::Crlf);
                } else {
                    preceding = Some(NewlineProfile::Lf);
                }
                break;
            }
        }
    }

    // Inspect succeeding terminator
    let mut succeeding = None;
    let mut idx = end;
    while idx < bytes.len() {
        if bytes[idx] == b'\n' {
            if idx > 0 && bytes[idx - 1] == b'\r' {
                succeeding = Some(NewlineProfile::Crlf);
            } else {
                succeeding = Some(NewlineProfile::Lf);
            }
            break;
        }
        idx += 1;
    }

    match (preceding, succeeding) {
        (Some(p), Some(s)) if p == s => match p {
            NewlineProfile::Crlf => LocalConvention::Crlf,
            _ => LocalConvention::Lf,
        },
        (Some(p), Some(s)) => LocalConvention::Ambiguous {
            details: format!(
                "preceding line ending is {} while succeeding line ending is {}; local convention cannot be inferred without guessing",
                p.as_str().to_uppercase(),
                s.as_str().to_uppercase()
            ),
        },
        (Some(p), None) => match p {
            NewlineProfile::Crlf => LocalConvention::Crlf,
            _ => LocalConvention::Lf,
        },
        (None, Some(s)) => match s {
            NewlineProfile::Crlf => LocalConvention::Crlf,
            _ => LocalConvention::Lf,
        },
        (None, None) => LocalConvention::None,
    }
}

/// Adapt replacement bytes to match the target newline profile.
/// Preserves any bare CR bytes as-is without stripping or synthesizing newlines.
/// Returns the adapted bytes and a boolean indicating whether adaptation occurred.
pub fn adapt_bytes_to_convention(bytes: &[u8], target: NewlineProfile) -> (Vec<u8>, bool) {
    match target {
        NewlineProfile::Lf => {
            if memchr::memmem::find(bytes, b"\r\n").is_some() {
                let mut out = Vec::with_capacity(bytes.len());
                let mut i = 0;
                let mut changed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\r' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            out.push(b'\n');
                            i += 2;
                            changed = true;
                        } else {
                            // Retain bare CR byte; never silently strip
                            out.push(b'\r');
                            i += 1;
                        }
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                (out, changed)
            } else {
                (bytes.to_vec(), false)
            }
        }
        NewlineProfile::Crlf => {
            let counts = count_newlines(bytes);
            if counts.lf > 0 {
                let mut out = Vec::with_capacity(bytes.len() + counts.lf);
                let mut i = 0;
                let mut changed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\r' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            out.push(b'\r');
                            out.push(b'\n');
                            i += 2;
                        } else {
                            // Retain bare CR byte; never normalize to CRLF
                            out.push(b'\r');
                            i += 1;
                        }
                    } else if bytes[i] == b'\n' {
                        out.push(b'\r');
                        out.push(b'\n');
                        i += 1;
                        changed = true;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                (out, changed)
            } else {
                (bytes.to_vec(), false)
            }
        }
        NewlineProfile::Mixed | NewlineProfile::None => (bytes.to_vec(), false),
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

pub fn extract_line_endings(bytes: &[u8]) -> Vec<LineEnding> {
    let mut endings = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                endings.push(LineEnding::Crlf);
            } else {
                endings.push(LineEnding::Lf);
            }
        }
        i += 1;
    }
    endings
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub content: &'a [u8],
    pub ending: Option<LineEnding>,
}

pub fn split_lines(bytes: &[u8]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > line_start && bytes[i - 1] == b'\r' {
                lines.push(Line {
                    content: &bytes[line_start..i - 1],
                    ending: Some(LineEnding::Crlf),
                });
            } else {
                lines.push(Line {
                    content: &bytes[line_start..i],
                    ending: Some(LineEnding::Lf),
                });
            }
            line_start = i + 1;
        }
        i += 1;
    }
    if line_start < bytes.len() {
        lines.push(Line {
            content: &bytes[line_start..],
            ending: None,
        });
    }
    lines
}

fn verify_retained_line_identity(
    orig_lines: &[Line<'_>],
    cand_lines: &[Line<'_>],
) -> Result<(), NewlineViolation> {
    use std::collections::HashMap;

    let mut orig_counts: HashMap<&[u8], (usize, usize)> = HashMap::new();
    for line in orig_lines {
        let entry = orig_counts.entry(line.content).or_insert((0, 0));
        match line.ending {
            Some(LineEnding::Lf) => entry.0 += 1,
            Some(LineEnding::Crlf) => entry.1 += 1,
            None => {}
        }
    }

    let mut cand_counts: HashMap<&[u8], (usize, usize)> = HashMap::new();
    for line in cand_lines {
        let entry = cand_counts.entry(line.content).or_insert((0, 0));
        match line.ending {
            Some(LineEnding::Lf) => entry.0 += 1,
            Some(LineEnding::Crlf) => entry.1 += 1,
            None => {}
        }
    }

    for (content, &(cand_lf, cand_crlf)) in &cand_counts {
        if let Some(&(orig_lf, orig_crlf)) = orig_counts.get(content) {
            if cand_crlf > orig_crlf && cand_lf < orig_lf {
                return Err(NewlineViolation::UnauthorisedConversion {
                    original: "mixed (line ending LF)",
                    result: "converted to CRLF",
                    details: format!(
                        "Unchanged retained line {:?} ending was converted from LF to CRLF without explicit conversion authority.",
                        String::from_utf8_lossy(content)
                    ),
                });
            }
            if cand_lf > orig_lf && cand_crlf < orig_crlf {
                return Err(NewlineViolation::UnauthorisedConversion {
                    original: "mixed (line ending CRLF)",
                    result: "converted to LF",
                    details: format!(
                        "Unchanged retained line {:?} ending was converted from CRLF to LF without explicit conversion authority.",
                        String::from_utf8_lossy(content)
                    ),
                });
            }
        }
    }

    Ok(())
}

pub fn replacement_matches_or_reduces_old_lines(
    old_lines: &[Line<'_>],
    rep_lines: &[Line<'_>],
) -> bool {
    if old_lines.is_empty() || rep_lines.is_empty() {
        return false;
    }

    for rep in rep_lines {
        for old in old_lines {
            if old.content == rep.content && old.ending != rep.ending {
                return false;
            }
        }
    }

    let old_contents: Vec<&[u8]> = old_lines.iter().map(|l| l.content).collect();
    let rep_contents: Vec<&[u8]> = rep_lines.iter().map(|l| l.content).collect();

    let diff =
        similar::capture_diff_slices(similar::Algorithm::Myers, &old_contents, &rep_contents);
    for op in diff {
        match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    if old_lines[old_index + k].ending != rep_lines[new_index + k].ending {
                        return false;
                    }
                }
            }
            similar::DiffOp::Delete { .. } => {}
            similar::DiffOp::Insert { .. } => return false,
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                if old_len == new_len {
                    for k in 0..old_len {
                        if old_lines[old_index + k].ending != rep_lines[new_index + k].ending {
                            return false;
                        }
                    }
                } else {
                    return false;
                }
            }
        }
    }

    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewlineViolation {
    UnauthorisedConversion {
        original: &'static str,
        result: &'static str,
        details: String,
    },
    BareCarriageReturn {
        details: String,
    },
    AmbiguousLocalConvention {
        offset: usize,
        details: String,
    },
    MixedNewlinesIntroduced {
        details: String,
    },
    SplitCrlfBoundary {
        offset: usize,
        details: String,
    },
}

impl NewlineViolation {
    pub fn message(&self) -> String {
        match self {
            Self::UnauthorisedConversion { details, .. } => details.clone(),
            Self::BareCarriageReturn { details } => details.clone(),
            Self::AmbiguousLocalConvention { details, .. } => details.clone(),
            Self::MixedNewlinesIntroduced { details } => details.clone(),
            Self::SplitCrlfBoundary { details, .. } => details.clone(),
        }
    }
}

/// Adapt prospective byte edits so that replacement bytes match the source newline conventions.
/// If an edit falls into an ambiguous region of a mixed file and introduces newlines, returns an error.
pub fn adapt_edits_for_preservation(
    original: &[u8],
    edits: &[ByteEdit],
    allow_conversion: bool,
) -> Result<(Vec<ByteEdit>, bool), NewlineViolation> {
    // 1. Guard against edits that begin or end inside a CRLF sequence in the original
    for edit in edits {
        if edit.start > 0
            && edit.start < original.len()
            && original[edit.start - 1] == b'\r'
            && original[edit.start] == b'\n'
        {
            return Err(NewlineViolation::SplitCrlfBoundary {
                offset: edit.start,
                details: format!(
                    "Edit at byte {} begins inside a CRLF sequence (between \\r and \\n)",
                    edit.start
                ),
            });
        }
        if edit.end > 0
            && edit.end < original.len()
            && original[edit.end - 1] == b'\r'
            && original[edit.end] == b'\n'
        {
            return Err(NewlineViolation::SplitCrlfBoundary {
                offset: edit.end,
                details: format!(
                    "Edit at byte {} ends inside a CRLF sequence (between \\r and \\n)",
                    edit.end
                ),
            });
        }
    }

    if allow_conversion {
        return Ok((edits.to_vec(), false));
    }

    let orig_profile = detect_profile(original);
    let mut adapted_edits = Vec::with_capacity(edits.len());
    let mut any_adapted = false;

    for edit in edits {
        let rep_counts = count_newlines(&edit.replacement);
        if rep_counts.bare_cr > 0 {
            return Err(NewlineViolation::BareCarriageReturn {
                details: format!(
                    "Replacement text for edit at byte {} contains {} bare carriage return(s); bare CR is refused by the newline firewall",
                    edit.start, rep_counts.bare_cr
                ),
            });
        }

        let (new_replacement, was_adapted) = match orig_profile {
            NewlineProfile::Lf => adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Lf),
            NewlineProfile::Crlf => {
                adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Crlf)
            }
            NewlineProfile::Mixed => {
                let old_slice = if edit.start <= edit.end && edit.end <= original.len() {
                    &original[edit.start..edit.end]
                } else {
                    b""
                };
                let old_endings = extract_line_endings(old_slice);
                let rep_endings = extract_line_endings(&edit.replacement);

                let old_lines = split_lines(old_slice);
                let rep_lines = split_lines(&edit.replacement);

                if !old_endings.is_empty()
                    && (old_endings == rep_endings
                        || replacement_matches_or_reduces_old_lines(&old_lines, &rep_lines))
                {
                    // Replacement preserves or legitimately reduces existing endings; no adaptation needed
                    (edit.replacement.clone(), false)
                } else {
                    let local = infer_local_convention(original, edit.start, edit.end);
                    match local {
                        LocalConvention::Lf => {
                            adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Lf)
                        }
                        LocalConvention::Crlf => {
                            adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Crlf)
                        }
                        LocalConvention::Ambiguous { details } => {
                            if rep_counts.lf > 0 || rep_counts.crlf > 0 {
                                return Err(NewlineViolation::AmbiguousLocalConvention {
                                    offset: edit.start,
                                    details: format!(
                                        "Refused edit at byte {}: {details}. Supply exact local line endings matching the intended line, or convert the entire file to a uniform convention with convert-newlines.",
                                        edit.start
                                    ),
                                });
                            }
                            (edit.replacement.clone(), false)
                        }
                        LocalConvention::None => (edit.replacement.clone(), false),
                    }
                }
            }
            NewlineProfile::None => {
                if rep_counts.crlf > 0 && rep_counts.lf > 0 {
                    adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Lf)
                } else {
                    (edit.replacement.clone(), false)
                }
            }
        };

        if was_adapted {
            any_adapted = true;
        }
        adapted_edits.push(ByteEdit {
            start: edit.start,
            end: edit.end,
            replacement: new_replacement,
        });
    }

    Ok((adapted_edits, any_adapted))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreservationCheck {
    pub conversion_authorized: bool,
    pub original_counts: NewlineCounts,
    pub candidate_counts: NewlineCounts,
}

/// The Core Newline Firewall gate.
/// Verifies that candidate bytes do not violate newline preservation contracts before commit.
pub fn validate_candidate_preservation(
    original: &[u8],
    candidate: &[u8],
    edits: &[ByteEdit],
    conversion_authorized: bool,
) -> Result<PreservationCheck, NewlineViolation> {
    for edit in edits {
        if edit.start > 0
            && edit.start < original.len()
            && original[edit.start - 1] == b'\r'
            && original[edit.start] == b'\n'
        {
            return Err(NewlineViolation::SplitCrlfBoundary {
                offset: edit.start,
                details: format!(
                    "Edit at byte {} begins inside a CRLF sequence (between \\r and \\n)",
                    edit.start
                ),
            });
        }
        if edit.end > 0
            && edit.end < original.len()
            && original[edit.end - 1] == b'\r'
            && original[edit.end] == b'\n'
        {
            return Err(NewlineViolation::SplitCrlfBoundary {
                offset: edit.end,
                details: format!(
                    "Edit at byte {} ends inside a CRLF sequence (between \\r and \\n)",
                    edit.end
                ),
            });
        }
    }

    let orig_counts = count_newlines(original);
    let cand_counts = count_newlines(candidate);
    let orig_profile = orig_counts.profile();
    let cand_profile = cand_counts.profile();

    // Invariant: No bare carriage returns may be introduced
    if cand_counts.bare_cr > orig_counts.bare_cr {
        return Err(NewlineViolation::BareCarriageReturn {
            details: format!(
                "Candidate introduces {} bare CR byte(s); bare carriage returns are refused by the newline firewall",
                cand_counts.bare_cr - orig_counts.bare_cr
            ),
        });
    }

    if conversion_authorized {
        if cand_profile == NewlineProfile::Mixed {
            return Err(NewlineViolation::MixedNewlinesIntroduced {
                details: "Intentional newline conversion resulted in a mixed newline profile; conversion must produce uniform LF or CRLF. Ensure all lines are converted, or use threadmoth convert-newlines <PATH> --to <lf|crlf>.".into(),
            });
        }
        return Ok(PreservationCheck {
            conversion_authorized: true,
            original_counts: orig_counts,
            candidate_counts: cand_counts,
        });
    }

    // Invariant: Uniform LF file must never become mixed or CRLF
    if orig_profile == NewlineProfile::Lf
        && (cand_profile == NewlineProfile::Crlf || cand_profile == NewlineProfile::Mixed)
    {
        return Err(NewlineViolation::UnauthorisedConversion {
            original: "lf",
            result: cand_profile.as_str(),
            details: format!(
                "Source file is uniformly LF ({} lines). Proposed mutation introduces {} CRLF line ending(s) without explicit conversion authority. Refusing to produce a mixed file.",
                orig_counts.lf,
                cand_counts.crlf
            ),
        });
    }

    // Invariant: Uniform CRLF file must never become mixed or LF
    if orig_profile == NewlineProfile::Crlf
        && (cand_profile == NewlineProfile::Lf || cand_profile == NewlineProfile::Mixed)
    {
        return Err(NewlineViolation::UnauthorisedConversion {
            original: "crlf",
            result: cand_profile.as_str(),
            details: format!(
                "Source file is uniformly CRLF ({} lines). Proposed mutation introduces {} lone LF line ending(s) without explicit conversion authority. Refusing to produce a mixed file.",
                orig_counts.crlf,
                cand_counts.lf
            ),
        });
    }

    // Invariant: File with no newlines must not have mixed newlines introduced
    if orig_profile == NewlineProfile::None && cand_profile == NewlineProfile::Mixed {
        return Err(NewlineViolation::MixedNewlinesIntroduced {
            details: "Source file had no line endings. Proposed mutation introduced mixed LF and CRLF line endings. Expected uniform line endings.".into(),
        });
    }

    // Invariant: In mixed files, enforce preservation at the level of retained source content and edits
    if orig_profile == NewlineProfile::Mixed {
        let orig_lines = split_lines(original);
        let cand_lines = split_lines(candidate);

        let orig_endings: Vec<LineEnding> = orig_lines.iter().filter_map(|l| l.ending).collect();
        let cand_endings: Vec<LineEnding> = cand_lines.iter().filter_map(|l| l.ending).collect();

        if orig_endings != cand_endings {
            verify_retained_line_identity(&orig_lines, &cand_lines)?;
        }

        for edit in edits {
            let old_slice = if edit.start <= edit.end && edit.end <= original.len() {
                &original[edit.start..edit.end]
            } else {
                b""
            };
            let old_endings = extract_line_endings(old_slice);
            let rep_endings = extract_line_endings(&edit.replacement);

            // 1. Identical line ending sequences: exact preservation
            if old_endings == rep_endings {
                continue;
            }

            // 2. Intra-line edit (no line endings in old_slice)
            if old_endings.is_empty() {
                if rep_endings.is_empty() {
                    continue;
                }
                // Inserting newlines within a line: must match local convention
                let local = infer_local_convention(original, edit.start, edit.end);
                match local {
                    LocalConvention::Lf => {
                        if rep_endings.contains(&LineEnding::Crlf) {
                            return Err(NewlineViolation::UnauthorisedConversion {
                                original: "mixed (local region LF)",
                                result: "mixed (introduced CRLF)",
                                details: format!(
                                    "Edit at byte {} is in a locally uniform LF region, but replacement contains CRLF endings.",
                                    edit.start
                                ),
                            });
                        }
                    }
                    LocalConvention::Crlf => {
                        if rep_endings.contains(&LineEnding::Lf) {
                            return Err(NewlineViolation::UnauthorisedConversion {
                                original: "mixed (local region CRLF)",
                                result: "mixed (introduced LF)",
                                details: format!(
                                    "Edit at byte {} is in a locally uniform CRLF region, but replacement contains lone LF endings.",
                                    edit.start
                                ),
                            });
                        }
                    }
                    LocalConvention::Ambiguous { details } => {
                        return Err(NewlineViolation::AmbiguousLocalConvention {
                            offset: edit.start,
                            details: format!(
                                "Edit at byte {} falls in an ambiguous mixed region: {}. Supply exact local line endings matching the intended line, or convert the entire file to a uniform convention with convert-newlines.",
                                edit.start, details
                            ),
                        });
                    }
                    LocalConvention::None => {}
                }
                continue;
            }

            // 3. Deleting lines without introducing any new line endings
            if rep_endings.is_empty() {
                continue;
            }

            // 4. Old region was uniformly CRLF
            if old_endings.iter().all(|&e| e == LineEnding::Crlf) {
                if rep_endings.contains(&LineEnding::Lf) {
                    return Err(NewlineViolation::UnauthorisedConversion {
                        original: "mixed (contains CRLF)",
                        result: "converted to LF",
                        details: format!(
                            "Edit at byte {} replaces CRLF line ending(s) with lone LF ending(s) without explicit conversion authority. Pass allow_newline_conversion to authorize line ending conversion, or use convert-newlines.",
                            edit.start
                        ),
                    });
                }
                continue;
            }

            // 5. Old region was uniformly LF
            if old_endings.iter().all(|&e| e == LineEnding::Lf) {
                if rep_endings.contains(&LineEnding::Crlf) {
                    return Err(NewlineViolation::UnauthorisedConversion {
                        original: "mixed (contains LF)",
                        result: "converted to CRLF",
                        details: format!(
                            "Edit at byte {} replaces lone LF line ending(s) with CRLF ending(s) without explicit conversion authority. Pass allow_newline_conversion to authorize line ending conversion, or use convert-newlines.",
                            edit.start
                        ),
                    });
                }
                continue;
            }

            // 6. Old region itself contained both LF and CRLF, but rep_endings != old_endings:
            let edit_old_lines = split_lines(old_slice);
            let edit_rep_lines = split_lines(&edit.replacement);
            if replacement_matches_or_reduces_old_lines(&edit_old_lines, &edit_rep_lines) {
                continue;
            }

            // Otherwise, the mapping between differing line endings across the boundary is ambiguous
            // or represents an unauthorized conversion of a retained ending.
            return Err(NewlineViolation::UnauthorisedConversion {
                original: "mixed (conflicting line endings)",
                result: "converted line endings",
                details: format!(
                    "Edit at byte {} modifies line endings across differing conventions without explicit conversion authority. Preserved endings must match the original lines exactly.",
                    edit.start
                ),
            });
        }
    }

    Ok(PreservationCheck {
        conversion_authorized: false,
        original_counts: orig_counts,
        candidate_counts: cand_counts,
    })
}

/// Discover project newline policy for a specific file path from .editorconfig or .gitattributes.
/// Falls back deterministically to LF. No operating-system-dependent newline behaviour.
pub fn discover_file_newline_policy(workspace_path: &Path, file_path: &str) -> NewlineProfile {
    let normalized = file_path.replace('\\', "/");
    let clean_path = normalized.strip_prefix("./").unwrap_or(&normalized);
    let file_name = clean_path.rsplit('/').next().unwrap_or(clean_path);

    // 1. Check .editorconfig
    if let Some(profile) = query_editorconfig_policy(workspace_path, clean_path, file_name) {
        return profile;
    }

    // 2. Check .gitattributes
    if let Some(profile) = query_gitattributes_policy(workspace_path, clean_path, file_name) {
        return profile;
    }

    // 3. Deterministic default: LF
    NewlineProfile::Lf
}

pub fn discover_project_newline_policy(workspace_path: &Path) -> Option<NewlineProfile> {
    Some(discover_file_newline_policy(workspace_path, "*"))
}

fn query_editorconfig_policy(
    workspace_path: &Path,
    rel_path: &str,
    file_name: &str,
) -> Option<NewlineProfile> {
    let path = workspace_path.join(".editorconfig");
    let content = std::fs::read_to_string(&path).ok()?;

    let mut current_matches = false;
    let mut selected_policy = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let pattern = &trimmed[1..trimmed.len() - 1];
            current_matches = match_glob_pattern(pattern, rel_path, file_name);
            continue;
        }

        if current_matches {
            if let Some((key, val)) = trimmed.split_once('=') {
                if key.trim().eq_ignore_ascii_case("end_of_line") {
                    let v = val.trim().to_ascii_lowercase();
                    if v == "lf" {
                        selected_policy = Some(NewlineProfile::Lf);
                    } else if v == "crlf" {
                        selected_policy = Some(NewlineProfile::Crlf);
                    }
                }
            }
        }
    }

    selected_policy
}

fn query_gitattributes_policy(
    workspace_path: &Path,
    rel_path: &str,
    file_name: &str,
) -> Option<NewlineProfile> {
    let path = workspace_path.join(".gitattributes");
    let content = std::fs::read_to_string(&path).ok()?;

    let mut selected_policy = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };

        if match_glob_pattern(pattern, rel_path, file_name) {
            for attr in parts {
                if attr.eq_ignore_ascii_case("eol=crlf") {
                    selected_policy = Some(NewlineProfile::Crlf);
                } else if attr.eq_ignore_ascii_case("eol=lf") {
                    selected_policy = Some(NewlineProfile::Lf);
                }
            }
        }
    }

    selected_policy
}

fn match_glob_pattern(pattern: &str, rel_path: &str, file_name: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }
    if pattern == rel_path || pattern == file_name {
        return true;
    }
    let Ok(re) = glob_to_regex(pattern) else {
        return false;
    };
    re.is_match(rel_path) || re.is_match(file_name)
}

fn glob_to_regex(glob: &str) -> Result<regex::Regex, regex::Error> {
    let mut regex_str = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        regex_str.push_str("(?:.*/)?");
                    } else {
                        regex_str.push_str(".*");
                    }
                } else {
                    regex_str.push_str("[^/]*");
                }
            }
            '?' => regex_str.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '^' | '$' | '|' | '[' | ']' | '\\' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            '{' => {
                regex_str.push_str("(?:");
            }
            '}' => {
                regex_str.push(')');
            }
            ',' => {
                regex_str.push('|');
            }
            other => regex_str.push(other),
        }
    }
    regex_str.push('$');
    regex::Regex::new(&regex_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_and_profile_detection() {
        assert_eq!(detect_profile(b"hello world"), NewlineProfile::None);
        assert_eq!(detect_profile(b"hello\nworld\n"), NewlineProfile::Lf);
        assert_eq!(detect_profile(b"hello\r\nworld\r\n"), NewlineProfile::Crlf);
        assert_eq!(detect_profile(b"hello\nworld\r\n"), NewlineProfile::Mixed);
        assert_eq!(detect_profile(b"hello\rworld"), NewlineProfile::Mixed);

        let counts = count_newlines(b"one\r\ntwo\nthree\rfour\r\n");
        assert_eq!(counts.crlf, 2);
        assert_eq!(counts.lf, 1);
        assert_eq!(counts.bare_cr, 1);
        assert_eq!(counts.profile(), NewlineProfile::Mixed);
    }

    #[test]
    fn adaptation_preserves_uniform_files() {
        let crlf_input = b"line1\r\nline2\r\n";
        let (lf_adapted, changed) = adapt_bytes_to_convention(crlf_input, NewlineProfile::Lf);
        assert!(changed);
        assert_eq!(lf_adapted, b"line1\nline2\n");

        let lf_input = b"line1\nline2\n";
        let (crlf_adapted, changed) = adapt_bytes_to_convention(lf_input, NewlineProfile::Crlf);
        assert!(changed);
        assert_eq!(crlf_adapted, b"line1\r\nline2\r\n");

        // Already uniform input undergoes no change
        let (same, changed) = adapt_bytes_to_convention(lf_input, NewlineProfile::Lf);
        assert!(!changed);
        assert_eq!(same, lf_input);
    }

    #[test]
    fn local_convention_inference_in_mixed_file() {
        let content = b"first\nsecond\nthird\r\nfourth\r\n";
        // Offset in the LF portion (first line)
        assert_eq!(infer_local_convention(content, 0, 5), LocalConvention::Lf);
        // Offset in the CRLF portion (fourth line)
        assert_eq!(
            infer_local_convention(content, 20, 26),
            LocalConvention::Crlf
        );
        // Offset at the boundary between LF and CRLF
        let boundary_idx = 13; // after 'second\n' and before 'third\r\n'
        match infer_local_convention(content, boundary_idx, boundary_idx) {
            LocalConvention::Ambiguous { .. } => {}
            other => panic!("Expected Ambiguous at boundary, got {other:?}"),
        }
    }

    #[test]
    fn core_firewall_blocks_unauthorized_conversion() {
        let original = b"fn main() {\n    println!(\"hello\");\n}\n";
        // Candidate has CRLF injected into uniform LF file
        let bad_candidate = b"fn main() {\n    println!(\"hello\");\r\n}\n";
        let edits = vec![ByteEdit {
            start: 16,
            end: 35,
            replacement: b"println!(\"hello\");\r\n".to_vec(),
        }];

        let result = validate_candidate_preservation(original, bad_candidate, &edits, false);
        assert!(matches!(
            result,
            Err(NewlineViolation::UnauthorisedConversion { .. })
        ));

        // With conversion authorized, it is allowed
        let uniform_crlf_candidate = b"fn main() {\r\n    println!(\"hello\");\r\n}\r\n";
        let result =
            validate_candidate_preservation(original, uniform_crlf_candidate, &edits, true);
        assert!(result.is_ok());
    }

    #[test]
    fn core_firewall_rejects_bare_carriage_return() {
        let original = b"hello\nworld\n";
        let candidate = b"hello\rworld\n";
        let edits = vec![ByteEdit {
            start: 5,
            end: 6,
            replacement: b"\r".to_vec(),
        }];
        let result = validate_candidate_preservation(original, candidate, &edits, false);
        assert!(matches!(
            result,
            Err(NewlineViolation::BareCarriageReturn { .. })
        ));
    }
}
