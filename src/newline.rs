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
/// Returns the adapted bytes and a boolean indicating whether adaptation occurred.
pub fn adapt_bytes_to_convention(bytes: &[u8], target: NewlineProfile) -> (Vec<u8>, bool) {
    match target {
        NewlineProfile::Lf => {
            if memchr::memmem::find(bytes, b"\r\n").is_some()
                || memchr::memchr(b'\r', bytes).is_some()
            {
                let mut out = Vec::with_capacity(bytes.len());
                let mut i = 0;
                let mut changed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\r' {
                        changed = true;
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            out.push(b'\n');
                            i += 2;
                        } else {
                            // Strip bare CR
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
            // First check if adaptation is needed: any lone LF or bare CR
            let counts = count_newlines(bytes);
            if counts.lf > 0 || counts.bare_cr > 0 {
                let mut out = Vec::with_capacity(bytes.len() + counts.lf);
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i] == b'\r' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                            out.push(b'\r');
                            out.push(b'\n');
                            i += 2;
                        } else {
                            // Bare CR -> normalize to CRLF
                            out.push(b'\r');
                            out.push(b'\n');
                            i += 1;
                        }
                    } else if bytes[i] == b'\n' {
                        out.push(b'\r');
                        out.push(b'\n');
                        i += 1;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                (out, true)
            } else {
                (bytes.to_vec(), false)
            }
        }
        NewlineProfile::Mixed | NewlineProfile::None => (bytes.to_vec(), false),
    }
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
}

impl NewlineViolation {
    pub fn message(&self) -> String {
        match self {
            Self::UnauthorisedConversion { details, .. } => details.clone(),
            Self::BareCarriageReturn { details } => details.clone(),
            Self::AmbiguousLocalConvention { details, .. } => details.clone(),
            Self::MixedNewlinesIntroduced { details } => details.clone(),
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
    if allow_conversion {
        return Ok((edits.to_vec(), false));
    }

    let orig_profile = detect_profile(original);
    let mut adapted_edits = Vec::with_capacity(edits.len());
    let mut any_adapted = false;

    for edit in edits {
        let (new_replacement, was_adapted) = match orig_profile {
            NewlineProfile::Lf => adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Lf),
            NewlineProfile::Crlf => {
                adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Crlf)
            }
            NewlineProfile::Mixed => {
                let local = infer_local_convention(original, edit.start, edit.end);
                match local {
                    LocalConvention::Lf => {
                        adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Lf)
                    }
                    LocalConvention::Crlf => {
                        adapt_bytes_to_convention(&edit.replacement, NewlineProfile::Crlf)
                    }
                    LocalConvention::Ambiguous { details } => {
                        let rep_counts = count_newlines(&edit.replacement);
                        if rep_counts.lf > 0 || rep_counts.crlf > 0 || rep_counts.bare_cr > 0 {
                            return Err(NewlineViolation::AmbiguousLocalConvention {
                                offset: edit.start,
                                details: format!(
                                    "Refused edit at byte {}: {details}. Set allow_newline_conversion to override or supply exact local line endings.",
                                    edit.start
                                ),
                            });
                        }
                        (edit.replacement.clone(), false)
                    }
                    LocalConvention::None => (edit.replacement.clone(), false),
                }
            }
            NewlineProfile::None => {
                // For files with no newlines, preserve uniform LF as deterministic default if replacement introduces lines
                let rep_counts = count_newlines(&edit.replacement);
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
                details: "Intentional newline conversion resulted in a mixed newline profile; conversion must produce uniform LF or CRLF".into(),
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

    // Invariant: In mixed files, edits in locally uniform regions must not introduce conflicting endings
    if orig_profile == NewlineProfile::Mixed && cand_profile == NewlineProfile::Mixed {
        for edit in edits {
            let local = infer_local_convention(original, edit.start, edit.end);
            match local {
                LocalConvention::Lf => {
                    let rep_counts = count_newlines(&edit.replacement);
                    if rep_counts.crlf > 0 {
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
                    let rep_counts = count_newlines(&edit.replacement);
                    if rep_counts.lf > 0 {
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
                    let rep_counts = count_newlines(&edit.replacement);
                    if rep_counts.lf > 0 || rep_counts.crlf > 0 {
                        return Err(NewlineViolation::AmbiguousLocalConvention {
                            offset: edit.start,
                            details: format!(
                                "Edit at byte {} falls in an ambiguous mixed region: {}",
                                edit.start, details
                            ),
                        });
                    }
                }
                LocalConvention::None => {}
            }
        }
    }

    Ok(PreservationCheck {
        conversion_authorized: false,
        original_counts: orig_counts,
        candidate_counts: cand_counts,
    })
}

/// Discover project newline policy from .editorconfig or .gitattributes if available in workspace.
pub fn discover_project_newline_policy(workspace_path: &Path) -> Option<NewlineProfile> {
    let editorconfig = workspace_path.join(".editorconfig");
    if let Ok(content) = std::fs::read_to_string(&editorconfig) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("end_of_line") {
                if let Some((_, val)) = trimmed.split_once('=') {
                    let v = val.trim().to_lowercase();
                    if v == "lf" {
                        return Some(NewlineProfile::Lf);
                    } else if v == "crlf" {
                        return Some(NewlineProfile::Crlf);
                    }
                }
            }
        }
    }

    let gitattributes = workspace_path.join(".gitattributes");
    if let Ok(content) = std::fs::read_to_string(&gitattributes) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.contains("eol=lf") {
                return Some(NewlineProfile::Lf);
            } else if trimmed.contains("eol=crlf") {
                return Some(NewlineProfile::Crlf);
            }
        }
    }

    None
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
