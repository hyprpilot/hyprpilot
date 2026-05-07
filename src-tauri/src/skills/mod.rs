//! Skill loader — parses `<root>/<slug>/SKILL.md` bundles across
//! every configured root and exposes them to the daemon via
//! `SkillsRegistry`. Reload is captain-driven: the palette's
//! "reload skills" entry calls `skills/reload` (mirrored as a Tauri
//! command); fs-watching was dropped because edit-time noise from
//! editors / git ops burnt through the debouncer faster than skills
//! changed.
//!
//! Skill delivery onto the wire flows exclusively through the
//! palette-driven `Attachment` shape on `UserTurnInput::Prompt` — no
//! inline-token expansion runs server-side; raw user text passes
//! through the `session/submit` handler verbatim.

pub mod commands;
mod loader;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
use tracing::{info, warn};

/// Directory-name slug. Constructor enforces the
/// `[a-z0-9][a-z0-9_-]*` shape so filesystem + RPC lookups share one
/// ground truth — a string that doesn't parse can't live in the
/// registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillSlug(String);

impl SkillSlug {
    /// Validate `raw` as a skill slug. Rejects empty, path separators,
    /// `..`, and anything outside `[a-z0-9_-]` (must also start with
    /// alphanum).
    pub fn parse(raw: &str) -> Result<Self, SlugError> {
        if raw.is_empty() {
            return Err(SlugError::Empty);
        }
        if raw == "." || raw == ".." {
            return Err(SlugError::Reserved);
        }
        if raw.contains('/') || raw.contains('\\') {
            return Err(SlugError::Separator);
        }
        let mut chars = raw.chars();
        let first = chars.next().expect("non-empty");
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(SlugError::BadLead);
        }
        for c in chars {
            let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
            if !ok {
                return Err(SlugError::BadChar(c));
            }
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SkillSlug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for SkillSlug {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for SkillSlug {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(|e| D::Error::custom(format!("invalid skill slug '{raw}': {e}")))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SlugError {
    #[error("slug is empty")]
    Empty,
    #[error("slug cannot be '.' or '..'")]
    Reserved,
    #[error("slug cannot contain path separators")]
    Separator,
    #[error("slug must start with [a-z0-9]")]
    BadLead,
    #[error("slug contains invalid character '{0}' — must match [a-z0-9_-]")]
    BadChar(char),
}

/// One loaded skill. Carries the full body + frontmatter so the RPC
/// `skills/get` handler can render everything; the listing endpoint
/// emits a [`SkillSummary`] instead to keep wire payloads slim.
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub slug: SkillSlug,
    pub title: String,
    pub description: String,
    pub body: String,
    pub path: PathBuf,
    /// Raw YAML frontmatter; `serde_yaml::Value` to stay agnostic of
    /// the author's schema.
    pub frontmatter: YamlValue,
    /// Relative paths extracted from markdown links in the body.
    pub references: Vec<String>,
}

/// Slim wire shape for `skills/list`. `body` + `frontmatter` stay
/// behind `skills/get` so a listing over a thousand skills doesn't
/// ship megabytes of markdown.
#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub slug: SkillSlug,
    pub title: String,
    pub description: String,
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            slug: s.slug.clone(),
            title: s.title.clone(),
            description: s.description.clone(),
        }
    }
}

/// Owned skill catalogue. Carries its configured roots `dirs` so
/// call sites never re-pass them. `reload` rescans every root.
/// First-root-wins on slug collision (warn names both paths); missing
/// roots warn + skip (no auto-mkdir, no canonicalize).
pub struct SkillsRegistry {
    entries: Vec<crate::config::ResolvedSkillEntry>,
    skills: RwLock<HashMap<SkillSlug, Skill>>,
    order: RwLock<Vec<SkillSlug>>,
}

impl SkillsRegistry {
    /// Build a registry scanning every root in `entries`. Does *not*
    /// call `reload` — callers trigger the initial load explicitly so
    /// boot-time failures surface in the daemon's logs next to the
    /// other init steps. Roots are stored as-is; `reload` skips
    /// missing ones with a warning. Per-entry `ignore` glob (when
    /// present) drops slugs matching any pattern post-load.
    #[must_use]
    pub fn new(entries: Vec<crate::config::ResolvedSkillEntry>) -> Self {
        Self {
            entries,
            skills: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
        }
    }

