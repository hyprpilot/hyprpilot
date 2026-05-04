//! Token hydration + completion-side resolves into wire-side
//! attachments.
//!
//! Companion to [`crate::completion::source`] — sources DETECT
//! captain-typed patterns at compose time (skills, paths, ripgrep);
//! hydrators RESOLVE the picked / referenced patterns into
//! `Attachment` payloads for the wire.
//!
//! Two flavours:
//!  - inline tokens of the shape `#{<scheme>://<value>}` projected at
//!    `session_submit` time via [`TokenHydrator`] / [`TokenHydrators`].
//!    Today only `skills://` is registered; future schemes plug in by
//!    pushing onto the registry the daemon constructs at boot.
//!  - file-attachment hydration ([`file::read_file_for_attachment`])
//!    pairs with the [`crate::completion::source::path::PathSource`]
//!    completion source: captain picks a path completion, the
//!    composer-commit hits this resolver to read the file body into
//!    the attachment's wire shape.
//!
//! Every hydrator owns one URL scheme. Tokens whose scheme has no
//! registered hydrator, or whose value doesn't resolve, are dropped
//! silently with a `warn!` — over-strict parsing degrades UX more
//! than the surviving token text harms it.

use std::sync::Arc;

use regex::Regex;

use crate::adapters::Attachment;

pub mod file;
pub mod skills;

pub use skills::SkillTokenHydrator;

/// One URL scheme's hydration logic. The trait is intentionally
/// minimal: project the value-portion of a parsed token into an
/// `Attachment` (or `None` to drop). All higher-level parsing —
/// scheme dispatch, token boundaries, warn-on-miss — lives in
/// [`TokenHydrators`].
pub trait TokenHydrator: Send + Sync {
    /// URL scheme this hydrator owns (e.g. `"skills"`). Matched
    /// case-sensitively against the parsed token's scheme part.
    fn scheme(&self) -> &'static str;

    /// Project the value (everything between `://` and the closing
    /// `}`) into an `Attachment`. Return `None` when the value can't
    /// be resolved — caller logs a `warn!` and drops the token.
    fn hydrate(&self, value: &str) -> Option<Attachment>;
}

/// Registry of [`TokenHydrator`]s keyed by their scheme. Construct
/// once at boot, hand by `Arc` to every call site that turns prompt
/// text into attachments.
#[derive(Default, Clone)]
pub struct TokenHydrators {
    hydrators: Vec<Arc<dyn TokenHydrator>>,
}

impl TokenHydrators {
    #[must_use]
    pub fn new() -> Self {
        Self { hydrators: Vec::new() }
    }

    #[must_use]
    pub fn with(mut self, hydrator: Arc<dyn TokenHydrator>) -> Self {
        self.hydrators.push(hydrator);
        self
    }

    /// Walk every `#{<scheme>://<value>}` token in `text` (in order)
    /// and dispatch to the matching hydrator. Unknown schemes / unknown
    /// values warn-and-drop. Order in the output mirrors order in the
    /// source text.
    pub fn hydrate_all(&self, text: &str) -> Vec<Attachment> {
        // `#{scheme://value}` — scheme is `[a-z][a-z0-9_-]*`, value
        // captures everything up to the closing `}`. Greedy `[^}]*`
        // is safe because `}` is forbidden inside a token.
        let re = match Regex::new(r"#\{([a-z][a-z0-9_-]*)://([^}]*)\}") {
            Ok(r) => r,
            Err(err) => {
                tracing::error!(%err, "token hydrate: regex compile failed");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for caps in re.captures_iter(text) {
            let scheme = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let Some(hydrator) = self.hydrators.iter().find(|h| h.scheme() == scheme) else {
                tracing::warn!(scheme, value, "token hydrate: no hydrator for scheme");
                continue;
            };
            match hydrator.hydrate(value) {
                Some(att) => out.push(att),
                None => tracing::warn!(scheme, value, "token hydrate: value did not resolve"),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct FakeHydrator {
        scheme: &'static str,
        accept: &'static str,
    }
    impl TokenHydrator for FakeHydrator {
        fn scheme(&self) -> &'static str {
            self.scheme
        }
        fn hydrate(&self, value: &str) -> Option<Attachment> {
            if value == self.accept {
                Some(Attachment {
                    slug: value.to_string(),
                    path: PathBuf::from("/tmp/fake"),
                    body: String::new(),
                    title: None,
                    data: None,
                    mime: None,
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn dispatches_to_matching_scheme() {
        let hydrators = TokenHydrators::new()
            .with(Arc::new(FakeHydrator {
                scheme: "skills",
                accept: "git-commit",
            }))
            .with(Arc::new(FakeHydrator {
                scheme: "prompt",
                accept: "p1",
            }));
        let out = hydrators.hydrate_all("see #{skills://git-commit} and #{prompt://p1} please");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].slug, "git-commit");
        assert_eq!(out[1].slug, "p1");
    }

    #[test]
    fn drops_unknown_scheme() {
        let hydrators = TokenHydrators::new().with(Arc::new(FakeHydrator {
            scheme: "skills",
            accept: "anything",
        }));
        let out = hydrators.hydrate_all("a #{unknown://x} b");
        assert!(out.is_empty());
    }

    #[test]
    fn drops_unresolved_value() {
        let hydrators = TokenHydrators::new().with(Arc::new(FakeHydrator {
            scheme: "skills",
            accept: "git-commit",
        }));
        let out = hydrators.hydrate_all("a #{skills://nope} b");
        assert!(out.is_empty());
    }

    #[test]
    fn handles_back_to_back_tokens() {
        let hydrators = TokenHydrators::new().with(Arc::new(FakeHydrator {
            scheme: "skills",
            accept: "x",
        }));
        let out = hydrators.hydrate_all("#{skills://x}#{skills://x}");
        assert_eq!(out.len(), 2);
    }
}
