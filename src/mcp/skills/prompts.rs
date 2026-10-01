//! Prompt files the skills server serves as MCP prompts — the profile's
//! own `system_prompt` files plus every `*.md` in a `[[mcp.skills.
//! prompts]]` directory.
//!
//! The point is reload. A system prompt is baked into the vendor at
//! launch and nothing can replace it mid-session, but the same file
//! served as a prompt can be re-invoked after an edit — a slash command
//! in Claude Code and opencode, a tool call in Hermes, a resource read
//! everywhere else.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::config::ResolvedSkillEntry;

/// Where a server's prompts come from: the profile's `system_prompt`
/// files and the `[[mcp.skills.prompts]]` directories. The launcher
/// builds it to gate injection and to write the sidecar's argv; the
/// sidecar builds the same thing back from that argv and reloads it on
/// every rescan.
#[derive(Debug, Clone, Default)]
pub struct PromptSources {
    pub files: Vec<PathBuf>,
    pub dirs: Vec<ResolvedSkillEntry>,
}

impl PromptSources {
    #[must_use]
    pub fn load(&self) -> Vec<Prompt> {
        load_prompts(&self.files, &self.dirs)
    }
}

/// One prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The frontmatter `name`, else the file stem. What the prompt is
    /// listed and fetched as, so it carries MCP's tool-name charset.
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The file with any frontmatter fence stripped.
    pub body: String,
    pub path: PathBuf,
}

/// Load every prompt: `files` first, in order, then each directory's
/// `*.md` entries sorted by name. A file that cannot be read, has no
/// body or carries an unusable name is skipped with a warning; the
/// first prompt to claim a name keeps it.
#[must_use]
pub fn load_prompts(files: &[PathBuf], dirs: &[ResolvedSkillEntry]) -> Vec<Prompt> {
    let mut out: Vec<Prompt> = Vec::new();
    let mut seen = HashSet::new();
    let mut keep = |prompt: Prompt, out: &mut Vec<Prompt>| {
        if seen.insert(prompt.name.clone()) {
            out.push(prompt);
        } else {
            warn!(name = %prompt.name, path = %prompt.path.display(), "skills: prompt name taken — first wins");
        }
    };

    for file in files {
        if let Some(prompt) = read_prompt(file) {
            keep(prompt, &mut out);
        }
    }
    for entry in dirs {
        let Ok(read) = std::fs::read_dir(&entry.dir) else {
            warn!(dir = %entry.dir.display(), "skills: prompt directory unreadable — skipping");
            continue;
        };
        let mut paths: Vec<PathBuf> = read
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|ft| ft.is_file()))
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|ext| ext == "md")
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('.'))
            })
            .collect();
        paths.sort();
        for path in paths {
            let Some(prompt) = read_prompt(&path) else { continue };
            if entry.include.as_ref().is_some_and(|glob| !glob.is_match(&prompt.name))
                || entry.ignore.as_ref().is_some_and(|glob| glob.is_match(&prompt.name))
            {
                continue;
            }
            keep(prompt, &mut out);
        }
    }
    out
}

fn read_prompt(path: &Path) -> Option<Prompt> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            warn!(path = %path.display(), %err, "skills: prompt file unreadable — skipping");
            return None;
        }
    };
    let (frontmatter, body) = super::loader::split_frontmatter(&text);
    let body = body.trim();
    if body.is_empty() {
        warn!(path = %path.display(), "skills: prompt file has no body — skipping");
        return None;
    }
    let field = |key: &str| {
        frontmatter
            .get(key)
            .and_then(yaml_serde::Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let name = field("name").or_else(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))?;
    if !valid_name(&name) {
        warn!(path = %path.display(), name, "skills: prompt name must be 1-128 of [A-Za-z0-9_.-] — skipping");
        return None;
    }
    Some(Prompt {
        name,
        title: field("title"),
        description: field("description"),
        body: body.to_owned(),
        path: path.to_path_buf(),
    })
}

/// MCP's tool-name charset, which clients also use to build the prompt's
/// slash command — a name outside it would not survive that.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn dir_entry(dir: &Path, include: &[&str], ignore: &[&str]) -> ResolvedSkillEntry {
        let compile = |patterns: &[&str]| {
            (!patterns.is_empty()).then(|| {
                let mut builder = globset::GlobSetBuilder::new();
                for p in patterns {
                    builder.add(globset::Glob::new(p).unwrap());
                }
                builder.build().unwrap()
            })
        };
        ResolvedSkillEntry {
            dir: dir.to_path_buf(),
            ignore_patterns: ignore.iter().map(|s| s.to_string()).collect(),
            ignore: compile(ignore),
            include_patterns: include.iter().map(|s| s.to_string()).collect(),
            include: compile(include),
            watch: true,
        }
    }

    fn names(prompts: &[Prompt]) -> Vec<&str> {
        prompts.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn a_file_is_named_by_its_stem_and_served_without_its_fence() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(&file, "---\ndescription: house rules\n---\n\n# Rules\n").unwrap();

        let prompts = load_prompts(&[file], &[]);
        assert_eq!(names(&prompts), ["AGENTS"]);
        assert_eq!(prompts[0].description.as_deref(), Some("house rules"));
        assert_eq!(prompts[0].body, "# Rules");
    }

    #[test]
    fn a_frontmatter_name_wins_over_the_stem() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("base.md");
        fs::write(&file, "---\nname: house\n---\nbody").unwrap();
        assert_eq!(names(&load_prompts(&[file], &[])), ["house"]);
    }

    /// Directories are flat, sorted, `.md` only, and filtered by the same
    /// include/ignore pair a skill root carries.
    #[test]
    fn a_directory_serves_its_markdown_through_its_globs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("nested")).unwrap();
        for name in [
            "review.md",
            "plan.md",
            "draft-x.md",
            "notes.txt",
            ".hidden.md",
            "nested/deep.md",
        ] {
            fs::write(tmp.path().join(name), "body").unwrap();
        }
        let prompts = load_prompts(&[], &[dir_entry(tmp.path(), &[], &["draft-*"])]);
        assert_eq!(names(&prompts), ["plan", "review"]);

        let only = load_prompts(&[], &[dir_entry(tmp.path(), &["review"], &[])]);
        assert_eq!(names(&only), ["review"]);
    }

    /// The profile's own system prompt comes first, so a directory entry
    /// can never shadow it.
    #[test]
    fn the_first_prompt_to_claim_a_name_keeps_it() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("dir");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("base.md"), "from the directory").unwrap();
        let file = tmp.path().join("base.md");
        fs::write(&file, "from the profile").unwrap();

        let prompts = load_prompts(&[file], &[dir_entry(&dir, &[], &[])]);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].body, "from the profile");
    }

    #[test]
    fn an_empty_or_badly_named_prompt_is_skipped() {
        let tmp = TempDir::new().unwrap();
        let empty = tmp.path().join("empty.md");
        fs::write(&empty, "---\nname: empty\n---\n").unwrap();
        let spaced = tmp.path().join("spaced.md");
        fs::write(&spaced, "---\nname: has space\n---\nbody").unwrap();
        let missing = tmp.path().join("missing.md");

        assert!(load_prompts(&[empty, spaced, missing], &[]).is_empty());
    }
}
