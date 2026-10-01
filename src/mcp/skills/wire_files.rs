//! A skill's own files — every file in its bundle directory, which
//! SEP-2640 serves as `skill://<skill-path>/<file-path>` resources and
//! lists, with a sha256 digest and a size, in the skill's `resources`.
//!
//! The bytes are read ONCE per rescan and served from memory. A host
//! verifies what it reads against the digest it was listed, so serving
//! the disk at read time would let an edit inside the watcher's debounce
//! window hand back bytes that match nothing — and would follow a file
//! swapped for a symlink after the scan. Unchanged files (same size and
//! mtime) are carried over from the previous scan instead of re-read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use tracing::warn;

/// SEP-2640 "Limits": what every conforming host must accept. A skill
/// past either is not guaranteed loadable anywhere, so it is not served.
pub const MAX_FILES: usize = 512;
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// One file of a skill bundle.
#[derive(Debug, Clone)]
pub struct BundleFile {
    /// Path relative to the skill's directory, `/`-separated — the
    /// `<file-path>` of its URI.
    pub rel: String,
    pub abs: PathBuf,
    pub bytes: Arc<[u8]>,
    /// `sha256:<64 lowercase hex>`, over exactly `bytes`.
    pub digest: String,
    modified: Option<SystemTime>,
}

impl BundleFile {
    #[must_use]
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// The MIME type a resource read reports. Text a model can read gets
    /// a text type; anything that is not UTF-8 is served as a blob.
    #[must_use]
    pub fn mime_type(&self) -> &'static str {
        let ext = Path::new(&self.rel)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        match ext {
            "md" | "markdown" => "text/markdown",
            "json" => "application/json",
            "yaml" | "yml" => "application/yaml",
            "toml" => "application/toml",
            "py" => "text/x-python",
            "sh" | "bash" | "zsh" => "text/x-shellscript",
            "html" | "htm" => "text/html",
            "csv" => "text/csv",
            _ if self.text().is_some() => "text/plain",
            _ => "application/octet-stream",
        }
    }

    /// The file as text, when it is UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }

    /// Standard padded base64 of the bytes — the encoding a blob
    /// resource carries. Hand-rolled: it is the only use, and none of the
    /// crates in the tree export one.
    #[must_use]
    pub fn base64(&self) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(self.bytes.len().div_ceil(3) * 4);
        for chunk in self.bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}

/// Why a bundle is not served.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BundleError {
    #[error("{0} files exceed the {MAX_FILES}-file limit")]
    TooManyFiles(usize),
    #[error("{0} bytes exceed the {MAX_BYTES}-byte limit")]
    TooLarge(u64),
}

/// Every served file under `dir`, in walk order: hidden and ignored
/// entries skipped and symlinks never followed, by the same walker
/// discovery uses. `previous` maps absolute paths from the last scan, so
/// a file whose size and mtime are unchanged is reused rather than read
/// and hashed again.
pub fn read_bundle(dir: &Path, previous: &HashMap<PathBuf, BundleFile>) -> Result<Vec<BundleFile>, BundleError> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in super::loader::walk(dir) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let Some(rel) = relative(dir, &abs) else { continue };
        let Ok(meta) = std::fs::symlink_metadata(&abs) else {
            continue;
        };
        let modified = meta.modified().ok();
        let file = match previous.get(&abs) {
            Some(old) if old.modified == modified && old.size() == meta.len() => old.clone(),
            _ => match std::fs::read(&abs) {
                Ok(bytes) => BundleFile {
                    digest: digest(&bytes),
                    bytes: bytes.into(),
                    rel,
                    abs,
                    modified,
                },
                Err(err) => {
                    warn!(path = %abs.display(), %err, "skills: unreadable bundle file — not served");
                    continue;
                }
            },
        };
        total += file.size();
        files.push(file);
        if files.len() > MAX_FILES {
            return Err(BundleError::TooManyFiles(files.len()));
        }
        if total > MAX_BYTES {
            return Err(BundleError::TooLarge(total));
        }
    }
    Ok(files)
}

