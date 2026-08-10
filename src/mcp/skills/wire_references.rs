//! Frontmatter reference resolution — reuses the shared skill
//! loader's parsed frontmatter.
//!
//! A skill's YAML frontmatter declares `references: [path1, path2, ...]`;
//! the loader resolves each path relative to the skill bundle's
//! directory, reads the file, and concatenates everything with
//! `--- <basename> ---\n<body>` delimiters. Missing files append a
//! `--- NOT FOUND: <path> ---` marker so the consumer sees the gap
//! without the request failing.

use std::path::Path;

/// Parsed frontmatter — only the slice we care about. The full skill
/// loader keeps richer data; the sidecar only needs the references
/// array to do its bundle.
#[derive(Debug, Default, Clone)]
pub struct FrontmatterRefs {
    pub references: Vec<String>,
}

/// Pull the `references` array out of already-parsed YAML frontmatter.
/// This is the sidecar path: the shared loader has parsed the same
/// frontmatter once while building the `Skill`, so MCP metadata and
/// reference bundling should read that source instead of reparsing the
/// markdown body.
#[must_use]
pub fn frontmatter_references(value: &serde_yaml::Value) -> FrontmatterRefs {
    let mut refs = Vec::new();
    if let Some(seq) = value.get("references").and_then(serde_yaml::Value::as_sequence) {
        for item in seq {
            if let Some(s) = item.as_str() {
                refs.push(s.to_string());
            }
        }
    }
    FrontmatterRefs { references: refs }
}

/// Header emitted before every reference body.
///
/// YAML-shaped and self-delimiting, because a bundle is appended to the
/// skill body: a bare `--- <basename> ---` renders as a horizontal rule
/// mid-document, so a reader cannot tell where the skill stops and a
/// reference starts. This block can only be a delimiter, and it carries
/// the declared path rather than just a filename.
///
/// `name` is the stem, not the filename — skill bodies cite references
/// as `` `output-diff` ``, so the header matches what the prose says.
fn reference_header(name: &str, path: &str, missing: bool) -> String {
    let status = if missing { "\n  status: not-found" } else { "" };
    format!("---\nreference:\n  name: {name}\n  path: {path}{status}\n---\n")
}

/// Bundle output: every reference path in `refs.references` resolved
/// relative to `bundle_dir`, file body read, each preceded by its
/// [`reference_header`]. A file that cannot be read yields a header
/// carrying `status: not-found` **in its declared position** — a
/// trailing summary would say a reference is missing without saying
/// where it belonged in the sequence — and does not abort the bundle.
#[must_use]
pub fn bundle_references(bundle_dir: &Path, refs: &FrontmatterRefs) -> String {
    let mut out = String::new();
    for rel in &refs.references {
        let name = Path::new(rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(rel.as_str());
        if !out.is_empty() {
            out.push('\n');
        }
        match std::fs::read_to_string(bundle_dir.join(rel)) {
            Ok(body) => {
                out.push_str(&reference_header(name, rel, false));
                out.push_str(&body);
                if !body.ends_with('\n') {
                    out.push('\n');
                }
            }
            Err(_) => out.push_str(&reference_header(name, rel, true)),
        }
    }
    out
}

/// Append `bundle` to a skill `body` under a banner naming the skill and
/// how many references follow. Without the banner an appended bundle
/// reads as more skill body; with it, the boundary is stated and the
/// count is checkable against the frontmatter.
///
/// An empty bundle returns `body` untouched — a skill declaring nothing
/// gains no trailing marker.
#[must_use]
pub fn append_references(body: &str, slug: &str, count: usize, bundle: &str) -> String {
    if bundle.is_empty() {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len() + bundle.len() + 96);
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "\n---\nskill_references:\n  skill: {slug}\n  count: {count}\n---\n\n"
    ));
    out.push_str(bundle);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_references_from_yaml_value() {
        let value: serde_yaml::Value =
            serde_yaml::from_str("references:\n  - ../references/a.md\n  - ./b.md\n").unwrap();
        let refs = frontmatter_references(&value);
        assert_eq!(refs.references, vec!["../references/a.md", "./b.md"]);
    }

    #[test]
    fn missing_references_is_empty() {
        let value: serde_yaml::Value = serde_yaml::from_str("name: no-refs\n").unwrap();
        let refs = frontmatter_references(&value);
        assert!(refs.references.is_empty());
    }

    #[test]
    fn bundle_concatenates_with_delimiters_and_reports_missing() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha body").unwrap();
        let refs = FrontmatterRefs {
            references: vec!["a.md".into(), "sub/missing.md".into()],
        };
        let bundle = bundle_references(dir.path(), &refs);
        assert!(bundle.contains("---\nreference:\n  name: a\n  path: a.md\n---\nalpha body"));
        assert!(bundle.contains("  name: missing\n  path: sub/missing.md\n  status: not-found"));
    }

    #[test]
    fn missing_reference_keeps_its_declared_position() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();
        let refs = FrontmatterRefs {
            references: vec!["gone.md".into(), "b.md".into()],
        };
        let bundle = bundle_references(dir.path(), &refs);
        let gone = bundle.find("name: gone").unwrap();
        let beta = bundle.find("name: b\n").unwrap();
        assert!(gone < beta, "missing reference must stay in declaration order");
    }

    #[test]
    fn append_wraps_bundle_in_a_counted_banner() {
        let out = append_references("skill body", "git-commit", 2, "BUNDLE");
        assert!(out.starts_with("skill body\n"));
        assert!(out.contains("---\nskill_references:\n  skill: git-commit\n  count: 2\n---\n"));
        assert!(out.ends_with("BUNDLE"));
    }

    #[test]
    fn append_is_a_noop_without_references() {
        assert_eq!(append_references("skill body", "solo", 0, ""), "skill body");
    }
}