    /// Rescan the on-disk layout; replace the in-memory table on
    /// success. Roots are processed in iteration order — earlier
    /// roots win on slug collision.
    pub fn reload(&self) -> Result<()> {
        let mut order = Vec::new();
        let mut map: HashMap<SkillSlug, Skill> = HashMap::new();
        for entry in &self.entries {
            if !entry.dir.exists() {
                warn!(dir = %entry.dir.display(), "skills root does not exist — skipping");
                continue;
            }
            let loaded = loader::load_skills(&entry.dir)?;
            for skill in loaded {
                if let Some(glob) = &entry.ignore {
                    if glob.is_match(skill.slug.as_str()) {
                        warn!(
                            slug = %skill.slug,
                            dir = %entry.dir.display(),
                            "skills registry: slug matches ignore glob — skipping",
                        );
                        continue;
                    }
                }
                if let Some(prev) = map.get(&skill.slug) {
                    warn!(
                        slug = %skill.slug,
                        kept = %prev.path.display(),
                        skipped = %skill.path.display(),
                        "skills registry: slug collision — first root wins",
                    );
                    continue;
                }
                order.push(skill.slug.clone());
                map.insert(skill.slug.clone(), skill);
            }
        }
        let count = map.len();
        {
            let mut skills = self.skills.write().expect("skills lock poisoned");
            let mut ord = self.order.write().expect("order lock poisoned");
            *skills = map;
            *ord = order;
        }
        let dirs_display: Vec<String> = self.entries.iter().map(|e| e.dir.display().to_string()).collect();
        info!(count, dirs = ?dirs_display, "skills registry: reloaded");
        Ok(())
    }

    /// Snapshot of every loaded skill, sorted by slug. Clones are
    /// cheap — skill bodies are behind `Arc` / owned strings and the
    /// caller usually pulls one or two per call.
    #[must_use]
    pub fn list(&self) -> Vec<Skill> {
        let skills = self.skills.read().expect("skills lock poisoned");
        let order = self.order.read().expect("order lock poisoned");
        order.iter().filter_map(|slug| skills.get(slug).cloned()).collect()
    }

    /// Lookup by slug. Returns an owned clone so the caller doesn't
    /// hold the read lock across their work.
    #[must_use]
    pub fn get(&self, slug: &SkillSlug) -> Option<Skill> {
        let skills = self.skills.read().expect("skills lock poisoned");
        skills.get(slug).cloned()
    }

    /// Materialise a Claude Code SDK plugin at `plugin_dir` carrying
    /// only the skills currently in this registry (post-ignore-glob
    /// filter). The plugin layout is the upstream contract:
    ///
    /// ```text
    /// <plugin_dir>/
    ///   .claude-plugin/
    ///     plugin.json   { "name": "...", "version": "1.0.0" }
    ///   skills/
    ///     <slug-a> -> <real-skill-dir-a>   (symlink)
    ///     <slug-b> -> <real-skill-dir-b>   (symlink)
    /// ```
    ///
    /// Each `<slug>` is a symlink to the real `<root>/<slug>/` dir
    /// that holds `SKILL.md` and any companion files — the SDK reads
    /// `SKILL.md` directly off the symlink target. Repeated calls
    /// against the same `plugin_dir` clear the existing `skills/`
    /// subdirectory first so a stale leftover from a crashed prior
    /// run can't shadow the current registry's view.
    pub fn materialize_plugin(&self, plugin_dir: &Path, plugin_name: &str) -> std::io::Result<()> {
        let manifest_dir = plugin_dir.join(".claude-plugin");
        let skills_dir = plugin_dir.join("skills");

        if skills_dir.exists() {
            std::fs::remove_dir_all(&skills_dir)?;
        }
        std::fs::create_dir_all(&manifest_dir)?;
        std::fs::create_dir_all(&skills_dir)?;

        let manifest = serde_json::json!({
            "name": plugin_name,
            "version": "1.0.0",
            "description": "hyprpilot per-instance filtered skill plugin",
        });
        std::fs::write(
            manifest_dir.join("plugin.json"),
            serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
        )?;

        let skills = self.skills.read().expect("skills lock poisoned");
        let order = self.order.read().expect("order lock poisoned");
        let mut linked = 0usize;
        for slug in order.iter() {
            let Some(skill) = skills.get(slug) else { continue };
            let Some(source_dir) = skill.path.parent() else {
                warn!(slug = %slug, "skills materialize: skill path has no parent — skipping");
                continue;
            };
            let link_target = skills_dir.join(slug.as_str());
            #[cfg(unix)]
            if let Err(err) = std::os::unix::fs::symlink(source_dir, &link_target) {
                warn!(slug = %slug, %err, "skills materialize: symlink failed — skipping");
                continue;
            }
            linked += 1;
        }
        info!(
            plugin = %plugin_dir.display(),
            count = linked,
            "skills registry: materialised plugin",
        );
        Ok(())
    }

