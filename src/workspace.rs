#![forbid(unsafe_code)]

use crate::path::{PathNamespace, PathNormalizer};
use crate::protocol::MAX_FILE_BYTES;
use atomic_write_file::AtomicWriteFile;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("path traversal attempt detected: {0}")]
    Traversal(String),
    #[error("requested path is outside the configured workspace root: {target} (workspace: {workspace_root})")]
    WorkspaceRootMismatch {
        target: String,
        workspace_root: String,
    },
    #[error("symlink escape attempt detected: {0}")]
    SymlinkEscape(String),
    #[error("path not found: {0}")]
    NotFound(String),
    #[error("stale file identity: expected {expected}, observed {actual}")]
    StaleIdentity { expected: String, actual: String },
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("destination already exists: {0}")]
    AlreadyExists(String),
    #[error("path cannot be mapped from its declared namespace: {0}")]
    UnmappablePath(String),
    #[error("resource limit exceeded for {dimension}: {actual} > {limit}")]
    ResourceLimit {
        dimension: String,
        limit: usize,
        actual: usize,
    },
    #[error("workspace mutation lock is held by another Threadmoth process: {lock_path}")]
    Busy { lock_path: String },
}

#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

/// A bounded cross-process lock for the complete mutation boundary. The lock
/// is deliberately a separate file in the workspace root, so an interrupted
/// process leaves an inspectable, fail-closed busy marker rather than allowing
/// two Threadmoth writers to proceed concurrently.
pub struct MutationLock {
    path: PathBuf,
    _file: fs::File,
}

