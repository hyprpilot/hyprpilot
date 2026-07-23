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

/// Bundle output: every reference path in `refs.references` resolved
/// relative to `bundle_dir`, file body read, all concatenated with
/// `--- <basename> ---\n<body>\n` delimiters. Missing files surface as
/// `--- NOT FOUND: <path> ---` markers and do not abort the bundle.
#[must_use]
pub fn bundle_references(bundle_dir: &Path, refs: &FrontmatterRefs) -> String {
    let mut out = String::new();
    let mut missing: Vec<String> = Vec::new();
    for rel in &refs.references {
        let abs = bundle_dir.join(rel);
        match std::fs::read_to_string(&abs) {
            Ok(body) => {
                let basename = abs.file_name().and_then(|s| s.to_str()).unwrap_or(rel.as_str());
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("--- {basename} ---\n{body}\n"));
            }
            Err(_) => missing.push(rel.clone()),
        }
    }
    if !missing.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("--- NOT FOUND: {} ---\n", missing.join(", ")));
    }
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
            references: vec!["a.md".into(), "missing.md".into()],
        };
        let bundle = bundle_references(dir.path(), &refs);
        assert!(bundle.contains("--- a.md ---\nalpha body"));
        assert!(bundle.contains("--- NOT FOUND: missing.md ---"));
    }
}
