//! Reference resolution and the reference wire shape.
//!
//! A skill's YAML frontmatter declares `references: [path1, path2, ...]`,
//! each resolved relative to the skill bundle's directory.
//!
//! **A reference is addressed by its canonical PATH.** Not by a slug,
//! not by a name: those describe where a citation was found, while the
//! path is what the citation IS. The same shared file is declared by
//! dozens of skills, so a name plus a slug is one of many addresses for
//! one file, and a caller holding `output-diff` from one skill cannot
//! tell that another skill's citation is the same body. The path can be
//! compared, so it de-duplicates; it is unique by construction, so
//! nothing needs shadowing or collision rules; and it is what the
//! manifest already publishes.
//!
//! **A caller-supplied path is checked against the DECLARED set, never
//! joined.** The registry knows every path some skill actually declares;
//! anything else is refused. So the addressing surface reaches exactly
//! the files the skills already reference and no others.
//!
//! The DECLARED spelling (`../references/output-diff.md`) is meaningless
//! outside its bundle dir and never reaches the wire — only the
//! canonicalized absolute form, which collapses `..` so two spellings of
//! one file compare equal.
//!
//! A reference may carry its own frontmatter, parsed with the loader's
//! `split_frontmatter` rather than a second implementation, and served
//! with the fence stripped: leaving it in would put a bare `---` block
//! directly under our own header, where it reads as a delimiter rather
//! than as data.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::wire_metadata::frontmatter_json;
use super::wire_time::FileStat;

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
pub fn frontmatter_references(value: &yaml_serde::Value) -> FrontmatterRefs {
    let mut refs = Vec::new();
    if let Some(seq) = value.get("references").and_then(yaml_serde::Value::as_sequence) {
        for item in seq {
            if let Some(s) = item.as_str() {
                refs.push(s.to_string());
            }
        }
    }
    FrontmatterRefs { references: refs }
}

/// One resolved reference: which file it is, what to call it, when it
/// changed, and its own frontmatter.
///
/// Resolved per request rather than cached with the skill — a reference
/// is edited far more often than the skill that declares it, and
/// caching would serve a stale convention (and a stale mtime, the one
/// thing these fields exist to report) until an unrelated `reload`.
#[derive(Debug, Clone)]
pub struct ReferenceEntry {
    /// Canonical absolute path — the reference's identity AND its
    /// address. `None` only when the file cannot be resolved.
    pub path: Option<String>,
    /// Display label: the reference's frontmatter `name`, else the file
    /// stem. Never an address — two skills may legitimately use one
    /// label for different files.
    pub name: String,
    pub stat: FileStat,
    /// The reference's own frontmatter, verbatim. Nothing is defaulted
    /// in: hyprpilot enforces no invocation gate, so inventing a
    /// `disableModelInvocation` would imply a restriction that does not
    /// exist.
    pub metadata: Map<String, Value>,
    /// Declared, but the file could not be read.
    pub missing: bool,
    /// Body with any frontmatter fence stripped.
    body: String,
}

/// Resolve one file into an entry, reading its frontmatter for a name
/// override and its body for later bundling.
fn entry_for(path: &Path) -> ReferenceEntry {
    let stat = FileStat::read(path);
    let (frontmatter, body, missing) = match std::fs::read_to_string(path) {
        Ok(text) => {
            let (fm, body) = super::loader::split_frontmatter(&text);
            (fm, body.to_string(), false)
        }
        Err(_) => (yaml_serde::Value::Null, String::new(), true),
    };
    ReferenceEntry {
        path: std::fs::canonicalize(path).ok().map(|p| p.display().to_string()),
        name: resolve_name(&frontmatter, path),
        stat,
        metadata: frontmatter_json(&frontmatter),
        missing,
        body,
    }
}

/// Resolve every reference a skill declares, in declaration order.
#[must_use]
pub fn resolve(bundle_dir: &Path, refs: &FrontmatterRefs) -> Vec<ReferenceEntry> {
    refs.references
        .iter()
        .map(|rel| entry_for(&bundle_dir.join(rel)))
        .collect()
}

/// Resolve an explicit list of already-validated paths.
///
/// The caller is responsible for having checked each path against the
/// declared set — this function reads what it is given.
#[must_use]
pub fn resolve_paths(paths: &[String]) -> Vec<ReferenceEntry> {
    paths.iter().map(|p| entry_for(Path::new(p))).collect()
}

/// Every canonical path a skill declares. Built once per `reload` so a
/// load request can be validated without touching the filesystem.
#[must_use]
pub fn declared_paths(bundle_dir: &Path, refs: &FrontmatterRefs) -> Vec<String> {
    refs.references
        .iter()
        .filter_map(|rel| std::fs::canonicalize(bundle_dir.join(rel)).ok())
        .map(|p| p.display().to_string())
        .collect()
}