impl Drop for MutationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Workspace {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, WorkspaceError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                WorkspaceError::NotFound(root.as_ref().display().to_string())
            } else {
                WorkspaceError::Io(e)
            }
        })?;
        if !root.is_dir() {
            return Err(WorkspaceError::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                "workspace root must be a directory",
            )));
        }
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn acquire_mutation_lock(&self) -> Result<MutationLock, WorkspaceError> {
        let path = self.root.join(".threadmoth-mutation.lock");
        for attempt in 0..20 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let marker = format!("pid={}\n", std::process::id());
                    file.write_all(marker.as_bytes())?;
                    file.sync_all()?;
                    return Ok(MutationLock { path, _file: file });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if attempt == 19 {
                        return Err(WorkspaceError::Busy {
                            lock_path: path.display().to_string(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(WorkspaceError::Io(error)),
            }
        }
        Err(WorkspaceError::Busy {
            lock_path: path.display().to_string(),
        })
    }

    /// Resolve a caller path in its declared namespace, returning one
    /// workspace-relative spelling for every later read and write. Absolute
    /// Windows/WSL paths are accepted only when their resolved target is under
    /// this workspace; unmappable namespace paths fail closed.
    pub fn resolve_namespaced_path(
        &self,
        path: &str,
        namespace: &PathNamespace,
    ) -> Result<String, WorkspaceError> {
        #[cfg(target_os = "windows")]
        if matches!(namespace, PathNamespace::Wsl { .. }) {
            let normalized = path.replace('\\', "/");
            let valid_mount = normalized
                .strip_prefix("/mnt/")
                .and_then(|mounted| mounted.split_once('/'))
                .is_some_and(|(letter, rest)| {
                    letter.len() == 1
                        && letter.as_bytes()[0].is_ascii_alphabetic()
                        && !rest.is_empty()
                });
            if normalized.starts_with('/') && !valid_mount {
                return Err(WorkspaceError::UnmappablePath(path.into()));
            }
        }
        let normalized = PathNormalizer::normalize(path, namespace);
        reject_parent_components(&normalized)?;
        let native = PathNormalizer::to_native_path(path, namespace);
        if native.is_absolute() {
            let resolved =
                if native.exists() {
                    std::fs::canonicalize(&native)?
                } else {
                    let parent = native.parent().ok_or_else(|| {
                        WorkspaceError::UnmappablePath(native.display().to_string())
                    })?;
                    let parent = std::fs::canonicalize(parent).map_err(|error| {
                        if error.kind() == io::ErrorKind::NotFound {
                            WorkspaceError::NotFound(parent.display().to_string())
                        } else {
                            WorkspaceError::Io(error)
                        }
                    })?;
                    parent.join(native.file_name().ok_or_else(|| {
                        WorkspaceError::UnmappablePath(native.display().to_string())
                    })?)
                };
            let relative = relative_if_within(&resolved, &self.root).ok_or_else(|| {
                WorkspaceError::WorkspaceRootMismatch {
                    target: path.into(),
                    workspace_root: self.root.display().to_string(),
                }
            })?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            reject_internal_namespace(&relative)?;
            return Ok(relative);
        }
        reject_internal_namespace(&normalized)?;
        self.resolve_path(&normalized)?;
        Ok(normalized)
    }

    /// Check the budget boundary using resolved filesystem paths. A prefix is
    /// an authority root only when its canonical location is inside the
    /// canonical workspace; the target's deepest existing ancestor is also
    /// resolved so not-yet-created leaves cannot hide a junction or symlink.
    pub fn is_within_allowed_prefix(
        &self,
        target: &str,
        prefix: &str,
    ) -> Result<bool, WorkspaceError> {
        let normalized_prefix = prefix.replace('\\', "/");
        reject_parent_components(&normalized_prefix)?;
        reject_internal_namespace(&normalized_prefix)?;
        let prefix_path = self.resolve_path(&normalized_prefix)?;
        let target_path = self.resolve_path(target)?;
        if !prefix_path.exists() {
            return Err(WorkspaceError::NotFound(prefix.into()));
        }
        let prefix_metadata = fs::metadata(&prefix_path)?;
        if prefix_metadata.is_dir() {
            Ok(path_is_within(&target_path, &prefix_path))
        } else {
            Ok(path_is_within(&target_path, &prefix_path)
                && path_is_within(&prefix_path, &target_path))
        }
    }

    pub fn resolve_path<P: AsRef<Path>>(&self, rel_path: P) -> Result<PathBuf, WorkspaceError> {
        let rel = rel_path.as_ref();
        reject_internal_namespace(&rel.to_string_lossy())?;
        if rel.is_absolute() || rel.to_string_lossy().as_bytes().get(1) == Some(&b':') {
            return Err(WorkspaceError::Traversal(format!(
                "absolute path not allowed: {}",
                rel.display()
            )));
        }
        let mut components = Vec::new();
        for c in rel.components() {
            match c {
                Component::ParentDir => {
                    return Err(WorkspaceError::Traversal(rel.display().to_string()))
                }
                Component::Normal(c) => components.push(c.to_owned()),
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(WorkspaceError::Traversal(rel.display().to_string()))
                }
            }
        }
        let mut candidate = self.root.clone();
        for c in &components {
            candidate.push(c);
        }
        let resolved = canonicalize_candidate(&candidate)?;
        if !path_is_within(&resolved, &self.root) {
            return Err(WorkspaceError::SymlinkEscape(
                candidate.display().to_string(),
            ));
        }
        Ok(resolved)
    }

    /// Resolve a destination without canonicalising its final spelling. This
    /// matters for case-only renames on case-insensitive filesystems. Every
    /// existing component is still canonicalised for the same containment
    /// check used by `resolve_path`.
    pub fn resolve_destination_path<P: AsRef<Path>>(
        &self,
        rel_path: P,
    ) -> Result<PathBuf, WorkspaceError> {
        let rel = rel_path.as_ref();
        if rel.is_absolute() || rel.to_string_lossy().as_bytes().get(1) == Some(&b':') {
            return Err(WorkspaceError::Traversal(format!(
                "absolute path not allowed: {}",
                rel.display()
            )));
        }
        let mut components = Vec::new();
        for component in rel.components() {
            match component {
                Component::ParentDir => {
                    return Err(WorkspaceError::Traversal(rel.display().to_string()))
                }
                Component::Normal(component) => components.push(component.to_owned()),
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(WorkspaceError::Traversal(rel.display().to_string()))
                }
            }
        }
        let mut candidate = self.root.clone();
        for component in &components {
            candidate.push(component);
        }
        let resolved = canonicalize_candidate(&candidate)?;
        if !path_is_within(&resolved, &self.root) {
            return Err(WorkspaceError::SymlinkEscape(
                candidate.display().to_string(),
            ));
        }
        Ok(candidate)
    }

    pub fn read_file<P: AsRef<Path>>(&self, rel_path: P) -> Result<Vec<u8>, WorkspaceError> {
        let resolved = self.resolve_path(rel_path)?;
        if !resolved.is_file() {
            return Err(WorkspaceError::NotFound(resolved.display().to_string()));
        }
        let length = fs::metadata(&resolved)?.len();
        if length > MAX_FILE_BYTES as u64 {
            return Err(WorkspaceError::ResourceLimit {
                dimension: "max_file_bytes".into(),
                limit: MAX_FILE_BYTES,
                actual: length.min(usize::MAX as u64) as usize,
            });
        }
        Ok(fs::read(resolved)?)
    }

    pub fn create_file_new<P: AsRef<Path>>(
        &self,
        rel_path: P,
        bytes: &[u8],
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(rel_path)?;
        if resolved.exists() {
            return Err(WorkspaceError::AlreadyExists(
                resolved.display().to_string(),
            ));
        }
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent)?;
            self.ensure_parent(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&resolved)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }

    pub fn delete_file_checked<P: AsRef<Path>>(
        &self,
        rel_path: P,
        expected_hash: &str,
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(rel_path)?;
        self.ensure_file_size(&resolved)?;
        let current = fs::read(&resolved)?;
        let actual = sha256(&current);
        if actual != expected_hash {
            return Err(WorkspaceError::StaleIdentity {
                expected: expected_hash.into(),
                actual,
            });
        }
        fs::remove_file(resolved)?;
        Ok(())
    }

    pub fn rename_file_checked<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        source: P,
        destination: Q,
        expected_hash: &str,
        destination_absent: bool,
    ) -> Result<(), WorkspaceError> {
        let source = self.resolve_path(source)?;
        let destination = self.resolve_destination_path(destination)?;
        self.ensure_file_size(&source)?;
        let current = fs::read(&source)?;
        let actual = sha256(&current);
        if actual != expected_hash {
            return Err(WorkspaceError::StaleIdentity {
                expected: expected_hash.into(),
                actual,
            });
        }
        if destination_absent && destination.exists() {
            let same_source = fs::canonicalize(&destination)
                .map(|path| path == source)
                .unwrap_or(false);
            if !same_source {
                return Err(WorkspaceError::AlreadyExists(
                    destination.display().to_string(),
                ));
            }
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
            self.ensure_parent(parent)?;
        }
        fs::rename(source, destination)?;
        Ok(())
    }

    pub fn write_file_atomic<P: AsRef<Path>>(
        &self,
        rel_path: P,
        bytes: &[u8],
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(rel_path)?;
        if let Some(parent) = resolved.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
            self.ensure_parent(parent)?;
        }
        self.ensure_windows_replaceable(&resolved)?;
        let mut file = AtomicWriteFile::open(&resolved)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.commit()?;
        Ok(())
    }

    /// Stage and replace only if the object still has the observed content hash.
    pub fn write_file_atomic_checked<P: AsRef<Path>>(
        &self,
        rel_path: P,
        expected_hash: &str,
        bytes: &[u8],
    ) -> Result<(), WorkspaceError> {
        let resolved = self.resolve_path(rel_path)?;
        self.ensure_file_size(&resolved)?;
        let observed = fs::read(&resolved)?;
        let actual = sha256(&observed);
        if actual != expected_hash {
            return Err(WorkspaceError::StaleIdentity {
                expected: expected_hash.into(),
                actual,
            });
        }
        if let Some(parent) = resolved.parent() {
            self.ensure_parent(parent)?;
        }
        self.ensure_windows_replaceable(&resolved)?;
        let mut file = AtomicWriteFile::open(&resolved)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.commit()?;
        Ok(())
    }

    fn ensure_windows_replaceable(&self, _path: &Path) -> Result<(), WorkspaceError> {
        #[cfg(target_os = "windows")]
        if _path.exists() && fs::metadata(_path)?.permissions().readonly() {
            return Err(WorkspaceError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "read-only destination cannot be atomically replaced on Windows: {}",
                    _path.display()
                ),
            )));
        }
        Ok(())
    }

    fn ensure_parent(&self, parent: &Path) -> Result<(), WorkspaceError> {
        let canonical = fs::canonicalize(parent)?;
        if !path_is_within(&canonical, &self.root) {
            return Err(WorkspaceError::SymlinkEscape(parent.display().to_string()));
        }
        Ok(())
    }

    fn ensure_file_size(&self, path: &Path) -> Result<(), WorkspaceError> {
        let length = fs::metadata(path)?.len();
        if length > MAX_FILE_BYTES as u64 {
            return Err(WorkspaceError::ResourceLimit {
                dimension: "max_file_bytes".into(),
                limit: MAX_FILE_BYTES,
                actual: length.min(usize::MAX as u64) as usize,
            });
        }
        Ok(())
    }
}

