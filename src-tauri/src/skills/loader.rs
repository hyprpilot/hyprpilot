//! File-system loader for `<skills_dir>/<slug>/SKILL.md` bundles.
//! Mirrors `wayland/scripts/lib/skills.py::load_skills`: enumerate
//! every direct subdirectory, parse YAML frontmatter + markdown body
//! out of `SKILL.md`, skip bad entries with a warn log instead of
//! failing the whole registry.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_yaml::Value as YamlValue;
use tracing::warn;

use super::{Skill, SkillSlug};

/// Walk `dir` looking for `<slug>/SKILL.md`. Missing dir / unreadable
/// entries log + return an empty list; a bad individual skill logs
/// + is skipped. Always returns `Ok` — hard failure is the watcher's problem, not the loader's.
pub(crate) fn load_skills(dir: &Path) -> Result<Vec<Skill>> {
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(err) => {
            warn!(dir = %dir.display(), %err, "skills loader: read_dir failed — empty registry");
            return Ok(Vec::new());
        }
    };

    let mut out: Vec<Skill> = Vec::new();
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // symlink_metadata doesn't follow the link, so a dangling or
        // loop-forming symlink to a directory never hits is_dir().
        match entry.file_type() {
            Ok(ft) if ft.is_symlink() => {
                warn!(path = %path.display(), "skills loader: symlink entry — skipping");
                continue;
            }
            Ok(_) => {}
            Err(err) => {
                warn!(path = %path.display(), %err, "skills loader: file_type failed — skipping");
                continue;
            }
        }
        if !path.is_dir() {
            continue;
        }
        let Some(name_os) = path.file_name() else {
            continue;
        };
        let Some(name) = name_os.to_str() else {
            continue;
        };
        names.push((name.to_owned(), path));
    }
    names.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, skill_dir) in names {
        let slug = match SkillSlug::parse(&name) {
            Ok(s) => s,
            Err(err) => {
                warn!(name, %err, "skills loader: invalid slug — skipping");
                continue;
            }
        };
        let md = skill_dir.join("SKILL.md");
        if !md.is_file() {
            continue;
        }
        match parse_skill(&md, slug.clone()) {
            Ok(Some(skill)) => out.push(skill),
            Ok(None) => {}
            Err(err) => warn!(path = %md.display(), %err, "skills loader: parse failed — skipping"),
        }
    }
    Ok(out)
}

fn parse_skill(path: &Path, slug: SkillSlug) -> Result<Option<Skill>> {
    let text = fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&text);
    let body = body.trim().to_owned();
    if body.is_empty() {
        warn!(path = %path.display(), "skills loader: empty body — skipping");
        return Ok(None);
    }
    let title = frontmatter_str(&frontmatter, "title")
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let description = frontmatter_str(&frontmatter, "description")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Guidance for {}", slug.as_str()));
    let references = extract_references(&body);
    Ok(Some(Skill {
        slug,
        title,
        description,
        body,
        path: path.to_path_buf(),
        frontmatter,
        references,
    }))
}

/// Split `---\n…\n---\n<body>`. Missing / malformed frontmatter →
/// `(YamlValue::Null, original)`.
fn split_frontmatter(text: &str) -> (YamlValue, &str) {
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
    let parsed = serde_yaml::from_str::<YamlValue>(fm_text).unwrap_or_else(|err| {
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

/// Markdown link references `[text](target)` where `target` is a
/// relative path (no URL scheme). Mirrors the python loader's regex.
fn extract_references(body: &str) -> Vec<String> {
    static LINK_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("valid regex"));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for caps in LINK_PATTERN.captures_iter(body) {
        let target = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        if target.is_empty() {
            continue;
        }
        if target.contains("://") || target.starts_with('#') || target.starts_with("mailto:") {
            continue;
        }
        if seen.insert(target.to_owned()) {
            out.push(target.to_owned());
        }
    }
    out
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

    #[test]
    fn empty_dir_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let skills = load_skills(tmp.path()).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn missing_dir_returns_empty_vec() {
        let skills = load_skills(Path::new("/nonexistent-skills-dir-xyz")).unwrap();
        assert!(skills.is_empty());
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
        assert_eq!(s.references, vec!["../README.md".to_string()]);
        assert_eq!(s.title, "git-commit");
    }

    #[test]
    fn missing_title_field_resolves_to_empty_string() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "no-title",
            r#"---
description: no title field
---

# Body Heading

Body content.
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert_eq!(skills.len(), 1);
        // No `title` in frontmatter → empty string. Authors must
        // declare `title` explicitly; the H1 fallback was deleted.
        assert_eq!(skills[0].title, "");
    }

    #[test]
    fn skips_directory_without_skill_md() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("no-skill")).unwrap();
        let skills = load_skills(tmp.path()).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn skips_empty_body() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "empty",
            r#"---
description: nothing here
---

"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn bad_frontmatter_falls_back_to_empty_and_still_loads_body() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "broken",
            r#"---
: this is not
  : valid yaml
---

# Broken but still usable

Body kept.
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.slug.as_str(), "broken");
        assert!(s.body.contains("Body kept."));
    }

    #[test]
    fn invalid_slug_name_is_skipped() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "Invalid_CAPS",
            r#"---
description: irrelevant
---
# Title
body
"#,
        );
        write_skill(
            tmp.path(),
            "ok-slug",
            r#"---
description: kept
---
# Title
body
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].slug.as_str(), "ok-slug");
    }

    #[test]
    fn skills_are_sorted_by_slug() {
        let tmp = TempDir::new().unwrap();
        for name in ["zzz-last", "aaa-first", "mmm-middle"] {
            write_skill(
                tmp.path(),
                name,
                &format!(
                    r#"---
description: {name}
---

body
"#
                ),
            );
        }
        let skills = load_skills(tmp.path()).unwrap();
        let order: Vec<&str> = skills.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(order, ["aaa-first", "mmm-middle", "zzz-last"]);
    }

    #[test]
    fn references_ignore_absolute_urls_and_anchors() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "refs",
            r#"---
description: with refs
---

# References

See [docs](./docs.md), [github](https://github.com), [anchor](#foo),
and [mail](mailto:a@b.com). Plus [relative](../other.md).
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        let refs = &skills[0].references;
        assert!(refs.contains(&"./docs.md".to_string()));
        assert!(refs.contains(&"../other.md".to_string()));
        assert!(!refs.iter().any(|r| r.starts_with("http")));
        assert!(!refs.iter().any(|r| r.starts_with('#')));
        assert!(!refs.iter().any(|r| r.starts_with("mailto")));
    }

    #[test]
    fn description_falls_back_when_missing() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "nodesc",
            r#"---
name: nodesc
---

# Heading

body
"#,
        );
        let skills = load_skills(tmp.path()).unwrap();
        assert_eq!(skills[0].description, "Guidance for nodesc");
    }
}
