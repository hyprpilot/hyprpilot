//! Path containment for fs/terminal primitives.
//!
//! `Sandbox::resolve(path)` resolves `path` (absolute or relative to the
//! sandbox root) and confirms the result lands under the canonicalized
//! root after full symlink resolution. Write targets can point at paths
//! that don't exist yet, so we fall back to resolving the deepest
//! existing ancestor and lexically attaching the remainder —
//! `std::fs::canonicalize` alone would reject any not-yet-created file.
//! `normpath` and `path-clean` both stop short of this "parent exists,
//! target doesn't" shape: `path-clean` is purely lexical (misses
//! symlink escapes), `normpath::normalize` demands the full path exist
//! on disk.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("path escapes sandbox root: {0}")]
    Escape(PathBuf),
    #[error("io error resolving {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Path container with a canonicalized root. Root is resolved at
/// construction so a missing base fails fast rather than on every
/// resolve call.
#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SandboxError> {
        let raw = root.into();
        let canonical = raw.canonicalize().map_err(|source| SandboxError::Io {
            path: raw.clone(),
            source,
        })?;
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `path` (absolute or relative to the root) and verify it
    /// stays under `self.root` after symlink resolution. Accepts paths
    /// whose final component doesn't exist yet (writes), as long as
    /// the deepest existing ancestor canonicalizes into the sandbox.
    pub fn resolve(&self, path: &Path) -> Result<PathBuf, SandboxError> {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        let lexical = lexically_normalize(&joined);
        let resolved = resolve_existing_prefix(&lexical)?;

        if !resolved.starts_with(&self.root) {
            return Err(SandboxError::Escape(path.to_path_buf()));
        }
        Ok(resolved)
    }
}

/// Collapse `.` / `..` segments without touching the filesystem. Leading
/// `..` above the root is dropped — can't escape an absolute root lexically.
fn lexically_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the deepest existing ancestor and reattach the remaining
/// not-yet-existing suffix. Lets callers write to paths whose final
/// component hasn't been created — while still following symlinks on
/// every directory in the resolved chain.
fn resolve_existing_prefix(path: &Path) -> Result<PathBuf, SandboxError> {
    let mut suffix: Vec<OsString> = Vec::new();
    let mut cursor: PathBuf = path.to_path_buf();

    loop {
        match cursor.canonicalize() {
            Ok(mut canon) => {
                for seg in suffix.into_iter().rev() {
                    canon.push(seg);
                }
                return Ok(canon);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let Some(name) = cursor.file_name().map(|n| n.to_os_string()) else {
                    return Err(SandboxError::Io {
                        path: path.to_path_buf(),
                        source: err,
                    });
                };
                suffix.push(name);
                if !cursor.pop() {
                    return Err(SandboxError::Io {
                        path: path.to_path_buf(),
                        source: err,
                    });
                }
            }
            Err(source) => {
                return Err(SandboxError::Io { path: cursor, source });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sandbox(root: &Path) -> Sandbox {
        Sandbox::new(root).expect("sandbox constructs")
    }

    #[test]
    fn relative_inside_sandbox_ok() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ok.txt"), "x").unwrap();
        let resolved = sandbox(dir.path()).resolve(Path::new("ok.txt")).unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn absolute_inside_sandbox_ok() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ok.txt");
        fs::write(&target, "x").unwrap();
        let resolved = sandbox(dir.path()).resolve(&target).unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn parent_escape_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("sub");
        fs::create_dir_all(&nested).unwrap();
        let err = sandbox(&nested).resolve(Path::new("../../../etc/passwd")).unwrap_err();
        assert!(matches!(err, SandboxError::Escape(_)), "got {err:?}");
    }

    #[test]
    fn absolute_outside_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = sandbox(dir.path()).resolve(Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, SandboxError::Escape(_)), "got {err:?}");
    }

    #[test]
    fn write_target_may_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = sandbox(dir.path()).resolve(Path::new("new/nested/file.txt")).unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        let tail: Vec<_> = resolved.components().rev().take(3).collect();
        let tail_str: Vec<_> = tail
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(tail_str, vec!["file.txt", "nested", "new"]);
    }

    #[test]
    fn missing_base_rejected_at_construction() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let err = Sandbox::new(&missing).unwrap_err();
        assert!(matches!(err, SandboxError::Io { .. }), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        let outer = tempfile::tempdir().unwrap();
        let sandbox_root = outer.path().join("sandbox");
        fs::create_dir(&sandbox_root).unwrap();
        let secret = outer.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();
        let link = sandbox_root.join("escape");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let err = sandbox(&sandbox_root).resolve(Path::new("escape")).unwrap_err();
        assert!(matches!(err, SandboxError::Escape(_)), "got {err:?}");
    }
}
