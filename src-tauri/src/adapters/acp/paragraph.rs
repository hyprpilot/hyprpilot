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
//! `agent_thought_trailing`), reset on every `TurnState::open`.
//! The actor's notification handler calls
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
///    exactly one `\n` (not `\n\n`) AND prior contributes nothing
///    (`prior_trailing == 0`). Vendors stream chunks like
///    `"\nPara 2."` expecting a paragraph break, but a single
///    `\n` is just a soft break in markdown. Prepend another `\n`
///    so the chunk's own leading newline reads as `\n\n`.
///
/// **No-op cases** — we ALSO short-circuit when prior + the chunk's
/// own leading newline already sum to ≥ 2. Without this the boundary
/// `prior_trailing == 1` + incoming `"\nFoo"` would land on
/// `\n + \n + \nFoo = \n\n\nFoo` — visually still one paragraph
/// break (markdown collapses any `\n\n+` run) but our `\n` is
/// wasted injection. Cleanest is to emit nothing when the agent
/// already provided enough newlines.
///
/// Chunks that lead with `\n\n` get no lift either — they already
/// carry the paragraph break.
pub(crate) fn soft_lift_prefix(prior_trailing: u8, incoming: &str) -> &'static str {
    if incoming.starts_with("\n\n") {
        return "";
    }
    let incoming_leads_with_newline = incoming.starts_with('\n');
    // Combined leading newlines at the boundary already reach the
    // markdown paragraph-break threshold — no lift needed, and any
    // prefix we returned would be wasted characters on the wire.
    let combined = u32::from(prior_trailing) + u32::from(incoming_leads_with_newline);
    if combined >= 2 {
        return "";
    }
    if incoming_leads_with_newline {
        return "\n";
    }
    if prior_trailing == 1 {
        return "\n";
    }
    ""
}

