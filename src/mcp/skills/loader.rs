//! File-system loader for skill bundles: every directory under a root
//! that holds a `SKILL.md` is a skill, at any depth, so
//! `<root>/acme/billing/refunds/SKILL.md` is the skill `acme/billing/
//! refunds` and a skill may nest inside another. Parses YAML
//! frontmatter + markdown body out of each `SKILL.md`, and skips a bad
//! entry with a warn log instead of failing the whole registry.

use std::fs;
use std::path::{Component, Path};

use anyhow::Result;
use tracing::warn;
use yaml_serde::Value as YamlValue;

use super::{Skill, SkillSlug};

/// The one walker every skill-tree scan goes through — discovery here
/// and the bundle file listing the sidecar serves — so the two can never
/// disagree about which files exist.
///
/// Hidden entries and anything `.gitignore`d are skipped, which is what
/// keeps a script's `.venv` from becoming hundreds of served files.
/// `require_git(false)` honours an ignore file outside a repository too.
/// Symlinks are not followed: a link could point a skill at any file on
/// the host.
pub(crate) fn walk(dir: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(dir)
        .require_git(false)
        .follow_links(false)
        .sort_by_file_name(std::ffi::OsStr::cmp)
        .build()
}

/// Walk `dir` for every `SKILL.md` below it. A missing root yields an
/// empty list; a bad individual skill logs and is skipped. Always
/// returns `Ok` — a bad skill root never aborts the registry build.
pub(crate) fn load_skills(dir: &Path) -> Result<Vec<Skill>> {
    let mut out: Vec<Skill> = Vec::new();
    for entry in walk(dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!(dir = %dir.display(), %err, "skills loader: walk error — skipping entry");
                continue;
            }
        };
        if entry.file_name() != "SKILL.md" || !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let md = entry.path();
        let Some(slug) = slug_for(dir, md) else {
            continue;
        };
        match parse_skill(md, slug) {
            Ok(Some(skill)) => out.push(skill),
            Ok(None) => {}
            Err(err) => warn!(path = %md.display(), %err, "skills loader: parse failed — skipping"),
        }
    }
    out.sort_by(|a, b| a.slug.as_str().cmp(b.slug.as_str()));
    Ok(out)
}

/// The slug a `SKILL.md` at `md` gets under `root`: its directory's
/// path relative to the root. `None`, with the reason logged, when that
/// is not a valid slug — including a `SKILL.md` at the root itself,
/// which would be a skill with no name.
fn slug_for(root: &Path, md: &Path) -> Option<SkillSlug> {
    let rel = md.parent()?.strip_prefix(root).ok()?;
    let mut segments = Vec::new();
    for component in rel.components() {
        let Component::Normal(segment) = component else {
            return None;
        };
        segments.push(segment.to_str()?);
    }
    let raw = segments.join("/");
    match SkillSlug::parse(&raw) {
        Ok(slug) => Some(slug),
        Err(err) => {
            warn!(path = %md.display(), %err, "skills loader: invalid skill path — skipping");
            None
        }
    }
}

fn parse_skill(path: &Path, slug: SkillSlug) -> Result<Option<Skill>> {
    let text = fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&text);
    let body = body.trim().to_owned();
    if body.is_empty() {
        warn!(path = %path.display(), "skills loader: empty body — skipping");
        return Ok(None);
    }
    // SEP-2640 serves the frontmatter verbatim, requires `name` and
    // `description` in it, and makes the URI's last segment equal `name`
    // — a host refuses anything else, so serving it would only move the
    // failure somewhere the author cannot see it. Unparseable
    // frontmatter arrives here as `Null` and fails the same check.
    let Some(name) = frontmatter_str(&frontmatter, "name") else {
        warn!(path = %path.display(), "skills loader: no frontmatter `name` — skipping");
        return Ok(None);
    };
    if name != slug.name() {
        warn!(
            path = %path.display(),
            name,
            expected = slug.name(),
            "skills loader: frontmatter `name` differs from its directory — skipping"
        );
        return Ok(None);
    }
    let Some(description) = frontmatter_str(&frontmatter, "description").filter(|d| !d.trim().is_empty()) else {
        warn!(path = %path.display(), "skills loader: no frontmatter `description` — skipping");
        return Ok(None);
    };
    let description = description.to_owned();
    let title = frontmatter_str(&frontmatter, "title")
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    Ok(Some(Skill {
        slug,
        title,
        description,
        body,
        path: path.to_path_buf(),
        frontmatter,
    }))
}

/// Split `---\n…\n---\n<body>`. Missing / malformed frontmatter →
/// `(YamlValue::Null, original)`.
///
/// Shared with `wire_references`: a reference file may carry its own
/// frontmatter, and parsing that with a second implementation is how the
/// two drift.
pub(crate) fn split_frontmatter(text: &str) -> (YamlValue, &str) {
    let stripped = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"));
    let Some(rest) = stripped else {
        return (YamlValue::Null, text);
    };
    let Some(end_idx) = find_fence_end(rest) else {
        return (YamlValue::Null, text);
    };
    let (fm_text, body_with_fence) = rest.split_at(end_idx);
    let body = body_with_fence
        .strip_prefix("---\n")
        .or_else(|| body_with_fence.strip_prefix("---\r\n"))
        .unwrap_or(body_with_fence);
    let parsed = yaml_serde::from_str::<YamlValue>(fm_text).unwrap_or_else(|err| {
        warn!(%err, "skills loader: frontmatter yaml parse failed — treating as empty");
        YamlValue::Null
    });
    (parsed, body)
}

