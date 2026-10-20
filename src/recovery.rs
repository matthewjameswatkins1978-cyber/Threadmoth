#![forbid(unsafe_code)]

use crate::engine::compute_sha256;
use crate::protocol::SUPPORTED_PROTOCOL_VERSIONS;
use crate::workspace::{Workspace, WorkspaceError};
use atomic_write_file::AtomicWriteFile;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};

#[doc(hidden)]
pub mod fault_injection {
    use std::cell::Cell;

    thread_local! {
        static FAIL_COMMIT_MARKER: Cell<bool> = const { Cell::new(false) };
        static FAIL_REMOVE_JOURNAL: Cell<bool> = const { Cell::new(false) };
    }

    pub fn set_fail_commit_marker(fail: bool) {
        FAIL_COMMIT_MARKER.with(|c| c.set(fail));
    }

    pub fn fail_commit_marker() -> bool {
        FAIL_COMMIT_MARKER.with(|c| c.get())
            || std::env::var_os("THREADMOTH_FAULT_COMMIT_MARKER").is_some()
    }

    pub fn set_fail_remove_journal(fail: bool) {
        FAIL_REMOVE_JOURNAL.with(|c| c.set(fail));
    }

    pub fn fail_remove_journal() -> bool {
        FAIL_REMOVE_JOURNAL.with(|c| c.get())
            || std::env::var_os("THREADMOTH_FAULT_REMOVE_JOURNAL").is_some()
    }

    pub fn reset() {
        FAIL_COMMIT_MARKER.with(|c| c.set(false));
        FAIL_REMOVE_JOURNAL.with(|c| c.set(false));
    }
}

