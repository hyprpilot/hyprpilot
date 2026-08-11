//! Reference resolution and the reference wire shape.
//!
//! A skill's YAML frontmatter declares `references: [path1, path2, ...]`.
//! Each path resolves relative to the skill bundle's directory and is
//! addressed on the wire by a NAME — the file stem, or the reference's
//! own frontmatter `name` where it declares one.
//!
//! **The name is looked up, never joined.** A caller supplies a name; the
//! server matches it against this list and takes the path from the
//! frontmatter. No caller-supplied string reaches `Path::join`, so the
//! reference surface adds no traversal reachable from a request.
//!
//! **The DECLARED path never reaches the wire; the RESOLVED one does.**
//! `../references/output-diff.md` is meaningless outside its bundle dir
//! and is not published. The canonicalized absolute path is, as
//! `path` — because a name is an address, not an IDENTITY, and
//! de-duplication needs identity. The same shared file is declared by
//! dozens of skills, so `git-commit/output-diff` and
//! `git-push/output-diff` are one file that must not be loaded twice;
//! meanwhile two skills' own `./references/output-diff.md` share a name
//! and are different files. Only the resolved path separates those
//! cases, and canonicalizing collapses `..` so two spellings of one file
//! compare equal.
//!
//! A reference may carry its own frontmatter, parsed with the loader's
//! `split_frontmatter` rather than a second implementation, and served
//! with the fence stripped: leaving it in would put a bare `---` block
//! directly under our own header, where it reads as a delimiter rather
//! than as data.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value};
use tracing::warn;

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

/// One resolved reference: how it is addressed, when it changed, and
/// its own metadata. Built per request rather than cached with the
/// skill — a reference is edited far more often than the skill that
/// declares it, and caching would serve a stale convention (and a stale
/// mtime, which is the one thing these fields exist to report) until an
/// unrelated `reload`.
#[derive(Debug, Clone)]
pub struct ReferenceEntry {
    /// Wire name: the reference's frontmatter `name`, else the file
    /// stem. Never contains a path separator.
    pub name: String,
    /// `None` when this entry is shadowed — a shadowed entry has no
    /// address of its own, and pointing it at the winner's URI would be
    /// a confidently wrong answer.
    pub uri: Option<String>,
    /// Canonical absolute path — the reference's IDENTITY, and the only
    /// thing that tells a caller two skills cite the same file. `None`
    /// when it cannot be resolved (the file is missing).
    pub path: Option<String>,
    pub stat: FileStat,
    /// The reference's own frontmatter, verbatim.
    pub metadata: Map<String, Value>,
    /// A later declaration whose name was already taken. Kept in the
    /// manifest (never silently dropped) and still served by the full
    /// bundle, but not individually addressable.
    pub shadowed: bool,
    /// Declared, but the file could not be read.
    pub missing: bool,
    /// Body with any frontmatter fence stripped.
    body: String,
}

/// Resolve every declared reference: read it, parse its frontmatter,
/// derive its name, and stat it.
///
/// Name resolution is first-wins, matching how `SkillsRegistry` already
/// resolves a slug collision across roots. A loser is marked `shadowed`
/// and warned rather than dropped — silently losing a declared
/// reference is the exact failure the reference surface exists to
/// prevent.
#[must_use]
pub fn resolve(bundle_dir: &Path, slug: &str, refs: &FrontmatterRefs) -> Vec<ReferenceEntry> {
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::with_capacity(refs.references.len());

    for rel in &refs.references {
        let path = bundle_dir.join(rel);
        let stat = FileStat::read(&path);
        let (frontmatter, body, missing) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (fm, body) = super::loader::split_frontmatter(&text);
                (fm, body.to_string(), false)
            }
            Err(_) => (serde_yaml::Value::Null, String::new(), true),
        };

        let name = resolve_name(&frontmatter, rel);
        let shadowed = !taken.insert(name.clone());
        if shadowed {
            warn!(
                skill = slug,
                name = %name,
                declaration = %rel,
                "mcp::skills: two references resolve to the same name — the first wins; \
                 this one stays in the full bundle but is not individually addressable"
            );
        }

        out.push(ReferenceEntry {
            uri: (!shadowed).then(|| reference_uri(slug, &name)),
            // Canonicalized so `../references/x.md` from two different
            // skills collapses to one identity a caller can compare.
            path: std::fs::canonicalize(&path).ok().map(|p| p.display().to_string()),
            metadata: frontmatter_json(&frontmatter),
            name,
            stat,
            shadowed,
            missing,
            body,
        });
    }

    out
}

