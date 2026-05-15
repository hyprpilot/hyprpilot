//! Daemon-side markdown-paragraph fold for streamed agent chunks.
//!
//! The ACP wire ships one `agent_message_chunk` (and one
//! `agent_thought_chunk`) per content-block delta the vendor emits.
//! Each notification carries a fragment of text; concatenating those
//! fragments verbatim is what every frontend ultimately renders. The
//! problem is markdown: `"Para 1.\nPara 2."` reads as ONE paragraph
//! with a soft line break, not two paragraphs — the markdown
//! specification requires a blank line (`\n\n`) between paragraphs.
//!
//! When the agent emits chunks where the accumulated tail ends on a
//! single `\n`, we prepend one more `\n` to the next chunk's text
//! BEFORE the daemon emits the `acp:transcript` event AND BEFORE it
//! lands in the mirror. Every frontend (Vue desktop, Vue remote,
//! `hyprpilot.nvim`, `ctl`, any future client) receives chunks that
//! are concatenation-safe — `text + text + text` renders as the
//! paragraph-separated markdown the agent intended.
//!
//! Soft-lift only: we promote a single trailing `\n` to `\n\n` but
//! never invent a paragraph break from scratch. Streaming token
//! bursts (`"Hello, "` + `"world"`) emit verbatim — promoting a
//! non-newline boundary to `\n\n` would split a mid-sentence chunk
//! pair into bogus paragraphs.
//!
//! Per-turn state lives on `TurnState` (`agent_text_trailing` +
//! `agent_thought_trailing`), reset on every `open_real` /
//! `open_synthetic`. The actor's notification handler calls
//! `TurnState::note_agent_text` / `note_agent_thought` before emit
//! to get the prefix and update the running tail count.

/// Compute the prefix to prepend to an incoming chunk so the
/// boundary between the prior accumulated text and the incoming
/// chunk lifts to a markdown paragraph break.
///
/// Two complementary cases trigger a lift:
///
/// 1. **Boundary lift** — prior accumulated text ended on exactly
///    one `\n` AND the incoming chunk leads with non-newline
///    content. Prepending `\n` pushes the boundary to `\n\n`.
/// 2. **Chunk-self lift** — the incoming chunk itself leads with
///    exactly one `\n` (not `\n\n`). Vendors stream chunks like
///    `"\nPara 2."` expecting a paragraph break, but a single
///    `\n` is just a soft break in markdown. Prepend another `\n`
///    so the chunk's own leading newline reads as `\n\n`.
///
/// Chunks that already lead with `\n\n` get no lift — they
/// already carry the paragraph break.
pub(crate) fn soft_lift_prefix(prior_trailing: u8, incoming: &str) -> &'static str {
    if incoming.starts_with("\n\n") {
        return "";
    }
    if incoming.starts_with('\n') {
        return "\n";
    }
    if prior_trailing == 1 {
        return "\n";
    }
    ""
}

