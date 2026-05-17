//! Frontmatter reference resolution — mirrors mcphub's pattern
//! (`~/.config/nvim/lua/ck/plugins/mcphub-nvim.lua:572-589`).
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

/// Pull the `references` array out of a SKILL.md's YAML frontmatter.
/// Returns an empty `FrontmatterRefs` when there is no frontmatter or
/// `references` is missing/empty.
#[must_use]
pub fn parse_frontmatter_references(body: &str) -> FrontmatterRefs {
    let Some(yaml) = strip_frontmatter(body) else {
        return FrontmatterRefs::default();
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(yaml) {
        Ok(v) => v,
        Err(_) => return FrontmatterRefs::default(),
    };
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

/// Pull the YAML between the leading `---` and the next `---`. Returns
/// `None` if the file isn't a frontmatter-shaped markdown doc.
fn strip_frontmatter(body: &str) -> Option<&str> {
    let body = body.strip_prefix("---\n").or_else(|| body.strip_prefix("---\r\n"))?;
    let end = body.find("\n---\n").or_else(|| body.find("\r\n---\r\n"))?;
    Some(&body[..end])
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
    fn parses_references_array() {
        let body = "---\nreferences:\n  - ../references/a.md\n  - ./b.md\n---\nbody";
        let refs = parse_frontmatter_references(body);
        assert_eq!(refs.references, vec!["../references/a.md", "./b.md"]);
    }

    #[test]
    fn missing_frontmatter_is_empty() {
        let refs = parse_frontmatter_references("plain markdown");
        assert!(refs.references.is_empty());
    }

    #[test]
    fn malformed_yaml_is_empty() {
        let refs = parse_frontmatter_references("---\nreferences: [unclosed\n---\n");
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