/// A reference's wire name: its frontmatter `name` when it declares a
/// usable one, else the file stem.
///
/// A declared name carrying a path separator, or one that is blank, is
/// rejected back to the stem — the name rides a URI segment, and a
/// `/`-bearing name would produce a URI that parses as something else
/// entirely.
fn resolve_name(frontmatter: &serde_yaml::Value, rel: &str) -> String {
    let declared = frontmatter
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty() && !n.contains('/') && !n.contains('\\'));
    match declared {
        Some(name) => name.to_string(),
        None => stem(rel),
    }
}

/// The file stem — filename without its final extension. Extensions are
/// a storage detail; the wire addresses `output-diff`, not
/// `output-diff.md`.
fn stem(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel)
        .to_string()
}

fn reference_uri(slug: &str, name: &str) -> String {
    format!("hyprpilot://references/{slug}/{name}")
}

/// Render a JSON scalar as a bare YAML value. Strings lose their quotes
/// — these are paths, names and timestamps, and the header is read, not
/// parsed back. Anything non-scalar is JSON-encoded rather than dropped.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl ReferenceEntry {
    /// This entry's manifest row: what it is called, how to fetch it,
    /// which file it actually is, and when it changed.
    #[must_use]
    pub fn manifest_row(&self) -> Value {
        let mut row = Map::new();
        row.insert("name".into(), Value::String(self.name.clone()));
        if let Some(uri) = &self.uri {
            row.insert("uri".into(), Value::String(uri.clone()));
        }
        // Identity, so a caller holding this file under another skill's
        // name can tell it already has it.
        if let Some(path) = &self.path {
            row.insert("path".into(), Value::String(path.clone()));
        }
        self.stat.extend(&mut row);
        if self.shadowed {
            row.insert("shadowed".into(), Value::Bool(true));
        }
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
    /// YAML-shaped and self-delimiting, because a bundle is appended to
    /// the skill body: a bare `--- <basename> ---` renders as a
    /// horizontal rule mid-document, so a reader cannot tell where the
    /// skill stops and a reference starts. This block can only be a
    /// delimiter.
    /// Built from [`Self::manifest_row`] so the two can never disagree:
    /// a reference you actually fetched carries the SAME full metadata
    /// the manifest advertised — including `path`, so a reader can tell
    /// this is the file it already holds under another skill's name.
    ///
    /// Full detail is affordable here and not in a listing: this is
    /// emitted once per reference you deliberately asked for, whereas
    /// `resources/list` pays for every skill in the catalogue.
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

/// Concatenate the selected references, each preceded by its header.
///
/// `select` of `None` bundles everything. `Some(names)` bundles exactly
/// those, in the caller's order; **an empty slice bundles nothing** —
/// an explicitly empty selection must never decay into its opposite,
/// the same rule `--no-delegates` follows for an empty profile scope.
///
/// An unknown name is an `Err` naming every unknown entry: addressing a
/// reference that does not exist is a caller error, and answering it
/// with a partial bundle would let the caller believe it received what
/// it asked for. A DECLARED name whose file cannot be read is different
/// — that is skill-data rot, and it surfaces as a `status: not-found`
/// header in its declared position so the gap is visible where it
/// belongs.
///
/// A repeated name is served ONCE. Without that, `["x", "x", "x"]`
/// returns three copies of one body — a caller assembling a selection
/// from several steps of a skill would amplify its own response for no
/// gain.
pub fn bundle(entries: &[ReferenceEntry], select: Option<&[String]>) -> Result<String, Vec<String>> {
    let chosen: Vec<&ReferenceEntry> = match select {
        None => entries.iter().collect(),
        Some(names) => {
            let mut chosen: Vec<&ReferenceEntry> = Vec::with_capacity(names.len());
            let mut unknown = Vec::new();
            let mut seen = BTreeSet::new();
            for name in names {
                if !seen.insert(name.clone()) {
                    continue;
                }
                // A shadowed entry is not individually addressable — it
                // has no URI, so accepting it by name here would make
                // the name resolve to two different things depending on
                // which door the caller used.
                match entries.iter().find(|e| &e.name == name && !e.shadowed) {
                    Some(entry) => chosen.push(entry),
                    None => unknown.push(name.clone()),
                }
            }
            if !unknown.is_empty() {
                return Err(unknown);
            }
            chosen
        }
    };

    let mut out = String::new();
    for entry in chosen {
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
    Ok(out)
}

/// The manifest as a JSON array — the full working view, for
/// `read_skill` and `load_skill_references`.
#[must_use]
pub fn manifest(entries: &[ReferenceEntry]) -> Value {
    Value::Array(entries.iter().map(ReferenceEntry::manifest_row).collect())
}

/// The individually addressable entry for `name`, if any.
///
/// Shadowed entries are excluded for the same reason `bundle` excludes
/// them: a name must resolve to one thing regardless of which door the
/// caller used.
#[must_use]
pub fn find<'a>(entries: &'a [ReferenceEntry], name: &str) -> Option<&'a ReferenceEntry> {
    entries.iter().find(|e| e.name == name && !e.shadowed)
}

