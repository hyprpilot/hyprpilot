//! Watch directory trees and report that something under them changed.
//!
//! Deliberately knows nothing about what the files ARE. A caller names
//! roots, optionally with per-root ignore globs, and gets one coalesced
//! signal per quiet window — never a description of the change, because
//! a consumer that rescans from scratch cannot use one and a consumer
//! that could would need a different filter anyway. `mcp skills` is the
//! first consumer, not the shape this is built around.
//!
//! Two stages of coalescing, and both are load-bearing. The debouncer
//! stitches an editor's write-temp-then-rename and a `git checkout`
//! storm into one event per final path; the unbounded channel then lets
//! the consumer drain a burst before doing any work. Neither drops a
//! change: the worst case is one extra pass over an already-current
//! tree.

use std::path::{Path, PathBuf};
use std::time::Duration;

use globset::GlobSet;
use notify_debouncer_full::notify::{self, Event, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// How long a burst must go quiet before it counts as one change.
///
/// A constant, not config: an editor's atomic save completes in single
/// digit milliseconds and a `git checkout` over a skills tree lands well
/// inside this, while 500 ms is below what an agent observes between
/// two tool calls. A captain has no way to measure what a different
/// value buys, and the consumer-side drain already bounds the work to
/// one pass per quiet window regardless.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// One tree to watch.
///
/// `ignore` is matched against the FIRST path component under `dir`,
/// which is the same scope a skill root applies to a slug. A consumer
/// with no such notion leaves it `None`.
#[derive(Debug, Clone)]
pub struct WatchRoot {
    pub dir: PathBuf,
    pub ignore: Option<GlobSet>,
    /// `false` leaves the root unarmed and reported [`WatchState::Off`].
    /// The one honest use is a filesystem that cannot deliver events at
    /// all — see [`WatchState`].
    pub watch: bool,
}

/// What the watcher has to say. Never names a path: see the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchSignal {
    /// Something under a watched root changed.
    Changed,
    /// The watcher lost coverage. Carries a rescan too — the tree moved
    /// under a directory that is now unwatched, so the last thing worth
    /// doing with that coverage is using it.
    Degraded(String),
}

impl WatchSignal {
    #[must_use]
    pub fn degraded(&self) -> Option<&str> {
        match self {
            Self::Degraded(reason) => Some(reason),
            Self::Changed => None,
        }
    }
}

/// Per-root coverage, as reported to whoever asks.
///
/// `Degraded` is not a failure to recover from in-process: the roots
/// that reach it (`ENOSPC`, a root that does not exist, a dead watcher
/// thread) all need something outside this process to change first.
/// `Off` is the captain's own choice, and the reason it exists is that
/// a network or FUSE mount accepts `inotify_add_watch` and then never
/// fires — there is no error to degrade on, so the only honest signal
/// is one the captain sets.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum WatchState {
    Watching,
    Degraded { reason: String },
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RootWatch {
    pub dir: PathBuf,
    #[serde(flatten)]
    pub state: WatchState,
}

/// Coverage across every root, for a consumer that wants to tell its own
/// caller whether freshness is guaranteed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct WatchStatus {
    pub roots: Vec<RootWatch>,
}

impl WatchStatus {
    /// True only when EVERY root is covered. A partial answer would be
    /// worse than none: a consumer reading `true` stops checking.
    #[must_use]
    pub fn active(&self) -> bool {
        !self.roots.is_empty() && self.roots.iter().all(|r| r.state == WatchState::Watching)
    }

    /// Mark every still-`Watching` root degraded. Used when the failure
    /// is the watcher itself rather than one root.
    pub fn degrade_all(&mut self, reason: &str) {
        for root in &mut self.roots {
            if root.state == WatchState::Watching {
                root.state = WatchState::Degraded {
                    reason: reason.to_string(),
                };
            }
        }
    }

    /// One line naming what is not covered, or `None` when everything
    /// is. Exists so a text-only client sees the caveat: a consumer that
    /// only renders structured output would hide it.
    #[must_use]
    pub fn summary_line(&self) -> Option<String> {
        let uncovered: Vec<String> = self
            .roots
            .iter()
            .filter_map(|root| match &root.state {
                WatchState::Watching => None,
                WatchState::Degraded { reason } => Some(format!("{} ({reason})", root.dir.display())),
                WatchState::Off => Some(format!("{} (watching turned off)", root.dir.display())),
            })
            .collect();
        if uncovered.is_empty() {
            return None;
        }
        Some(format!("Not watching: {}.", uncovered.join("; ")))
    }
}