/// A reference's display name: its frontmatter `name` when it declares
/// a usable one, else the file stem.
fn resolve_name(frontmatter: &yaml_serde::Value, path: &Path) -> String {
    let declared = frontmatter
        .get("name")
        .and_then(yaml_serde::Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty());
    match declared {
        Some(name) => name.to_string(),
        None => stem(path),
    }
}

/// The file stem — filename without its final extension. Extensions are
/// a storage detail; the label is `output-diff`, not `output-diff.md`.
fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map_or_else(|| path.display().to_string(), str::to_string)
}

/// Render a JSON scalar as a YAML value. Strings are quoted when they
/// could otherwise be misread — a value containing `:`, `#`, a newline,
/// or leading/trailing space would break the block or change its
/// meaning. Anything non-scalar is JSON-encoded, which is valid YAML.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s)
            if !s.is_empty()
                && s.trim() == s
                && !s.contains([':', '#', '\n', '\r', '"', '\'', '[', ']', '{', '}', ','])
                && !s.starts_with(['-', '&', '*', '!', '|', '>', '%', '@', '`', '?']) =>
        {
            s.clone()
        }
        Value::String(s) => Value::String(s.clone()).to_string(),
        other => other.to_string(),
    }
}

impl ReferenceEntry {
    /// This entry's manifest row: which file it is, what to call it,
    /// and when it changed. `path` is both the identity and the address
    /// to pass back to `read_skill_references`.
    #[must_use]
    pub fn manifest_row(&self) -> Value {
        let mut row = Map::new();
        if let Some(path) = &self.path {
            row.insert("path".into(), Value::String(path.clone()));
        }
        row.insert("name".into(), Value::String(self.name.clone()));
        self.stat.extend(&mut row);
        if self.missing {
            row.insert("status".into(), Value::String("not-found".into()));
        }
        if !self.metadata.is_empty() {
            row.insert("metadata".into(), Value::Object(self.metadata.clone()));
        }
        row.into()
    }

    /// Header emitted before this reference's body in a bundle.
    ///
    /// YAML-shaped and self-delimiting, because a bundle may be appended
    /// to a skill body: a bare `--- <basename> ---` renders as a
    /// horizontal rule mid-document, so a reader cannot tell where the
    /// skill stops and a reference starts. This block can only be a
    /// delimiter.
    ///
    /// Built from [`Self::manifest_row`] so a reference you fetched
    /// carries the SAME metadata the manifest advertised. Full detail is
    /// affordable here and not in a listing: this is emitted once per
    /// reference you deliberately asked for.
    fn header(&self) -> String {
        let mut out = String::from("---\nreference:\n");
        let Value::Object(row) = self.manifest_row() else {
            unreachable!("manifest_row builds an object")
        };
        for (key, value) in &row {
            match value {
                Value::Object(nested) => {
                    out.push_str(&format!("  {key}:\n"));
                    for (k, v) in nested {
                        out.push_str(&format!("    {k}: {}\n", scalar(v)));
                    }
                }
                other => out.push_str(&format!("  {key}: {}\n", scalar(other))),
            }
        }
        out.push_str("---\n");
        out
    }
}