/// A text manifest naming every reference and its URI.
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
         note: bodies are NOT included above - read a uri below, or call \
         `load_skill_references`\n  available:\n",
        entries.len()
    );
    for entry in entries {
        out.push_str(&format!("    - name: {}\n", entry.name));
        match &entry.uri {
            Some(uri) => out.push_str(&format!("      uri: {uri}\n")),
            None => out.push_str("      shadowed: true\n"),
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
        let value: serde_yaml::Value =
            serde_yaml::from_str("references:\n  - ../references/a.md\n  - ./b.md\n").unwrap();
        let refs = frontmatter_references(&value);
        assert_eq!(refs.references, vec!["../references/a.md", "./b.md"]);
    }

    #[test]
    fn missing_references_is_empty() {
        let value: serde_yaml::Value = serde_yaml::from_str("name: no-refs\n").unwrap();
        assert!(frontmatter_references(&value).references.is_empty());
    }

    /// The name is the stem for BOTH declaration forms — the shared
    /// `../references/x.md` and the skill-local `./references/x.md` —
    /// and never carries the extension.
    #[test]
    fn the_name_is_the_stem_for_both_declaration_forms() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::create_dir_all(dir.path().join("../shared")).ok();
        fs::write(dir.path().join("references/local.md"), "local body").unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["./references/local.md"]));

        assert_eq!(entries[0].name, "local");
        assert_eq!(entries[0].uri.as_deref(), Some("hyprpilot://references/s/local"));
    }

    /// `file_stem` strips only the LAST extension, so a dotted filename
    /// keeps its inner dots. Harmless for URI parsing (the split is on
    /// `/`), but pinned so it is a decision rather than a surprise.
    #[test]
    fn a_dotted_filename_keeps_its_inner_dots() {
        assert_eq!(stem("../references/plan.v2.md"), "plan.v2");
    }

    /// A reference's own frontmatter renames it, and the fence is
    /// stripped from the served body.
    #[test]
    fn frontmatter_renames_the_reference_and_the_fence_is_stripped() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("a.md"),
            "---\nname: renamed\ntitle: A\n---\nthe actual body\n",
        )
        .unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["a.md"]));

        assert_eq!(entries[0].name, "renamed");
        assert_eq!(entries[0].uri.as_deref(), Some("hyprpilot://references/s/renamed"));
        let bundled = bundle(&entries, None).unwrap();
        // Our header, then the body — the reference's own fence is
        // consumed into `metadata`, never replayed as a second `---`
        // block that a reader would take for a delimiter.
        assert!(bundled.ends_with("---\nthe actual body\n"), "{bundled}");
        assert_eq!(bundled.matches("\n---\n").count(), 1, "exactly one fence: {bundled}");
        // Its keys survive, inside our header rather than loose above
        // the body.
        assert!(bundled.contains("    title: A"));
    }

    /// A declared name that cannot ride a URI segment falls back to the
    /// stem rather than producing a URI that parses as something else.
    #[test]
    fn an_unusable_declared_name_falls_back_to_the_stem() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "---\nname: \"has/slash\"\n---\nbody\n").unwrap();
        fs::write(dir.path().join("b.md"), "---\nname: \"   \"\n---\nbody\n").unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["a.md", "b.md"]));

        assert_eq!(entries[0].name, "a");
        assert_eq!(entries[1].name, "b");
    }

    /// A reference's frontmatter rides through verbatim, and NOTHING is
    /// defaulted into it. hyprpilot enforces no invocation gate — a
    /// stamped `disableModelInvocation` would imply a restriction that
    /// does not exist, on every entry.
    #[test]
    fn reference_frontmatter_rides_through_without_invented_keys() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "body\n").unwrap();
        fs::write(dir.path().join("b.md"), "---\nowner: captain\n---\nbody\n").unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["a.md", "b.md"]));

        assert!(
            entries[0].metadata.is_empty(),
            "no frontmatter means no metadata, not a fabricated default: {:?}",
            entries[0].metadata
        );
        assert_eq!(
            entries[1].metadata.get("owner").and_then(Value::as_str),
            Some("captain")
        );
    }

    /// The canonical path is the reference's IDENTITY: two skills citing
    /// one shared file must produce the SAME path, or a caller cannot
    /// tell it already holds the body.
    #[test]
    fn two_skills_citing_one_file_report_the_same_path() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("references")).unwrap();
        fs::write(root.path().join("references/shared.md"), "shared body").unwrap();
        fs::create_dir_all(root.path().join("skill-a")).unwrap();
        fs::create_dir_all(root.path().join("skill-b")).unwrap();

        let a = resolve(
            &root.path().join("skill-a"),
            "skill-a",
            &refs(&["../references/shared.md"]),
        );
        let b = resolve(
            &root.path().join("skill-b"),
            "skill-b",
            &refs(&["../references/shared.md"]),
        );

        assert!(a[0].path.is_some());
        assert_eq!(a[0].path, b[0].path, "one file must have one identity");
        assert_ne!(a[0].uri, b[0].uri, "but each skill addresses it under its own slug");
    }

    /// First-wins on a name collision: the loser keeps its manifest row
    /// (never silently dropped) but has no URI of its own, and is still
    /// carried by the full bundle.
    #[test]
    fn a_name_collision_shadows_the_loser_without_dropping_it() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::write(dir.path().join("references/dup.md"), "local body").unwrap();
        fs::write(dir.path().join("dup.md"), "shared body").unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["dup.md", "./references/dup.md"]));

        assert_eq!(entries.len(), 2, "the loser is kept, not dropped");
        assert!(!entries[0].shadowed);
        assert_eq!(entries[0].uri.as_deref(), Some("hyprpilot://references/s/dup"));
        assert!(entries[1].shadowed);
        assert_eq!(
            entries[1].uri, None,
            "a shadowed entry must not advertise the winner's uri"
        );

        // Still served by the full bundle.
        let all = bundle(&entries, None).unwrap();
        assert!(all.contains("shared body"));
        assert!(all.contains("local body"));

        // But not addressable by name — the name resolves to one thing.
        let picked = bundle(&entries, Some(&["dup".to_string()])).unwrap();
        assert!(picked.contains("shared body"));
        assert!(!picked.contains("local body"));
    }

    /// An explicitly empty selection bundles NOTHING. Letting `[]` mean
    /// "everything" is the same footgun `--no-delegates` exists to avoid:
    /// an empty list must never decay into its opposite.
    #[test]
    fn an_empty_selection_bundles_nothing_rather_than_everything() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();
        let entries = resolve(dir.path(), "s", &refs(&["a.md"]));

        assert!(bundle(&entries, Some(&[])).unwrap().is_empty());
        assert!(bundle(&entries, None).unwrap().contains("alpha"));
    }

    /// An unknown name is an error naming every unknown entry — a
    /// partial bundle would let the caller believe it got what it asked
    /// for.
    #[test]
    fn unknown_names_error_and_are_all_named() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();
        let entries = resolve(dir.path(), "s", &refs(&["a.md"]));

        let err = bundle(
            &entries,
            Some(&["a".to_string(), "nope".to_string(), "also-nope".to_string()]),
        )
        .unwrap_err();

        assert_eq!(err, vec!["nope", "also-nope"]);
    }

    /// A declared-but-unreadable file is skill-data rot, not a caller
    /// error: it surfaces as a marker IN ITS DECLARED POSITION, because
    /// a trailing summary would say a reference is missing without
    /// saying where it belonged.
    #[test]
    fn a_missing_file_keeps_its_declared_position_as_a_marker() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();

        let entries = resolve(dir.path(), "s", &refs(&["gone.md", "b.md"]));
        let bundled = bundle(&entries, None).unwrap();

        assert!(entries[0].missing);
        let gone = bundled.find("name: gone").unwrap();
        let beta = bundled.find("name: b\n").unwrap();
        assert!(gone < beta, "a missing reference stays in declaration order");
        assert!(bundled.contains("status: not-found"));
    }

    /// The DECLARED path stays internal — it is meaningless outside its
    /// bundle dir. The RESOLVED one is published as identity, but only
    /// on the manifest: the bundle header and footer are for reading,
    /// not for locating.
    #[test]
    fn the_declared_path_stays_internal_and_only_the_manifest_carries_identity() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("references")).unwrap();
        fs::write(dir.path().join("references/output-diff.md"), "body").unwrap();

        let entries = resolve(dir.path(), "git-commit", &refs(&["./references/output-diff.md"]));

        for surface in [
            manifest(&entries).to_string(),
            bundle(&entries, None).unwrap(),
            manifest_footer(&entries, "git-commit"),
        ] {
            assert!(
                !surface.contains("./references/output-diff.md"),
                "the declared spelling leaked: {surface}"
            );
        }
        assert!(manifest(&entries).to_string().contains("\"path\":"));
        assert!(!manifest_footer(&entries, "git-commit").contains("path:"));
    }

    /// A repeated name is served once — a caller assembling a selection
    /// across several steps must not amplify its own response.
    #[test]
    fn a_repeated_selection_is_served_once() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "ALPHA-BODY").unwrap();
        let entries = resolve(dir.path(), "s", &refs(&["a.md"]));

        let out = bundle(&entries, Some(&["a".into(), "a".into(), "a".into()])).unwrap();

        assert_eq!(out.matches("ALPHA-BODY").count(), 1);
    }

    #[test]
    fn the_manifest_carries_name_uri_and_timestamps() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();

        let rows = manifest(&resolve(dir.path(), "s", &refs(&["a.md"])));
        let row = &rows[0];

        assert_eq!(row["name"], "a");
        assert_eq!(row["uri"], "hyprpilot://references/s/a");
        assert_eq!(row["size"], 5);
        assert!(row["modified"].as_str().unwrap().ends_with('Z'));
    }

    /// The footer is the in-band signal for clients that never surface
    /// `_meta` — it must name every reference and how to reach it.
    #[test]
    fn the_footer_names_every_reference_and_says_bodies_are_absent() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "alpha").unwrap();
        fs::write(dir.path().join("b.md"), "beta").unwrap();

        let footer = manifest_footer(&resolve(dir.path(), "s", &refs(&["a.md", "b.md"])), "s");

        assert!(footer.contains("count: 2"));
        assert!(footer.contains("NOT included"));
        assert!(footer.contains("uri: hyprpilot://references/s/a"));
        assert!(footer.contains("uri: hyprpilot://references/s/b"));
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