    #[cfg(test)]
    #[must_use]
    pub fn count(&self) -> usize {
        self.skills.read().expect("skills lock poisoned").len()
    }
}

impl std::fmt::Debug for SkillsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dirs: Vec<&PathBuf> = self.entries.iter().map(|e| &e.dir).collect();
        f.debug_struct("SkillsRegistry").field("dirs", &dirs).finish()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    fn seed_skill(dir: &Path, slug: &str, desc: &str, body: &str) {
        let skill_dir = dir.join(slug);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\ndescription: {desc}\n---\n\n# {slug}\n\n{body}\n"),
        )
        .unwrap();
    }

    fn entry(dir: PathBuf) -> crate::config::ResolvedSkillEntry {
        crate::config::ResolvedSkillEntry { dir, ignore: None }
    }

    fn entry_with_ignore(dir: PathBuf, patterns: &[&str]) -> crate::config::ResolvedSkillEntry {
        let mut builder = globset::GlobSetBuilder::new();
        for p in patterns {
            builder.add(globset::Glob::new(p).expect("test glob compiles"));
        }
        crate::config::ResolvedSkillEntry {
            dir,
            ignore: Some(builder.build().expect("test glob set builds")),
        }
    }

    fn build_registry(tmp: &TempDir) -> SkillsRegistry {
        SkillsRegistry::new(vec![entry(tmp.path().to_path_buf())])
    }

    #[test]
    fn slug_parse_rejects_bad_shapes() {
        assert!(SkillSlug::parse("").is_err());
        assert!(SkillSlug::parse(".").is_err());
        assert!(SkillSlug::parse("..").is_err());
        assert!(SkillSlug::parse("foo/bar").is_err());
        assert!(SkillSlug::parse("Foo").is_err());
        assert!(SkillSlug::parse("-leading").is_err());
        assert!(SkillSlug::parse("has space").is_err());
        assert!(SkillSlug::parse("ok").is_ok());
        assert!(SkillSlug::parse("my-skill_v2").is_ok());
        assert!(SkillSlug::parse("1leading-digit").is_ok());
    }

    #[test]
    fn reload_fills_registry_in_dir_order() {
        let tmp = TempDir::new().unwrap();
        seed_skill(tmp.path(), "a", "alpha", "alpha body");
        seed_skill(tmp.path(), "b", "beta", "beta body");
        let reg = build_registry(&tmp);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 2);

        let list = reg.list();
        let ids: Vec<&str> = list.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn get_returns_some_for_known_and_none_for_unknown() {
        let tmp = TempDir::new().unwrap();
        seed_skill(tmp.path(), "known", "k", "k body");
        let reg = build_registry(&tmp);
        reg.reload().unwrap();
        let ok = SkillSlug::parse("known").unwrap();
        let miss = SkillSlug::parse("missing").unwrap();
        assert!(reg.get(&ok).is_some());
        assert!(reg.get(&miss).is_none());
    }

    #[test]
    fn multi_root_loads_skills_from_every_existing_dir() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        seed_skill(a.path(), "a-skill", "from a", "a body");
        seed_skill(b.path(), "b-skill", "from b", "b body");
        let reg = SkillsRegistry::new(vec![entry(a.path().to_path_buf()), entry(b.path().to_path_buf())]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 2);
        assert!(reg.get(&SkillSlug::parse("a-skill").unwrap()).is_some());
        assert!(reg.get(&SkillSlug::parse("b-skill").unwrap()).is_some());
    }

    #[test]
    fn multi_root_first_wins_on_slug_collision() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        seed_skill(a.path(), "shared", "from a", "FROM_A");
        seed_skill(b.path(), "shared", "from b", "FROM_B");
        let reg = SkillsRegistry::new(vec![entry(a.path().to_path_buf()), entry(b.path().to_path_buf())]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 1);
        let kept = reg.get(&SkillSlug::parse("shared").unwrap()).unwrap();
        assert!(kept.body.contains("FROM_A"));
        assert!(kept.path.starts_with(a.path()));
    }

    #[test]
    fn missing_root_warns_and_skips_without_panic() {
        let a = TempDir::new().unwrap();
        seed_skill(a.path(), "alpha", "alpha", "alpha body");
        let missing = std::path::PathBuf::from("/nonexistent-skills-root-xyz-k268");
        let reg = SkillsRegistry::new(vec![entry(missing), entry(a.path().to_path_buf())]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 1);
        assert!(reg.get(&SkillSlug::parse("alpha").unwrap()).is_some());
    }

    #[test]
    fn ignore_glob_drops_matching_slugs() {
        let tmp = TempDir::new().unwrap();
        seed_skill(tmp.path(), "git-commit", "git", "body");
        seed_skill(tmp.path(), "work-internal", "work", "body");
        seed_skill(tmp.path(), "work-experimental", "work", "body");
        let reg = SkillsRegistry::new(vec![entry_with_ignore(tmp.path().to_path_buf(), &["work-*"])]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 1);
        assert!(reg.get(&SkillSlug::parse("git-commit").unwrap()).is_some());
        assert!(reg.get(&SkillSlug::parse("work-internal").unwrap()).is_none());
    }

    #[test]
    fn materialize_plugin_writes_manifest_and_symlinks_only_visible_skills() {
        let src = TempDir::new().unwrap();
        seed_skill(src.path(), "git-commit", "git", "body");
        seed_skill(src.path(), "work-internal", "work", "body");
        seed_skill(src.path(), "work-experimental", "work", "body");
        let reg = SkillsRegistry::new(vec![entry_with_ignore(src.path().to_path_buf(), &["work-*"])]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 1, "only git-commit survives the ignore filter");

        let plugin_root = TempDir::new().unwrap();
        let plugin_dir = plugin_root.path().join("plugin");
        reg.materialize_plugin(&plugin_dir, "hyprpilot-skills-test").unwrap();

        // Manifest exists and has the right shape.
        let manifest_path = plugin_dir.join(".claude-plugin/plugin.json");
        assert!(manifest_path.is_file(), "plugin.json was created");
        let manifest_text = fs::read_to_string(&manifest_path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["name"], "hyprpilot-skills-test");
        assert_eq!(manifest["version"], "1.0.0");

        // Symlinks: only git-commit, NOT the work-* entries.
        let skills_dir = plugin_dir.join("skills");
        let git_commit_link = skills_dir.join("git-commit");
        assert!(git_commit_link.is_symlink(), "git-commit symlink present");
        assert!(!skills_dir.join("work-internal").exists(), "work-internal filtered out");
        assert!(
            !skills_dir.join("work-experimental").exists(),
            "work-experimental filtered out"
        );

        // Symlink resolves to the real skill dir holding SKILL.md.
        let resolved = fs::canonicalize(&git_commit_link).unwrap();
        assert!(resolved.join("SKILL.md").is_file(), "symlink target carries SKILL.md");
    }

    #[test]
    fn materialize_plugin_clears_stale_skills_subdir() {
        let src = TempDir::new().unwrap();
        seed_skill(src.path(), "fresh", "f", "body");
        let reg = SkillsRegistry::new(vec![entry(src.path().to_path_buf())]);
        reg.reload().unwrap();

        let plugin_root = TempDir::new().unwrap();
        let plugin_dir = plugin_root.path().join("plugin");
        // Pre-seed a stale subdir entry that no longer matches the
        // active registry — simulates a crashed prior run leaving
        // stale state.
        let skills_dir = plugin_dir.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        let stale = skills_dir.join("ghost-from-prior-run");
        fs::create_dir(&stale).unwrap();

        reg.materialize_plugin(&plugin_dir, "hyprpilot-skills-clear-test")
            .unwrap();
        assert!(!stale.exists(), "stale subdir cleared");
        assert!(skills_dir.join("fresh").is_symlink(), "fresh skill linked");
    }

    #[test]
    fn skill_summary_does_not_leak_body() {
        let skill = Skill {
            slug: SkillSlug::parse("x").unwrap(),
            title: "X".into(),
            description: "desc".into(),
            body: "SECRET BODY MATERIAL".into(),
            path: PathBuf::from("/tmp/x"),
            frontmatter: YamlValue::Null,
            references: Vec::new(),
        };
        let summary = SkillSummary::from(&skill);
        let v = serde_json::to_value(&summary).unwrap();
        assert!(v.get("body").is_none());
        assert!(v.get("frontmatter").is_none());
        assert!(v.get("references").is_none());
        assert_eq!(v["slug"], "x");
        assert_eq!(v["title"], "X");
        assert_eq!(v["description"], "desc");
    }
}