/// The armed watcher. Opaque and inert — it exists to be held for as
/// long as the signals matter and dropped after.
///
/// Dropping only sets the debouncer's stop flag, so teardown never
/// blocks on the watcher thread.
pub struct Watcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

/// What [`arm`] hands back.
pub struct Armed {
    /// `None` when no root was armed, so nothing spawns a thread that
    /// can never fire.
    pub watcher: Option<Watcher>,
    pub signals: UnboundedReceiver<WatchSignal>,
    pub status: WatchStatus,
}

/// Arm one watcher over every root whose `watch` is on.
///
/// Never fails: a root that cannot be watched is reported degraded and
/// the rest are still armed. Losing the watcher leaves a consumer
/// exactly as stale as it was before one existed, and nothing is
/// widened — so taking the whole surface down over an inotify limit
/// would trade a working server for a freshness guarantee.
#[must_use]
pub fn arm(roots: &[WatchRoot], debounce: Duration) -> Armed {
    let (tx, signals) = unbounded_channel();
    let mut status = WatchStatus::default();

    let armable: Vec<&WatchRoot> = roots.iter().filter(|r| r.watch).collect();
    for root in roots.iter().filter(|r| !r.watch) {
        status.roots.push(RootWatch {
            dir: root.dir.clone(),
            state: WatchState::Off,
        });
    }
    if armable.is_empty() {
        return Armed {
            watcher: None,
            signals,
            status,
        };
    }

    let filters: Vec<WatchRoot> = armable.iter().map(|r| (*r).clone()).collect();
    let handler = move |result: DebounceEventResult| dispatch(&tx, &filters, result);

    let mut debouncer = match new_debouncer(debounce, None, handler) {
        Ok(d) => d,
        Err(err) => {
            // No watcher at all. Every armable root is uncovered, and
            // saying so is the whole point of the status.
            tracing::warn!(%err, "watch: could not start the watcher — roots are unwatched");
            for root in armable {
                status.roots.push(RootWatch {
                    dir: root.dir.clone(),
                    state: WatchState::Degraded {
                        reason: err.to_string(),
                    },
                });
            }
            return Armed {
                watcher: None,
                signals,
                status,
            };
        }
    };

    let mut armed_any = false;
    for root in armable {
        let state = match debouncer.watch(&root.dir, RecursiveMode::Recursive) {
            Ok(()) => {
                armed_any = true;
                tracing::debug!(dir = %root.dir.display(), "watch: armed");
                WatchState::Watching
            }
            Err(err) => {
                let reason = describe(&err);
                tracing::warn!(dir = %root.dir.display(), %reason, "watch: root is unwatched");
                WatchState::Degraded { reason }
            }
        };
        status.roots.push(RootWatch {
            dir: root.dir.clone(),
            state,
        });
    }

    Armed {
        // Holding a debouncer that watches nothing keeps a thread alive
        // for signals that cannot arrive.
        watcher: armed_any.then_some(Watcher { _debouncer: debouncer }),
        signals,
        status,
    }
}

/// Turn one debouncer callback into at most one signal of each kind.
///
/// Runs on the debouncer's own thread. It must never panic: the release
/// profile aborts, which would take the whole process down over a
/// filesystem event.
fn dispatch(tx: &UnboundedSender<WatchSignal>, roots: &[WatchRoot], result: DebounceEventResult) {
    match result {
        Ok(events) => {
            if events.iter().any(|event| relevant(event, roots)) {
                let _ = tx.send(WatchSignal::Changed);
            }
        }
        Err(errors) => {
            // `MaxFilesWatch` here means a directory created just now
            // could not be watched, so coverage is already partial.
            for err in errors {
                let _ = tx.send(WatchSignal::Degraded(describe(&err)));
            }
        }
    }
}

