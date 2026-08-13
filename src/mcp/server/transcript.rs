//! Per-vendor answer extraction from a session transcript.
//!
//! `turns.jsonl` is the vendor's own event stream, appended to across
//! every turn of a conversation. Getting the answer out of it means
//! knowing three things that differ per vendor: which event carries the
//! text, where the latest turn starts, and whether the run failed
//! upstream instead of answering.
//!
//! Callers used to do this by hand with `jq`, and the two ways it goes
//! wrong are both silent:
//!
//! - **Scoping with `tail -n1`.** `tail` counts LINES, so a multi-line
//!   answer is truncated to its last line — a measured three-paragraph
//!   opencode answer came back as one word. Scoping has to happen where
//!   the events are still events, which is why everything here slices by
//!   event index and never by line.
//! - **Running only the answer query.** It matches nothing on a failed
//!   run, so an auth or billing error reports as "the agent returned
//!   nothing". [`extract`] returns the error instead, so one read cannot
//!   miss it.

use serde_json::Value;

/// What a transcript's latest turn produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    /// The agent's own reply.
    Text(String),
    /// The run failed upstream — auth, quota, model availability. The
    /// answer query matches nothing in this case, which is why it is a
    /// variant rather than an absence.
    Failed(String),
    /// Nothing terminal yet: the turn is still running, or it ended
    /// without producing either.
    Pending,
}

impl Answer {
    /// Render for a resource read. The distinction survives as a prefix
    /// rather than a separate field, because a resource read is text.
    pub(crate) fn render(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Failed(msg) => format!("error: {msg}"),
            Self::Pending => String::new(),
        }
    }
}

/// Extract the latest turn's answer from a whole transcript body.
///
/// An `error` event wins over any text: a run that failed upstream may
/// still have emitted prose before dying, and reporting that prose as
/// the answer is how a billing failure reads as a short successful reply.
pub(crate) fn extract(body: &str, provider: crate::config::AgentProvider) -> Answer {
    let events: Vec<Value> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();

    if let Some(err) = last_error(&events) {
        return Answer::Failed(err);
    }

    let text = match provider {
        crate::config::AgentProvider::ClaudeCode => last_field(&events, |event| {
            (event.get("type").and_then(Value::as_str) == Some("result"))
                .then(|| event.get("result").and_then(Value::as_str))
                .flatten()
        }),
        crate::config::AgentProvider::Codex => last_field(&events, |event| {
            let is_message = event.get("type").and_then(Value::as_str) == Some("item.completed")
                && event.get("item").and_then(|i| i.get("type")).and_then(Value::as_str) == Some("agent_message");
            is_message
                .then(|| event.get("item").and_then(|i| i.get("text")).and_then(Value::as_str))
                .flatten()
        }),
        crate::config::AgentProvider::OpenCode => opencode_latest_turn(&events),
    };

    text.map_or(Answer::Pending, Answer::Text)
}

/// The last `error` event's message, if the run failed.
///
/// Upstream failures land HERE and leave `stderr.log` empty — a real
/// 402 produced exactly that — so a caller checking only stderr reports
/// "no output" for a billing error.
fn last_error(events: &[Value]) -> Option<String> {
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("error"))
        .filter_map(|event| {
            let err = event.get("error")?;
            err.get("data")
                .and_then(|d| d.get("message"))
                .and_then(Value::as_str)
                .or_else(|| err.get("name").and_then(Value::as_str))
                .map(str::to_owned)
        })
        .next_back()
}

/// Last matching value — `last`, not `first`, because `session_send`
/// appends to the same transcript. Taking the first match reports turn
/// 1's answer as the reply to turn 5, confidently and forever.
fn last_field(events: &[Value], pick: impl Fn(&Value) -> Option<&str>) -> Option<String> {
    events.iter().filter_map(pick).next_back().map(str::to_owned)
}