const RECOVERY_DIR: &str = ".threadmoth-recovery";
const LEGACY_RECOVERY_DIR: &str = ".suture-recovery";
pub(crate) const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_JOURNAL_ENTRIES: usize = 256;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JournalPhase {
    Prepared,
    Committed,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Journal {
    pub protocol_version: String,
    pub transaction_id: String,
    pub entries: Vec<JournalEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<JournalPhase>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JournalEntry {
    pub path: String,
    pub pre_hash: String,
    pub candidate_hash: String,
    // New journals use compact base64 strings. The custom deserializer also
    // accepts the v1.5.1 decimal arrays so upgrades remain recoverable.
    #[serde(with = "base64_bytes")]
    pub original: Vec<u8>,
    #[serde(with = "base64_bytes")]
    pub candidate: Vec<u8>,
}

fn serialize_journal(journal: &Journal) -> Result<Vec<u8>, WorkspaceError> {
    let bytes =
        serde_json::to_vec(journal).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(WorkspaceError::ResourceLimit {
            dimension: "max_journal_bytes".into(),
            limit: MAX_JOURNAL_BYTES,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

pub(crate) fn check_journal_size(journal: &Journal) -> Result<(), WorkspaceError> {
    serialize_journal(journal).map(|_| ())
}

pub fn write_journal(workspace: &Workspace, journal: &Journal) -> Result<(), WorkspaceError> {
    let mut j = journal.clone();
    if j.phase.is_none() {
        j.phase = Some(JournalPhase::Prepared);
    }
    let bytes = serialize_journal(&j)?;
    let dir = workspace.root().join(RECOVERY_DIR);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", safe_id(&journal.transaction_id)));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn mark_journal_committed(
    workspace: &Workspace,
    transaction_id: &str,
) -> Result<(), WorkspaceError> {
    if fault_injection::fail_commit_marker() {
        return Err(WorkspaceError::Io(io::Error::other(
            "fault injected: mark_journal_committed failure",
        )));
    }
    let filename = format!("{}.json", safe_id(transaction_id));
    for dir in recovery_dirs(workspace) {
        let path = dir.join(&filename);
        if path.exists() {
            let bytes = fs::read(&path)?;
            let mut journal: Journal = serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            journal.phase = Some(JournalPhase::Committed);
            let updated_bytes = serialize_journal(&journal)?;

            // Staged atomic replacement: write to a temporary file in the same directory,
            // then atomically rename over the original path.
            // This guarantees the original journal is never truncated or destroyed if interrupted.
            let mut file = AtomicWriteFile::open(&path)?;
            std::io::Write::write_all(&mut file, &updated_bytes)?;
            file.flush()?;
            file.commit()?;
            return Ok(());
        }
    }
    Err(WorkspaceError::NotFound(format!(
        "journal for transaction {transaction_id} not found"
    )))
}

pub fn remove_journal(workspace: &Workspace, transaction_id: &str) -> Result<(), WorkspaceError> {
    if fault_injection::fail_remove_journal() {
        return Err(WorkspaceError::Io(io::Error::other(
            "fault injected: remove_journal failure",
        )));
    }
    let filename = format!("{}.json", safe_id(transaction_id));
    for dir in recovery_dirs(workspace) {
        match fs::remove_file(dir.join(&filename)) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    for dir in recovery_dirs(workspace) {
        remove_recovery_dir_if_empty(&dir);
    }
    Ok(())
}

fn recovery_dirs(workspace: &Workspace) -> [std::path::PathBuf; 2] {
    [
        workspace.root().join(RECOVERY_DIR),
        workspace.root().join(LEGACY_RECOVERY_DIR),
    ]
}

fn remove_recovery_dir_if_empty(dir: &std::path::Path) {
    // This is deliberately best-effort. A non-empty directory means another
    // transaction or a manual-recovery journal still needs it; remove_dir then
    // fails safely without disturbing that state.
    let _ = fs::remove_dir(dir);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryState {
    Original,
    Candidate,
    Manual,
}

pub fn recover_all(workspace: &Workspace) -> RecoveryReport {
    let mut report = RecoveryReport {
        inspected: 0,
        completed: 0,
        restored: 0,
        cleaned: 0,
        manual: Vec::new(),
    };
    let _lock = match workspace.acquire_mutation_lock() {
        Ok(lock) => lock,
        Err(error) => {
            report.manual.push(format!("WORKSPACE_BUSY: {error}"));
            return report;
        }
    };
    for dir in recovery_dirs(workspace) {
        recover_dir(workspace, &dir, &mut report);
    }
    report
}

#[derive(Serialize, Clone, Debug)]
pub struct RecoveryListReport {
    pub entries: Vec<RecoveryListEntry>,
}

#[derive(Serialize, Clone, Debug)]
pub struct RecoveryListEntry {
    pub transaction_id: String,
    pub journal_validity: String,
    pub member_count: usize,
    pub apparent_state: String,
    pub automatic_recovery_safe: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct RecoveryInspectReport {
    pub transaction_id: String,
    pub journal_validity: String,
    pub member_count: usize,
    pub members: Vec<RecoveryMemberInspection>,
    pub automatic_recovery_safe: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct RecoveryMemberInspection {
    pub path: String,
    pub pre_hash: String,
    pub candidate_hash: String,
    pub current_hash: Option<String>,
    pub classification: String,
    pub automatic_recovery_safe: bool,
}

pub fn list(workspace: &Workspace) -> RecoveryListReport {
    let mut entries = Vec::new();
    for path in journal_paths(workspace) {
        let transaction_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("unreadable")
            .to_owned();
        let journal = match validate_journal(workspace, &path) {
            Ok(journal) => journal,
            Err(_) => {
                entries.push(RecoveryListEntry {
                    transaction_id,
                    journal_validity: if path.exists() {
                        "invalid"
                    } else {
                        "unreadable"
                    }
                    .into(),
                    member_count: 0,
                    apparent_state: "invalid".into(),
                    automatic_recovery_safe: false,
                });
                continue;
            }
        };
        let states = member_inspections(workspace, &journal);
        let safe = states.iter().all(|member| member.automatic_recovery_safe);
        let all_candidate = states
            .iter()
            .all(|member| member.classification == "CANDIDATE");
        let all_original = states
            .iter()
            .all(|member| member.classification == "ORIGINAL");
        let (apparent_state, automatic_safe) = if !safe {
            ("drifted".to_string(), false)
        } else {
            match journal.phase {
                Some(JournalPhase::Committed) => {
                    if all_candidate {
                        ("complete".to_string(), true)
                    } else {
                        ("inconsistent".to_string(), false)
                    }
                }
                Some(JournalPhase::Prepared) => {
                    if all_original || all_candidate {
                        ("uncommitted".to_string(), true)
                    } else {
                        ("partial".to_string(), true)
                    }
                }
                None => {
                    if all_original {
                        ("uncommitted".to_string(), true)
                    } else if all_candidate {
                        ("ambiguous".to_string(), false)
                    } else {
                        ("partial".to_string(), true)
                    }
                }
            }
        };
        entries.push(RecoveryListEntry {
            transaction_id,
            journal_validity: "valid".into(),
            member_count: states.len(),
            apparent_state,
            automatic_recovery_safe: automatic_safe,
        });
    }
    RecoveryListReport { entries }
}

pub fn inspect(workspace: &Workspace, transaction_id: &str) -> RecoveryInspectReport {
    let Some(path) = find_journal(workspace, transaction_id) else {
        return RecoveryInspectReport {
            transaction_id: transaction_id.into(),
            journal_validity: "unreadable".into(),
            member_count: 0,
            members: Vec::new(),
            automatic_recovery_safe: false,
        };
    };
    let journal = match validate_journal(workspace, &path) {
        Ok(journal) => journal,
        Err(_) => {
            return RecoveryInspectReport {
                transaction_id: transaction_id.into(),
                journal_validity: "invalid".into(),
                member_count: 0,
                members: Vec::new(),
                automatic_recovery_safe: false,
            }
        }
    };
    let members = member_inspections(workspace, &journal);
    let all_candidate = members.iter().all(|m| m.classification == "CANDIDATE");
    let safe = members.iter().all(|member| member.automatic_recovery_safe);
    let automatic_recovery_safe = safe
        && match journal.phase {
            Some(JournalPhase::Committed) => all_candidate,
            Some(JournalPhase::Prepared) => true,
            None if all_candidate => false,
            _ => true,
        };
    RecoveryInspectReport {
        transaction_id: transaction_id.into(),
        journal_validity: "valid".into(),
        member_count: members.len(),
        automatic_recovery_safe,
        members,
    }
}

pub fn recover_transaction(workspace: &Workspace, transaction_id: &str) -> RecoveryReport {
    let mut report = RecoveryReport {
        inspected: 0,
        completed: 0,
        restored: 0,
        cleaned: 0,
        manual: Vec::new(),
    };
    let _lock = match workspace.acquire_mutation_lock() {
        Ok(lock) => lock,
        Err(error) => {
            report.manual.push(format!("WORKSPACE_BUSY: {error}"));
            return report;
        }
    };
    if let Some(path) = find_journal(workspace, transaction_id) {
        report.inspected = 1;
        recover_item(workspace, &path, &mut report);
    } else {
        report
            .manual
            .push(format!("transaction not found: {transaction_id}"));
    }
    report
}

fn recover_dir(workspace: &Workspace, dir: &std::path::Path, report: &mut RecoveryReport) {
    let entries = match fs::read_dir(dir) {
        Ok(x) => x,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(_) => return,
    };
    for item in entries.flatten() {
        if item.path().extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        report.inspected += 1;
        recover_item(workspace, &item.path(), report);
    }
    remove_recovery_dir_if_empty(dir);
}

fn recover_item(workspace: &Workspace, path: &std::path::Path, report: &mut RecoveryReport) {
    let journal: Journal = match validate_journal(workspace, path) {
        Ok(x) => x,
        Err(reason) => {
            report.manual.push(format!("{}: {reason}", path.display()));
            return;
        }
    };
    let mut states = Vec::with_capacity(journal.entries.len());
    for entry in &journal.entries {
        let current = match workspace.read_file(&entry.path) {
            Ok(x) => x,
            Err(_) => {
                states.push(EntryState::Manual);
                report.manual.push(format!("{}: unreadable", entry.path));
                continue;
            }
        };
        let current_hash = compute_sha256(&current);
        if current_hash == entry.candidate_hash {
            states.push(EntryState::Candidate);
        } else if current_hash != entry.pre_hash {
            states.push(EntryState::Manual);
            report
                .manual
                .push(format!("{}: changed after interruption", entry.path));
        } else {
            states.push(EntryState::Original);
        }
    }
    if states.contains(&EntryState::Manual) {
        return;
    }
    if journal.phase == Some(JournalPhase::Committed) {
        if states.iter().all(|state| *state == EntryState::Candidate) {
            if fs::remove_file(path).is_ok() {
                report.completed += 1;
                report.cleaned += 1;
            }
        } else {
            // A committed transaction must NEVER automatically become an uncommitted rollback!
            // If members exhibit unexpected or mixed states, preserve the journal and report
            // the inconsistency for human inspection.
            report.manual.push(format!(
                "{}: committed transaction has unexpected member states (not all candidate); manual inspection required",
                path.display()
            ));
        }
        return;
    }
    if journal.phase.is_none() && states.iter().all(|state| *state == EntryState::Candidate) {
        // Ambiguous legacy journal state (all candidate, but unrecorded commit decision).
        report.manual.push(format!(
            "{}: ambiguous legacy journal state (all candidate, commit unrecorded); manual recovery required",
            path.display()
        ));
        return;
    }
    if states.iter().all(|state| *state == EntryState::Original) {
        if fs::remove_file(path).is_ok() {
            report.cleaned += 1;
        }
        return;
    }
    let mut safe = true;
    for (entry, state) in journal.entries.iter().rev().zip(states.into_iter().rev()) {
        if state != EntryState::Candidate {
            continue;
        }
        match workspace.write_file_atomic_checked(
            &entry.path,
            &entry.candidate_hash,
            &entry.original,
        ) {
            Ok(()) => report.restored += 1,
            Err(_) => {
                safe = false;
                report.manual.push(entry.path.clone());
            }
        }
    }
    if safe && fs::remove_file(path).is_ok() {
        report.cleaned += 1;
    }
}

fn journal_paths(workspace: &Workspace) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for dir in recovery_dirs(workspace) {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                paths.push(entry.path());
            }
        }
    }
    paths.sort();
    paths
}

fn find_journal(workspace: &Workspace, transaction_id: &str) -> Option<std::path::PathBuf> {
    if transaction_id.is_empty() || safe_id(transaction_id) != transaction_id {
        return None;
    }
    journal_paths(workspace)
        .into_iter()
        .find(|path| path.file_stem().and_then(|value| value.to_str()) == Some(transaction_id))
}

fn validate_journal(workspace: &Workspace, path: &std::path::Path) -> Result<Journal, String> {
    let filename_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid journal filename".to_owned())?;
    let bytes = fs::read(path).map_err(|error| format!("unreadable journal: {error}"))?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(format!("journal exceeds {MAX_JOURNAL_BYTES} bytes"));
    }
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid JSON: {error}"))?;
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&journal.protocol_version.as_str()) {
        return Err("unsupported journal protocol version".into());
    }
    if journal.transaction_id.is_empty()
        || safe_id(&journal.transaction_id) != journal.transaction_id
        || filename_id != journal.transaction_id
    {
        return Err("transaction ID and filename disagree".into());
    }
    if journal.entries.is_empty() || journal.entries.len() > MAX_JOURNAL_ENTRIES {
        return Err("invalid journal entry count".into());
    }
    let mut paths = std::collections::HashSet::new();
    for entry in &journal.entries {
        if entry.path.is_empty()
            || entry.path.starts_with('/')
            || entry.path.contains('\\')
            || entry
                .path
                .split('/')
                .any(|part| part == ".." || part.is_empty())
            || !paths.insert(entry.path.clone())
        {
            return Err(format!("invalid or duplicate member path: {}", entry.path));
        }
        workspace
            .resolve_path(&entry.path)
            .map_err(|error| format!("unsafe member path {}: {error}", entry.path))?;
        if entry.original.len() > crate::protocol::MAX_FILE_BYTES
            || entry.candidate.len() > crate::protocol::MAX_FILE_BYTES
        {
            return Err(format!("embedded payload too large: {}", entry.path));
        }
        if compute_sha256(&entry.original) != entry.pre_hash
            || compute_sha256(&entry.candidate) != entry.candidate_hash
        {
            return Err(format!("embedded payload hash mismatch: {}", entry.path));
        }
    }
    Ok(journal)
}

fn member_inspections(workspace: &Workspace, journal: &Journal) -> Vec<RecoveryMemberInspection> {
    journal
        .entries
        .iter()
        .map(|entry| match workspace.read_file(&entry.path) {
            Ok(current) => {
                let current_hash = compute_sha256(&current);
                let classification = if current_hash == entry.pre_hash {
                    "ORIGINAL"
                } else if current_hash == entry.candidate_hash {
                    "CANDIDATE"
                } else {
                    "DRIFTED"
                };
                RecoveryMemberInspection {
                    path: entry.path.clone(),
                    pre_hash: entry.pre_hash.clone(),
                    candidate_hash: entry.candidate_hash.clone(),
                    current_hash: Some(current_hash),
                    classification: classification.into(),
                    automatic_recovery_safe: classification != "DRIFTED",
                }
            }
            Err(_) => RecoveryMemberInspection {
                path: entry.path.clone(),
                pre_hash: entry.pre_hash.clone(),
                candidate_hash: entry.candidate_hash.clone(),
                current_hash: None,
                classification: "UNREADABLE".into(),
                automatic_recovery_safe: false,
            },
        })
        .collect()
}

#[derive(Serialize, Clone, Debug)]
pub struct RecoveryReport {
    pub inspected: usize,
    pub completed: usize,
    pub restored: usize,
    pub cleaned: usize,
    pub manual: Vec<String>,
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;
    use tempfile::TempDir;

    fn journal_for(workspace: &Workspace, transaction_id: &str, entries: Vec<JournalEntry>) {
        write_journal(
            workspace,
            &Journal {
                protocol_version: "1.1.0".into(),
                transaction_id: transaction_id.into(),
                entries,
                phase: Some(JournalPhase::Prepared),
            },
        )
        .unwrap();
    }

    fn entry(path: &str, original: &[u8], candidate: &[u8]) -> JournalEntry {
        JournalEntry {
            path: path.into(),
            pre_hash: compute_sha256(original),
            candidate_hash: compute_sha256(candidate),
            original: original.to_vec(),
            candidate: candidate.to_vec(),
        }
    }

    #[test]
    fn recovery_preserves_a_fully_completed_commit() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("x.txt", b"new").unwrap();
        journal_for(
            &workspace,
            "completed",
            vec![entry("x.txt", b"old", b"new")],
        );
        mark_journal_committed(&workspace, "completed").unwrap();

        let report = recover_all(&workspace);

        assert_eq!(report.completed, 1);
        assert_eq!(report.restored, 0);
        assert_eq!(workspace.read_file("x.txt").unwrap(), b"new");
        assert!(!temp.path().join(".threadmoth-recovery").exists());
    }

    #[test]
    fn recovery_rolls_back_only_a_partial_commit() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("a.txt", b"new-a").unwrap();
        workspace.write_file_atomic("b.txt", b"old-b").unwrap();
        journal_for(
            &workspace,
            "partial",
            vec![
                entry("a.txt", b"old-a", b"new-a"),
                entry("b.txt", b"old-b", b"new-b"),
            ],
        );

        let report = recover_all(&workspace);

        assert_eq!(report.completed, 0);
        assert_eq!(report.restored, 1);
        assert_eq!(workspace.read_file("a.txt").unwrap(), b"old-a");
        assert_eq!(workspace.read_file("b.txt").unwrap(), b"old-b");
        assert!(!temp.path().join(".threadmoth-recovery").exists());
    }

    #[test]
    fn recovery_refuses_to_touch_an_entry_changed_after_interruption() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("x.txt", b"unexpected").unwrap();
        journal_for(&workspace, "changed", vec![entry("x.txt", b"old", b"new")]);

        let report = recover_all(&workspace);

        assert_eq!(report.restored, 0);
        assert_eq!(report.completed, 0);
        assert!(!report.manual.is_empty());
        assert_eq!(workspace.read_file("x.txt").unwrap(), b"unexpected");
        assert!(temp
            .path()
            .join(".threadmoth-recovery/changed.json")
            .exists());
        assert!(temp.path().join(".threadmoth-recovery").exists());
    }

    #[test]
    fn remove_journal_removes_the_directory_when_it_becomes_empty() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("x.txt", b"old").unwrap();
        journal_for(&workspace, "finished", vec![entry("x.txt", b"old", b"new")]);

        remove_journal(&workspace, "finished").unwrap();

        assert!(!temp.path().join(".threadmoth-recovery").exists());
    }

    #[test]
    fn remove_journal_keeps_directory_when_another_journal_remains() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("x.txt", b"old").unwrap();
        let recovery_entry = entry("x.txt", b"old", b"new");
        journal_for(&workspace, "one", vec![recovery_entry.clone()]);
        journal_for(&workspace, "two", vec![recovery_entry]);

        remove_journal(&workspace, "one").unwrap();

        assert!(temp.path().join(".threadmoth-recovery").exists());
        assert!(temp.path().join(".threadmoth-recovery/two.json").exists());
    }

    #[test]
    fn recovery_reads_legacy_suture_directory() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        workspace.write_file_atomic("x.txt", b"new").unwrap();
        let legacy = temp.path().join(LEGACY_RECOVERY_DIR);
        fs::create_dir_all(&legacy).unwrap();
        let journal = serde_json::json!({
            "protocol_version": "1.1.0",
            "transaction_id": "legacy",
            "phase": "committed",
            "entries": [{
                "path": "x.txt",
                "pre_hash": compute_sha256(b"old"),
                "candidate_hash": compute_sha256(b"new"),
                "original": [111, 108, 100],
                "candidate": [110, 101, 119]
            }]
        });
        fs::write(
            legacy.join("legacy.json"),
            serde_json::to_vec_pretty(&journal).unwrap(),
        )
        .unwrap();

        let report = recover_all(&workspace);

        assert_eq!(report.completed, 1);
        assert_eq!(report.cleaned, 1);
        assert!(!legacy.exists());
        assert_eq!(workspace.read_file("x.txt").unwrap(), b"new");
    }

    #[test]
    fn oversized_journal_is_refused_before_creating_recovery_state() {
        let temp = TempDir::new().unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        let payload = vec![b'a'; 4 * 1024 * 1024];
        let error = write_journal(
            &workspace,
            &Journal {
                protocol_version: PROTOCOL_VERSION.into(),
                transaction_id: "oversized".into(),
                entries: vec![entry("x.txt", &payload, &payload)],
                phase: Some(JournalPhase::Prepared),
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            WorkspaceError::ResourceLimit {
                dimension,
                limit,
                actual,
            } if dimension == "max_journal_bytes"
                && limit == MAX_JOURNAL_BYTES
                && actual > limit
        ));
        assert!(!temp.path().join(RECOVERY_DIR).exists());
    }

    #[test]
    fn one_megabyte_binary_journal_fits_without_decimal_array_expansion() {
        let payload: Vec<u8> = (0..1_048_576).map(|index| (index % 256) as u8).collect();
        let journal = Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "one-megabyte".into(),
            entries: vec![entry("x.bin", &payload, &payload)],
            phase: Some(JournalPhase::Prepared),
        };

        let encoded = serialize_journal(&journal).unwrap();
        assert!(encoded.len() < 4 * 1024 * 1024);
        let encoded_text = String::from_utf8(encoded.clone()).unwrap();
        assert!(encoded_text.contains("\"original\":\""));
        assert!(!encoded_text.contains("\"original\":["));
        check_journal_size(&journal).unwrap();
    }

    #[test]
    fn journal_round_trip_preserves_arbitrary_binary_bytes() {
        let original = vec![0, 1, 2, 127, 128, 254, 255];
        let candidate = vec![255, 0, 17, 200, 128, 3];
        let journal = Journal {
            protocol_version: PROTOCOL_VERSION.into(),
            transaction_id: "binary-round-trip".into(),
            entries: vec![entry("x.bin", &original, &candidate)],
            phase: Some(JournalPhase::Prepared),
        };

        let decoded: Journal =
            serde_json::from_slice(&serialize_journal(&journal).unwrap()).unwrap();
        assert_eq!(decoded.entries[0].original, original);
        assert_eq!(decoded.entries[0].candidate, candidate);
    }

    #[test]
    fn journal_sizes_remain_bounded_for_representative_payloads() {
        for size in [100 * 1024, 500 * 1024, 1024 * 1024] {
            let payload = vec![0; size];
            let journal = Journal {
                protocol_version: PROTOCOL_VERSION.into(),
                transaction_id: format!("size-{size}"),
                entries: vec![entry("x.bin", &payload, &payload)],
                phase: Some(JournalPhase::Prepared),
            };
            let encoded = serialize_journal(&journal).unwrap();
            assert!(encoded.len() < MAX_JOURNAL_BYTES);
            let decoded: Journal = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded.entries[0].original, payload);
        }
    }

    #[test]
    fn legacy_decimal_array_journal_remains_recoverable() {
        let legacy = serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "transaction_id": "legacy-array",
            "entries": [{
                "path": "x.bin",
                "pre_hash": compute_sha256(&[0, 255]),
                "candidate_hash": compute_sha256(&[1, 128]),
                "original": [0, 255],
                "candidate": [1, 128]
            }]
        });
        let decoded: Journal = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.entries[0].original, vec![0, 255]);
        assert_eq!(decoded.entries[0].candidate, vec![1, 128]);
    }
}

