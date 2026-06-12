use std::cmp::Reverse;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use path_absolutize::Absolutize;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::adapters::profile::ResolvedInstance;
use crate::config::AgentProvider;

#[derive(Debug, Clone)]
pub(super) struct RestoreSession {
    pub id: String,
    pub title: String,
    pub cwd: Option<PathBuf>,
    pub updated_at_ms: Option<i64>,
}

impl fmt::Display for RestoreSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cwd = self
            .cwd
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unknown cwd>".into());
        let title = sanitize_title(&self.title);
        write!(f, "{}  {}  {}", short_id(&self.id), title, cwd)
    }
}

pub(super) fn list_restorable_sessions(resolved: &ResolvedInstance, all: bool) -> Result<Vec<RestoreSession>> {
    let mut sessions = match resolved.agent.provider {
        AgentProvider::AcpClaudeCode => list_claude_sessions()?,
        AgentProvider::AcpCodex => list_codex_sessions()?,
        AgentProvider::AcpOpenCode => list_opencode_sessions(resolved)?,
        AgentProvider::Acp => {
            anyhow::bail!(
                "agent '{}' uses provider 'acp'; direct restore needs a built-in provider",
                resolved.agent.id
            )
        }
    };
    if !all {
        if let Some(cwd) = resolved.agent.cwd.as_ref() {
            sessions.retain(|session| {
                session
                    .cwd
                    .as_ref()
                    .is_some_and(|session_cwd| same_cwd(session_cwd, cwd))
            });
        }
    }
    sessions.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then_with(|| a.id.cmp(&b.id)));

    Ok(sessions)
}

fn list_claude_sessions() -> Result<Vec<RestoreSession>> {
    let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) else {
        return Ok(Vec::new());
    };
    let projects = home.join(".claude/projects");
    if !projects.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for path in find_files_named(&projects, "sessions-index.json") {
        let body = match fs::read_to_string(&path) {
            Ok(body) => body,
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cli spawn: skipping unreadable claude sessions index");
                continue;
            }
        };
        let index: ClaudeIndex = match serde_json::from_str(&body) {
            Ok(index) => index,
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cli spawn: skipping malformed claude sessions index");
                continue;
            }
        };
        out.extend(
            index
                .entries
                .into_iter()
                .filter(|entry| !entry.is_sidechain)
                .map(|entry| RestoreSession {
                    id: entry.session_id,
                    title: first_non_empty([
                        entry.summary.and_then(non_empty),
                        non_empty(entry.first_prompt),
                        Some("Claude session".into()),
                    ]),
                    cwd: entry.project_path,
                    updated_at_ms: entry.file_mtime,
                }),
        );
    }

    Ok(out)
}

fn list_codex_sessions() -> Result<Vec<RestoreSession>> {
    let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) else {
        return Ok(Vec::new());
    };
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let Some(db_path) = latest_codex_state_db(&codex_home)? else {
        return Ok(Vec::new());
    };
    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open codex state db {}", db_path.display()))?;
    let mut stmt = conn.prepare(
        "select id, title, first_user_message, preview, cwd, coalesce(updated_at_ms, updated_at * 1000)
         from threads
         where archived = 0
         order by coalesce(updated_at_ms, updated_at * 1000) desc",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let first_user_message: String = row.get(2)?;
        let preview: String = row.get(3)?;
        let cwd: String = row.get(4)?;
        let updated_at_ms: Option<i64> = row.get(5)?;

        Ok(RestoreSession {
            id,
            title: first_non_empty([
                non_empty(title),
                non_empty(preview),
                non_empty(first_user_message),
                Some("Codex session".into()),
            ]),
            cwd: non_empty(cwd).map(PathBuf::from),
            updated_at_ms,
        })
    })?;

    Ok(rows.filter_map(|row| row.ok()).collect())
}

fn list_opencode_sessions(resolved: &ResolvedInstance) -> Result<Vec<RestoreSession>> {
    let command = resolved
        .agent
        .spawn
        .as_ref()
        .map(|spawn| expand_value(&spawn.command, "agents.spawn.command"))
        .unwrap_or_else(|| "opencode".into());
    let output = Command::new(&command)
        .args(["session", "list", "--format", "json"])
        .output()
        .with_context(|| format!("running {command} session list --format json"))?;
    if !output.status.success() {
        anyhow::bail!(
            "{command} session list --format json failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let entries: Vec<OpenCodeSession> =
        serde_json::from_slice(&output.stdout).context("parse opencode session list JSON")?;

    Ok(entries
        .into_iter()
        .map(|entry| RestoreSession {
            id: entry.id,
            title: first_non_empty([non_empty(entry.title), Some("OpenCode session".into())]),
            cwd: entry.directory,
            updated_at_ms: entry.updated,
        })
        .collect())
}

fn latest_codex_state_db(home: &Path) -> Result<Option<PathBuf>> {
    let mut candidates = Vec::new();
    if home.exists() {
        for entry in fs::read_dir(home).with_context(|| format!("read {}", home.display()))? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with("state_") && name.ends_with(".sqlite") {
                let mtime = entry.metadata().and_then(|meta| meta.modified()).ok();
                candidates.push((mtime, path));
            }
        }
    }
    candidates.sort_by_key(|candidate| Reverse(candidate.0));

    Ok(candidates.into_iter().map(|(_, path)| path).next())
}

fn find_files_named(root: &Path, name: &str) -> Vec<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut out = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
                out.push(path);
            }
        }
    }

    out
}

fn same_cwd(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        path.absolutize()
            .map(|path| path.into_owned())
            .unwrap_or_else(|_| path.to_path_buf())
    })
}

fn expand_value(raw: &str, ctx: &str) -> String {
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(std::env::var(name).ok())
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "cli spawn: env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> String {
    values.into_iter().flatten().next().unwrap_or_default()
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn sanitize_title(title: &str) -> String {
    let mut out = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 100 {
        out = out.chars().take(100).collect();
        out.push('…');
    }

    out
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[derive(Debug, Deserialize)]
struct ClaudeIndex {
    #[serde(default)]
    entries: Vec<ClaudeEntry>,
}

#[derive(Debug, Deserialize)]
struct ClaudeEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "fileMtime")]
    file_mtime: Option<i64>,
    #[serde(rename = "firstPrompt", default)]
    first_prompt: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: Option<PathBuf>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSession {
    id: String,
    #[serde(default)]
    title: String,
    updated: Option<i64>,
    #[serde(default)]
    directory: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_sanitizer_flattens_and_truncates() {
        let title = sanitize_title("hello\nthere ".repeat(20).as_str());

        assert!(title.ends_with('…'));
        assert!(!title.contains('\n'));
    }

    #[test]
    fn cwd_compare_handles_relative_paths() {
        assert!(same_cwd(Path::new("."), &std::env::current_dir().unwrap()));
    }
}