/// Compute a stronger prefix for a content-block boundary — the
/// vendor's `messageId` switched between two text chunks within the
/// same turn (typically a tool call interrupted text generation and
/// a fresh content block started after). The Claude / Codex models
/// emit a fresh content block whose first text token starts directly
/// with the new sentence — no leading whitespace, no leading newline.
/// Naive concat reads as `"...prior sentence.New sentence..."`.
///
/// Forces `\n\n` between the two so markdown renders a paragraph
/// break. Falls through cleanly when the natural state already
/// reaches `\n\n` (prior ends with `\n\n`, incoming leads with
/// `\n\n`, or the combination is on track to land there).
pub(crate) fn paragraph_break_prefix(prior_trailing: u8, incoming: &str) -> &'static str {
    if incoming.starts_with("\n\n") {
        return "";
    }
    if prior_trailing >= 2 {
        return "";
    }
    if incoming.starts_with('\n') {
        // prior + "\n" + chunk's own "\n…" lands on \n\n.
        if prior_trailing >= 1 {
            return "";
        }
        return "\n";
    }
    if prior_trailing == 1 {
        // prior "\n" + our "\n" = "\n\n" at the boundary.
        return "\n";
    }
    // prior 0, incoming non-newline → need full "\n\n".
    "\n\n"
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
    fn soft_lift_promotes_single_leading_newline_in_chunk_when_prior_contributes_nothing() {
        // Chunk starts with exactly one `\n` (not `\n\n`), and prior
        // trailing is zero. Prepend `\n` so the chunk's own leading
        // newline reads as a markdown paragraph break.
        assert_eq!(soft_lift_prefix(0, "\nPara 2."), "\n");
    }

    #[test]
    fn soft_lift_skips_chunk_self_lift_when_prior_already_contributes_a_newline() {
        // Prior trailing `\n` + the chunk's own leading `\n` already
        // sum to `\n\n` at the boundary — markdown paragraph break is
        // already on the wire. Lifting would emit a wasted extra `\n`
        // that markdown collapses anyway.
        assert_eq!(soft_lift_prefix(1, "\nPara 2."), "");
        assert_eq!(soft_lift_prefix(2, "\nPara 2."), "");
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

    // ── paragraph_break_prefix ────────────────────────────────────

    #[test]
    fn paragraph_break_forces_double_newline_on_clean_boundary() {
        // Captain's screenshot bug — tool call interrupts text, the
        // next text chunk arrives with no leading newline at all.
        // Without the forced break, concat reads "...behind.Now bg".
        assert_eq!(paragraph_break_prefix(0, "Found the real cause"), "\n\n");
    }

    #[test]
    fn paragraph_break_collapses_to_single_newline_when_prior_has_one() {
        // Prior trailing == 1, incoming non-newline → "\n" prefix
        // pushes the boundary to "\n\n".
        assert_eq!(paragraph_break_prefix(1, "Found the real cause"), "\n");
    }

    #[test]
    fn paragraph_break_returns_empty_when_prior_already_paragraph_terminated() {
        assert_eq!(paragraph_break_prefix(2, "Found"), "");
    }

    #[test]
    fn paragraph_break_returns_empty_when_chunk_brings_its_own_double_newline() {
        assert_eq!(paragraph_break_prefix(0, "\n\nFound"), "");
    }

    #[test]
    fn paragraph_break_handles_chunk_with_single_leading_newline() {
        // prior 0, chunk "\nFound" → prepend "\n" → "\n\nFound"
        assert_eq!(paragraph_break_prefix(0, "\nFound"), "\n");
        // prior 1, chunk "\nFound" → natural concat "\n\nFound", no prefix
        assert_eq!(paragraph_break_prefix(1, "\nFound"), "");
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

    // ── safety invariants ─────────────────────────────────────────────
    //
    // Captain's invariant: the lift logic must NEVER inject enough
    // characters at the boundary between two chunks to produce more
    // than one markdown paragraph break (one blank line) — even when
    // the agent's intent already carried zero or one newline at the
    // boundary. Stated mechanically:
    //
    //   1. Every prefix returned by either lift function contains ONLY
    //      `\n` characters — no spaces, no other content. So a prefix
    //      can only ever extend a contiguous newline run; it cannot
    //      sneak content between two runs that would turn one
    //      paragraph break into two.
    //   2. Every prefix has length <= 2. Combined with (1), the maximum
    //      we add to any newline run is 2 characters (\n\n). Markdown
    //      collapses every run of `\n\n+` between non-empty content
    //      into exactly ONE paragraph break.
    //
    // The two tests below exhaustively pin both invariants across the
    // full input space — there's no path through either function that
    // returns a non-newline character or a string longer than 2.

    #[test]
    fn lift_prefix_only_ever_contains_newlines() {
        let chunks: &[&str] = &[
            "",
            "Foo",
            "\nFoo",
            "\n\nFoo",
            "\n\n\nFoo",
            "\n",
            "\n\n",
            "\n\n\n",
            " ",
            " Foo",
            "Foo\n",
            "Foo\nBar",
        ];

        for &prior in &[0u8, 1, 2] {
            for &chunk in chunks {
                for prefix in [soft_lift_prefix(prior, chunk), paragraph_break_prefix(prior, chunk)] {
                    assert!(
                        prefix.chars().all(|c| c == '\n'),
                        "prefix returned a non-newline character: prior={prior}, chunk={chunk:?}, prefix={prefix:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn lift_prefix_never_exceeds_two_characters() {
        let chunks: &[&str] = &[
            "",
            "Foo",
            "\nFoo",
            "\n\nFoo",
            "\n\n\nFoo",
            "\n",
            "\n\n",
            "\n\n\n",
            " ",
            " Foo",
            "Foo\n",
            "Foo\nBar",
        ];

        for &prior in &[0u8, 1, 2] {
            for &chunk in chunks {
                let soft = soft_lift_prefix(prior, chunk);
                let forced = paragraph_break_prefix(prior, chunk);

                assert!(
                    soft.len() <= 2,
                    "soft_lift_prefix exceeded 2 chars: prior={prior}, chunk={chunk:?}, prefix={soft:?}",
                );
                assert!(
                    forced.len() <= 2,
                    "paragraph_break_prefix exceeded 2 chars: prior={prior}, chunk={chunk:?}, prefix={forced:?}",
                );
            }
        }
    }

    #[test]
    fn soft_lift_never_creates_a_break_when_no_newline_signal_is_present() {
        // The soft-lift path is the conservative one: it only ever
        // fires on a chunk boundary that ALREADY carries a newline
        // signal (prior tail ends \n, OR incoming leads with \n).
        // A clean non-newline boundary (mid-sentence token burst)
        // MUST emit nothing, no matter the prior state — verified
        // explicitly here so a future refactor can't silently flip
        // it into the "inject a break" branch.
        let non_newline_chunks: &[&str] = &["Foo", " ", " Foo", "Foo\nBar", ""];

        for &prior in &[0u8, 2] {
            for &chunk in non_newline_chunks {
                assert_eq!(
                    soft_lift_prefix(prior, chunk),
                    "",
                    "soft_lift injected a break on a non-newline boundary: prior={prior}, chunk={chunk:?}",
                );
            }
        }
    }
}