mod base64_bytes {
    use super::*;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a base64 string or legacy byte array")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                decode(value).map_err(E::custom)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = Vec::new();
                while let Some(byte) = sequence.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(bytes)
            }
        }

        deserializer.deserialize_any(BytesVisitor)
    }

    fn encode(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let first = chunk[0] as usize;
            let second = chunk.get(1).copied().unwrap_or_default() as usize;
            let third = chunk.get(2).copied().unwrap_or_default() as usize;
            output.push(ALPHABET[first >> 2] as char);
            output.push(ALPHABET[((first & 0x03) << 4) | (second >> 4)] as char);
            output.push(if chunk.len() > 1 {
                ALPHABET[((second & 0x0f) << 2) | (third >> 6)] as char
            } else {
                '='
            });
            output.push(if chunk.len() > 2 {
                ALPHABET[third & 0x3f] as char
            } else {
                '='
            });
        }
        output
    }

    fn decode(value: &str) -> Result<Vec<u8>, &'static str> {
        let input = value.as_bytes();
        if input.len() % 4 != 0 {
            return Err("base64 length is not a multiple of four");
        }
        let mut output = Vec::with_capacity(input.len() / 4 * 3);
        for (chunk_index, chunk) in input.chunks(4).enumerate() {
            let last = chunk_index + 1 == input.len() / 4;
            let a = value_of(chunk[0]).ok_or("invalid base64 character")?;
            let b = value_of(chunk[1]).ok_or("invalid base64 character")?;
            let c = if chunk[2] == b'=' {
                if !last || chunk[3] != b'=' {
                    return Err("invalid base64 padding");
                }
                0
            } else {
                value_of(chunk[2]).ok_or("invalid base64 character")?
            };
            let d = if chunk[3] == b'=' {
                if !last {
                    return Err("invalid base64 padding");
                }
                0
            } else {
                value_of(chunk[3]).ok_or("invalid base64 character")?
            };
            output.push((a << 2 | b >> 4) as u8);
            if chunk[2] != b'=' {
                output.push((b << 4 | c >> 2) as u8);
            }
            if chunk[3] != b'=' {
                output.push((c << 6 | d) as u8);
            }
        }
        Ok(output)
    }

    fn value_of(byte: u8) -> Option<u32> {
        ALPHABET
            .iter()
            .position(|value| *value == byte)
            .map(|value| value as u32)
    }
}
