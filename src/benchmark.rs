use crate::pipeline::execute_request;
use crate::presentation::{human_bytes, human_duration_us, rule};
use crate::protocol::{
    Cardinality, DesiredStateOperation, EffectBudget, OperationPayload, Outcome, Request,
    PROTOCOL_VERSION,
};
use crate::provider::text::TextOperation;
use crate::workspace::Workspace;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Quick,
    Standard,
    Tough,
}

impl Profile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "quick" => Some(Self::Quick),
            "standard" => Some(Self::Standard),
            "tough" => Some(Self::Tough),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Tough => "tough",
        }
    }

    fn cases(self) -> Vec<CaseSpec> {
        let mut cases = vec![
            CaseSpec::new("tiny", 21, 25),
            CaseSpec::new("config_30k", 30_008, 25),
            CaseSpec::new("text_1m", 1_000_008, 25),
        ];
        match self {
            Self::Quick => {
                for case in &mut cases {
                    case.iterations = 3;
                }
            }
            Self::Standard => cases.push(CaseSpec::new("text_5m", 5_000_008, 5)),
            Self::Tough => {
                cases = vec![
                    CaseSpec::new("tiny", 21, 100),
                    CaseSpec::new("config_30k", 30_008, 50),
                    CaseSpec::new("text_1m", 1_000_008, 25),
                    CaseSpec::new("text_5m", 5_000_008, 10),
                    CaseSpec::new("text_32m", 32_000_008, 3),
                    CaseSpec::new("many_lines_2m", 2_000_000, 10),
                    CaseSpec::new("long_line_refinement_2m", 2_000_008, 5),
                    CaseSpec::new("small_files_250", 2_500, 1),
                ];
            }
        }
        cases
    }
}

#[derive(Clone, Debug)]
struct CaseSpec {
    name: &'static str,
    bytes: usize,
    iterations: usize,
}

impl CaseSpec {
    const fn new(name: &'static str, bytes: usize, iterations: usize) -> Self {
        Self {
            name,
            bytes,
            iterations,
        }
    }
}

#[derive(Serialize)]
struct CaseResult {
    name: &'static str,
    bytes: usize,
    iterations: usize,
    wrong_applied: usize,
    average_us: f64,
    certificate_bytes: usize,
    state: &'static str,
}

#[derive(Serialize)]
struct Report {
    tool: &'static str,
    profile: &'static str,
    state: &'static str,
    cases: Vec<CaseResult>,
    wrong_applied: usize,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("threadmoth-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn request(path: &str) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "release-benchmark".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::Text(TextOperation::Replace {
            target: "FINDME".into(),
            replacement: "FOUND".into(),
        }),
    }
}

fn refinement_request(path: &str, desired_bytes: Vec<u8>) -> Request {
    Request {
        version: PROTOCOL_VERSION.into(),
        request_id: "release-benchmark-long-line-refinement".into(),
        allow_generated: false,
        allow_newline_conversion: false,
        file_path: path.into(),
        namespace: Default::default(),
        expected_pre_hash: None,
        region_guard: None,
        candidate_guard: None,
        cardinality: Cardinality::ExactlyOne,
        budget: EffectBudget::default(),
        operation: OperationPayload::DesiredState(DesiredStateOperation::Replace { desired_bytes }),
    }
}

fn payload(case: &CaseSpec) -> Vec<u8> {
    if case.name == "tiny" {
        return b"prefix FINDME suffix\n".to_vec();
    }
    if case.name == "many_lines_2m" {
        let mut bytes = Vec::with_capacity(case.bytes);
        while bytes.len() + 16 < case.bytes {
            bytes.extend_from_slice(b"line padding\n");
        }
        bytes.extend_from_slice(b"FINDME\n");
        return bytes;
    }
    if case.name == "long_line_refinement_2m" {
        let mut bytes = vec![b'x'; case.bytes.saturating_sub(15)];
        bytes.extend_from_slice(b" FINDME tail\n");
        return bytes;
    }
    let mut bytes = vec![b'a'; case.bytes.saturating_sub(8)];
    bytes.extend_from_slice(b"\nFINDME\n");
    bytes
}