/// Fold the prefix + chunk into the running trailing-newline tally,
/// capped at 2. Walks chunk-first (the typical case where the chunk
/// ends on its own content); falls back into the prefix and finally
/// the prior counter when the chunk is empty or all-newlines.
///
/// Cap of 2 matches markdown's paragraph-break threshold — any
/// additional trailing newlines collapse to the same visual.
pub(crate) fn fold_trailing(prior: u8, prefix: &str, chunk: &str) -> u8 {
    let mut count: u32 = 0;

    for c in chunk.chars().rev() {
        if c == '\n' {
            count += 1;
            if count >= 2 {
                return 2;
            }
        } else {
            return count as u8;
        }
    }

    // Chunk was all-newlines (or empty). Walk into the prefix the
    // emit step just produced.
    for c in prefix.chars().rev() {
        if c == '\n' {
            count += 1;
            if count >= 2 {
                return 2;
            }
        } else {
            return count as u8;
        }
    }

    // Both were all-newlines (or empty). The remaining context lives
    // in the prior tally — add it in.
    ((count + prior as u32).min(2)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── soft_lift_prefix ───────────────────────────────────────────

    #[test]
    fn soft_lift_returns_empty_when_prior_is_paragraph_terminated() {
        assert_eq!(soft_lift_prefix(2, "Para 2."), "");
    }

    #[test]
    fn soft_lift_returns_empty_when_incoming_leads_with_double_newline() {
        assert_eq!(soft_lift_prefix(1, "\n\nPara 2."), "");
    }

    #[test]
    fn soft_lift_promotes_single_trailing_newline_to_paragraph_break() {
        // Prior text ended `Para 1.\n` (trailing == 1); incoming
        // starts with non-newline. Prefix lifts to \n\n.
        assert_eq!(soft_lift_prefix(1, "Para 2."), "\n");
    }

    #[test]
    fn soft_lift_promotes_single_leading_newline_in_chunk_to_paragraph_break() {
        // Chunk starts with exactly one `\n` (not `\n\n`). Prepend `\n`
        // so the chunk's own leading newline reads as a markdown
        // paragraph break — independent of prior trailing state.
        assert_eq!(soft_lift_prefix(0, "\nPara 2."), "\n");
        assert_eq!(soft_lift_prefix(1, "\nPara 2."), "\n");
        assert_eq!(soft_lift_prefix(2, "\nPara 2."), "\n");
    }

    #[test]
    fn soft_lift_returns_empty_when_chunk_already_has_double_leading_newline() {
        // `\n\n` is already a paragraph break — no lift needed.
        assert_eq!(soft_lift_prefix(0, "\n\nPara 2."), "");
        assert_eq!(soft_lift_prefix(1, "\n\nPara 2."), "");
        assert_eq!(soft_lift_prefix(2, "\n\nPara 2."), "");
    }

    #[test]
    fn soft_lift_returns_empty_on_non_newline_boundary_to_avoid_over_injection() {
        // Streaming token burst: prior ends with non-newline char
        // (`"Hello, "`), incoming starts with non-newline (`"world!"`).
        // We MUST NOT invent a paragraph break.
        assert_eq!(soft_lift_prefix(0, "world!"), "");
    }

    #[test]
    fn soft_lift_handles_empty_incoming() {
        assert_eq!(soft_lift_prefix(1, ""), "\n");
        assert_eq!(soft_lift_prefix(0, ""), "");
        assert_eq!(soft_lift_prefix(2, ""), "");
    }

    // ── fold_trailing ──────────────────────────────────────────────

    #[test]
    fn fold_trailing_resets_to_zero_on_non_newline_chunk() {
        assert_eq!(fold_trailing(2, "", "Hello"), 0);
        assert_eq!(fold_trailing(1, "\n", "world"), 0);
    }

    #[test]
    fn fold_trailing_counts_one_newline_at_chunk_end() {
        assert_eq!(fold_trailing(0, "", "Para 1.\n"), 1);
    }

    #[test]
    fn fold_trailing_caps_at_two_for_chunks_with_many_trailing_newlines() {
        assert_eq!(fold_trailing(0, "", "Para 1.\n\n\n"), 2);
        assert_eq!(fold_trailing(0, "", "Para 1.\n\n"), 2);
    }

    #[test]
    fn fold_trailing_extends_prior_count_when_chunk_is_all_newlines() {
        // Prior had 1 trailing newline; chunk is "\n"; combined is 2.
        assert_eq!(fold_trailing(1, "", "\n"), 2);
        // Prior 1, chunk "" — no change.
        assert_eq!(fold_trailing(1, "", ""), 1);
        // Prior 0, chunk "\n" → count 1.
        assert_eq!(fold_trailing(0, "", "\n"), 1);
    }

    #[test]
    fn fold_trailing_accounts_for_a_lift_prefix_when_chunk_is_empty_or_all_newlines() {
        // Prior 1, prefix "\n", chunk "" — combined trailing is 2.
        assert_eq!(fold_trailing(1, "\n", ""), 2);
        // Prior 0, prefix "\n", chunk "" — combined trailing is 1.
        assert_eq!(fold_trailing(0, "\n", ""), 1);
    }

    #[test]
    fn fold_trailing_handles_a_chunk_whose_content_dominates_the_count() {
        // Even if prior was 2, a chunk ending on non-newline resets.
        assert_eq!(fold_trailing(2, "", "abc"), 0);
        // A chunk ending on `\n` resets-then-counts: trailing is 1.
        assert_eq!(fold_trailing(2, "", "abc\n"), 1);
    }
}
