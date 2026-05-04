//! Git status snapshot for the captain's cwd. Wraps `git2`
//! (libgit2 bindings) — a well-known good crate — so the header
//! pill can render `branch ↑N ↓M` without shelling out to `git`.
//!
//! `snapshot(path)` walks up from `path` looking for the enclosing
//! repo (so a subdirectory of a repo still resolves), reads the
//! current `HEAD` ref, and computes ahead/behind against the
//! configured upstream when one exists. Returns `None` when the
//! path doesn't sit inside any git repo — captains running in a
//! non-repo cwd see no pill at all.
//!
//! The header doesn't need dirty / staged-counts today; if the
//! captain wants those later we add fields to the snapshot, not a
//! new module. `git2`'s `statuses()` walk is the path for that.
use std::path::Path;

use anyhow::Result;
use git2::{BranchType, Repository};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<usize>,
    /// Worktree name when the repo is a `git-worktree` checkout
    /// rather than the primary checkout. Currently unset; reserved
    /// for the future capture path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

/// Resolve the git status for `path`, walking up to find the
/// enclosing repo. Returns `Ok(None)` when no repo encloses the
/// path. Returns `Err` only on libgit2-internal failures (not
/// "no repo" — that's a clean None).
pub fn snapshot(path: &Path) -> Result<Option<GitStatus>> {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let head = match repo.head() {
        Ok(h) => h,
        // Detached HEAD with no commits / fresh init — no branch to
        // surface. Treat as "no status" rather than an error.
        Err(_) => return Ok(None),
    };
    let branch = if head.is_branch() {
        head.shorthand().unwrap_or("HEAD").to_string()
    } else {
        // Detached HEAD: surface the abbreviated commit id instead
        // of bare "HEAD" so the captain knows where they are.
        head.target()
            .map(|oid| oid.to_string()[..7].to_string())
            .unwrap_or_else(|| "HEAD".to_string())
    };
    let (ahead, behind) = ahead_behind(&repo, &branch).unwrap_or((None, None));
    Ok(Some(GitStatus {
        branch,
        ahead,
        behind,
        worktree: None,
    }))
}

/// Compute `(ahead, behind)` of `branch` against its configured
/// upstream. Returns `(None, None)` when the branch has no upstream
/// (typical for fresh local branches) — the header reads that as
/// "no remote drift to surface".
fn ahead_behind(repo: &Repository, branch: &str) -> Result<(Option<usize>, Option<usize>)> {
    let local = repo.find_branch(branch, BranchType::Local)?;
    let upstream = match local.upstream() {
        Ok(u) => u,
        Err(_) => return Ok((None, None)),
    };
    let local_oid = local
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("branch ref has no target oid"))?;
    let upstream_oid = upstream
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("upstream ref has no target oid"))?;
    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
    Ok((Some(ahead), Some(behind)))
}