fn run_case(workspace: &Workspace, root: &std::path::Path, case: &CaseSpec) -> CaseResult {
    let path = format!("{}.txt", case.name);
    let bytes = payload(case);
    fs::write(root.join(&path), &bytes).expect("benchmark fixture write failed");
    let request = if case.name == "long_line_refinement_2m" {
        refinement_request(&path, vec![b'y'; bytes.len()])
    } else {
        request(&path)
    };
    let start = Instant::now();
    let mut wrong_applied = 0;
    let mut certificate_bytes = 0;
    for _ in 0..case.iterations {
        let certificate = execute_request(workspace, &request, true);
        if certificate.outcome != Outcome::Applied || certificate.post_hash.is_none() {
            wrong_applied += 1;
        }
        certificate_bytes = serde_json::to_vec(&certificate)
            .expect("benchmark certificate serialization failed")
            .len();
    }
    CaseResult {
        name: case.name,
        bytes: bytes.len(),
        iterations: case.iterations,
        wrong_applied,
        average_us: start.elapsed().as_secs_f64() * 1_000_000.0 / case.iterations as f64,
        certificate_bytes,
        state: if wrong_applied == 0 { "PASS" } else { "FAIL" },
    }
}

fn run_small_files(workspace: &Workspace, root: &std::path::Path) -> CaseResult {
    let count = 250;
    let start = Instant::now();
    let mut wrong_applied = 0;
    let request = request("small-files/file.txt");
    fs::create_dir_all(root.join("small-files")).expect("small-file fixture directory failed");
    for index in 0..count {
        let path = root.join("small-files/file.txt");
        fs::write(&path, format!("file {index} FINDME\n"))
            .expect("small-file fixture write failed");
        let certificate = execute_request(workspace, &request, true);
        if certificate.outcome != Outcome::Applied || certificate.post_hash.is_none() {
            wrong_applied += 1;
        }
    }
    CaseResult {
        name: "small_files_250",
        bytes: count * 10,
        iterations: count,
        wrong_applied,
        average_us: start.elapsed().as_secs_f64() * 1_000_000.0 / count as f64,
        certificate_bytes: serde_json::to_vec(&execute_request(workspace, &request, true))
            .expect("benchmark certificate serialization failed")
            .len(),
        state: if wrong_applied == 0 { "PASS" } else { "FAIL" },
    }
}

fn print_human_report(report: &Report) {
    println!("THREADMOTH BENCHMARK");
    println!("{}", rule());
    println!("Profile     {}", report.profile);
    println!("Version     {}", env!("CARGO_PKG_VERSION"));
    println!("Mode        dry-run + correctness checked");
    println!();
    println!(
        "{:<20} {:>10} {:>7} {:>12} {:>7} {:>8}",
        "Case", "Size", "Runs", "Average", "Wrong", "Result"
    );
    println!("{}", rule());
    for result in &report.cases {
        println!(
            "{:<20} {:>10} {:>7} {:>12} {:>7} {:>8}",
            result.name,
            human_bytes(result.bytes),
            result.iterations,
            human_duration_us(result.average_us),
            result.wrong_applied,
            result.state,
        );
    }
    println!("{}", rule());
    let passed = report
        .cases
        .iter()
        .filter(|result| result.state == "PASS")
        .count();
    println!(
        "{}  {}/{} cases · {} wrong mutations · correctness checked",
        report.state,
        passed,
        report.cases.len(),
        report.wrong_applied,
    );
}

pub fn run(profile: Profile, json: bool) -> i32 {
    let root = match TempRoot::new("benchmark") {
        Ok(root) => root,
        Err(error) => {
            eprintln!("benchmark setup failed: {error}");
            return 3;
        }
    };
    let workspace = match Workspace::new(&root.0) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("benchmark workspace setup failed: {error}");
            return 3;
        }
    };
    let mut results = Vec::new();
    for case in profile.cases() {
        let result = if case.name == "small_files_250" {
            run_small_files(&workspace, &root.0)
        } else {
            run_case(&workspace, &root.0, &case)
        };
        results.push(result);
    }
    let wrong_applied = results.iter().map(|result| result.wrong_applied).sum();
    let state = if wrong_applied == 0 { "PASS" } else { "FAIL" };
    let report = Report {
        tool: "threadmoth benchmark",
        profile: profile.name(),
        state,
        cases: results,
        wrong_applied,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print_human_report(&report);
    }
    i32::from(wrong_applied != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_profile_still_has_three_cases() {
        assert_eq!(Profile::Quick.cases().len(), 3);
    }

    #[test]
    fn profile_parser_keeps_legacy_values() {
        assert_eq!(Profile::parse("quick"), Some(Profile::Quick));
        assert_eq!(Profile::parse("standard"), Some(Profile::Standard));
        assert_eq!(Profile::parse("tough"), Some(Profile::Tough));
        assert_eq!(Profile::parse("unknown"), None);
    }
}
