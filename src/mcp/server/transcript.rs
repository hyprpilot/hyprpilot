//! Per-vendor answer extraction from a session transcript.
//!
//! Each turn writes its own `turns/<n>/turns.jsonl`, so everything
//! here sees exactly one turn and never has to find where it began.
//! What still differs per vendor is which event carries the text and
//! whether the run failed upstream instead of answering.
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

/// Extract ONE turn's outcome from its slice of the transcript.
///
/// The caller passes one turn's whole file, so no boundary has to be
/// found. That matters because nothing in a transcript marks where a
/// turn began, and a turn that DIES emits no terminal event at all —
/// when turns shared a file, guessing the boundary is how a turn-2
/// billing error became the answer to turn 3.
pub(crate) fn extract(turn: &str, provider: crate::config::AgentProvider) -> Answer {
    let events: Vec<Value> = turn
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();

    // An error outranks text from the same turn: a run that died
    // mid-sentence would otherwise read as a short successful answer.
    if let Some(err) = turn_error(&events) {
        return Answer::Failed(err);
    }

    let text = match provider {
        crate::config::AgentProvider::ClaudeCode => claude_answer(&events),
        crate::config::AgentProvider::Codex => last_field(&events, |event| {
            let is_message = event.get("type").and_then(Value::as_str) == Some("item.completed")
                && event.get("item").and_then(|i| i.get("type")).and_then(Value::as_str) == Some("agent_message");
            is_message
                .then(|| event.get("item").and_then(|i| i.get("text")).and_then(Value::as_str))
                .flatten()
        }),
        // opencode emits a `text` part per block of prose, including one
        // BEFORE its tool calls, so the answer is every block of the
        // turn rather than the last one.
        crate::config::AgentProvider::OpenCode => {
            let blocks: Vec<&str> = events
                .iter()
                .filter(|e| e.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|e| e.get("part").and_then(|p| p.get("text")).and_then(Value::as_str))
                .collect();
            (!blocks.is_empty()).then(|| blocks.join("\n"))
        }
    };

    text.map_or(Answer::Pending, Answer::Text)
}

/// claude carries the answer on the event that closes the turn, and
/// flags a failed one with `is_error`. Returning that text as an answer
/// hides a credit-balance or overload failure as a short reply.
fn claude_answer(events: &[Value]) -> Option<String> {
    let event = events
        .iter()
        .rev()
        .find(|e| e.get("type").and_then(Value::as_str) == Some("result"))?;
    let text = event.get("result").and_then(Value::as_str);
    if event.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Some(format!(
            "error: {}",
            text.unwrap_or("the vendor reported a failed result")
        ));
    }
    text.map(str::to_owned)
}

/// An `error` event in this turn.
///
/// Two shapes, because the vendors disagree: opencode nests the message
/// under `error.data.message`, codex puts it at the top level. Matching
/// only one silently drops the other's failures.
fn turn_error(events: &[Value]) -> Option<String> {
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("error"))
        .filter_map(|event| {
            if let Some(err) = event.get("error") {
                return err
                    .get("data")
                    .and_then(|d| d.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| err.get("name").and_then(Value::as_str))
                    .or_else(|| err.as_str())
                    .map(str::to_owned);
            }
            event.get("message").and_then(Value::as_str).map(str::to_owned)
        })
        .next_back()
}

/// Last matching value in the turn.
fn last_field(events: &[Value], pick: impl Fn(&Value) -> Option<&str>) -> Option<String> {
    events.iter().filter_map(pick).next_back().map(str::to_owned)
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

    #[test]
    fn codex_reads_the_agent_message_item() {
        let body = lines(&[
            serde_json::json!({ "type": "item.completed", "item": { "type": "reasoning", "text": "ignored" } }),
            serde_json::json!({ "type": "item.completed", "item": { "type": "agent_message", "text": "the answer" } }),
            serde_json::json!({ "type": "turn.completed" }),
        ]);

        assert_eq!(extract(&body, AgentProvider::Codex), Answer::Text("the answer".into()));
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

    /// claude flags a failed result with `is_error` and still fills
    /// `result`. Returning that as an answer hides a credit-balance
    /// failure as a short reply.
    #[test]
    fn a_claude_error_result_is_not_an_answer() {
        let body = lines(&[serde_json::json!({
            "type": "result", "is_error": true, "result": "Credit balance is too low"
        })]);

        assert_eq!(
            extract(&body, AgentProvider::ClaudeCode),
            Answer::Text("error: Credit balance is too low".into())
        );
    }

    /// codex puts its error message at the top level rather than under
    /// `error.data`. Matching only opencode's shape drops it silently.
    #[test]
    fn a_codex_top_level_error_is_matched() {
        let body = lines(&[serde_json::json!({ "type": "error", "message": "stream disconnected" })]);

        assert_eq!(
            extract(&body, AgentProvider::Codex),
            Answer::Failed("stream disconnected".into())
        );
    }

    /// opencode emits a `text` part per block of prose, including one
    /// BEFORE its tool calls, so the answer is every block of the turn
    /// rather than the last one.
    #[test]
    fn opencode_joins_every_block_of_the_turn() {
        let body = lines(&[
            serde_json::json!({ "type": "text", "part": { "text": "I'll check first." } }),
            serde_json::json!({ "type": "tool_use", "part": { "tool": "read" } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "tool-calls" } }),
            serde_json::json!({ "type": "text", "part": { "text": "Here is the result." } }),
            serde_json::json!({ "type": "step_finish", "part": { "reason": "stop" } }),
        ]);

        assert_eq!(
            extract(&body, AgentProvider::OpenCode),
            Answer::Text("I'll check first.\nHere is the result.".into())
        );
    }

    /// A turn that DIED emits no terminal event, so an error is all it
    /// leaves. It must still be found — that message is the only thing
    /// explaining the failure.
    #[test]
    fn a_turn_that_died_still_reports_its_error() {
        let body = lines(&[
            serde_json::json!({ "type": "text", "part": { "text": "starting" } }),
            serde_json::json!({ "type": "error", "error": { "data": { "message": "Payment Required" } } }),
        ]);

        assert_eq!(
            extract(&body, AgentProvider::OpenCode),
            Answer::Failed("Payment Required".into())
        );
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
