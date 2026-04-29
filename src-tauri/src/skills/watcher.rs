//! `notify`-backed recursive watcher across every configured skills
//! root.
//!
//! One `Debouncer` owns N `.watch()` subscriptions — one per existing
//! root. Every debounced batch triggers a single `registry.reload()`.
//! Debounce window is 500ms, matching the task spec and leaving
//! plenty of slack for editor save-swap patterns.
//!
//! Missing roots are skipped (warn + no `.watch()`); a watched root
//! that disappears at runtime drops its subscription on the
//! corresponding `Remove(Folder)` notification — no auto-rearm. The
//! explicit recovery path is `ctl skills reload` after the user
//! recreates the directory.

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::event::{EventKind, ModifyKind, RemoveKind, RenameMode};
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache};
use tracing::{debug, info, warn};

use super::SkillsRegistry;

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

type SkillsDebouncer = Debouncer<notify::RecommendedWatcher, RecommendedCache>;

/// Spawn a detached thread watching every existing root in
/// `registry.dirs()`. The thread owns the `Debouncer` (dropping it
/// stops every watch) and keeps running until the process exits.
/// Roots that don't exist warn + are skipped; the registry serves
/// zero skills for them. A watched root that gets removed drops its
/// `.watch()` subscription with a warn — the user re-arms via
/// `ctl skills reload` after re-creating the directory.
pub fn spawn_watcher(registry: Arc<SkillsRegistry>) -> Result<()> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(DEBOUNCE_WINDOW, None, tx).context("build skills watcher")?;

    let mut watched: Vec<PathBuf> = Vec::new();
    for dir in registry.dirs() {
        if !dir.exists() {
            warn!(dir = %dir.display(), "skills watcher: root does not exist — skipping");
            continue;
        }
        match debouncer.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => {
                info!(dir = %dir.display(), "skills watcher: armed");
                watched.push(dir.clone());
            }
            Err(err) => warn!(dir = %dir.display(), %err, "skills watcher: watch failed — skipping"),
        }
    }

    if watched.is_empty() {
        info!("skills watcher: no live roots — watcher inert");
    }

    thread::Builder::new()
        .name("skills-watcher".into())
        .spawn(move || run_loop(debouncer, watched, rx, registry))
        .context("spawn skills watcher thread")?;
    Ok(())
}

fn run_loop(
    mut debouncer: SkillsDebouncer,
    mut watched: Vec<PathBuf>,
    rx: mpsc::Receiver<DebounceEventResult>,
    registry: Arc<SkillsRegistry>,
) {
    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                if events.is_empty() {
                    continue;
                }
                debug!(count = events.len(), "skills watcher: debounced batch");
                drop_removed_roots(&mut debouncer, &mut watched, &events);
                if let Err(err) = registry.reload() {
                    warn!(%err, "skills watcher: reload failed");
                }
            }
            Ok(Err(errs)) => {
                for err in errs {
                    warn!(%err, "skills watcher: notify error");
                }
            }
            Err(_) => {
                info!("skills watcher: channel closed — exiting");
                return;
            }
        }
    }
}

/// On `Remove(Folder)` (or rename-from) for any watched root,
/// drop the corresponding `.watch()` subscription and warn. No
/// auto-rearm — the user re-creates the dir + runs
/// `ctl skills reload` to restore live updates.
fn drop_removed_roots(debouncer: &mut SkillsDebouncer, watched: &mut Vec<PathBuf>, events: &[DebouncedEvent]) {
    let removed: Vec<PathBuf> = events
        .iter()
        .filter_map(|ev| match ev.event.kind {
            EventKind::Remove(RemoveKind::Folder) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                ev.event.paths.first().cloned()
            }
            _ => None,
        })
        .collect();

    if removed.is_empty() {
        return;
    }

    watched.retain(|root| {
        if removed.iter().any(|r| r == root) {
            warn!(
                dir = %root.display(),
                "skills watcher: root removed — events suspended for this root (run `ctl skills reload` to recover)",
            );
            if let Err(err) = debouncer.unwatch(root) {
                warn!(dir = %root.display(), %err, "skills watcher: unwatch failed");
            }
            return false;
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    /// Flaky timing on slow runners makes us poll instead of
    /// single-shot waiting. Keeps the test robust without stretching
    /// the debounce window.
    fn poll_count(registry: &SkillsRegistry, target: usize, max: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < max {
            if registry.count() == target {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn watcher_reloads_when_new_skill_appears() {
        let tmp = TempDir::new().unwrap();
        let registry = Arc::new(SkillsRegistry::new(vec![tmp.path().to_path_buf()]));
        registry.reload().unwrap();
        assert_eq!(registry.count(), 0);

        spawn_watcher(registry.clone()).expect("watcher armed");

        let skill_dir = tmp.path().join("fresh");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: fresh\n---\n\n# fresh\n\nbody\n",
        )
        .unwrap();

        assert!(
            poll_count(&registry, 1, Duration::from_secs(5)),
            "expected watcher to load fresh skill within 5s (count={})",
            registry.count()
        );
    }

    #[test]
    fn watcher_picks_up_fresh_skills_in_each_root() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        let registry = Arc::new(SkillsRegistry::new(vec![
            a.path().to_path_buf(),
            b.path().to_path_buf(),
        ]));
        registry.reload().unwrap();
        spawn_watcher(registry.clone()).expect("watcher armed");

        for (root, slug) in [(a.path(), "alpha"), (b.path(), "beta")] {
            let d = root.join(slug);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("SKILL.md"),
                format!("---\ndescription: {slug}\n---\n\n# {slug}\n\nbody\n"),
            )
            .unwrap();
        }

        assert!(
            poll_count(&registry, 2, Duration::from_secs(5)),
            "expected watcher to load skills from both roots within 5s (count={})",
            registry.count()
        );
    }

    #[test]
    fn watcher_skips_missing_root_without_panic() {
        let a = TempDir::new().unwrap();
        let missing = PathBuf::from("/nonexistent-skills-root-watch-test-k268");
        let registry = Arc::new(SkillsRegistry::new(vec![missing, a.path().to_path_buf()]));
        registry.reload().unwrap();
        spawn_watcher(registry.clone()).expect("watcher armed");

        let d = a.path().join("survivor");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), "---\ndescription: s\n---\n\n# s\n\nbody\n").unwrap();
        assert!(poll_count(&registry, 1, Duration::from_secs(5)));
    }
}