/// Concatenate `entries`, each preceded by its header.
///
/// A declared file that cannot be read contributes its header alone,
/// carrying `status: not-found` **in its declared position** — a
/// trailing summary would say a reference is missing without saying
/// where it belonged.
#[must_use]
pub fn bundle(entries: &[ReferenceEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&entry.header());
        if !entry.missing {
            out.push_str(&entry.body);
            if !entry.body.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// The manifest as a JSON array.
#[must_use]
pub fn manifest(entries: &[ReferenceEntry]) -> Value {
    Value::Array(entries.iter().map(ReferenceEntry::manifest_row).collect())
}

/// A text manifest naming every reference and the path that fetches it.
///
/// This is the safety net for the paths where structured metadata does
/// not reach the model: a skill RESOURCE read returns text plus `_meta`,
/// and many clients never surface `_meta`. Without this footer an
/// attached skill would lose its references with no in-context signal —
/// the exact silent gap that bundling-by-default used to prevent.
#[must_use]
pub fn manifest_footer(entries: &[ReferenceEntry], slug: &str) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n---\nskill_references:\n  skill: {slug}\n  count: {}\n  \
         note: bodies are NOT included above - pass the paths below to \
         `read_skill_references`\n  available:\n",
        entries.len()
    );
    for entry in entries {
        out.push_str(&format!("    - name: {}\n", scalar(&Value::String(entry.name.clone()))));
        match &entry.path {
            Some(path) => out.push_str(&format!("      path: {path}\n")),
            None => out.push_str("      status: not-found\n"),
        }
    }
    out.push_str("---\n");
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

/// Canonicalize a caller-supplied path so it can be compared against
/// the declared set. Returns `None` when it does not resolve.
#[must_use]
pub fn canonical(raw: &str) -> Option<String> {
    std::fs::canonicalize(PathBuf::from(raw))
        .ok()
        .map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn refs(paths: &[&str]) -> FrontmatterRefs {
        FrontmatterRefs {
            references: paths.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn parses_references_from_yaml_value() {
        let value: yaml_serde::Value =
            yaml_serde::from_str("references:\n  - ../references/a.md\n  - ./b.md\n").unwrap();
        let refs = frontmatter_references(&value);
        assert_eq!(refs.references, vec!["../references/a.md", "./b.md"]);
    }

    #[test]
    fn missing_references_is_empty() {
        let value: yaml_serde::Value = yaml_serde::from_str("name: no-refs\n").unwrap();
        assert!(frontmatter_references(&value).references.is_empty());
    }

    /// The path is the identity: two skills citing one shared file must
    /// produce the SAME path, or a caller cannot tell it already holds
    /// the body. Two skills' own local files must NOT.
    #[test]
    fn one_file_has_one_path_and_distinct_files_do_not() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("references")).unwrap();
        fs::write(root.path().join("references/shared.md"), "shared").unwrap();
        for skill in ["skill-a", "skill-b"] {
            fs::create_dir_all(root.path().join(skill).join("references")).unwrap();
            fs::write(root.path().join(skill).join("references/local.md"), "local").unwrap();
        }

        let a = resolve(
            &root.path().join("skill-a"),
            &refs(&["../references/shared.md", "./references/local.md"]),
        );
        let b = resolve(
            &root.path().join("skill-b"),
            &refs(&["../references/shared.md", "./references/local.md"]),
        );

        assert_eq!(a[0].path, b[0].path, "one shared file must have one identity");
        assert_ne!(a[1].path, b[1].path, "distinct local files must not collide");
        // The LABEL collides where the path does not — which is exactly
        // why the label is not the address.
        assert_eq!(a[1].name, b[1].name);
    }

    /// Two references with the same label inside ONE skill are both
    /// fully addressable — there is no shadowing, because the address
    /// is the path and paths are unique by construction.
    #[test]
    fn a_repeated_label_shadows_nothing() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::write(dir.path().join("references/dup.md"), "LOCAL").unwrap();
        fs::write(dir.path().join("dup.md"), "SHARED").unwrap();

        let entries = resolve(dir.path(), &refs(&["dup.md", "./references/dup.md"]));

        assert_eq!(entries[0].name, entries[1].name, "labels collide");
        assert_ne!(entries[0].path, entries[1].path, "addresses do not");
        assert!(entries.iter().all(|e| e.path.is_some()));
        let all = bundle(&entries);
        assert!(all.contains("SHARED") && all.contains("LOCAL"));
    }

    /// The declared spelling never reaches the wire — only the resolved
    /// form, which has no `..` left to follow.
    #[test]
    fn only_the_canonical_path_is_published() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::write(dir.path().join("references/output-diff.md"), "body").unwrap();

        let entries = resolve(dir.path(), &refs(&["./references/output-diff.md"]));

        for surface in [
            manifest(&entries).to_string(),
            bundle(&entries),
            manifest_footer(&entries, "git-commit"),
        ] {
            assert!(
                !surface.contains("./references/"),
                "declared spelling leaked: {surface}"
            );
            assert!(!surface.contains(".."), "unresolved traversal leaked: {surface}");
        }
        assert!(entries[0].path.as_ref().unwrap().ends_with("references/output-diff.md"));
    }

    /// A reference's own frontmatter renames it and rides through, and
    /// its fence is consumed rather than replayed as a second delimiter.
    #[test]
    fn frontmatter_renames_the_reference_and_the_fence_is_stripped() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("a.md"),
            "---\nname: renamed\ntitle: A\n---\nthe actual body\n",
        )
        .unwrap();

        let entries = resolve(dir.path(), &refs(&["a.md"]));

        assert_eq!(entries[0].name, "renamed");
        let bundled = bundle(&entries);
        assert!(bundled.ends_with("---\nthe actual body\n"), "{bundled}");
        assert_eq!(bundled.matches("\n---\n").count(), 1, "exactly one fence: {bundled}");
        assert!(bundled.contains("    title: A"));
    }

    /// Nothing is invented into a reference's metadata. hyprpilot
    /// enforces no invocation gate, so a stamped `disableModelInvocation`
    /// would imply a restriction that does not exist.
    #[test]
    fn no_keys_are_invented_into_reference_metadata() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "body\n").unwrap();

        let entries = resolve(dir.path(), &refs(&["a.md"]));

        assert!(entries[0].metadata.is_empty(), "{:?}", entries[0].metadata);
    }

    /// A declared-but-unreadable file is skill-data rot: it keeps its
    /// declared position and says so, rather than vanishing.
    #[test]
    fn a_missing_file_keeps_its_declared_position_as_a_marker() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();

        let entries = resolve(dir.path(), &refs(&["gone.md", "b.md"]));
        let bundled = bundle(&entries);

        assert!(entries[0].missing);
        assert!(entries[0].path.is_none(), "an unresolvable file has no address");
        let gone = bundled.find("name: gone").unwrap();
        let beta = bundled.find("name: b\n").unwrap();
        assert!(gone < beta, "a missing reference stays in declaration order");
        assert!(bundled.contains("status: not-found"));
    }

    /// `declared_paths` is what a load request is validated against, so
    /// it must agree with the manifest's `path` exactly.
    #[test]
    fn declared_paths_match_the_manifest_addresses() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();

        let declared = declared_paths(dir.path(), &refs(&["a.md", "b.md", "gone.md"]));
        let entries = resolve(dir.path(), &refs(&["a.md", "b.md", "gone.md"]));

        let addresses: Vec<String> = entries.iter().filter_map(|e| e.path.clone()).collect();
        assert_eq!(declared, addresses);
        assert_eq!(declared.len(), 2, "an unresolvable declaration contributes no address");
    }

    /// The load gate is `canonical()` + membership in the declared set.
    /// Canonicalizing FIRST is what lets a caller pass any spelling of a
    /// declared file, and what stops a different file sneaking in under
    /// a spelling that merely looks like one.
    #[test]
    fn canonical_normalises_into_the_declared_set_without_widening_it() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::write(dir.path().join("references/a.md"), "alpha").unwrap();
        fs::write(dir.path().join("undeclared.md"), "nope").unwrap();
        let declared: std::collections::HashSet<String> = declared_paths(dir.path(), &refs(&["./references/a.md"]))
            .into_iter()
            .collect();

        let direct = canonical(&dir.path().join("references/a.md").display().to_string()).unwrap();
        assert!(declared.contains(&direct));

        // A different spelling of the SAME file resolves in.
        let roundabout = canonical(&dir.path().join("references/../references/a.md").display().to_string()).unwrap();
        assert!(declared.contains(&roundabout));
        assert_eq!(direct, roundabout);

        // A real file that no skill declares stays out, even though it
        // sits inside the same bundle dir.
        let sibling = canonical(&dir.path().join("undeclared.md").display().to_string()).unwrap();
        assert!(!declared.contains(&sibling));

        // A path that does not resolve at all yields nothing to check.
        assert!(canonical("/nonexistent-hyprpilot-probe-xyz").is_none());
    }

    #[test]
    fn resolve_paths_reads_exactly_what_it_is_given() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "ALPHA").unwrap();
        fs::write(dir.path().join("b.md"), "BETA").unwrap();
        let declared = declared_paths(dir.path(), &refs(&["a.md", "b.md"]));

        let out = bundle(&resolve_paths(&declared[..1]));

        assert!(out.contains("ALPHA"));
        assert!(!out.contains("BETA"));
    }

    /// A value that would break the YAML block gets quoted rather than
    /// silently corrupting the header a reader relies on as a delimiter.
    #[test]
    fn header_values_that_would_break_yaml_are_quoted() {
        assert_eq!(scalar(&Value::String("output-diff".into())), "output-diff");
        assert_eq!(scalar(&Value::String("has: colon".into())), "\"has: colon\"");
        assert_eq!(scalar(&Value::String("two\nlines".into())), "\"two\\nlines\"");
        assert_eq!(scalar(&Value::String("- leading dash".into())), "\"- leading dash\"");
        assert_eq!(scalar(&Value::String(" padded ".into())), "\" padded \"");
        assert_eq!(scalar(&serde_json::json!(["a", "b"])), "[\"a\",\"b\"]");
    }

    #[test]
    fn the_footer_names_every_reference_and_its_path() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();

        let footer = manifest_footer(&resolve(dir.path(), &refs(&["a.md", "b.md"])), "s");

        assert!(footer.contains("count: 2"));
        assert!(footer.contains("NOT included"));
        assert_eq!(footer.matches("      path: /").count(), 2);
    }

    #[test]
    fn the_footer_is_empty_for_a_skill_declaring_nothing() {
        assert!(manifest_footer(&[], "s").is_empty());
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