/// `sha256:<hex>` over `bytes`, the SEP-2640 digest format.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn relative(dir: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(dir).ok()?;
    let segments: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(segments?.join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn rels(files: &[BundleFile]) -> Vec<&str> {
        files.iter().map(|f| f.rel.as_str()).collect()
    }

    #[test]
    fn every_served_file_is_listed_with_its_digest_and_size() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("scripts")).unwrap();
        fs::write(tmp.path().join("SKILL.md"), "skill").unwrap();
        fs::write(tmp.path().join("scripts/run.py"), "print(1)").unwrap();

        let files = read_bundle(tmp.path(), &HashMap::new()).unwrap();
        assert_eq!(rels(&files), ["SKILL.md", "scripts/run.py"]);
        assert_eq!(files[1].size(), 8);
        assert_eq!(files[1].digest, digest(b"print(1)"));
        assert_eq!(files[1].mime_type(), "text/x-python");
    }

    /// The format a host compares against, byte for byte.
    #[test]
    fn a_digest_is_lowercase_hex_sha256() {
        assert_eq!(
            digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// The captain's real tree carries a script `.venv` per skill — a
    /// hundred megabytes that must never become served files.
    #[test]
    fn hidden_and_ignored_files_are_not_part_of_a_bundle() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("scripts/.venv/lib")).unwrap();
        fs::create_dir_all(tmp.path().join("scripts/__pycache__")).unwrap();
        fs::write(tmp.path().join("SKILL.md"), "skill").unwrap();
        fs::write(tmp.path().join("scripts/.venv/lib/x.py"), "x").unwrap();
        fs::write(tmp.path().join("scripts/__pycache__/x.pyc"), "x").unwrap();
        fs::write(tmp.path().join(".gitignore"), "__pycache__/\n").unwrap();

        let files = read_bundle(tmp.path(), &HashMap::new()).unwrap();
        assert_eq!(rels(&files), ["SKILL.md"]);
    }

    #[test]
    fn a_bundle_past_the_file_limit_is_refused() {
        let tmp = TempDir::new().unwrap();
        for i in 0..=MAX_FILES {
            fs::write(tmp.path().join(format!("f{i}.md")), "x").unwrap();
        }
        assert_eq!(
            read_bundle(tmp.path(), &HashMap::new()).unwrap_err(),
            BundleError::TooManyFiles(MAX_FILES + 1)
        );
    }

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        let tmp = TempDir::new().unwrap();
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("foobar", "Zm9vYmFy"),
        ] {
            fs::write(tmp.path().join("v"), input).unwrap();
            let file = read_bundle(tmp.path(), &HashMap::new()).unwrap().remove(0);
            assert_eq!(file.base64(), expected, "{input:?}");
        }
    }

    #[test]
    fn non_utf8_content_is_a_blob() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("icon.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let files = read_bundle(tmp.path(), &HashMap::new()).unwrap();
        assert!(files[0].text().is_none());
        assert_eq!(files[0].mime_type(), "application/octet-stream");
    }

    /// An unchanged file is reused from the previous scan, so a rescan
    /// costs a stat per file rather than a read and a hash.
    #[test]
    fn an_unchanged_file_is_carried_over() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("SKILL.md"), "skill").unwrap();
        let first = read_bundle(tmp.path(), &HashMap::new()).unwrap();
        let previous: HashMap<PathBuf, BundleFile> = first.iter().map(|f| (f.abs.clone(), f.clone())).collect();

        let second = read_bundle(tmp.path(), &previous).unwrap();
        assert!(Arc::ptr_eq(&first[0].bytes, &second[0].bytes));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_not_served() {
        let tmp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("link")).unwrap();
        assert!(read_bundle(tmp.path(), &HashMap::new()).unwrap().is_empty());
    }
}
