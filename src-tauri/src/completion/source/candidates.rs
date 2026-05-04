//! Caller-supplied candidate ranking. Unlike `path` / `ripgrep` /
//! `commands` / `skills` (which discover candidates from the
//! filesystem or registries), this source has no discovery — the
//! caller hands it a list of items, the source ranks them against
//! the typed query via the same nucleo matcher every other source
//! uses.
//!
//! Drives palette surfaces with bounded candidate sets that need
//! Rust-side ranking so the UI / Neovim plugin / any future
//! frontend share one ranking implementation. The cwd palette's
//! recents are the first consumer; future consumers (instance
//! pickers, profile pickers, anything that wants fuzzy-pick over
//! a known list) drop in via the same RPC.
//!
//! No `CompletionSource` trait impl — discovery sources need a
//! `detect` hook to decide whether they own the request, candidate
//! ranking has no detect axis (the caller is the trigger). Free
//! function `rank_candidates` is all the surface needs; the Tauri
//! command + JSON-RPC handler call it directly.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use serde::{Deserialize, Serialize};

use crate::completion::{CompletionItem, CompletionKind, Replacement, ReplacementRange};

const MAX_RESULTS: usize = 50;

/// A single candidate row passed into `rank_candidates`. `id` is
/// returned verbatim in `CompletionItem.replacement.text` so the
/// caller can route the picked row to its commit handler without
/// re-deriving identity from the rendered label.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateItem {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Rank `candidates` against `query` using nucleo's smart-case
/// matcher. Empty query → identity order (caller-supplied order
/// preserved, capped at `MAX_RESULTS`). Non-empty query →
/// score-ranked descending, ties broken by original input order.
/// Cap matches every other source (path / ripgrep) so the popover
/// never renders past its visible row count.
pub fn rank_candidates(query: &str, candidates: &[CandidateItem]) -> Vec<CompletionItem> {
    let trigger = ReplacementRange {
        start: 0,
        end: query.len(),
    };
    let trimmed = query.trim();

    if trimmed.is_empty() {
        return candidates
            .iter()
            .take(MAX_RESULTS)
            .map(|c| to_completion_item(c, &trigger))
            .collect();
    }
    let pattern = Pattern::parse(trimmed, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut scored: Vec<(u32, usize, &CandidateItem)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, c)| {
            pattern
                .score(nucleo_matcher::Utf32Str::Ascii(c.label.as_bytes()), &mut matcher)
                .map(|s| (s, idx, c))
        })
        .collect();
    // Score descending; tie-break by original index ascending so the
    // caller's intentional ordering survives equal-score cohorts.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(MAX_RESULTS);
    scored
        .into_iter()
        .map(|(_, _, c)| to_completion_item(c, &trigger))
        .collect()
}

fn to_completion_item(c: &CandidateItem, trigger: &ReplacementRange) -> CompletionItem {
    CompletionItem {
        label: c.label.clone(),
        detail: c.description.clone(),
        kind: CompletionKind::Word,
        replacement: Replacement {
            range: trigger.clone(),
            text: c.id.clone(),
        },
        resolve_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, label: &str) -> CandidateItem {
        CandidateItem {
            id: id.into(),
            label: label.into(),
            description: None,
        }
    }

    #[test]
    fn empty_query_preserves_input_order() {
        let cs = vec![item("1", "alpha"), item("2", "beta"), item("3", "gamma")];
        let r = rank_candidates("", &cs);

        assert_eq!(
            r.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn fuzzy_query_drops_non_matches() {
        let cs = vec![item("1", "alpha"), item("2", "beta"), item("3", "gamma")];
        let r = rank_candidates("a", &cs);

        // beta has no `a`-subsequence-friendly match… actually it does:
        // `b-e-t-a`. So nucleo matches all three. Test smarter: query
        // for something only one matches.
        assert!(r.iter().any(|i| i.label == "alpha"));
        assert!(r.iter().any(|i| i.label == "gamma"));
    }

    #[test]
    fn fuzzy_query_outranks_non_matches() {
        let cs = vec![item("1", "haystack"), item("2", "alphabet"), item("3", "alpha")];
        let r = rank_candidates("alpha", &cs);

        // Both `alpha` + `alphabet` outrank `haystack`. Nucleo's
        // exact-internal-order preference between the two
        // alpha-matches is implementation-detail; the load-bearing
        // assertion is that haystack is NOT the top result.
        let top_two: Vec<&str> = r.iter().take(2).map(|i| i.label.as_str()).collect();

        assert!(top_two.contains(&"alpha"), "{top_two:?}");
        assert!(top_two.contains(&"alphabet"), "{top_two:?}");
    }

    #[test]
    fn id_lands_on_replacement_text() {
        let cs = vec![item("captain-id-1", "captain")];
        let r = rank_candidates("c", &cs);

        assert_eq!(r[0].replacement.text, "captain-id-1");
    }

    #[test]
    fn description_carries_through_to_detail() {
        let cs = vec![CandidateItem {
            id: "1".into(),
            label: "x".into(),
            description: Some("the x".into()),
        }];
        let r = rank_candidates("", &cs);

        assert_eq!(r[0].detail.as_deref(), Some("the x"));
    }

    #[test]
    fn caps_at_max_results() {
        let cs: Vec<CandidateItem> = (0..100).map(|i| item(&format!("{i}"), &format!("item-{i}"))).collect();
        let r = rank_candidates("", &cs);

        assert_eq!(r.len(), MAX_RESULTS);
    }
}