fn reject_internal_namespace(path: &str) -> Result<(), WorkspaceError> {
    let normalized = path.replace('\\', "/");
    if normalized == ".threadmoth-mutation.lock"
        || normalized == ".threadmoth-recovery"
        || normalized.starts_with(".threadmoth-recovery/")
        || normalized == ".suture-recovery"
        || normalized.starts_with(".suture-recovery/")
    {
        return Err(WorkspaceError::Traversal(format!(
            "Threadmoth recovery namespace is reserved: {path}"
        )));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn reject_parent_components(path: &str) -> Result<(), WorkspaceError> {
    if Path::new(path)
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(WorkspaceError::Traversal(path.into()));
    }
    Ok(())
}

/// Canonicalize an existing target or its deepest existing ancestor, then
/// append the still-missing leaf components. This resolves symlinks and
/// Windows reparse points before a create/write path is authorized.
fn canonicalize_candidate(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut ancestor = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => {
                let mut resolved = fs::canonicalize(&ancestor)?;
                if !suffix.is_empty() && !fs::metadata(&resolved)?.is_dir() {
                    return Err(WorkspaceError::NotFound(path.display().to_string()));
                }
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| WorkspaceError::NotFound(path.display().to_string()))?
                    .to_owned();
                suffix.push(name);
                if !ancestor.pop() {
                    return Err(WorkspaceError::NotFound(path.display().to_string()));
                }
            }
            Err(error) => return Err(WorkspaceError::Io(error)),
        }
    }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut path_components = path.components();
        root.components().all(|root_component| {
            path_components
                .next()
                .is_some_and(|path_component| path_component == root_component)
        })
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

