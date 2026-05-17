//! Per-instance skill set the daemon hands to the sidecar via repeated
//! `--skill <slug>=<path-to-SKILL.md>` CLI args.
//!
//! The sidecar deliberately does NOT call back into the daemon for the
//! skill catalog. Per-instance scoping happens at arg-passing time, not
//! at runtime — the daemon already knows which skills the active
//! profile resolves to, and the sidecar trusts the manifest verbatim.
//!
//! On `reload`, the sidecar re-reads every `SKILL.md` from its known
//! paths. Adding/removing skills mid-session requires the vendor to
//! respawn the sidecar (vendor-driven, out of scope for `reload`).

use std::path::{Path, PathBuf};

/// One entry from the daemon's `--skill slug=path` CLI arg. `path`
/// always points at the `SKILL.md` file itself, not the bundle
/// directory — the references resolver derives the bundle dir via
/// `path.parent()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub slug: String,
    pub path: PathBuf,
}

impl ManifestEntry {
    /// Bundle directory — the parent of `SKILL.md`. References declared
    /// in frontmatter resolve relative to this directory.
    #[must_use]
    pub fn bundle_dir(&self) -> Option<&Path> {
        self.path.parent()
    }
}

/// `clap` value parser for the repeated `--skill <slug>=<path>` arg.
/// Returns a sharp error on missing `=` or an empty slug so the daemon
/// gets a clean failure mode if it ever ships a malformed entry.
///
/// # Errors
/// Returns a string error suitable for `clap`'s `value_parser` when the
/// input doesn't contain `=` or has an empty slug / path component.
pub fn parse_skill_arg(raw: &str) -> Result<ManifestEntry, String> {
    let (slug, path) = raw
        .split_once('=')
        .ok_or_else(|| format!("--skill must be `<slug>=<path>`; got {raw:?}"))?;
    if slug.is_empty() {
        return Err(format!("--skill slug is empty in {raw:?}"));
    }
    if path.is_empty() {
        return Err(format!("--skill path is empty in {raw:?}"));
    }
    Ok(ManifestEntry {
        slug: slug.to_string(),
        path: PathBuf::from(path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed() {
        let m = parse_skill_arg("git-commit=/tmp/skills/git-commit/SKILL.md").unwrap();
        assert_eq!(m.slug, "git-commit");
        assert_eq!(m.path, PathBuf::from("/tmp/skills/git-commit/SKILL.md"));
    }

    #[test]
    fn bundle_dir_is_parent() {
        let m = ManifestEntry {
            slug: "x".into(),
            path: PathBuf::from("/tmp/x/SKILL.md"),
        };
        assert_eq!(m.bundle_dir(), Some(Path::new("/tmp/x")));
    }

    #[test]
    fn rejects_missing_separator() {
        assert!(parse_skill_arg("git-commit").is_err());
    }

    #[test]
    fn rejects_empty_components() {
        assert!(parse_skill_arg("=/path").is_err());
        assert!(parse_skill_arg("slug=").is_err());
    }
}