/// opencode emits no terminal event and no per-turn event.
///
/// It emits a `text` part per block of prose — one mid-turn before its
/// tool calls, another at the end — so "the last text part" is one block
/// of the answer, not the answer. The only turn boundary in the file is
/// `step_finish` with `reason: "stop"`; intermediate steps carry
/// `reason: "tool-calls"`. So the latest turn is every `text` between
/// the previous `stop` and the last one.
fn opencode_latest_turn(events: &[Value]) -> Option<String> {
    #[derive(PartialEq)]
    enum Marker {
        Text(String),
        TurnEnd,
    }

    let markers: Vec<Marker> = events
        .iter()
        .filter_map(|event| match event.get("type").and_then(Value::as_str) {
            Some("text") => event
                .get("part")
                .and_then(|p| p.get("text"))
                .and_then(Value::as_str)
                .map(|t| Marker::Text(t.to_owned())),
            Some("step_finish") => (event.get("part").and_then(|p| p.get("reason")).and_then(Value::as_str)
                == Some("stop"))
            .then_some(Marker::TurnEnd),
            _ => None,
        })
        .collect();

    // No `stop` at all means the turn never finished cleanly — treat it
    // as pending rather than guessing, and let the caller consult
    // `session_status`.
    let end = markers.iter().rposition(|m| *m == Marker::TurnEnd)?;
    let start = markers[..end]
        .iter()
        .rposition(|m| *m == Marker::TurnEnd)
        .map_or(0, |i| i + 1);

    let text: Vec<&str> = markers[start..end]
        .iter()
        .filter_map(|m| match m {
            Marker::Text(t) => Some(t.as_str()),
            Marker::TurnEnd => None,
        })
        .collect();

    (!text.is_empty()).then(|| text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;

    fn lines(events: &[Value]) -> String {
        events.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n")
    }

    /// The bug the `jq` guidance carried for a while: `tail -n1` counts
    /// lines, so a multi-line answer lost everything but its last line.
    /// Slicing by event cannot do that.
    #[test]
    fn a_multi_line_answer_survives_intact() {
        let body = lines(&[serde_json::json!({ "type": "result", "result": "line one\nline two\nline three" })]);

        assert_eq!(
            extract(&body, AgentProvider::ClaudeCode),
            Answer::Text("line one\nline two\nline three".into())
        );
    }

    /// `session_send` appends to the same transcript, so an unscoped
    /// query matches every turn oldest-first. Taking the first match
    /// answers turn 5 with turn 1's reply.
    #[test]
    fn the_latest_turn_wins_across_a_conversation() {
        let body = lines(&[
            serde_json::json!({ "type": "result", "result": "turn one" }),
            serde_json::json!({ "type": "result", "result": "turn two" }),
        ]);

        assert_eq!(
            extract(&body, AgentProvider::ClaudeCode),
            Answer::Text("turn two".into())
        );
    }

    #[test]
    fn codex_reads_the_agent_message_item() {
        let body = lines(&[
            serde_json::json!({ "type": "item.completed", "item": { "type": "reasoning", "text": "ignored" } }),
            serde_json::json!({ "type": "item.completed", "item": { "type": "agent_message", "text": "the answer" } }),
        ]);

        assert_eq!(extract(&body, AgentProvider::Codex), Answer::Text("the answer".into()));
    }

    /// opencode emits a `text` part per block of prose, including one
    /// BEFORE its tool calls, so "the last text part" is one block of
    /// the answer rather than the answer.
    #[test]
    fn opencode_joins_every_block_of_the_latest_turn() {
        let body = lines(&[
            serde_json::json!({ "type": "text", "part": { "text": "turn one answer" } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "stop" } }),
            serde_json::json!({ "type": "text", "part": { "text": "I'll check first." } }),
            serde_json::json!({ "type": "tool_use", "part": { "tool": "read" } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "tool-calls" } }),
            serde_json::json!({ "type": "text", "part": { "text": "Here is the result." } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "stop" } }),
        ]);

        assert_eq!(
            extract(&body, AgentProvider::OpenCode),
            Answer::Text("I'll check first.\nHere is the result.".into()),
            "both blocks of the LATEST turn, and nothing from the previous one"
        );
    }

    /// An unfinished opencode turn has no `stop`, so there is no
    /// boundary to slice on. Guessing would report a mid-turn block as
    /// the answer.
    #[test]
    fn opencode_without_a_stop_is_pending() {
        let body = lines(&[
            serde_json::json!({ "type": "text", "part": { "text": "thinking out loud" } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "tool-calls" } }),
        ]);

        assert_eq!(extract(&body, AgentProvider::OpenCode), Answer::Pending);
    }

    /// The failure the answer query goes blind on. A real 402 left
    /// `stderr.log` at zero bytes and the whole diagnosis in here, so a
    /// caller running only the answer query reports "returned nothing"
    /// for a billing error.
    #[test]
    fn an_upstream_error_is_reported_rather_than_silence() {
        let body = lines(&[serde_json::json!({
            "type": "error",
            "error": { "name": "ProviderError", "data": { "message": "Payment Required (statusCode 402)" } }
        })]);

        let answer = extract(&body, AgentProvider::OpenCode);
        assert_eq!(answer, Answer::Failed("Payment Required (statusCode 402)".into()));
        assert!(answer.render().starts_with("error: "));
    }

    /// An error wins over prose emitted before the failure — otherwise a
    /// run that died mid-sentence reads as a short successful answer.
    #[test]
    fn an_error_outranks_text_that_preceded_it() {
        let body = lines(&[
            serde_json::json!({ "type": "result", "result": "partial thought" }),
            serde_json::json!({ "type": "error", "error": { "name": "Overloaded" } }),
        ]);

        assert_eq!(
            extract(&body, AgentProvider::ClaudeCode),
            Answer::Failed("Overloaded".into())
        );
    }

    #[test]
    fn a_running_turn_has_no_answer_yet() {
        let body = lines(&[serde_json::json!({ "type": "tool_use", "part": { "tool": "read" } })]);

        assert_eq!(extract(&body, AgentProvider::ClaudeCode), Answer::Pending);
        assert_eq!(extract(&body, AgentProvider::OpenCode), Answer::Pending);
        assert_eq!(extract(&body, AgentProvider::Codex), Answer::Pending);
    }

    /// A transcript is appended to while the agent writes, so a torn
    /// final line is ordinary. It must not lose the events before it.
    #[test]
    fn a_partial_trailing_line_is_skipped_not_fatal() {
        let mut body = lines(&[serde_json::json!({ "type": "result", "result": "complete" })]);
        body.push_str("\n{\"type\":\"resu");

        assert_eq!(
            extract(&body, AgentProvider::ClaudeCode),
            Answer::Text("complete".into())
        );
    }
}
