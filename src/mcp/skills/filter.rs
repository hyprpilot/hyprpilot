//! Per-request slug filter for the skills server, carried by two HTTP
//! headers so one catalogue can serve callers that each want a narrow
//! slice of it:
//!
//! - `x-hyprpilot-skills-include` — a JSON array; when present, only
//!   matching slugs exist for the request.
//! - `x-hyprpilot-skills-exclude` — a JSON array; matching slugs never
//!   exist for the request. Exclude beats include on overlap.
//!
//! Each entry is a string. `/<regex>/` is a regular expression matched
//! against the WHOLE slug; anything else is an exact slug. A request
//! with neither header sees the full catalogue, so stdio and existing
//! HTTP callers are unchanged.
//!
//! A malformed header is an error, never an empty filter: dropping a bad
//! include would WIDEN what the caller sees.

use regex::Regex;

pub const INCLUDE_HEADER: &str = "x-hyprpilot-skills-include";
pub const EXCLUDE_HEADER: &str = "x-hyprpilot-skills-exclude";

#[derive(Debug)]
enum Matcher {
    Exact(String),
    Pattern(Regex),
}

impl Matcher {
    fn matches(&self, slug: &str) -> bool {
        match self {
            Self::Exact(s) => s == slug,
            Self::Pattern(re) => re.is_match(slug),
        }
    }
}

#[derive(Debug)]
pub struct SkillFilter {
    include: Option<Vec<Matcher>>,
    exclude: Vec<Matcher>,
}

impl SkillFilter {
    /// `None` when neither header is set — the caller sees everything.
    pub fn parse(include: Option<&str>, exclude: Option<&str>) -> Result<Option<Self>, String> {
        if include.is_none() && exclude.is_none() {
            return Ok(None);
        }
        let include = include.map(|raw| matchers(INCLUDE_HEADER, raw)).transpose()?;
        let exclude = exclude
            .map(|raw| matchers(EXCLUDE_HEADER, raw))
            .transpose()?
            .unwrap_or_default();
        Ok(Some(Self { include, exclude }))
    }

    #[must_use]
    pub fn allows(&self, slug: &str) -> bool {
        if self.exclude.iter().any(|m| m.matches(slug)) {
            return false;
        }
        self.include
            .as_ref()
            .is_none_or(|inc| inc.iter().any(|m| m.matches(slug)))
    }
}

fn matchers(header: &str, raw: &str) -> Result<Vec<Matcher>, String> {
    let entries: Vec<String> =
        serde_json::from_str(raw).map_err(|e| format!("`{header}` must be a JSON array of strings: {e}"))?;
    entries
        .into_iter()
        .map(
            |entry| match entry.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
                Some(pattern) => Regex::new(&format!("^(?:{pattern})$"))
                    .map(Matcher::Pattern)
                    .map_err(|e| format!("`{header}` entry `{entry}` is not a valid regex: {e}")),
                None => Ok(Matcher::Exact(entry)),
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(include: Option<&str>, exclude: Option<&str>) -> SkillFilter {
        SkillFilter::parse(include, exclude).unwrap().unwrap()
    }

    #[test]
    fn no_header_is_no_filter() {
        assert!(SkillFilter::parse(None, None).unwrap().is_none());
    }

    #[test]
    fn exact_entries_do_not_match_by_prefix() {
        let f = filter(Some(r#"["git-commit"]"#), None);
        assert!(f.allows("git-commit"));
        assert!(!f.allows("git-commit-fixup"), "an exact entry is not a prefix");
    }

    #[test]
    fn a_regex_entry_matches_the_whole_slug() {
        let f = filter(Some(r#"["/git-.*/"]"#), None);
        assert!(f.allows("git-push"));
        assert!(!f.allows("agent-git-push"), "the pattern is anchored at both ends");
    }

    #[test]
    fn exclude_beats_include_and_works_alone() {
        let f = filter(Some(r#"["/linear-.*/"]"#), Some(r#"["linear-issue-create"]"#));
        assert!(f.allows("linear-comment"));
        assert!(!f.allows("linear-issue-create"));
        let only_exclude = filter(None, Some(r#"["/hermes-.*/"]"#));
        assert!(only_exclude.allows("git-push"));
        assert!(!only_exclude.allows("hermes-cron"));
    }

    #[test]
    fn an_empty_include_selects_nothing() {
        assert!(!filter(Some("[]"), None).allows("git-push"));
    }

    #[test]
    fn a_malformed_header_is_an_error_not_a_wider_view() {
        assert!(SkillFilter::parse(Some("git-push"), None).is_err());
        assert!(SkillFilter::parse(Some(r#"["/(unclosed/"]"#), None).is_err());
        assert!(SkillFilter::parse(None, Some(r#"[1]"#)).is_err());
    }
}
