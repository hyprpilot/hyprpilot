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
use std::path::PathBuf;
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
    dirs: Vec<PathBuf>,
    skills: RwLock<HashMap<SkillSlug, Skill>>,
    order: RwLock<Vec<SkillSlug>>,
}

impl SkillsRegistry {
    /// Build a registry scanning every root in `dirs`. Does *not*
    /// call `reload` — callers trigger the initial load explicitly so
    /// boot-time failures surface in the daemon's logs next to the
    /// other init steps. Roots are stored as-is; `reload` skips
    /// missing ones with a warning.
    #[must_use]
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            skills: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
        }
    }

    /// Rescan the on-disk layout; replace the in-memory table on
    /// success. Roots are processed in `dirs` order — earlier roots
    /// win on slug collision.
    pub fn reload(&self) -> Result<()> {
        let mut order = Vec::new();
        let mut map: HashMap<SkillSlug, Skill> = HashMap::new();
        for dir in &self.dirs {
            if !dir.exists() {
                warn!(dir = %dir.display(), "skills root does not exist — skipping");
                continue;
            }
            let loaded = loader::load_skills(dir)?;
            for skill in loaded {
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
        let dirs_display: Vec<String> = self.dirs.iter().map(|p| p.display().to_string()).collect();
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

    #[cfg(test)]
    #[must_use]
    pub fn count(&self) -> usize {
        self.skills.read().expect("skills lock poisoned").len()
    }
}

impl std::fmt::Debug for SkillsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillsRegistry").field("dirs", &self.dirs).finish()
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

    fn build_registry(tmp: &TempDir) -> SkillsRegistry {
        SkillsRegistry::new(vec![tmp.path().to_path_buf()])
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
        let reg = SkillsRegistry::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
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
        let reg = SkillsRegistry::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]);
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
        let reg = SkillsRegistry::new(vec![missing, a.path().to_path_buf()]);
        reg.reload().unwrap();
        assert_eq!(reg.count(), 1);
        assert!(reg.get(&SkillSlug::parse("alpha").unwrap()).is_some());
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
