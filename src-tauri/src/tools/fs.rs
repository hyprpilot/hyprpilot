//! Filesystem primitives used by the ACP adapter.
//!
//! `FsTools` wraps a `Sandbox` and exposes sandbox-constrained `read` /
//! `write` operations. Errors are a domain type (`FsError`) — ACP
//! mapping stays in the adapter layer.

use std::path::Path;

use tokio::io::AsyncWriteExt;

use super::sandbox::{Sandbox, SandboxError};

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error("io error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub struct FsTools {
    sandbox: Sandbox,
}

impl FsTools {
    pub fn new(sandbox: Sandbox) -> Self {
        Self { sandbox }
    }

    /// Read `path` (after sandbox resolution) and slice on 1-based
    /// `line` + `limit`. `line = None` reads from the start; `limit =
    /// None` reads through EOF.
    pub async fn read(&self, path: &Path, line: Option<u32>, limit: Option<u32>) -> Result<String, FsError> {
        let resolved = self.sandbox.resolve(path)?;
        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|source| FsError::Io {
                path: resolved.clone(),
                source,
            })?;
        Ok(slice_lines(&content, line, limit))
    }

    /// Write `content` to `path` (after sandbox resolution). Parents
    /// are created on demand; existing files are truncated.
    pub async fn write(&self, path: &Path, content: &str) -> Result<(), FsError> {
        let resolved = self.sandbox.resolve(path)?;
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|source| FsError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&resolved)
            .await
            .map_err(|source| FsError::Io {
                path: resolved.clone(),
                source,
            })?;
        file.write_all(content.as_bytes()).await.map_err(|source| FsError::Io {
            path: resolved.clone(),
            source,
        })?;
        file.sync_all().await.map_err(|source| FsError::Io {
            path: resolved.clone(),
            source,
        })?;
        Ok(())
    }
}

fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    match limit {
        None if start == 0 => content.to_string(),
        None => content.lines().skip(start).collect::<Vec<_>>().join("\n"),
        Some(n) => content
            .lines()
            .skip(start)
            .take(n as usize)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk(dir: &Path) -> FsTools {
        FsTools::new(Sandbox::new(dir).expect("sandbox"))
    }

    #[tokio::test]
    async fn read_happy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "one\ntwo\nthree\n").unwrap();

        let fs = mk(dir.path());
        let out = fs.read(&PathBuf::from("hello.txt"), None, None).await.unwrap();
        assert!(out.contains("one"));
        assert!(out.contains("three"));
    }

    #[tokio::test]
    async fn read_with_line_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\n").unwrap();

        let fs = mk(dir.path());
        let out = fs.read(&PathBuf::from("f.txt"), Some(2), Some(2)).await.unwrap();
        assert_eq!(out, "b\nc");
    }

    #[tokio::test]
    async fn read_outside_sandbox_denied() {
        let dir = tempfile::tempdir().unwrap();
        let fs = mk(dir.path());
        let err = fs.read(&PathBuf::from("/etc/passwd"), None, None).await.unwrap_err();
        assert!(matches!(err, FsError::Sandbox(SandboxError::Escape(_))), "got {err:?}");
    }

    #[tokio::test]
    async fn write_happy_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let fs = mk(dir.path());
        fs.write(&PathBuf::from("nested/out.txt"), "payload").await.unwrap();
        let contents = std::fs::read_to_string(dir.path().join("nested/out.txt")).unwrap();
        assert_eq!(contents, "payload");
    }

    #[tokio::test]
    async fn write_outside_sandbox_denied() {
        let dir = tempfile::tempdir().unwrap();
        let fs = mk(dir.path());
        let err = fs.write(&PathBuf::from("/tmp/escape.txt"), "x").await.unwrap_err();
        assert!(matches!(err, FsError::Sandbox(SandboxError::Escape(_))), "got {err:?}");
    }
}