fn find_fence_end(rest: &str) -> Option<usize> {
    // Search for a line that is exactly `---` (with either LF or CRLF).
    let mut search_start = 0usize;
    while search_start < rest.len() {
        let remaining = &rest[search_start..];
        let nl = remaining.find('\n')?;
        let line_end = search_start + nl;
        let line = &rest[search_start..line_end];
        let line_trimmed = line.strip_suffix('\r').unwrap_or(line);
        if line_trimmed == "---" {
            return Some(search_start);
        }
        search_start = line_end + 1;
    }
    None
}

fn frontmatter_str<'a>(fm: &'a YamlValue, key: &str) -> Option<&'a str> {
    fm.get(key).and_then(YamlValue::as_str)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_skill(dir: &Path, slug: &str, body: &str) {
        let skill_dir = dir.join(slug);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), body).unwrap();
    }

    fn valid(name: &str) -> String {
        format!("---\nname: {name}\ndescription: about {name}\n---\n\nbody of {name}\n")
    }

    fn slugs(skills: &[Skill]) -> Vec<&str> {
        skills.iter().map(|s| s.slug.as_str()).collect()
    }

    #[test]
    fn empty_dir_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn missing_dir_returns_empty_vec() {
        assert!(load_skills(Path::new("/nonexistent-skills-dir-xyz"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn loads_one_skill_with_frontmatter_and_body() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "git-commit",
            r#"---
name: git-commit
title: git-commit
description: Stage and commit changes
---

# git-commit

Body. See [README](../README.md) for more.
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.slug.as_str(), "git-commit");
        assert_eq!(s.description, "Stage and commit changes");
        assert!(s.body.contains("Body."));
        assert_eq!(s.title, "git-commit");
    }

    #[test]
    fn missing_title_field_resolves_to_empty_string() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "no-title", &valid("no-title"));
        assert_eq!(load_skills(tmp.path()).unwrap()[0].title, "");
    }

    #[test]
    fn skips_directory_without_skill_md() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("no-skill")).unwrap();
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn skips_empty_body() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "empty", "---\nname: empty\ndescription: d\n---\n\n");
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }

    /// The frontmatter is served verbatim and a SEP-2640 host refuses an
    /// entry without `name` and `description`, so neither an absent field
    /// nor a block that failed to parse may reach the catalogue.
    #[test]
    fn a_skill_the_spec_would_refuse_is_skipped() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "no-name", "---\ndescription: d\n---\n\nbody\n");
        write_skill(tmp.path(), "no-description", "---\nname: no-description\n---\n\nbody\n");
        write_skill(
            tmp.path(),
            "broken",
            "---\n: this is not\n  : valid yaml\n---\n\nbody\n",
        );
        write_skill(tmp.path(), "no-fence", "# just markdown\n");
        write_skill(tmp.path(), "kept", &valid("kept"));
        assert_eq!(slugs(&load_skills(tmp.path()).unwrap()), ["kept"]);
    }

    /// The URI's last segment IS the name, so a mismatch is two names
    /// for one skill and no host can load it.
    #[test]
    fn a_name_that_differs_from_its_directory_is_skipped() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "dir-name", &valid("other-name"));
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn invalid_slug_name_is_skipped() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "Invalid_CAPS", &valid("Invalid_CAPS"));
        write_skill(tmp.path(), "ok-slug", &valid("ok-slug"));
        assert_eq!(slugs(&load_skills(tmp.path()).unwrap()), ["ok-slug"]);
    }

    #[test]
    fn skills_are_sorted_by_slug() {
        let tmp = TempDir::new().unwrap();
        for name in ["zzz-last", "aaa-first", "mmm-middle"] {
            write_skill(tmp.path(), name, &valid(name));
        }
        assert_eq!(
            slugs(&load_skills(tmp.path()).unwrap()),
            ["aaa-first", "mmm-middle", "zzz-last"]
        );
    }

    /// A category directory is an organizational prefix, and a skill may
    /// sit inside another — both are discovered under their full path.
    #[test]
    fn nested_skills_are_discovered_under_their_path() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "acme/billing/refunds", &valid("refunds"));
        write_skill(tmp.path(), "outer", &valid("outer"));
        write_skill(tmp.path(), "outer/inner", &valid("inner"));
        assert_eq!(
            slugs(&load_skills(tmp.path()).unwrap()),
            ["acme/billing/refunds", "outer", "outer/inner"]
        );
    }

    /// A `.venv` or editor directory must never be a source of skills,
    /// and neither may anything the tree's own ignore files exclude.
    #[test]
    fn hidden_and_gitignored_trees_are_never_skills() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), ".hidden/sneaky", &valid("sneaky"));
        write_skill(tmp.path(), "vendored/thing", &valid("thing"));
        fs::write(tmp.path().join(".gitignore"), "vendored/\n").unwrap();
        write_skill(tmp.path(), "kept", &valid("kept"));
        assert_eq!(slugs(&load_skills(tmp.path()).unwrap()), ["kept"]);
    }

    /// A `SKILL.md` at the root would be a skill with no name.
    #[test]
    fn a_skill_md_at_the_root_is_ignored() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("SKILL.md"), valid("root")).unwrap();
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_skill_is_not_followed() {
        let tmp = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        write_skill(elsewhere.path(), "linked", &valid("linked"));
        std::os::unix::fs::symlink(elsewhere.path().join("linked"), tmp.path().join("linked")).unwrap();
        assert!(load_skills(tmp.path()).unwrap().is_empty());
    }
}