fn relative_if_within(path: &Path, root: &Path) -> Option<PathBuf> {
    if !path_is_within(path, root) {
        return None;
    }
    #[cfg(windows)]
    {
        Some(path.components().skip(root.components().count()).collect())
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathNamespace;
    use tempfile::TempDir;
    #[test]
    fn workspace_new_valid() {
        let t = TempDir::new().unwrap();
        assert!(Workspace::new(t.path()).is_ok());
    }
    #[test]
    fn traversal_rejected() {
        let t = TempDir::new().unwrap();
        let w = Workspace::new(t.path()).unwrap();
        assert!(matches!(
            w.resolve_path("../x"),
            Err(WorkspaceError::Traversal(_))
        ));
        assert!(matches!(
            w.resolve_path("/etc/passwd"),
            Err(WorkspaceError::Traversal(_))
        ));
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        use std::os::unix::fs::symlink;
        let t = TempDir::new().unwrap();
        let o = TempDir::new().unwrap();
        fs::write(o.path().join("x"), b"x").unwrap();
        symlink(o.path(), t.path().join("link")).unwrap();
        let w = Workspace::new(t.path()).unwrap();
        assert!(matches!(
            w.resolve_path("link/x"),
            Err(WorkspaceError::SymlinkEscape(_))
        ));
    }
    #[test]
    fn existing_file_replaced_and_stale_refused() {
        let t = TempDir::new().unwrap();
        let w = Workspace::new(t.path()).unwrap();
        w.write_file_atomic("x.txt", b"one").unwrap();
        let h = sha256(b"one");
        w.write_file_atomic_checked("x.txt", &h, b"two").unwrap();
        assert_eq!(w.read_file("x.txt").unwrap(), b"two");
        assert!(matches!(
            w.write_file_atomic_checked("x.txt", &h, b"three"),
            Err(WorkspaceError::StaleIdentity { .. })
        ));
    }

    #[test]
    fn absolute_declared_native_path_is_confined_to_workspace() {
        let t = TempDir::new().unwrap();
        std::fs::write(t.path().join("x.txt"), b"x").unwrap();
        let workspace = Workspace::new(t.path()).unwrap();
        let absolute = t.path().join("x.txt").to_string_lossy().into_owned();
        assert_eq!(
            workspace
                .resolve_namespaced_path(&absolute, &PathNamespace::Native)
                .unwrap(),
            "x.txt"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn readonly_checked_write_refuses_before_staging() {
        let t = TempDir::new().unwrap();
        let path = t.path().join("readonly.txt");
        fs::write(&path, b"old").unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        fs::set_permissions(&path, readonly_permissions).unwrap();
        let workspace = Workspace::new(t.path()).unwrap();
        let result = workspace.write_file_atomic_checked("readonly.txt", &sha256(b"old"), b"new");
        let entries: Vec<_> = fs::read_dir(t.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(matches!(
            result,
            Err(WorkspaceError::Io(ref error)) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert!(!entries
            .iter()
            .any(|name| name.starts_with(".readonly.txt.")));

        fs::set_permissions(&path, original_permissions).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn unmappable_wsl_path_is_explicitly_refused() {
        let t = TempDir::new().unwrap();
        let workspace = Workspace::new(t.path()).unwrap();
        assert!(matches!(
            workspace.resolve_namespaced_path(
                "/home/agent/file.txt",
                &PathNamespace::Wsl { distro: None }
            ),
            Err(WorkspaceError::UnmappablePath(_))
        ));
    }
}