/// Is this event worth a rescan?
///
/// Pure, so the whole filter is testable without a filesystem.
fn relevant(event: &Event, roots: &[WatchRoot]) -> bool {
    // A dropped-events overflow carries no usable paths. A full rescan
    // is what a consumer does anyway, so claim it unconditionally
    // rather than deciding from an event that lost its own detail.
    if event.need_rescan() {
        return true;
    }
    // Any path suffices: a rename out of an ignored slug into a live one
    // is a change to the live one.
    event
        .paths
        .iter()
        .any(|path| roots.iter().any(|root| covers(root, path)))
}

fn covers(root: &WatchRoot, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(&root.dir) else {
        return false;
    };
    let Some(ignore) = &root.ignore else {
        return true;
    };
    // Match the first component only. Deeper components are a slug's
    // own contents, and a root's globs name slugs — matching deeper
    // would let a glob written for a slug silently suppress a file
    // inside an unrelated one.
    // `map_or(true, ..)` rather than `is_none_or`: the crate's MSRV is
    // 1.77 and that helper landed in 1.82.
    rest.components()
        .next()
        .map_or(true, |first| !ignore.is_match(first.as_os_str()))
}

/// A reason a human can act on, not just the error's own words.
fn describe(err: &notify::Error) -> String {
    match &err.kind {
        notify::ErrorKind::MaxFilesWatch => {
            "inotify watch limit reached - raise fs.inotify.max_user_watches".to_string()
        }
        notify::ErrorKind::PathNotFound => "directory does not exist".to_string(),
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_full::notify::event::Flag;
    use notify_debouncer_full::notify::EventKind;

    fn globs(patterns: &[&str]) -> Option<GlobSet> {
        let mut builder = globset::GlobSetBuilder::new();
        for pat in patterns {
            builder.add(globset::Glob::new(pat).unwrap());
        }
        builder.build().ok()
    }

    fn root(dir: &str, ignore: &[&str]) -> WatchRoot {
        WatchRoot {
            dir: PathBuf::from(dir),
            ignore: globs(ignore),
            watch: true,
        }
    }

    fn touched(paths: &[&str]) -> Event {
        Event::new(EventKind::Modify(notify::event::ModifyKind::Any))
            .add_some_path(paths.first().map(PathBuf::from))
            .add_some_path(paths.get(1).map(PathBuf::from))
    }

    /// An overflow lost its own paths, so deciding from them would drop
    /// every change the kernel could not queue.
    #[test]
    fn a_rescan_flag_is_always_relevant() {
        let event = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(event.paths.is_empty());
        assert!(relevant(&event, &[root("/skills", &[])]));
    }

    #[test]
    fn an_event_under_an_ignored_first_component_is_dropped() {
        let roots = [root("/skills", &["work-*"])];
        assert!(!relevant(&touched(&["/skills/work-internal/SKILL.md"]), &roots));
        assert!(relevant(&touched(&["/skills/git-commit/SKILL.md"]), &roots));
    }

    /// The globs name slugs, so a captain's `*-laravel` must not reach
    /// into a live slug's contents and suppress a file there.
    #[test]
    fn an_ignore_glob_matches_the_first_component_only() {
        let roots = [root("/skills", &["*-laravel"])];
        assert!(!relevant(&touched(&["/skills/cluster-laravel/SKILL.md"]), &roots));
        assert!(relevant(&touched(&["/skills/references/scm/x-laravel"]), &roots));
    }

    #[test]
    fn an_event_outside_every_root_is_dropped() {
        assert!(!relevant(&touched(&["/elsewhere/a.md"]), &[root("/skills", &[])]));
    }

    /// A rename out of an ignored slug into a live one changes the live
    /// one, so either path being relevant is enough.
    #[test]
    fn a_rename_is_relevant_when_either_path_is() {
        let roots = [root("/skills", &["work-*"])];
        let event = touched(&["/skills/work-internal/SKILL.md", "/skills/live/SKILL.md"]);
        assert!(relevant(&event, &roots));
    }

    #[test]
    fn a_missing_root_degrades_without_taking_the_others_down() {
        let live = tempfile::tempdir().unwrap();
        let armed = arm(
            &[
                root("/nonexistent-hyprpilot-watch-probe", &[]),
                WatchRoot {
                    dir: live.path().to_path_buf(),
                    ignore: None,
                    watch: true,
                },
            ],
            Duration::from_millis(50),
        );
        assert!(armed.watcher.is_some());
        assert!(!armed.status.active());
        assert_eq!(
            armed.status.roots[0].state,
            WatchState::Degraded {
                reason: "directory does not exist".to_string()
            }
        );
        assert_eq!(armed.status.roots[1].state, WatchState::Watching);
    }

    #[test]
    fn a_root_with_watching_turned_off_is_reported_off_and_never_armed() {
        let dir = tempfile::tempdir().unwrap();
        let armed = arm(
            &[WatchRoot {
                dir: dir.path().to_path_buf(),
                ignore: None,
                watch: false,
            }],
            Duration::from_millis(50),
        );
        assert!(armed.watcher.is_none());
        assert_eq!(armed.status.roots[0].state, WatchState::Off);
        assert!(!armed.status.active());
    }

    /// No root means no thread, so nothing waits on signals that can
    /// never arrive.
    #[test]
    fn arming_nothing_yields_no_watcher_and_no_rows() {
        let armed = arm(&[], Duration::from_millis(50));
        assert!(armed.watcher.is_none());
        assert!(armed.status.roots.is_empty());
        assert!(!armed.status.active());
    }

    #[test]
    fn status_is_active_only_when_every_root_is_covered() {
        let mut status = WatchStatus {
            roots: vec![RootWatch {
                dir: PathBuf::from("/a"),
                state: WatchState::Watching,
            }],
        };
        assert!(status.active());
        assert!(status.summary_line().is_none());

        status.degrade_all("watcher thread exited");
        assert!(!status.active());
        assert_eq!(
            status.summary_line().as_deref(),
            Some("Not watching: /a (watcher thread exited).")
        );
    }

    /// The end-to-end pin: a real write under a real root reaches the
    /// channel. Everything above it is a filter; this is the thing.
    #[tokio::test]
    async fn an_edit_under_a_watched_root_signals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("alpha")).unwrap();
        let mut armed = arm(
            &[WatchRoot {
                dir: dir.path().to_path_buf(),
                ignore: None,
                watch: true,
            }],
            Duration::from_millis(50),
        );
        assert!(armed.status.active());

        std::fs::write(dir.path().join("alpha/SKILL.md"), "body").unwrap();

        let signal = tokio::time::timeout(Duration::from_secs(5), armed.signals.recv())
            .await
            .expect("no watch signal within 5s");
        assert_eq!(signal, Some(WatchSignal::Changed));
    }

    /// An ignored slug must not wake the consumer at all - the filter
    /// runs before the channel, so a noisy suppressed tree costs
    /// nothing.
    #[tokio::test]
    async fn an_edit_under_an_ignored_slug_never_signals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("work-internal")).unwrap();
        let mut armed = arm(
            &[WatchRoot {
                dir: dir.path().to_path_buf(),
                ignore: globs(&["work-*"]),
                watch: true,
            }],
            Duration::from_millis(50),
        );

        std::fs::write(dir.path().join("work-internal/SKILL.md"), "body").unwrap();

        let quiet = tokio::time::timeout(Duration::from_millis(750), armed.signals.recv()).await;
        assert!(quiet.is_err(), "an ignored slug woke the consumer");
    }

    #[test]
    fn a_watch_limit_error_names_the_sysctl_to_raise() {
        let err = notify::Error::new(notify::ErrorKind::MaxFilesWatch);
        assert!(describe(&err).contains("fs.inotify.max_user_watches"));
    }

    /// The signal a degraded root carries has to be distinguishable
    /// from an ordinary change, or a consumer cannot report coverage.
    #[test]
    fn a_watcher_error_becomes_a_degraded_signal() {
        let (tx, mut rx) = unbounded_channel();
        dispatch(
            &tx,
            &[root("/skills", &[])],
            Err(vec![notify::Error::new(notify::ErrorKind::MaxFilesWatch)]),
        );
        let signal = rx.try_recv().unwrap();
        assert!(signal.degraded().is_some_and(|r| r.contains("inotify watch limit")));
        assert_eq!(WatchSignal::Changed.degraded(), None);
    }
}
