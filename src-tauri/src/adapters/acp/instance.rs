//! `AcpInstance` — owner of one ACP-speaking agent subprocess: the
//! handle the registry keeps + the actor body that drives `initialize`
//! → `session/new` → `session/prompt` and forwards `SessionUpdate`s
//! to the registry's broadcast.
//!
//! `AcpInstance::start(...)` is the constructor (symmetric with
//! `AcpInstance::shutdown` from `InstanceActor`). It spawns the
//! long-lived task and returns the handle. The body, the
//! subprocess-spawn helper, and the prompt-block encoder all live
//! private to this module — they were once in separate
//! `runtime.rs` / `spawn.rs` files; consolidated so the actor's
//! lifecycle reads top-to-bottom in one place.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::{
    AudioContent, BlobResourceContents, CancelNotification, ClientCapabilities, ContentBlock, EmbeddedResource,
    EmbeddedResourceResource, FileSystemCapabilities, ImageContent, InitializeRequest, ListSessionsRequest,
    ListSessionsResponse, LoadSessionRequest, ModelId, NewSessionRequest, PromptRequest, ProtocolVersion, SessionId,
    SessionModeId, SetSessionModeRequest, SetSessionModelRequest, TextContent, TextResourceContents,
};
use agent_client_protocol::{ByteStreams, Client};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, error, info, trace, warn};

use super::agents::{match_provider_agent, SystemPromptInjection};
use super::client::{AcpClient, ClientEvent, SessionUpdateNotification};
use crate::adapters::instance::{InstanceActor, InstanceInfo, InstanceKey};
use crate::adapters::permission::PermissionController;
use crate::adapters::profile::ResolvedInstance;
use crate::adapters::transcript::Attachment;
use crate::adapters::{publish, Bootstrap, InstanceEvent, InstanceState, TerminalChunk, ToolIdentity};
use crate::config::AgentConfig;
use crate::tools::{TerminalToolEventKind, TerminalToolStream};

/// How long the registry waits for the actor to ack a `Shutdown`
/// command before dropping the handle.
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Single-lock turn-id state. Replaces two `Arc<RwLock<Option<String>>>`
/// — `current_turn_id` (any active turn, real or synthetic) and
/// `synthetic_turn_id` (out-of-turn-wrapper id when set) — that
/// previously raced across six write sites and three task graphs.
///
/// Invariants:
/// - When `synthetic` is `Some`, `current` MUST equal `synthetic` (a
///   synthetic IS the current turn).
/// - A real prompt arriving sets `current` and clears `synthetic`.
/// - Closing the synthetic clears `current` iff they still match
///   (a real prompt may have raced in between).
///
/// All mutations go through the typed methods; the field accessors
/// stay private so the invariant can't drift through field-level
/// writes.
#[derive(Debug, Default)]
struct TurnState {
    /// Currently open turn id, or `None`. A turn is a turn —
    /// regardless of whether the captain submitted a prompt or the
    /// daemon synthesized one to catch out-of-prompt agent activity
    /// (session/load replay, post-cancel residue, stray
    /// notifications). The [`Self::prompt_in_flight`] flag tracks
    /// the captain-submitted case; nothing else cares about the
    /// distinction. The earlier two-slot model (`current` +
    /// `synthetic`) collapsed into this one because every
    /// downstream consumer only ever needed "is some turn open" or
    /// "is the captain's prompt being awaited" — both answerable
    /// without naming an "auto" vs "real" category.
    current: Option<String>,
    /// `true` iff a captain `submit_prompt` is awaiting completion
    /// on [`Self::current`]. Drives the queue-strip decision: when
    /// `true`, incoming `Prompt` commands auto-enqueue; when
    /// `false` (idle OR an auto-turn from replay / stray
    /// notifications), they dispatch immediately. Set inside
    /// `TurnGuard::new`, cleared in its `Drop` / `complete()` path
    /// (RAII).
    prompt_in_flight: bool,
    /// True when the dispatcher routed at least one agent-output
    /// transcript item (`AgentText` / `AgentThought` / `AgentAttachment`
    /// / `ToolCall` / `ToolCallUpdate` / `Plan`) for the currently open
    /// turn. Reset on every `open`. Read by the
    /// prompt-future at completion: a vendor that returned a
    /// `stop_reason: null` AND emitted nothing during the turn used to
    /// surface as a bare `TurnEnded { error: None }`, which downstream
    /// frontends (hyprpilot-nvim in particular) render as a generic
    /// "Internal error" with no actionable signal. With this flag we
    /// can synthesize a specific error message instead.
    output_observed: bool,
    /// Trailing-newline count (capped at 2) of the accumulated
    /// `AgentText` chunks for the open turn. Drives the markdown
    /// paragraph lift in `note_agent_text` — when prior trailing == 1
    /// and the next chunk doesn't begin with `\n`, we prepend `\n`
    /// to the chunk so the boundary reaches `\n\n` and markdown
    /// renders two paragraphs instead of one with a soft break. See
    /// `acp::paragraph` for the rule.
    agent_text_trailing: u8,
    /// Same counter for the `AgentThought` stream — applied
    /// independently because thoughts ride a separate rendering
    /// surface (the thinking card) from agent text.
    agent_thought_trailing: u8,
    /// Streaming markdown-fence state for `AgentText`. Boundary
    /// prefixes are suppressed while inside a fenced code block so
    /// paragraph repair never injects blank lines into code.
    agent_text_fence: super::paragraph::FenceState,
    /// Same markdown-fence state for the `AgentThought` stream.
    agent_thought_fence: super::paragraph::FenceState,
    /// Most-recent vendor-emitted `messageId` on this turn's
    /// `agent_message_chunk` stream. Claude / Codex emit a fresh id
    /// per content block; a tool call between two text chunks
    /// produces two distinct messageIds. When the next chunk's id
    /// differs from this, we force a markdown paragraph break in
    /// the wire chunk's text — without it, the concat of
    /// `"...prior sentence."` then `"Next sentence..."` reads as
    /// `"...prior sentence.Next sentence..."` (captain's screenshot
    /// bug). Reset on `open`. `None` until
    /// the first chunk lands or when the vendor doesn't emit a
    /// messageId on the chunk (gracefully degrades to soft-lift).
    last_agent_text_message_id: Option<String>,
    /// Same as [`Self::last_agent_text_message_id`] for the thought
    /// stream — independent because the two render on different
    /// surfaces.
    last_agent_thought_message_id: Option<String>,
    /// `true` when a non-`AgentText` transcript item (typically a
    /// `ToolCall` / `ToolCallUpdate` / `Plan` / `AgentAttachment`)
    /// has landed since the last `AgentText` chunk. The next
    /// `AgentText` chunk treats this as a content-block boundary
    /// equivalent to a `messageId` switch — vendors don't always
    /// allocate a fresh `messageId` after a tool call returns
    /// (Claude regularly reuses the same id across text→tool→text),
    /// and without this flag the resumed text would concat directly
    /// onto the prior sentence ("...behind.Now bg..."). Reset after
    /// the next text chunk consumes it. Independent flag for the
    /// thought stream below — same reasoning, different stream.
    non_text_event_since_last_text: bool,
    /// `true` when a non-`AgentThought` transcript item has landed
    /// since the last `AgentThought` chunk. Mirrors
    /// [`Self::non_text_event_since_last_text`] for the thought
    /// stream.
    non_text_event_since_last_thought: bool,
    /// Role of the last emitted transcript item under
    /// [`Self::current`]. Drives the role-transition turn split on
    /// `session/load` replay: ACP streams the prior session's
    /// history through `session/update` notifications and emits NO
    /// turn boundaries, so the daemon's only signal that a new
    /// logical turn started is `user_message_chunk` arriving while
    /// the prior role was `Agent`. On that transition the daemon
    /// closes the current turn (emits `TurnEnded`) and opens a
    /// fresh auto-turn so every consumer sees real per-exchange
    /// turn ids — same heuristic the nvim plugin's renderer used
    /// to apply client-side, lifted here so every frontend
    /// benefits without each implementing the workaround. Reset on
    /// each turn open.
    last_role: Option<Role>,
}

/// Role of the last emitted item under the current turn. Used for
/// the role-transition turn split on `session/load` replay — see
/// [`TurnState::last_role`]. Tool calls and thoughts count as
/// `Agent` because they're part of the agent's work on the same
/// exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

impl Role {
    /// Classify a `TranscriptItem`. Returns `None` for kinds that
    /// don't participate in role grouping (`PermissionRequest`,
    /// `Unknown`) — those don't trigger a turn split either way.
    fn of(item: &crate::adapters::TranscriptItem) -> Option<Self> {
        use crate::adapters::TranscriptItem as TI;
        match item {
            TI::UserPrompt { .. } | TI::UserText { .. } => Some(Self::User),
            TI::AgentText { .. }
            | TI::AgentThought { .. }
            | TI::AgentAttachment(_)
            | TI::ToolCall(_)
            | TI::ToolCallUpdate(_)
            | TI::Plan(_) => Some(Self::Agent),
            TI::PermissionRequest(_) | TI::Unknown { .. } => None,
        }
    }
}

impl TurnState {
    fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// `true` iff the captain's `submit_prompt` is awaiting
    /// completion on the current turn. The queue-strip uses this
    /// to decide whether to enqueue an incoming `Prompt`
    /// (`true` → enqueue) or dispatch it (`false` → dispatch).
    /// Auto-opened turns from session/load replay or post-cancel
    /// residue return `false` here so a captain submitting on top
    /// of replay output dispatches immediately instead of getting
    /// stuck in the queue.
    fn prompt_in_flight(&self) -> bool {
        self.prompt_in_flight
    }

    /// Open a turn. Caller MUST have already cleared any prior
    /// turn (via `take_if_unguarded` or `take_current`) and
    /// emitted its `TurnEnded`. `prompt_in_flight` resets to
    /// `false`; the prompt-driven path (TurnGuard::new) sets it
    /// to `true` immediately after.
    fn open(&mut self, turn_id: String) {
        self.current = Some(turn_id);
        self.prompt_in_flight = false;
        self.reset_per_turn_lift_state();
    }

    /// Internal — clear every per-turn lift counter / cache on
    /// turn open. Called by [`Self::open`] so the boundary state
    /// is uniform regardless of how the turn was minted.
    fn reset_per_turn_lift_state(&mut self) {
        self.output_observed = false;
        self.agent_text_trailing = 0;
        self.agent_thought_trailing = 0;
        self.agent_text_fence.reset();
        self.agent_thought_fence.reset();
        self.last_agent_text_message_id = None;
        self.last_agent_thought_message_id = None;
        self.non_text_event_since_last_text = false;
        self.non_text_event_since_last_thought = false;
        self.last_role = None;
    }

    /// Compute the markdown-paragraph lift prefix for an incoming
    /// `AgentText` chunk AND fold the new trailing-newline count into
    /// the running tally. Returns the prefix (`""`, `"\n"`, or `"\n\n"`)
    /// — the caller is responsible for prepending it to the chunk
    /// text before emitting / persisting.
    ///
    /// `message_id` is the vendor-emitted content-block id (when
    /// present). When it differs from the prior chunk's id, the
    /// stronger `paragraph_break_prefix` runs instead of the
    /// soft-lift — Claude / Codex emit fresh ids per content block,
    /// and a tool call between two text chunks produces two distinct
    /// ids. The forced break turns "...prior sentence.New sentence"
    /// (the screenshot bug) into "...prior sentence.\n\nNew sentence"
    /// for every consumer of the concat.
    ///
    /// `None` for `message_id` falls back to soft-lift only — vendors
    /// without the `unstable_message_id` feature simply lose the
    /// content-block boundary detection (the soft-lift still catches
    /// the cases it can).
    ///
    /// Mutates state regardless of whether the prefix is non-empty —
    /// the tally must track every chunk so the NEXT chunk's lift
    /// decision sees the right prior state.
    ///
    /// Two boundary triggers force the stronger `paragraph_break_prefix`
    /// path (vs the conservative soft-lift):
    ///   1. **messageId switch** — the vendor opened a fresh content
    ///      block (Claude / Codex emit a new id per block).
    ///   2. **non-text event interrupt** — a `ToolCall` /
    ///      `ToolCallUpdate` / `Plan` / `AgentAttachment` landed
    ///      between the prior text chunk and this one. Vendors often
    ///      keep the SAME messageId across text→tool→text, so the
    ///      messageId check alone misses this case and the resumed
    ///      sentence concats directly onto the prior one. The
    ///      `non_text_event_since_last_text` flag closes the gap.
    ///
    /// Either trigger consumes (clears) `non_text_event_since_last_text`.
    fn note_agent_text(&mut self, incoming: &str, message_id: Option<&str>) -> &'static str {
        let prior = self.agent_text_trailing;
        let id_boundary = is_new_content_block(self.last_agent_text_message_id.as_deref(), message_id);
        let event_boundary = self.non_text_event_since_last_text;
        let boundary = id_boundary || event_boundary;
        let prefix = if self.agent_text_fence.in_fence() {
            ""
        } else if boundary {
            super::paragraph::paragraph_break_prefix(prior, incoming)
        } else {
            super::paragraph::soft_lift_prefix(prior, incoming)
        };

        self.agent_text_trailing = super::paragraph::fold_trailing(prior, prefix, incoming);
        self.agent_text_fence.observe(prefix, incoming);
        if let Some(id) = message_id {
            self.last_agent_text_message_id = Some(id.to_string());
        }
        self.non_text_event_since_last_text = false;
        prefix
    }

    /// `note_agent_text` for the `AgentThought` stream — independent
    /// trailing counter + messageId tracker + non-text-event flag
    /// since thoughts render on a separate surface (the thinking
    /// card) and have their own content-block id stream.
    fn note_agent_thought(&mut self, incoming: &str, message_id: Option<&str>) -> &'static str {
        let prior = self.agent_thought_trailing;
        let id_boundary = is_new_content_block(self.last_agent_thought_message_id.as_deref(), message_id);
        let event_boundary = self.non_text_event_since_last_thought;
        let boundary = id_boundary || event_boundary;
        let prefix = if self.agent_thought_fence.in_fence() {
            ""
        } else if boundary {
            super::paragraph::paragraph_break_prefix(prior, incoming)
        } else {
            super::paragraph::soft_lift_prefix(prior, incoming)
        };

        self.agent_thought_trailing = super::paragraph::fold_trailing(prior, prefix, incoming);
        self.agent_thought_fence.observe(prefix, incoming);
        if let Some(id) = message_id {
            self.last_agent_thought_message_id = Some(id.to_string());
        }
        self.non_text_event_since_last_thought = false;
        prefix
    }

    /// Mark that a non-text transcript item (tool call, plan, attachment,
    /// …) landed in the open turn. The NEXT `AgentText` /
    /// `AgentThought` chunk treats this as a content-block boundary
    /// (forces `paragraph_break_prefix`). Cleared by the next text /
    /// thought chunk. Idempotent.
    fn note_non_text_event(&mut self) {
        self.non_text_event_since_last_text = true;
        self.non_text_event_since_last_thought = true;
    }

    /// Mark the current turn as having emitted at least one agent-
    /// output transcript item. Idempotent.
    fn note_agent_output(&mut self) {
        self.output_observed = true;
    }

    /// Did the current turn produce any agent-output? Read by the
    /// prompt-future on completion to decide whether to synthesize a
    /// "no output" error when the vendor returns a null stop reason.
    fn output_observed(&self) -> bool {
        self.output_observed
    }

    /// Close the current turn iff `turn_id` still owns the slot.
    /// Returns true when the caller's id matched (so it can emit
    /// `TurnEnded` exactly once). Also clears `prompt_in_flight`.
    fn close_if_current(&mut self, turn_id: &str) -> bool {
        if self.current.as_deref() != Some(turn_id) {
            return false;
        }
        self.current = None;
        self.prompt_in_flight = false;
        true
    }

    /// Take and return the current turn id IFF no captain prompt
    /// is in flight. Used by:
    ///   - `submit_prompt`: yank any non-prompt-driven turn so a
    ///     fresh prompt-driven `open` doesn't collide.
    ///   - The role-transition split: close the current turn before
    ///     opening a fresh one for the new exchange.
    ///   - `LoadSession`-completion: close the last replay turn
    ///     left dangling when the load response resolves.
    ///
    /// Returns `None` when no turn is open OR when a prompt is in
    /// flight (callers shouldn't yank captain work).
    fn take_if_unguarded(&mut self) -> Option<String> {
        if self.prompt_in_flight {
            return None;
        }
        self.current.take()
    }

    /// Take and return the current turn id unconditionally,
    /// clearing `prompt_in_flight`. Used by the Cancel path —
    /// captain explicitly cancelled, so the prompt-in-flight
    /// signal must drop even if the prompt-future hasn't returned
    /// yet.
    fn take_current(&mut self) -> Option<String> {
        let cur = self.current.take()?;
        self.prompt_in_flight = false;
        Some(cur)
    }

    /// Record the role of the last item routed under the current
    /// turn. Read by [`Self::should_split_on`] for the role-
    /// transition split on `session/load` replay.
    fn note_role(&mut self, role: Role) {
        self.last_role = Some(role);
    }

    /// `true` iff a turn is open AND the captain isn't actively
    /// awaiting a prompt AND the last emitted role was `Agent`
    /// AND the incoming role is `User`. That's the role transition
    /// signalling "new logical exchange started" — split here.
    /// We guard on `!prompt_in_flight` because real prompt turns
    /// must NOT be split mid-flight by stray user-side events
    /// (the agent-side `UserText` echo path is the documented
    /// only legitimate case, and it doesn't trip this condition
    /// because the prior role is also user when the echo lands).
    fn should_split_on(&self, incoming: Role) -> bool {
        self.current.is_some()
            && !self.prompt_in_flight
            && self.last_role == Some(Role::Agent)
            && incoming == Role::User
    }
}

type SharedTurnState = Arc<tokio::sync::RwLock<TurnState>>;

/// `true` when the incoming chunk's `messageId` differs from the prior
/// chunk's id on the same stream — the signal that the vendor opened
/// a fresh content block. `None` on either side falls through to
/// `false` (no boundary): a vendor that never emits messageIds keeps
/// the soft-lift path; the first chunk of a turn has no prior so
/// `prior` is `None` and we treat it as a continuation rather than
/// inventing a boundary.
fn is_new_content_block(prior: Option<&str>, incoming: Option<&str>) -> bool {
    matches!((prior, incoming), (Some(p), Some(i)) if p != i)
}

/// RAII handle for one open `session/prompt` turn. Constructor emits
/// `TurnStarted` + claims the turn slot via `TurnState::open` (then
/// flips `prompt_in_flight = true`);
/// `complete(...)` emits `TurnEnded` with the agent-supplied stop
/// reason / error; `Drop` is the leak fallback — when the spawned
/// prompt task panics, the transport closes mid-turn, or the actor
/// unwinds before `complete` runs, the guard synthesises
/// `TurnEnded { stop_reason: "cancelled" }` and frees the slot.
///
/// The slot still mediates ownership across the actor + the spawn
/// future: a concurrent `Cancel` handler atomically takes the slot
/// and emits its own `TurnEnded { stop_reason: "cancelled" }`.
/// `complete` (and `Drop`) re-check ownership before emitting, so a
/// raced cancel doesn't double-fire.
struct TurnGuard {
    turn_id: String,
    instance_id: String,
    agent_id: String,
    session_id: String,
    events_tx: broadcast::Sender<InstanceEvent>,
    mirror: Arc<crate::adapters::InstanceMirror>,
    turn_state: SharedTurnState,
    completed: bool,
}

impl TurnGuard {
    async fn new(
        turn_id: String,
        agent_id: String,
        instance_id: String,
        session_id: String,
        events_tx: broadcast::Sender<InstanceEvent>,
        mirror: Arc<crate::adapters::InstanceMirror>,
        turn_state: SharedTurnState,
    ) -> Self {
        {
            let mut s = turn_state.write().await;
            s.open(turn_id.clone());
            s.prompt_in_flight = true;
        }
        let event = InstanceEvent::TurnStarted {
            agent_id: agent_id.clone(),
            instance_id: instance_id.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            started_at: now_epoch_ms(),
        };
        publish(&mirror, &events_tx, event).await;
        Self {
            turn_id,
            instance_id,
            agent_id,
            session_id,
            events_tx,
            mirror,
            turn_state,
            completed: false,
        }
    }

    /// Returns true when this guard still owned the slot at emit time
    /// (so the caller knows whether to follow up with the per-turn
    /// `InstanceMeta` refresh — only fires alongside an emit).
    ///
    /// When the caller passes `stop_reason: None` AND `error: None`
    /// AND the dispatcher never routed an agent-output transcript
    /// item for this turn, synthesize a specific `error` describing
    /// the empty-turn case. Without this, downstream frontends see a
    /// bare `TurnEnded` with no diagnostic signal at all — hyprpilot-
    /// nvim ends up labeling these as a generic "Internal error" which
    /// is both wrong (the daemon didn't error) and unhelpful (the
    /// captain has nothing to act on). Synthesizing here keeps every
    /// "no output, no reason" case readable to every transport without
    /// each one re-implementing the heuristic.
    async fn complete(mut self, stop_reason: Option<String>, mut error: Option<String>) -> bool {
        self.completed = true;
        let mut slot = self.turn_state.write().await;
        if !slot.close_if_current(&self.turn_id) {
            return false;
        }
        let output_observed = slot.output_observed();
        drop(slot);

        if stop_reason.is_none() && error.is_none() && !output_observed {
            error = Some(
                "agent ended the turn without emitting any output and without a \
                 stop reason — vendor returned a null `stop_reason` and no \
                 session updates landed between TurnStarted and TurnEnded. \
                 Re-run with `RUST_LOG=acp::wire=trace` to see the raw \
                 session/prompt response."
                    .to_string(),
            );
        }
        let event = InstanceEvent::TurnEnded {
            agent_id: self.agent_id.clone(),
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            stop_reason,
            error,
            ended_at: now_epoch_ms(),
        };
        publish(&self.mirror, &self.events_tx, event).await;
        true
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // Best-effort: if a concurrent task holds the slot we let it
        // win (it'll synthesise its own TurnEnded). try_write avoids
        // blocking + the awkward "Drop in async context" trap.
        let mut slot = match self.turn_state.try_write() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !slot.close_if_current(&self.turn_id) {
            return;
        }
        let event = InstanceEvent::TurnEnded {
            agent_id: self.agent_id.clone(),
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            stop_reason: Some("cancelled".to_string()),
            error: None,
            ended_at: now_epoch_ms(),
        };
        // The documented exception to `mirror::publish` (which is
        // async): Drop is sync, so the apply spawns onto the runtime
        // separately so the leak-path TurnEnded still lands in the
        // mirror.
        let mirror = self.mirror.clone();
        let evt_for_mirror = event.clone();
        tokio::spawn(async move {
            mirror.apply(&evt_for_mirror).await;
        });
        let _ = self.events_tx.send(event);
    }
}

/// Take the current turn iff it's an auto-turn and emit
/// `TurnEnded`. Used by every site that needs to close an auto-
/// turn cleanly: the role-transition split (in the session/update
/// handler) and the post-LoadSession quiet-down (after the
/// LoadSessionResponse resolves). Returns the closed turn id (if
/// any) so the caller can chain.
///
/// Replaced the previous `spawn_synthetic_close_after` 2500ms
/// quiet-window timer — wall-clock heuristics are unreliable
/// under fast replay (events keep coming within 2500ms, timer
/// never fires) and add background tasks that race the actor's
/// own lifecycle. Lifecycle-driven explicit closes are the right
/// shape: a turn closes when something definite happens (role
/// transition, load response, cancel, prompt completion), not
/// when an arbitrary timer ticks.
async fn close_auto_turn_if_open(
    turn_state: &SharedTurnState,
    events_tx: &broadcast::Sender<InstanceEvent>,
    mirror: &Arc<crate::adapters::InstanceMirror>,
    agent_id: &str,
    instance_id: &str,
    session_id: &str,
    stop_reason: &'static str,
) -> Option<String> {
    let auto = turn_state.write().await.take_if_unguarded()?;
    debug!(
        agent = %agent_id,
        session = %session_id,
        turn = %auto,
        stop_reason,
        "acp::instance: closing auto-turn"
    );
    let event = InstanceEvent::TurnEnded {
        agent_id: agent_id.to_string(),
        instance_id: instance_id.to_string(),
        session_id: session_id.to_string(),
        turn_id: auto.clone(),
        stop_reason: Some(stop_reason.into()),
        error: None,
        ended_at: 0,
    };
    publish(mirror, events_tx, event).await;
    Some(auto)
}

/// Register a typed `on_receive_request` handler that delegates to an
/// async `AcpClient` method returning `Result<Response,
/// agent_client_protocol::Error>`. One registration line per method
/// keeps the handler chain legible.
macro_rules! register_client_handler {
    ($builder:expr, $client:expr, $method:ident) => {{
        let client = $client.clone();
        $builder.on_receive_request(
            move |req, responder: agent_client_protocol::Responder<_>, _cx| {
                let client = client.clone();
                async move { responder.respond_with_result(client.$method(&req).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
    }};
}

/// Commands the per-instance actor accepts. The actor keeps state
/// internal; this enum is the only public surface the dispatcher
/// uses to drive it.
#[derive(Debug)]
pub enum InstanceCommand {
    Prompt {
        text: String,
        attachments: Vec<Attachment>,
        /// When `true`, skip the auto-route-into-queue branch and
        /// dispatch immediately. Used by `queue/dispatch` so popped
        /// items go on-wire NOW (ACP serialises on the wire so a
        /// concurrent active turn just chains behind; idle goes
        /// straight through). External `prompts/send` always supplies
        /// `false` so the captain's submit-while-busy auto-queues.
        force_dispatch: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Cancel {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Switch the active operational mode (e.g. claude-code's
    /// `plan` / `edit`). Sends ACP `session/set_mode` and updates the
    /// per-instance metadata so the next `InstanceMeta` event carries
    /// the new value.
    SetMode {
        mode_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Switch the active model on a live session. ACP gates this
    /// behind the `unstable_session_model` feature; our dependency
    /// has it enabled via `["unstable"]`.
    SetModel {
        model_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Switch a generic session config option — ACP's
    /// `session/set_config_option`. Carries
    /// `thought_level` (a reserved spec category for reasoning depth),
    /// `mode` / `model` (when the agent surfaces them via
    /// `configOptions` instead of dedicated mode / model surfaces), AND
    /// every vendor-specific category whose id starts with `_` (per
    /// spec: `_*` ids are free for custom use). The captain picks one
    /// of the offered values via the palette; this command sends the
    /// pick to the agent + adopts the response's
    /// `configOptions: Vec<SessionConfigOption>` as the new advertised
    /// set. Independent of `set_mode` / `set_model`: those address the
    /// dedicated wire methods; this one is the generic catch-all.
    SetConfigOption {
        config_id: String,
        value: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Ask the agent for its persisted session index. Works in any
    /// bootstrap mode — the actor is always past `initialize` by the
    /// time it processes commands.
    ListSessions {
        cwd: Option<std::path::PathBuf>,
        reply: oneshot::Sender<Result<ListSessionsResponse, String>>,
    },
    /// Read the actor's current cached metadata (cwd, current
    /// mode/model id, advertised mode/model lists). Powers the
    /// `instance_meta` Tauri command used by the palette pickers —
    /// the palette ALWAYS routes through this snapshot rather than
    /// reading the UI-side `useSessionInfo` cache, so a stale UI
    /// state can't desync the picker from the daemon's authoritative
    /// view.
    MetaSnapshot {
        reply: oneshot::Sender<MetaSnapshot>,
    },
    /// Shutdown hook — stops the actor after the current prompt
    /// (or immediately if idle). Reply carries the final state.
    Shutdown {
        reply: oneshot::Sender<()>,
    },
    /// Remove a specific item by id. Reply is `Ok(true)` when found.
    QueueRemove {
        item_id: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// Reorder: move an existing item to a new position (clamped to
    /// `[0, len-1]`). Same semantics as drag-and-drop.
    QueueMove {
        item_id: String,
        position: usize,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// Drop every item. Reply carries the count of dropped entries.
    QueueClear {
        reply: oneshot::Sender<Result<u32, String>>,
    },
    /// Read the current queue without mutating. Powers the
    /// `queue/list` RPC + first-observation hydration on the UI.
    QueueList {
        reply: oneshot::Sender<Vec<crate::adapters::queue::QueueItem>>,
    },
    /// In-place edit of a queued item. `text` overwrites the existing
    /// text; `attachments` (when `Some`) replaces the existing list
    /// wholesale (`None` leaves attachments untouched). `id`,
    /// `enqueued_seq`, and `enqueued_at` are preserved — the queued
    /// item keeps its slot + ordering.
    QueueEdit {
        item_id: String,
        text: String,
        attachments: Option<Vec<Attachment>>,
        reply: oneshot::Sender<Result<crate::adapters::queue::QueueItem, String>>,
    },
    /// Pop the head (or a specific item by id) AND dispatch it as a
    /// `session/prompt`. The pop happens before the prompt future
    /// fires, so a concurrent `prompts/cancel` aborts the in-flight
    /// turn but does NOT re-insert the popped item — captain
    /// explicit re-submit if they wanted it back.
    QueueDispatch {
        item_id: Option<String>,
        reply: oneshot::Sender<Result<crate::adapters::queue::QueueDispatchResult, String>>,
    },
}

/// Snapshot of the per-instance metadata the daemon caches off
/// `NewSessionResponse` / `LoadSessionResponse` / `set_mode` /
/// `set_model` replies. Read-only view; identical to the payload
/// the `acp:instance-meta` Tauri event carries, minus identity
/// fields the caller already knows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaSnapshot {
    pub session_id: Option<String>,
    pub cwd: String,
    pub current_mode_id: Option<String>,
    pub current_model_id: Option<String>,
    pub available_modes: Vec<crate::adapters::SessionModeInfo>,
    pub available_models: Vec<crate::adapters::SessionModelInfo>,
    pub mcps_count: usize,
}

/// Project an ACP `session/update` notification (as raw JSON, since
/// `TolerantSessionNotification` carries the payload untyped) into a
/// typed `TranscriptItem`. Returns the item the daemon publishes via
/// `InstanceEvent::Transcript`.
///
/// Unknown / future variants land as `TranscriptItem::Unknown`
/// carrying the raw `sessionUpdate` discriminator + payload — the UI
/// dispatches on `item.kind` (`unknown`) and can render a placeholder
/// or sub-dispatch on `item.kind` from the payload. Forward-compat
/// without bricking sessions.
/// Outcome of mapping one ACP `SessionUpdate` payload. Most variants
/// land in the transcript; metadata updates (`session_info_update`,
/// `current_mode_update`) are routed to dedicated `InstanceEvent`
/// variants instead since they aren't transcript content.
/// `Transcript` carries the full `TranscriptItem` enum (~250 bytes
/// for the largest variant: `ToolCall` with embedded `FormattedToolCall`).
/// Sibling variants (`SessionInfo`, `CurrentMode`, `AvailableCommands`)
/// are smaller; boxing the larger one would force a heap allocation
/// for every transcript chunk on the hot path. The size disparity is
/// the cost of the wire-shape simplicity.
#[allow(clippy::large_enum_variant)]
pub(crate) enum MappedUpdate {
    Transcript(crate::adapters::TranscriptItem),
    SessionInfo {
        title: Option<String>,
        updated_at: Option<String>,
    },
    CurrentMode {
        current_mode_id: String,
    },
    AvailableCommands {
        commands: Vec<crate::completion::source::commands::CommandSummary>,
    },
    /// Per-session usage telemetry — context budget + cost. claude-
    /// agent-acp emits this every few notifications during a turn;
    /// the UI accumulates it onto the active turn so the captain sees
    /// live spend + window utilisation as the turn streams.
    Usage {
        used: u64,
        size: u64,
        cost: Option<UsageCost>,
    },
    /// Adapter-defined session config options — `effort` (adaptive
    /// thinking), per-vendor toggles, etc. Mirrors the existing
    /// `mode` / `model` flow but generalised to the open category
    /// space. UI maps each category to a palette leaf so captains
    /// pick `effort: high` without memorising claude-agent-acp's
    /// magic vocab.
    ConfigOptions {
        categories: Vec<crate::adapters::SessionConfigOptionCategory>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub amount: f64,
    pub currency: String,
}

/// Outcome of one mapper call — the typed `MappedUpdate` plus the
/// envelope's `_meta` (vendor-specific extension data). `_meta` rides
/// alongside on `InstanceEvent::Transcript`; today no UI consumer
/// reads it, but the pass-through is wired so future per-vendor UI
/// hooks plug in without a wire change.
pub(crate) struct MappedSessionUpdate {
    pub mapped: MappedUpdate,
    pub meta: Option<serde_json::Value>,
    /// Vendor-emitted content-block id, extracted from
    /// `agent_message_chunk` / `agent_thought_chunk` payloads
    /// (`messageId` field on the wire, present under ACP's
    /// `unstable_message_id` feature). `None` for non-chunk updates
    /// and for vendors that don't emit one. Threaded into
    /// `TurnState::note_agent_text` / `note_agent_thought` so the
    /// daemon-side paragraph lift can detect content-block
    /// boundaries (a tool call between two text chunks switches
    /// the id; the lift forces `\n\n` across the boundary so the
    /// concat reads as two paragraphs).
    pub message_id: Option<String>,
}

/// Per-id running tool-call state — feeds the formatter on every
/// `tool_call_update` so the `formatted` snapshot reflects merged
/// state, not just the delta. Owned by the per-instance notification
/// task; cleared on session boundary (load_session swap, shutdown).
///
/// `started_at` captures wall-clock at first `tool_call` observation;
/// `completed_at` lands on first state transition to `Completed` /
/// `Failed`. Both as epoch milliseconds. Per-vendor formatters that
/// want a `Stat::Duration` read both off `FormatterContext` and emit
/// `ms = completed_at - started_at` once `completed_at.is_some()`.
#[derive(Debug, Default, Clone)]
pub(crate) struct RunningToolCall {
    pub wire_name: String,
    pub identity: ToolIdentity,
    pub tool_kind: String,
    pub raw_input: Option<serde_json::Value>,
    pub content: Vec<serde_json::Value>,
    pub started_at: u64,
    pub completed_at: Option<u64>,
}

pub(crate) type ToolCallCache = std::collections::HashMap<String, RunningToolCall>;

/// Per-instance bag of state needed to construct an
/// `InstanceEvent::InstanceMeta` event. Slow-changing identity bits
/// live as cloned strings; the four `Arc<RwLock<…>>` handles share
/// the same backing as the actor's per-instance metadata caches, so
/// `emit()` reads the current values atomically.
///
/// Replaces 6 inline 10-field struct-literal builds across the actor
/// — every site (Fresh / Resume bootstraps, prompt end, cancel end,
/// SetMode success, SetModel success) had its own copy. A single
/// `meta_ctx.emit(&events_tx, session_id).await` per site collapses
/// the noise + fixes the silent stale on the `SetConfigOption` arm
/// (which had no refresh emit at all).
#[derive(Clone)]
struct MetaEmitter {
    agent_id: String,
    instance_id: String,
    profile_id: Option<String>,
    cwd: String,
    current_mode: Arc<tokio::sync::RwLock<Option<String>>>,
    current_model: Arc<tokio::sync::RwLock<Option<String>>>,
    available_modes: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModeInfo>>>,
    available_models: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModelInfo>>>,
    mcps_count: usize,
    mirror: Arc<crate::adapters::InstanceMirror>,
}

impl MetaEmitter {
    async fn emit(&self, events_tx: &broadcast::Sender<InstanceEvent>, session_id: Option<String>) {
        let event = InstanceEvent::InstanceMeta {
            agent_id: self.agent_id.clone(),
            instance_id: self.instance_id.clone(),
            profile_id: self.profile_id.clone(),
            session_id,
            cwd: self.cwd.clone(),
            current_mode_id: self.current_mode.read().await.clone(),
            current_model_id: self.current_model.read().await.clone(),
            available_modes: self.available_modes.read().await.clone(),
            available_models: self.available_models.read().await.clone(),
            mcps_count: self.mcps_count,
        };
        publish(&self.mirror, events_tx, event).await;
    }
}

fn format_running(adapter_id: &str, running: &RunningToolCall) -> crate::tools::formatter::types::FormattedToolCall {
    use crate::tools::formatter::registry::FormatterContext;
    let registry = crate::adapters::acp::formatter_registry();
    let ctx = FormatterContext {
        wire_name: running.wire_name.as_str(),
        identity: &running.identity,
        kind: running.tool_kind.as_str(),
        raw_input: running.raw_input.as_ref(),
        adapter: adapter_id,
        content: &running.content,
        started_at: running.started_at,
        completed_at: running.completed_at,
    };
    registry.dispatch(&ctx)
}

fn tool_identity(adapter_id: &str, title: &str, raw_input: Option<&serde_json::Value>) -> ToolIdentity {
    if let Some(identity) = ToolIdentity::from_mcp_name(title) {
        return identity;
    }

    if adapter_id == "acp-codex" {
        if let Some((server, leaf)) = title.strip_prefix("Tool: ").and_then(|body| body.split_once('/')) {
            if !server.trim().is_empty() && !leaf.trim().is_empty() {
                return ToolIdentity::Mcp {
                    server: server.trim().to_string(),
                    leaf: leaf.trim().to_string(),
                };
            }
        }
        if let Some(raw) = raw_input {
            let server = raw
                .get("server")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty());
            let leaf = raw
                .get("tool")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty());
            if let (Some(server), Some(leaf)) = (server, leaf) {
                return ToolIdentity::Mcp {
                    server: server.trim().to_string(),
                    leaf: leaf.trim().to_string(),
                };
            }
        }
    }

    ToolIdentity::Native
}

/// Wall-clock now in epoch milliseconds. Used by the per-instance
/// notification task to stamp `started_at` / `completed_at` on every
/// running tool call. `SystemTime` over `Instant` so the value is
/// comparable across event boundaries (the UI computes differences
/// off the same scale via `Date.now()`).
fn now_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Strip leading ATX-style markdown headers (`#`, `##`, `###`, …) and
/// surrounding blank lines from a plan-step's content. Agents
/// occasionally embed a title row inside the first plan entry
/// (`### tasks` / `# plan • 12/13 done` / similar) — the structured
/// plan render in the UI already supplies a chrome header (`PLAN`),
/// so the agent's redundant header reads as a duplicated heading
/// when stacked on top. Stripping it on the daemon side keeps the
/// wire shape clean for every frontend without each having to
/// re-implement the normalization.
///
/// Only consecutive leading lines that match `^\s*#{1,6}\s` are
/// stripped — body content keeps its inline `#` headings (e.g. a
/// step whose body genuinely starts with text and mentions `#1` is
/// not affected). Returns the trimmed content with leading/trailing
/// whitespace tightened.
fn strip_plan_step_header(content: &str) -> String {
    let mut rest = content.trim_start_matches(['\n', '\r']);
    loop {
        // Try to peel one header-shaped line off the front.
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = &rest[..line_end];
        let trimmed = line.trim_start();
        let trimmed_bytes = trimmed.as_bytes();
        let hash_count = trimmed_bytes.iter().take(6).take_while(|b| **b == b'#').count();
        let first_after_hashes = trimmed_bytes.get(hash_count).copied();
        let is_header = hash_count >= 1 && matches!(first_after_hashes, Some(b' ' | b'\t'));
        if !is_header {
            break;
        }
        rest = &rest[line_end..];
        rest = rest.trim_start_matches(['\n', '\r']);
    }
    rest.trim_end().to_string()
}

pub(crate) fn map_session_update(
    update: serde_json::Value,
    tool_calls: &mut ToolCallCache,
    adapter_id: &str,
) -> MappedSessionUpdate {
    use crate::adapters::{
        Attachment, ChecklistStats, PermissionRequestRecord, PlanRecord, PlanStep, PlanStepStatus, ToolCallContentItem,
        ToolCallRecord, ToolCallState, ToolCallUpdateRecord, TranscriptItem,
    };

    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let meta = update.get("_meta").cloned();
    // ACP's `agent_message_chunk` / `agent_thought_chunk` carry an
    // optional `messageId` (camelCase on the wire) when the vendor
    // ships under the `unstable_message_id` feature. Claude / Codex
    // both emit fresh ids per content block — a tool call between
    // two text chunks produces two distinct ids. Threaded through
    // to the dispatcher so `TurnState` can force a paragraph break
    // across the boundary. Only meaningful on chunk variants; we
    // capture for every update and let the dispatcher ignore it
    // for non-chunk kinds.
    let message_id = update.get("messageId").and_then(|v| v.as_str()).map(str::to_string);

    fn chunk_text(update: &serde_json::Value) -> String {
        // Most ACP-spec'd chunks ride as `{ content: { type: "text",
        // text: "..." } }`. claude-agent-acp's `agent_thought_chunk`
        // is the outlier: the upstream Anthropic API surfaces
        // reasoning as `{ type: "thinking", thinking: "..." }`
        // content blocks, and the agent forwards the block shape
        // through unchanged (see anthropics/claude-agent-sdk-typescript).
        //
        // Pick the first NON-EMPTY value across the known shapes —
        // returning early on an empty `.text` would mask a populated
        // `.thinking` sibling if a vendor ships both in the same
        // delta. Walking the content array + concatenating handles
        // multi-block thought deltas.
        fn pick_nonempty(v: Option<&serde_json::Value>) -> Option<String> {
            v.and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
        }
        let content = match update.get("content") {
            Some(v) => v,
            None => return String::new(),
        };
        if let Some(s) = pick_nonempty(content.get("thinking")) {
            return s;
        }
        if let Some(s) = pick_nonempty(content.get("text")) {
            return s;
        }
        if let Some(arr) = content.as_array() {
            let mut out = String::new();
            for block in arr {
                if let Some(s) = pick_nonempty(block.get("thinking")) {
                    out.push_str(&s);
                } else if let Some(s) = pick_nonempty(block.get("text")) {
                    out.push_str(&s);
                }
            }
            return out;
        }
        String::new()
    }

    /// Project a single agent-emitted `ContentBlock` (the chunk's
    /// `content` slot) into either `AgentText` or `AgentAttachment`.
    /// Mirrors the user-side encoder in `build_prompt_blocks` —
    /// dispatches purely on `type` and (for `resource`) the inner
    /// resource discriminator. Unknown shapes fall through to
    /// `Unknown` so the UI logs the gap without bricking the session.
    fn project_agent_chunk_content(content: &serde_json::Value) -> TranscriptItem {
        let block_type = content.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        match block_type {
            "text" => TranscriptItem::AgentText {
                text: content.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            },
            "image" | "audio" => {
                let mime = content
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        if block_type == "image" {
                            "image/png".to_string()
                        } else {
                            "audio/wav".to_string()
                        }
                    });
                let data = content.get("data").and_then(|v| v.as_str()).map(str::to_string);
                // Synthesise a slug from the type + size hash so the UI
                // can dedupe; agents don't supply identifiers for
                // streaming binaries.
                let slug = format!("agent-{block_type}-{}", data.as_deref().map(str::len).unwrap_or(0));
                TranscriptItem::AgentAttachment(Attachment {
                    slug,
                    path: std::path::PathBuf::from(format!("agent-emitted-{block_type}")),
                    body: String::new(),
                    title: content.get("title").and_then(|v| v.as_str()).map(str::to_string),
                    data,
                    mime: Some(mime),
                })
            }
            "resource_link" => {
                let uri = content.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = content
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| uri.clone());
                let mime = content.get("mimeType").and_then(|v| v.as_str()).map(str::to_string);
                TranscriptItem::AgentAttachment(Attachment {
                    slug: format!("agent-link-{name}"),
                    path: std::path::PathBuf::from(uri),
                    body: String::new(),
                    title: content
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(Some(name)),
                    data: None,
                    mime,
                })
            }
            "resource" => {
                // `resource: { uri, text? | blob?, mimeType? }`
                let inner = content.get("resource").cloned().unwrap_or(serde_json::Value::Null);
                let uri = inner.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let mime = inner.get("mimeType").and_then(|v| v.as_str()).map(str::to_string);
                let text = inner.get("text").and_then(|v| v.as_str()).map(str::to_string);
                let blob = inner.get("blob").and_then(|v| v.as_str()).map(str::to_string);
                TranscriptItem::AgentAttachment(Attachment {
                    slug: format!("agent-resource-{uri}"),
                    path: std::path::PathBuf::from(&uri),
                    body: text.unwrap_or_default(),
                    title: Some(uri),
                    data: blob,
                    mime,
                })
            }
            other => TranscriptItem::Unknown {
                wire_kind: format!("agent_message_chunk:{other}"),
                payload: content.clone(),
            },
        }
    }

    fn parse_tool_state(s: Option<&str>) -> Option<ToolCallState> {
        match s? {
            "pending" => Some(ToolCallState::Pending),
            "in_progress" | "running" => Some(ToolCallState::Running),
            "completed" => Some(ToolCallState::Completed),
            "failed" => Some(ToolCallState::Failed),
            _ => None,
        }
    }

    fn parse_content(raw: &serde_json::Value) -> Vec<ToolCallContentItem> {
        raw.as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|piece| {
                        let kind = piece.get("type").and_then(|v| v.as_str())?;
                        match kind {
                            "content" | "text" => Some(ToolCallContentItem::Text {
                                text: piece
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| {
                                        piece
                                            .get("content")
                                            .and_then(|c| c.get("text"))
                                            .and_then(|v| v.as_str())
                                    })
                                    .unwrap_or("")
                                    .to_string(),
                            }),
                            "diff" | "file" => {
                                let path = piece.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let snippet = piece
                                    .get("newText")
                                    .or_else(|| piece.get("snippet"))
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                                Some(ToolCallContentItem::File { path, snippet })
                            }
                            _ => Some(ToolCallContentItem::Json { value: piece.clone() }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    let mapped = match kind.as_str() {
        "user_message_chunk" => MappedUpdate::Transcript(TranscriptItem::UserText {
            text: chunk_text(&update),
        }),
        "agent_message_chunk" => {
            // Per ACP `agent_message_chunk` carries one ContentBlock —
            // text in the common case, image / audio / resource /
            // resource_link in the multimodal case. Project onto the
            // text or attachment variants. UI demuxer routes either.
            let content = update.get("content").cloned().unwrap_or(serde_json::Value::Null);
            MappedUpdate::Transcript(project_agent_chunk_content(&content))
        }
        "agent_thought_chunk" => {
            let text = chunk_text(&update);
            if text.is_empty() {
                tracing::warn!(
                    target: "acp::thought",
                    raw = %update,
                    "agent_thought_chunk: extracted empty text — content shape \
                     not in {{text, thinking, [{{text|thinking}}]}}; UI thinking \
                     card will show no body"
                );
            } else {
                tracing::debug!(
                    target: "acp::thought",
                    text_len = text.len(),
                    "agent_thought_chunk extracted"
                );
            }
            MappedUpdate::Transcript(TranscriptItem::AgentThought { text })
        }
        "session_info_update" => MappedUpdate::SessionInfo {
            title: update.get("title").and_then(|v| v.as_str()).map(str::to_string),
            updated_at: update.get("updatedAt").and_then(|v| v.as_str()).map(str::to_string),
        },
        "current_mode_update" => MappedUpdate::CurrentMode {
            current_mode_id: update
                .get("currentModeId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "usage_update" => {
            let used = update.get("used").and_then(|v| v.as_u64()).unwrap_or(0);
            let size = update.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let cost = update.get("cost").and_then(|c| {
                let amount = c.get("amount").and_then(|v| v.as_f64())?;
                let currency = c.get("currency").and_then(|v| v.as_str())?.to_string();
                Some(UsageCost { amount, currency })
            });
            MappedUpdate::Usage { used, size, cost }
        }
        "config_option_update" => {
            // claude-agent-acp 0.21+ ships the FULL category set on
            // every notification; the UI replaces its cache wholesale
            // rather than merging deltas.
            use crate::adapters::{SessionConfigOptionCategory, SessionConfigOptionValue};
            let categories = update
                .get("configOptions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|cat| {
                            let raw_id = cat.get("id").and_then(|v| v.as_str())?;
                            let id = super::agents::display_config_option_id(adapter_id, raw_id);
                            let name = cat.get("name").and_then(|v| v.as_str())?.to_string();
                            let description = cat.get("description").and_then(|v| v.as_str()).map(str::to_string);
                            let current_value = cat.get("currentValue").and_then(|v| v.as_str()).map(str::to_string);
                            let options: Vec<SessionConfigOptionValue> = cat
                                .get("options")
                                .and_then(|v| v.as_array())
                                .map(|opts| {
                                    opts.iter()
                                        .filter_map(|o| {
                                            let value = o.get("value").and_then(|v| v.as_str())?.to_string();
                                            let name = o.get("name").and_then(|v| v.as_str())?.to_string();
                                            let description =
                                                o.get("description").and_then(|v| v.as_str()).map(str::to_string);
                                            Some(SessionConfigOptionValue {
                                                value,
                                                name,
                                                description,
                                            })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            Some(SessionConfigOptionCategory {
                                id,
                                name,
                                description,
                                current_value,
                                options,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            MappedUpdate::ConfigOptions { categories }
        }
        "available_commands_update" => {
            use crate::completion::source::commands::CommandSummary;
            let commands = update
                .get("availableCommands")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| {
                            let name = entry.get("name").and_then(|v| v.as_str())?.to_string();
                            let description = entry.get("description").and_then(|v| v.as_str()).map(str::to_string);
                            Some(CommandSummary { name, description })
                        })
                        .collect()
                })
                .unwrap_or_default();
            MappedUpdate::AvailableCommands { commands }
        }
        "tool_call" => {
            let id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_kind = update
                .get("kind")
                .and_then(|v| v.as_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "acp".to_string());
            let title = update.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let state =
                parse_tool_state(update.get("status").and_then(|v| v.as_str())).unwrap_or(ToolCallState::Pending);
            let raw_input = update.get("rawInput").cloned();
            let content = update.get("content").map(parse_content).unwrap_or_default();
            // Surface the full tool-call payload at debug. The
            // dedicated `acp::tool_call` target lets developers crank
            // the level for this subsystem alone via
            // `RUST_LOG=acp::tool_call=debug,info` when they need to
            // see a wire shape for a new formatter without drowning
            // in the rest of the daemon's debug stream.
            tracing::debug!(
                target: "acp::tool_call",
                id = %id,
                kind = %tool_kind,
                title = %title,
                state = ?state,
                raw_input = ?raw_input,
                content_blocks = content.len(),
                "acp::instance: tool_call payload (formatter input)"
            );
            // Update the per-id running cache so future updates
            // re-format against merged state. `started_at` captures
            // wall-clock at this first observation; `completed_at`
            // stays None until a state transition lands below.
            let running = RunningToolCall {
                wire_name: title.clone(),
                identity: tool_identity(adapter_id, &title, raw_input.as_ref()),
                tool_kind: tool_kind.clone(),
                raw_input: raw_input.clone(),
                content: update
                    .get("content")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                started_at: now_epoch_ms(),
                completed_at: None,
            };
            let formatted = format_running(adapter_id, &running);
            let identity = running.identity.clone();
            let started_at_ms = running.started_at;
            let completed_at_ms = running.completed_at;
            // Some agents emit the initial `tool_call` already in a
            // terminal state (fast tools that complete before they ever
            // stream a running chunk). Skip the cache insert in that
            // case so we don't leak entries the merge logic will never
            // see again.
            let is_terminal = matches!(state, ToolCallState::Completed | ToolCallState::Failed);
            if !is_terminal {
                tool_calls.insert(id.clone(), running);
            }
            MappedUpdate::Transcript(TranscriptItem::ToolCall(ToolCallRecord {
                id,
                identity,
                tool_kind,
                title,
                state,
                raw_input,
                content,
                formatted,
                started_at_ms,
                completed_at_ms,
            }))
        }
        "tool_call_update" => {
            let id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_kind = update.get("kind").and_then(|v| v.as_str()).map(str::to_ascii_lowercase);
            let title = update.get("title").and_then(|v| v.as_str()).map(str::to_string);
            let state = parse_tool_state(update.get("status").and_then(|v| v.as_str()));
            let raw_input = update.get("rawInput").cloned();
            let content = update.get("content").map(parse_content).unwrap_or_default();
            tracing::debug!(
                target: "acp::tool_call",
                id = %id,
                kind = ?tool_kind,
                title = ?title,
                state = ?state,
                raw_input = ?raw_input,
                content_blocks = content.len(),
                "acp::instance: tool_call_update payload (formatter input)"
            );
            // Merge delta into the running cache — every Some-value
            // patches; raw `content` from the wire appends. `wire_name`
            // is captured once on the initial `tool_call` and stays
            // frozen: the first observation is the tool's identity
            // ("Bash", "Read", "mcp__server__leaf"), later updates
            // re-purpose `title` as a verbose human display string
            // ("bash · ls /tmp") that would defeat per-(adapter,
            // wire_name) formatter dispatch if we let it through.
            let running = tool_calls.entry(id.clone()).or_default();
            // First-observation guard: a `tool_call_update` may land
            // before a `tool_call` did (some agents skip the latter
            // for fast tools). Stamp started_at on the cache miss
            // path so duration math has a left-edge to compare.
            if running.started_at == 0 {
                running.started_at = now_epoch_ms();
            }
            if running.wire_name.is_empty() {
                if let Some(t) = title.as_deref() {
                    running.wire_name = t.to_string();
                    running.identity = tool_identity(adapter_id, t, raw_input.as_ref());
                }
            }
            if let Some(k) = tool_kind.as_deref() {
                running.tool_kind = k.to_string();
            }
            if let Some(rv) = raw_input.as_ref() {
                running.raw_input = Some(rv.clone());
                if matches!(running.identity, ToolIdentity::Native) {
                    running.identity = tool_identity(adapter_id, running.wire_name.as_str(), Some(rv));
                }
            }
            if let Some(arr) = update.get("content").and_then(|v| v.as_array()) {
                running.content.extend(arr.iter().cloned());
            }

            // Stamp completed_at on the first transition to a terminal
            // state. Subsequent updates with the same state don't
            // re-stamp — duration is "first time we saw it done", not
            // "most-recent observation".
            if running.completed_at.is_none()
                && matches!(state, Some(ToolCallState::Completed) | Some(ToolCallState::Failed))
            {
                running.completed_at = Some(now_epoch_ms());
            }
            let formatted = format_running(adapter_id, running);
            let identity = running.identity.clone();
            let started_at_ms = running.started_at;
            let completed_at_ms = running.completed_at;
            // Drop the cache entry once the tool has reached a terminal
            // state — `formatted` is already on the outgoing record so
            // subscribers have everything they need. Holding onto every
            // running call forever leaks the actor's memory across long
            // sessions where each entry can carry diff blobs / large
            // raw inputs.
            let is_terminal = matches!(state, Some(ToolCallState::Completed) | Some(ToolCallState::Failed));
            if is_terminal {
                tool_calls.remove(&id);
            }
            MappedUpdate::Transcript(TranscriptItem::ToolCallUpdate(ToolCallUpdateRecord {
                id,
                identity: Some(identity),
                tool_kind,
                title,
                state,
                raw_input,
                content,
                formatted,
                started_at_ms,
                completed_at_ms,
            }))
        }
        "plan" => {
            let steps: Vec<PlanStep> = update
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|entry| PlanStep {
                            content: strip_plan_step_header(
                                entry.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                            ),
                            // Unknown wire strings deserialize to None
                            // (tolerant) — agents shipping a future
                            // variant won't crash the mapping.
                            priority: entry
                                .get("priority")
                                .and_then(|v| serde_json::from_value(v.clone()).ok()),
                            status: entry.get("status").and_then(|v| serde_json::from_value(v.clone()).ok()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let total = steps.len();
            let done = steps
                .iter()
                .filter(|s| s.status == Some(PlanStepStatus::Completed))
                .count();
            MappedUpdate::Transcript(TranscriptItem::Plan(PlanRecord {
                steps,
                stats: ChecklistStats { done, total },
            }))
        }
        "permission_request" => {
            let request_id = update
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool = update.get("tool").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tool_kind = update.get("kind").and_then(|v| v.as_str()).unwrap_or("acp").to_string();
            let args = update.get("args").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let raw_input = update.get("rawInput").cloned();
            // Raw ACP content array — the formatter consumes the
            // wire-shape `Vec<Value>` directly (same as the live
            // `PermissionRequested` event). UI-side parsing into
            // `ToolCallContentItem` is a different concern.
            let raw_content: Vec<serde_json::Value> = update
                .get("content")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let options = update
                .get("options")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let identity = tool_identity(adapter_id, &tool, raw_input.as_ref());
            // Same formatter dispatch the live `InstanceEvent::PermissionRequest`
            // emit uses (instance.rs ~2949). `started_at: 0` /
            // `completed_at: None` so formatters that key on
            // `completed_at.is_some()` skip emitting `Stat::Duration`
            // — the row is for an unstarted call, duration is meaningless.
            let formatted = {
                use crate::tools::formatter::registry::FormatterContext;
                let registry = crate::adapters::acp::formatter_registry();
                let ctx = FormatterContext {
                    wire_name: tool.as_str(),
                    identity: &identity,
                    kind: tool_kind.as_str(),
                    raw_input: raw_input.as_ref(),
                    adapter: adapter_id,
                    content: &raw_content,
                    started_at: 0,
                    completed_at: None,
                };
                registry.dispatch(&ctx)
            };
            MappedUpdate::Transcript(TranscriptItem::PermissionRequest(PermissionRequestRecord {
                request_id,
                tool,
                identity,
                tool_kind,
                args,
                raw_input,
                options,
                formatted,
            }))
        }
        _ => MappedUpdate::Transcript(TranscriptItem::Unknown {
            wire_kind: kind,
            payload: update,
        }),
    };
    MappedSessionUpdate {
        mapped,
        meta,
        message_id,
    }
}

/// Build the ACP `Vec<ContentBlock>` payload for one user turn.
///
/// Attachments dispatch onto an ACP `ContentBlock` variant purely
/// from MIME type — no per-attachment "is this an image?" flag
/// hardcoded into the type. The caller fills in `data` (base64 for
/// binary content) or `body` (text), tags the MIME, and the encoder
/// picks the right wire shape:
///
/// - `image/*` (with base64 `data`)  → `ContentBlock::Image`
/// - `audio/*` (with base64 `data`)  → `ContentBlock::Audio`
/// - any text-shaped MIME            → `ContentBlock::Resource(TextResourceContents)`
///   (text/markdown, text/plain, application/json, application/xml,
///   application/x-yaml, etc. — anything where the body is meaningful
///   as a UTF-8 string)
/// - everything else (with `data`)   → `ContentBlock::Resource(BlobResourceContents)`
///   (PDFs, archives, binaries — base64-encoded blob the agent can
///   reference by URI)
///
/// Falls back to a text resource when no `data` is present and the
/// MIME isn't image/audio — covers the legacy skill-attachment path
/// where `body` is a markdown string and `mime` is unset.
///
/// Prose text always lands last per the convention documented on
/// `UserTurnInput`: agents read context before instructions.
pub(crate) fn build_prompt_blocks(text: &str, attachments: &[Attachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(attachments.len() + 1);
    for att in attachments {
        blocks.push(attachment_to_block(att));
    }
    blocks.push(ContentBlock::Text(TextContent::new(text.to_owned())));
    blocks
}

/// Project a single attachment onto the matching ACP wire variant
/// based on its MIME type. Pure function — no I/O.
///
/// Skill attachments are not special-cased here. The hydrator
/// (`completion::hydration::skills::HyprpilotTokenHydrator`) builds
/// the markdown hydration blob inline when it resolves the
/// `#{hyprpilot://skills/<slug>}` token; from this point on the
/// attachment is a plain text resource carrying that blob as `body`,
/// projected through the regular `MimeCategory::Text` path below.
fn attachment_to_block(att: &Attachment) -> ContentBlock {
    let mime = att.mime_type();
    match mime_category(&mime) {
        MimeCategory::Image => {
            let mut img = ImageContent::new(att.data.clone().unwrap_or_default(), mime);
            img.uri = Some(att.file_uri());
            ContentBlock::Image(img)
        }
        // ACP's `AudioContent` carries no `uri` field (unlike
        // `ImageContent`), so the file_uri is intentionally dropped.
        MimeCategory::Audio => ContentBlock::Audio(AudioContent::new(att.data.clone().unwrap_or_default(), mime)),
        MimeCategory::Text => {
            let mut tr = TextResourceContents::new(att.body.clone(), att.file_uri());
            tr.mime_type = Some(mime);
            ContentBlock::Resource(EmbeddedResource::new(EmbeddedResourceResource::TextResourceContents(
                tr,
            )))
        }
        MimeCategory::Blob => {
            let mut blob = BlobResourceContents::new(att.data.clone().unwrap_or_default(), att.file_uri());
            blob.mime_type = Some(mime);
            ContentBlock::Resource(EmbeddedResource::new(EmbeddedResourceResource::BlobResourceContents(
                blob,
            )))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MimeCategory {
    Image,
    Audio,
    /// Text-shaped: encodes through `TextResourceContents` using the
    /// attachment's `body` field. Covers `text/*` plus the structured
    /// formats agents commonly reason over (`application/json`,
    /// `application/xml`, `application/x-yaml`, `application/toml`).
    Text,
    /// Catch-all for anything binary that's not image/audio — PDFs,
    /// archives, octets. Encodes through `BlobResourceContents` with
    /// the base64 payload from the attachment's `data` field.
    Blob,
}

fn mime_category(mime: &str) -> MimeCategory {
    if mime.starts_with("image/") {
        return MimeCategory::Image;
    }
    if mime.starts_with("audio/") {
        return MimeCategory::Audio;
    }
    if mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/x-yaml"
        || mime == "application/yaml"
        || mime == "application/toml"
        || mime == "application/x-toml"
        || mime == "application/javascript"
        || mime == "application/typescript"
    {
        return MimeCategory::Text;
    }
    MimeCategory::Blob
}

/// Captured stdio pair for a freshly-spawned agent subprocess.
struct ChildStdio {
    stdin: ChildStdin,
    stdout: ChildStdout,
}

/// Output of `spawn_subprocess`: the child + its stdio + optional
/// first-message-prefix the runtime prepends to the first
/// `session/prompt` for vendors without a launch-time hook.
struct SpawnedAgent {
    child: Child,
    stdio: ChildStdio,
    stderr: ChildStderr,
    first_message_prefix: Option<String>,
}

/// Spawn the configured agent subprocess. `system_prompt`, when set,
/// is routed through the vendor's `inject_system_prompt` hook —
/// either mutating `cmd` pre-spawn or returning text the runtime
/// prepends onto the first `session/prompt`.
fn spawn_subprocess(cfg: &AgentConfig, system_prompt: Option<&str>) -> Result<SpawnedAgent> {
    info!(
        agent = %cfg.id,
        provider = ?cfg.provider,
        cwd = ?cfg.cwd,
        command = ?cfg.command,
        has_system_prompt = system_prompt.is_some(),
        "acp::instance: launching agent subprocess"
    );

    let agent = match_provider_agent(cfg.provider);
    let mut cmd = agent.spawn(cfg);
    // Centralize stderr capture here rather than duplicating across
    // every vendor agent. Vendor SDKs (notably claude-agent-sdk under
    // claude-agent-acp) print noisy cleanup stack traces to stderr on
    // shutdown; piping keeps that out of the parent terminal.
    cmd.stderr(std::process::Stdio::piped());
    let first_message_prefix = match system_prompt {
        Some(prompt) => match agent.inject_system_prompt(&mut cmd, prompt) {
            SystemPromptInjection::Handled => None,
            SystemPromptInjection::FirstMessage(text) => Some(text),
        },
        None => None,
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            error!(agent = %cfg.id, provider = ?cfg.provider, %err, "acp::instance: failed to spawn agent");
            return Err(err)
                .with_context(|| format!("failed to spawn agent '{}' (provider {:?})", cfg.id, cfg.provider));
        }
    };

    let pid = child.id();

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdin not captured — check Stdio::piped()", cfg.id),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdout not captured — check Stdio::piped()", cfg.id),
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => bail!("agent '{}' stderr not captured — check Stdio::piped()", cfg.id),
    };

    info!(
        agent = %cfg.id,
        pid = ?pid,
        first_message_injection = first_message_prefix.is_some(),
        "acp::instance: agent subprocess spawned"
    );

    Ok(SpawnedAgent {
        child,
        stdio: ChildStdio { stdin, stdout },
        stderr,
        first_message_prefix,
    })
}

/// Handle the registry keeps after `AcpInstance::start`. Dropping it
/// cancels the actor (via the `cmd_tx` drop + the actor's select
/// loop observing `None` from the mpsc receiver).
#[derive(Debug)]
pub struct AcpInstance {
    pub key: InstanceKey,
    pub agent_id: String,
    /// `Some` when a `[[profiles]]` entry resolved during ensure,
    /// `None` for bare-agent resolutions (no profile selected).
    pub profile_id: Option<String>,
    /// Per-instance operational mode. Mirrored onto `InstanceInfo`.
    pub mode: Option<String>,
    pub cmd_tx: mpsc::UnboundedSender<InstanceCommand>,
    /// Populated after the first prompt's `session/new` resolves.
    /// `None` while the instance is still bootstrapping.
    pub session_id: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    /// Captain-set addressable name. Distinct from `key` (the
    /// canonical UUID). Mutated via `AdapterRegistry::rename`;
    /// validated as a slug at the rename boundary so it's always
    /// safe to display verbatim. `None` until the captain renames.
    pub name: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Per-instance skills catalogue, built from the active profile's
    /// `skills = [...]` (with profile→global fallback) at spawn time.
    /// The palette / autocomplete / inline-token hydrator all read
    /// from this — switching to another instance flips the visible
    /// skill set without touching any global cache. `reload()` is
    /// driven by the `skills/reload` RPC + the palette's "refresh
    /// skills" entry; both are addressed at the instance level so
    /// captains can re-walk one instance's roots without disturbing
    /// the others.
    pub skills: Arc<crate::skills::SkillsRegistry>,
    /// Per-instance running tool-call state. Lifted from a stack-local
    /// inside the actor's run loop so out-of-actor consumers (snapshot
    /// RPC handlers, transcript mirror) can read the merged in-flight
    /// state without round-tripping through the actor's command
    /// channel. The actor task takes a write lock around every
    /// `map_session_update` call and around the terminal-state
    /// Per-instance running tool-call cache, shared with the actor
    /// task via `RunParams`. Plumbed through the struct so future
    /// out-of-actor consumers (e.g. a tool-call snapshot RPC) can
    /// read it without an actor round-trip. No external readers
    /// today; the actor writes through its `RunParams` clone.
    #[allow(dead_code)]
    pub tool_calls: Arc<tokio::sync::RwLock<ToolCallCache>>,
    /// Per-instance write-through mirror of every emitted
    /// [`InstanceEvent`]. Read by the `Adapter::instance_mirror`
    /// path that powers the snapshot RPC. Writes go through
    /// `mirror::publish` to enforce apply-then-broadcast ordering.
    pub mirror: Arc<crate::adapters::InstanceMirror>,
    /// `--with-config` patch documents folded against the daemon's
    /// base `Config` at spawn time. Empty when the instance was
    /// constructed without overlays. Stored verbatim so
    /// `Adapter::restart` replays them against whatever config the
    /// daemon currently has (so a `daemon/reload` between spawn and
    /// restart picks up the new base while preserving the captain's
    /// overlays).
    pub config_patches: Vec<serde_json::Value>,
    /// Rolling tail of the agent subprocess's stderr — last few lines
    /// captured by the stderr-drain task. Read by the user-facing
    /// error path when `cmd_tx.send` fails ("actor closed before
    /// accepting prompt"), so the captain sees WHY the agent died
    /// without having to grep the daemon log. Bounded at
    /// `STDERR_TAIL_CAP` lines so a long-running agent with chatty
    /// stderr doesn't grow unbounded.
    pub stderr_tail: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    /// Absolute cwd this instance spawned in (NOT display-formatted —
    /// no `~` collapse). Computed once at `start()` time from
    /// `resolved.agent.cwd` (with `std::env::current_dir()` fallback)
    /// via `normalize_cwd` only, so `InstanceInfo` /
    /// `InstanceListEntry` can read it synchronously off the struct.
    /// The absolute form is load-bearing for client-side filters —
    /// hyprpilot.nvim's instances palette compares against
    /// `vim.fn.getcwd()` which is always absolute. The header
    /// chrome's display-formatted cwd reads from `MetaSnapshot.cwd`
    /// via `useSessionInfo` instead — different surface, different
    /// shape.
    pub cwd: String,
}

impl AcpInstance {
    pub async fn current_session_id(&self) -> Option<String> {
        self.session_id.read().await.as_ref().map(|id| id.0.to_string())
    }

    /// Snapshot the rolling stderr tail (last few lines). Returns an
    /// empty Vec when no stderr has been captured or when the mutex
    /// is poisoned (lock poisoning means the stderr-drain task
    /// panicked — fall through cleanly so the user-facing error
    /// path doesn't compound the failure). Caller joins the lines
    /// for display.
    #[must_use]
    pub fn recent_stderr(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Snapshot the captain-set name. Returns `None` when the captain
    /// hasn't renamed the instance yet (the auto-mint has no name).
    pub async fn current_name(&self) -> Option<String> {
        self.name.read().await.clone()
    }

    /// Overwrite the captain-set name. Caller (`AdapterRegistry::rename`)
    /// is responsible for validation + uniqueness; this is a raw write.
    pub async fn set_name(&self, name: Option<String>) {
        *self.name.write().await = name;
    }

    /// Send the actor a `SetMode` command and await the agent's
    /// `session/set_mode` reply. Surfacing errors as `String` matches
    /// the rest of the actor's reply shape (mapped into `RpcError`
    /// upstream by `AcpAdapter::set_session_mode`).
    pub async fn set_mode(&self, mode_id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::SetMode { mode_id, reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())?
    }

    /// Send the actor a `SetModel` command and await the agent's
    /// `session/set_model` reply. ACP gates this method behind
    /// `unstable_session_model`; our crate enables it via the
    /// `["unstable"]` umbrella feature on `agent-client-protocol`.
    pub async fn set_model(&self, model_id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::SetModel { model_id, reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())?
    }

    /// Send the actor a `SetConfigOption` command — ACP's
    /// `session/set_config_option`. Generic catch-all for vendor
    /// extension knobs the agent advertises in
    /// `NewSessionResponse.configOptions`; spec-reserved categories
    /// (`mode` / `model` / `thought_level`) MAY also flow through here
    /// when the agent surfaces them on configOptions instead of the
    /// dedicated wire methods. Captain picks one of the offered values
    /// from the palette; the actor sends the request, captures the
    /// response's full `configOptions` array, and refreshes the
    /// per-instance meta cache so the next palette open sees the new
    /// state.
    pub async fn set_config_option(&self, config_id: String, value: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(InstanceCommand::SetConfigOption {
                config_id,
                value,
                reply,
            })
            .is_err()
        {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())?
    }

    /// Snapshot the actor's per-instance metadata (cwd, current
    /// mode/model id, advertised lists). Powers the `instance_meta`
    /// Tauri command — every palette open routes through here so
    /// the picker reads the daemon's authoritative cache, not a
    /// UI-side mirror of past `acp:instance-meta` events.
    pub async fn meta_snapshot(&self) -> Result<MetaSnapshot, String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::MetaSnapshot { reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())
    }

    /// Test-only stub builder. Constructs an `AcpInstance` carrying
    /// the supplied `mirror` Arc but no live actor — `cmd_tx`'s
    /// receiver is dropped immediately so any command send fails
    /// closed. Used by snapshot RPC tests that only read mirror
    /// state through [`crate::adapters::Adapter::instance_mirror`].
    #[cfg(test)]
    #[must_use]
    pub fn stub_for_tests(key: InstanceKey, mirror: Arc<crate::adapters::InstanceMirror>) -> Self {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel::<InstanceCommand>();
        // Drop the receiver so any subsequent send fails. Tests that
        // need a working actor go through `start` instead.
        Self {
            key,
            agent_id: "test-agent".into(),
            profile_id: None,
            mode: None,
            cmd_tx,
            session_id: Arc::new(tokio::sync::RwLock::new(None)),
            name: Arc::new(tokio::sync::RwLock::new(None)),
            skills: Arc::new(crate::skills::SkillsRegistry::new(Vec::new())),
            tool_calls: Arc::new(tokio::sync::RwLock::new(ToolCallCache::default())),
            mirror,
            config_patches: Vec::new(),
            stderr_tail: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            cwd: "/tmp/test-stub".into(),
        }
    }

    /// Spawn the per-instance actor task and return its handle.
    /// Symmetric with [`Self::shutdown`]: the registry calls `start`
    /// to bring an instance up and `shutdown` to tear it down.
    ///
    /// `bootstrap` picks between `session/new` (`Fresh`),
    /// `session/load` (`Resume`), or neither (`ListOnly`). The actor
    /// publishes lifecycle + transcript + permission events onto
    /// `events_tx`.
    ///
    /// `mcps_override` is the per-instance MCP enabled-list override;
    /// `None` falls back to `profile.mcps`. `Some(vec![])` is the
    /// explicit "no MCPs" override.
    #[must_use]
    pub fn start(params: StartParams) -> Self {
        let StartParams {
            resolved,
            key,
            profile_id,
            events_tx,
            bootstrap,
            permissions,
            mcps,
            skills,
            commands_cache,
            config_patches,
        } = params;
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<InstanceCommand>();
        let initial = match &bootstrap {
            Bootstrap::Resume(id) => Some(SessionId::new(id.clone())),
            Bootstrap::Fresh | Bootstrap::ListOnly => None,
        };
        let session_id = Arc::new(tokio::sync::RwLock::new(initial));
        let tool_calls = Arc::new(tokio::sync::RwLock::new(ToolCallCache::default()));
        let mirror = Arc::new(crate::adapters::InstanceMirror::new());
        let mode = resolved.mode.clone();
        let instance_id = key.as_string();

        // Compute the absolute cwd here so `InstanceInfo` /
        // `InstanceListEntry` can read it synchronously off the
        // struct. Deliberately NOT display-formatted (no `~`
        // collapse): consumers like hyprpilot.nvim's palette filter
        // compare byte-for-byte against `vim.fn.getcwd()` which
        // returns an absolute path. The header chrome reads
        // `MetaSnapshot.cwd` (display form) via `useSessionInfo`,
        // not this field — the two surfaces serve different
        // purposes.
        let cwd_absolute = resolved
            .agent
            .cwd
            .as_ref()
            .map(|p| crate::tools::path::normalize_cwd(&p.to_string_lossy()))
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "/".into())
            });

        // Mode is a per-instance operational override (e.g.
        // claude-code's `plan` / `edit`). Surface it so UI pickers
        // see it; vendor-specific wire injection lands in the agent
        // impl (today only logged here).
        if let Some(m) = &mode {
            tracing::info!(
                agent = %resolved.agent.id,
                instance = %instance_id,
                mode = %m,
                "acp::instance: mode set"
            );
        }

        let cmd_tx_self = cmd_tx.clone();
        let stderr_tail = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::with_capacity(
            STDERR_TAIL_CAP,
        )));
        let instance = AcpInstance {
            key,
            agent_id: resolved.agent.id.clone(),
            profile_id,
            mode,
            cmd_tx,
            session_id: session_id.clone(),
            name: Arc::new(tokio::sync::RwLock::new(None)),
            skills,
            tool_calls: tool_calls.clone(),
            mirror: mirror.clone(),
            config_patches,
            stderr_tail: stderr_tail.clone(),
            cwd: cwd_absolute,
        };

        tokio::spawn(run(RunParams {
            stderr_tail,
            resolved,
            instance_id,
            cmd_rx,
            cmd_tx_self,
            events_tx,
            session_id_slot: session_id,
            tool_calls,
            mirror,
            bootstrap,
            permissions,
            mcps,
            commands_cache,
        }));

        instance
    }
}

#[async_trait]
impl InstanceActor for AcpInstance {
    fn info(&self) -> InstanceInfo {
        // session_id read is sync-safe on the RwLock's try_read, but
        // we don't need it here — the registry's list path populates
        // it async via `current_session_id` when it matters. Keep
        // this call sync so the generic registry can assemble the
        // snapshot without an async fn on the trait.
        let session_id = self
            .session_id
            .try_read()
            .ok()
            .and_then(|s| s.as_ref().map(|id| id.0.to_string()));
        // Same try_read pattern as session_id — the rename path uses
        // a write lock briefly, every other reader (`info()`, ctl
        // listing, UI labels) is read-only. `try_read` succeeds in
        // the steady state; on lock contention we read `None` for
        // this snapshot tick and the next event sync corrects it.
        let name = self.name.try_read().ok().and_then(|n| n.clone());
        InstanceInfo {
            id: self.key.as_string(),
            name,
            agent_id: self.agent_id.clone(),
            profile_id: self.profile_id.clone(),
            session_id,
            mode: self.mode.clone(),
            cwd: self.cwd.clone(),
        }
    }

    async fn name(&self) -> Option<String> {
        self.current_name().await
    }

    async fn set_name(&self, name: Option<String>) {
        AcpInstance::set_name(self, name).await
    }

    async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::Shutdown { reply: tx }).is_err() {
            return;
        }
        let _ = tokio::time::timeout(SHUTDOWN_ACK_TIMEOUT, rx).await;
    }
}

/// The long-lived actor body. Owns the ACP `ConnectionTo<Agent>`,
/// Params for `AcpInstance::start`. Captures everything an instance
/// actor needs at construction time; same shape funnels into `run`
/// via `RunParams`.
pub struct StartParams {
    pub resolved: ResolvedInstance,
    pub key: InstanceKey,
    pub profile_id: Option<String>,
    pub events_tx: broadcast::Sender<InstanceEvent>,
    pub bootstrap: Bootstrap,
    pub permissions: Arc<dyn PermissionController>,
    pub mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    /// Per-instance skills catalogue. Always set — the adapter builds
    /// either the profile's `skills = [...]` view or the global
    /// fallback. Empty registries are valid; the captain may have
    /// `skills = []`. See `AcpInstance.skills` for the consumer
    /// contract.
    pub skills: Arc<crate::skills::SkillsRegistry>,
    pub commands_cache: Option<crate::completion::source::commands::CommandsCache>,
    /// `--with-config` patch documents to store on the instance for
    /// restart replay. Default to empty when not overlaying.
    pub config_patches: Vec<serde_json::Value>,
}

/// Internal `run` actor params — superset of `StartParams` with the
/// command-channel receiver + the shared session-id slot the registry
/// also reads.
struct RunParams {
    resolved: ResolvedInstance,
    instance_id: String,
    cmd_rx: mpsc::UnboundedReceiver<InstanceCommand>,
    /// Self-clone of the actor's command sender. Captured into the
    /// actor so `QueueDispatch` can re-enqueue a `Prompt` onto its
    /// own mailbox — that re-routes through the existing system-
    /// prompt-injection + TurnGuard + attachments machinery without
    /// duplicating any of it inline.
    cmd_tx_self: mpsc::UnboundedSender<InstanceCommand>,
    events_tx: broadcast::Sender<InstanceEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    /// Shared running tool-call state — same `Arc` the public
    /// `AcpInstance.tool_calls` exposes. The actor writes through it
    /// at every `map_session_update`; out-of-actor readers (snapshot
    /// RPC handlers) take a read lock.
    tool_calls: Arc<tokio::sync::RwLock<ToolCallCache>>,
    /// Shared write-through mirror — same `Arc` the public
    /// `AcpInstance.mirror` exposes. The actor calls `mirror.apply(...)`
    /// alongside every `events_tx.send(...)` so out-of-actor consumers
    /// (snapshot RPC handlers) read consistent state without round-
    /// tripping through the actor's command channel.
    mirror: Arc<crate::adapters::InstanceMirror>,
    bootstrap: Bootstrap,
    permissions: Arc<dyn PermissionController>,
    mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    commands_cache: Option<crate::completion::source::commands::CommandsCache>,
    /// Rolling stderr tail — shared with the `AcpInstance` handle so
    /// out-of-actor consumers (the submit_prompt / list_sessions error
    /// paths) can read the last few lines when the agent dies before
    /// the actor accepts a command.
    stderr_tail: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
}

/// Cap on stderr lines kept in the rolling tail. 20 lines is plenty
/// of context for the bunx-cache-miss / agent-startup-failure case
/// without growing the per-instance heap when long-lived agents stream
/// chatty stderr.
const STDERR_TAIL_CAP: usize = 20;

/// the child process, the dispatch loop. Spawned by
/// [`AcpInstance::start`].
/// Broadcast the current per-instance queue. The local `VecDeque` is
/// cloned into a `Vec` (the wire shape); `publish` writes through the
/// mirror BEFORE the broadcast so a snapshot reader observing the
/// event already sees the post-change state.
async fn publish_queue_changed(
    mirror: &Arc<crate::adapters::InstanceMirror>,
    events_tx: &broadcast::Sender<InstanceEvent>,
    agent_id: &str,
    instance_id: &str,
    queue: &std::collections::VecDeque<crate::adapters::queue::QueueItem>,
) {
    let event = InstanceEvent::QueueChanged {
        agent_id: agent_id.to_string(),
        instance_id: instance_id.to_string(),
        items: queue.iter().cloned().collect(),
    };
    publish(mirror, events_tx, event).await;
}

async fn run(params: RunParams) {
    let RunParams {
        resolved,
        instance_id,
        mut cmd_rx,
        cmd_tx_self,
        events_tx,
        session_id_slot,
        tool_calls,
        mirror,
        bootstrap,
        permissions,
        mcps,
        commands_cache,
        stderr_tail,
    } = params;
    let agent_id = resolved.agent.id.clone();
    let starting_event = InstanceEvent::State {
        agent_id: agent_id.clone(),
        instance_id: instance_id.clone(),
        session_id: None,
        state: InstanceState::Starting,
    };
    publish(&mirror, &events_tx, starting_event).await;

    let cfg = {
        let mut cfg = resolved.agent.clone();
        cfg.model = resolved.model.clone();
        cfg
    };
    // Filter the per-entry system_prompt list against the bootstrap
    // variant — entries whose `inject.on_create` (Fresh) or
    // `inject.on_update` (Resume) is true survive and concatenate
    // into the spawn prompt. Skipped entries leave nothing on the
    // wire.
    let prompt_for_spawn = resolved.system_prompt_for(&bootstrap);
    let resolved_mode = resolved.mode.clone();
    let resolved_profile_id = resolved.profile_id.clone();

    // Emit the captain-facing "system prompt attached" banner event
    // when at least one entry actually injected. `files` is the
    // resolved per-bootstrap subset — captains see WHICH files rode
    // along, not just that something did.
    if prompt_for_spawn.is_some() {
        let files = resolved
            .system_prompt_files_for(&bootstrap)
            .into_iter()
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>();
        let event = InstanceEvent::SystemPromptInjected {
            agent_id: agent_id.clone(),
            instance_id: instance_id.clone(),
            files,
        };
        publish(&mirror, &events_tx, event).await;
    }

    let (mut child, stdio, stderr, mut first_message_prefix) = match spawn_subprocess(&cfg, prompt_for_spawn.as_deref())
    {
        Ok(spawned) => (
            spawned.child,
            spawned.stdio,
            spawned.stderr,
            spawned.first_message_prefix,
        ),
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::instance: spawn failed");
            let event = InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            };
            publish(&mirror, &events_tx, event).await;
            return;
        }
    };

    // Drain the subprocess's stderr into tracing so vendor-SDK cleanup
    // noise lands in our rolling log file instead of the parent
    // terminal. Each line goes through at `info!` with an `agent_stderr`
    // target so users can filter via
    // `RUST_LOG=hyprpilot=info,agent_stderr=warn`. Task ends on stream
    // close (child exit).
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let agent_for_stderr = agent_id.clone();
        let tail_for_stderr = stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::info!(target: "agent_stderr", agent = %agent_for_stderr, "{line}");
                        // Also push to the rolling tail so the
                        // user-facing "actor closed before accepting
                        // prompt" path can surface WHY. Lock briefly,
                        // append, evict from the front when over the
                        // cap. Mutex (not RwLock) — this is the only
                        // writer, contention is the read on a
                        // submit-failure path.
                        if let Ok(mut buf) = tail_for_stderr.lock() {
                            if buf.len() == STDERR_TAIL_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(line);
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stderr",
                            agent = %agent_for_stderr,
                            %err,
                            "stderr read error"
                        );
                        break;
                    }
                }
            }
        });
    }

    // Tee stdout → tracing + ACP transport. Stdout IS the ACP wire
    // channel so we can't just redirect it; we read each line, emit
    // it at `trace!` target `agent_stdout`, then forward the original
    // bytes into a duplex pipe the transport reads from. Filter in
    // with `RUST_LOG=agent_stdout=trace`; noisy (every JSON-RPC
    // frame) so `trace` is deliberately opt-in.
    let transport_stdout = {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (mut tee_writer, tee_reader) = tokio::io::duplex(64 * 1024);
        let agent_for_stdout = agent_id.clone();
        let child_stdout = stdio.stdout;
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\n', '\r']);
                        if !trimmed.is_empty() {
                            tracing::trace!(target: "agent_stdout", agent = %agent_for_stdout, "{trimmed}");
                        }
                        if let Err(err) = tee_writer.write_all(line.as_bytes()).await {
                            tracing::warn!(
                                target: "agent_stdout",
                                agent = %agent_for_stdout,
                                %err,
                                "tee forward failed"
                            );
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stdout",
                            agent = %agent_for_stdout,
                            %err,
                            "stdout read error"
                        );
                        break;
                    }
                }
            }
        });
        tee_reader
    };

    let (client_events_tx, mut client_events_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let sandbox_root: PathBuf = cfg
        .cwd
        .clone()
        .map(|p| crate::tools::path::normalize_cwd(&p.to_string_lossy()).into())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
    let client = match AcpClient::with_instance_id(
        client_events_tx,
        sandbox_root,
        permissions.clone(),
        mcps.clone(),
        Some(instance_id.clone()),
    ) {
        Ok(c) => {
            let agent = match_provider_agent(cfg.provider);
            c.with_permission_tool_identity(Arc::new(move |update| agent.permission_tool_identity(update)))
        }
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::instance: sandbox init failed");
            let event = InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            };
            publish(&mirror, &events_tx, event).await;
            return;
        }
    };

    let transport = ByteStreams::new(stdio.stdin.compat_write(), transport_stdout.compat());

    let events_tx_notif = events_tx.clone();
    let mirror_notif = mirror.clone();
    let agent_id_notif = agent_id.clone();
    let instance_id_notif = instance_id.clone();
    let session_id_forward = session_id_slot.clone();
    // Per-instance running tool-call state — feeds the formatter on
    // every `tool_call_update` so the snapshot reflects merged state
    // (not just the delta). Shared with `AcpInstance.tool_calls` so
    // out-of-actor consumers (snapshot RPC handlers, transcript
    // mirror) can read the merged in-flight cache without round-
    // tripping through the actor's command channel. The actor takes
    // a write lock around every `map_session_update` call (which also
    // owns the terminal-state eviction). The `Arc` is owned by the
    // `AcpInstance`; this binding is just the actor's handle.
    let tool_call_cache = tool_calls;
    // Adapter id for per-vendor formatter override dispatch — the
    // `[[agents]] provider` string (acp-claude-code / acp-codex /
    // acp-opencode / acp).
    let provider_id_for_fmt: String = match cfg.provider {
        crate::config::AgentProvider::AcpClaudeCode => "acp-claude-code",
        crate::config::AgentProvider::AcpCodex => "acp-codex",
        crate::config::AgentProvider::AcpOpenCode => "acp-opencode",
        crate::config::AgentProvider::Acp => "acp",
    }
    .to_string();
    // Single-lock turn-id state under one RwLock. `TurnState` exposes
    // typed methods (`open` / `take_*` / `close_if_current`) so the
    // dispatcher can stamp events with the current turn id without
    // racing across writers.
    //
    // A turn is a turn — origin is just provenance. `prompt_in_flight`
    // flips `true` inside `TurnGuard::new` (post-`open`) and back
    // `false` on `complete`/`Drop`, so the queue-strip knows whether
    // captain work is on the wire. Out-of-prompt activity (session/
    // load replay, role-transition splits, push notifications) calls
    // `open` and leaves the flag `false`. `take_current` (cancel) and
    // `take_if_unguarded` (captain submit / role-split / load-resolve)
    // clear the slot.
    let turn_state: SharedTurnState = Arc::new(tokio::sync::RwLock::new(TurnState::default()));

    // ── Per-instance queue ───────────────────────────────────────
    //
    // FIFO of captain-staged prompts the actor serves under
    // `queue/*` RPC + Tauri `queue_*` commands. Single-mailbox
    // ordering: every mutation lands here in arrival order, so
    // `enqueued_seq` ties never happen. Local to the actor — the
    // mirror caches a clone for snapshot reads (see `queue_snapshot`)
    // and the broadcast carries the full list on every change so
    // every connected client (Vue desktop, mobile WS, hyprpilot-nvim)
    // reconciles by replacement.
    let mut queue: std::collections::VecDeque<crate::adapters::queue::QueueItem> = std::collections::VecDeque::new();
    let mut queue_next_seq: u64 = 0;

    // Bridge live terminal output → InstanceEvent::Terminal. The ACP
    // `terminal/output` request remains a polled snapshot path
    // (agent-side); the UI consumes this push stream so it never
    // re-polls.
    {
        let mut rx = client.subscribe_terminals();
        let events_tx = events_tx.clone();
        let mirror = mirror.clone();
        let agent_id = agent_id.clone();
        let instance_id = instance_id.clone();
        let session_id_slot = session_id_slot.clone();
        let turn_state = turn_state.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        let session_id = match session_id_slot.read().await.clone() {
                            Some(sid) => sid.0.to_string(),
                            None => evt.session_key.clone(),
                        };
                        let turn_id = turn_state.read().await.current().map(str::to_string);
                        let chunk = match evt.kind {
                            TerminalToolEventKind::Output { stream, data } => TerminalChunk::Output {
                                stream: match stream {
                                    TerminalToolStream::Stdout => crate::adapters::TerminalStream::Stdout,
                                    TerminalToolStream::Stderr => crate::adapters::TerminalStream::Stderr,
                                },
                                data,
                            },
                            TerminalToolEventKind::Exit { exit_code, signal } => {
                                TerminalChunk::Exit { exit_code, signal }
                            }
                        };
                        let event = InstanceEvent::Terminal {
                            agent_id: agent_id.clone(),
                            instance_id: instance_id.clone(),
                            session_id,
                            turn_id,
                            terminal_id: evt.terminal_id,
                            chunk,
                        };
                        publish(&mirror, &events_tx, event).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, instance = %instance_id, "acp::instance: terminal-event bridge lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    let dispatch = async move |connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>| {
        debug!(agent = %agent_id_notif, "acp::instance: sending initialize request");
        let init = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                    ClientCapabilities::new()
                        .fs(FileSystemCapabilities::new().read_text_file(true).write_text_file(true))
                        .terminal(true),
                ),
            )
            .block_task()
            .await?;
        // Capability probes — `agent_capabilities.session_capabilities`
        // gates the unstable `session/resume` and `session/close`
        // surfaces (both behind `unstable_session_*` features in the
        // schema crate, both already enabled by the umbrella
        // `unstable` feature on `agent-client-protocol`). Falling
        // through to `false` when the agent doesn't advertise the
        // capability — we silently use the legacy paths
        // (LoadSession / CancelNotification).
        let resume_supported = init.agent_capabilities.session_capabilities.resume.is_some();
        let close_supported = init.agent_capabilities.session_capabilities.close.is_some();
        info!(
            agent = %agent_id_notif,
            protocol = ?init.protocol_version,
            load_session = init.agent_capabilities.load_session,
            resume_session = resume_supported,
            close_session = close_supported,
            "acp::instance: initialized"
        );

        // Normalize the configured cwd at the actor boundary so
        // every downstream sink (the spawn `current_dir`, the
        // `InstanceMeta { cwd }` display, the agent-persisted
        // `session.cwd` byte-compare in the sessions palette filter)
        // agrees on the same absolute path. Without this, a
        // captain-typed `~/proj/foo` lands raw in `cfg.cwd`, the
        // spawn separately re-expands it, and the UI's sessions
        // filter compares the raw `~/...` form against the agent's
        // already-absolute `session.cwd` — yielding the spurious
        // "no sessions" the captain hit when changing cwd.
        let cwd: PathBuf = cfg
            .cwd
            .clone()
            .map(|p| crate::tools::path::normalize_cwd(&p.to_string_lossy()).into())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let load_supported = init.agent_capabilities.load_session;

        // Per-instance metadata snapshot. The daemon emits this on
        // `InstanceEvent::InstanceMeta` because claude-agent-acp doesn't
        // proactively send `SessionInfoUpdate` / `CurrentModeUpdate`
        // notifications — the UI would otherwise never see cwd / mode
        // / model values. Display-formatted (home → `~`) here so
        // every wire consumer renders the same shape — frontends
        // never need to know the captain's `$HOME` to do their
        // own collapse.
        let cwd_str = crate::tools::path::display_cwd(&cwd.to_string_lossy());
        let current_mode_meta: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(resolved_mode.clone()));
        let current_model_meta: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(cfg.model.clone()));
        let available_modes_meta: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModeInfo>>> =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let available_models_meta: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModelInfo>>> =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // Project the per-instance MCP catalog onto ACP's typed
        // `McpServer` Vec for injection at `session/new` /
        // `session/load`. Empty when no files are configured (or all
        // entries failed projection); the agent gets an empty list and
        // runs with whatever it discovers natively.
        let mcp_servers: Vec<agent_client_protocol::schema::McpServer> = match &mcps {
            Some(reg) => reg.to_acp_servers(),
            None => Vec::new(),
        };
        // Resolved MCP count for the header `+N mcps` pill — captured
        // once at the top of the actor body and threaded through every
        // `InstanceMeta` emit. Reads via `list().len()` because the
        // registry's `count()` accessor is `cfg(test)` only.
        let mcps_count = mcps.as_ref().map(|reg| reg.list().len()).unwrap_or(0);

        // Single emitter for every `InstanceMeta` refresh in this
        // actor. Cloned into spawned tasks; reads the four `RwLock`
        // metas atomically per emit.
        let meta_emitter = MetaEmitter {
            agent_id: agent_id_notif.clone(),
            instance_id: instance_id_notif.clone(),
            profile_id: resolved_profile_id.clone(),
            cwd: cwd_str.clone(),
            current_mode: current_mode_meta.clone(),
            current_model: current_model_meta.clone(),
            available_modes: available_modes_meta.clone(),
            available_models: available_models_meta.clone(),
            mcps_count,
            mirror: mirror_notif.clone(),
        };
        if !mcp_servers.is_empty() {
            info!(
                agent = %agent_id_notif,
                count = mcp_servers.len(),
                "acp::instance: injecting mcp servers"
            );
        }

        let session_id: Option<SessionId> = match bootstrap {
            Bootstrap::Fresh => {
                debug!(agent = %agent_id_notif, "acp::instance: sending session/new");
                let mut req = NewSessionRequest::new(cwd.clone());
                req.mcp_servers = mcp_servers.clone();
                let new_session = connection.send_request(req).block_task().await?;
                let sid = new_session.session_id.clone();
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::instance: session/new accepted"
                );
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                // Pull `currentModeId` + `availableModes` off the
                // `NewSessionResponse.modes` field — ACP's only
                // emission of this list (no streaming variant exists).
                if let Some(modes) = &new_session.modes {
                    let advertised: Vec<crate::adapters::SessionModeInfo> = modes
                        .available_modes
                        .iter()
                        .map(|m| crate::adapters::SessionModeInfo {
                            id: m.id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_modes_meta.write().await = advertised;
                    *current_mode_meta.write().await = Some(modes.current_mode_id.0.to_string());
                }
                // Same shape as modes, gated by ACP's `unstable_session_model`
                // feature (our crate enables `["unstable"]`). claude-agent-acp
                // populates this with the agent's advertised model list +
                // current selection; without reading it here the picker's
                // `availableModels` stays empty even though the actor knows
                // how to flip via `SetSessionModelRequest`.
                // Captain-configured mode (`profile.mode`) wins when the
                // agent advertised it. Mirrors the model branch below.
                // Without this, the session boots in the agent's default
                // mode (`default` for claude-code) regardless of profile
                // setting — captain has to manually flip via the mode
                // picker on every spawn.
                if let Some(modes) = &new_session.modes {
                    if let Some(want) = resolved_mode.as_deref() {
                        let current = modes.current_mode_id.0.to_string();
                        let advertised = modes.available_modes.iter().any(|m| m.id.0.as_ref() == want);
                        if advertised && want != current {
                            tracing::info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                session = %sid,
                                from = %current,
                                to = %want,
                                "acp::instance: applying configured mode via session/set_mode"
                            );
                            let req = SetSessionModeRequest::new(
                                sid.clone(),
                                SessionModeId::from(std::sync::Arc::<str>::from(want)),
                            );
                            match connection.send_request(req).block_task().await {
                                Ok(_) => {
                                    *current_mode_meta.write().await = Some(want.to_string());
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        agent = %agent_id_notif,
                                        session = %sid,
                                        target_mode = %want,
                                        %err,
                                        "acp::instance: session/set_mode failed at spawn — keeping agent default"
                                    );
                                }
                            }
                        }
                    }
                }
                if let Some(models) = &new_session.models {
                    let advertised: Vec<crate::adapters::SessionModelInfo> = models
                        .available_models
                        .iter()
                        .map(|m| crate::adapters::SessionModelInfo {
                            id: m.model_id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    let current = models.current_model_id.0.to_string();

                    *available_models_meta.write().await = advertised.clone();
                    *current_model_meta.write().await = Some(current.clone());

                    // Captain-configured model wins when (a) it's set, (b) the
                    // agent advertised an available_models list (so we know
                    // session/set_model is supported), and (c) it differs from
                    // the agent's default selection. Spawn-time env-var
                    // injection (claude-code's `ANTHROPIC_MODEL`,
                    // opencode's `OPENCODE_MODEL`) is best-effort — opencode
                    // in particular often resolves to its own default unless
                    // a config file backs the env, and stale parent-process
                    // env can ride through silently. set_model after
                    // session/new is the canonical lever.
                    if let Some(want) = cfg.model.as_deref() {
                        if want != current && advertised.iter().any(|m| m.id == want) {
                            tracing::info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                session = %sid,
                                from = %current,
                                to = %want,
                                "acp::instance: applying configured model via session/set_model"
                            );
                            let req = SetSessionModelRequest::new(
                                sid.clone(),
                                ModelId::from(std::sync::Arc::<str>::from(want)),
                            );
                            match connection.send_request(req).block_task().await {
                                Ok(_) => {
                                    *current_model_meta.write().await = Some(want.to_string());
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        agent = %agent_id_notif,
                                        session = %sid,
                                        target_model = %want,
                                        %err,
                                        "acp::instance: session/set_model failed at spawn — keeping agent default"
                                    );
                                }
                            }
                        }
                    }
                }
                let event = InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                };
                mirror_notif.apply(&event).await;
                let _ = events_tx_notif.send(event);
                meta_emitter.emit(&events_tx_notif, Some(sid.0.to_string())).await;
                Some(sid)
            }
            Bootstrap::Resume(sid) => {
                let sid = SessionId::new(sid);
                // Prefer `session/load` when advertised — it's the
                // method that actually replays prior history as
                // `session/update` notifications. claude-agent-acp
                // advertises `session/resume` too but resume returns
                // success without re-streaming the transcript, so
                // restored sessions render empty. Fall back to resume
                // only for vendors that ship resume but not load.
                if !resume_supported && !load_supported {
                    warn!(
                        agent = %agent_id_notif,
                        "acp::instance: neither session/resume nor session/load advertised by agent"
                    );
                    let event = InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    };
                    mirror_notif.apply(&event).await;
                    let _ = events_tx_notif.send(event);
                    return Err(
                        agent_client_protocol::Error::method_not_found().data(serde_json::json!({
                            "reason": format!("{}: neither session/resume nor session/load supported", agent_id_notif),
                        })),
                    );
                }
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                // Read mode + model state off whichever response we
                // got. Resume + Load share the same `modes` / `models`
                // shape; collapse both branches via a tiny tuple to
                // avoid duplicating the projection logic.
                let (modes_state, models_state) = if load_supported {
                    debug!(agent = %agent_id_notif, session = %sid, "acp::instance: sending session/load");
                    let mut load_req = LoadSessionRequest::new(sid.clone(), cwd.clone());
                    load_req.mcp_servers = mcp_servers.clone();
                    let load_resp = match connection.send_request(load_req).block_task().await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!(agent = %agent_id_notif, %err, "acp::instance: session/load failed");
                            let event = InstanceEvent::State {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: Some(sid.0.to_string()),
                                state: InstanceState::Error,
                            };
                            mirror_notif.apply(&event).await;
                            let _ = events_tx_notif.send(event);
                            return Err(err);
                        }
                    };
                    info!(
                        agent = %agent_id_notif,
                        instance = %instance_id_notif,
                        session = %sid,
                        "acp::instance: session/load accepted"
                    );
                    (load_resp.modes, load_resp.models)
                } else {
                    use agent_client_protocol::schema::ResumeSessionRequest;
                    debug!(agent = %agent_id_notif, session = %sid, "acp::instance: sending session/resume");
                    let mut req = ResumeSessionRequest::new(sid.clone(), cwd.clone());
                    req.mcp_servers = mcp_servers.clone();
                    let resp = match connection.send_request(req).block_task().await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!(agent = %agent_id_notif, %err, "acp::instance: session/resume failed");
                            let event = InstanceEvent::State {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: Some(sid.0.to_string()),
                                state: InstanceState::Error,
                            };
                            mirror_notif.apply(&event).await;
                            let _ = events_tx_notif.send(event);
                            return Err(err);
                        }
                    };
                    info!(
                        agent = %agent_id_notif,
                        instance = %instance_id_notif,
                        session = %sid,
                        "acp::instance: session/resume accepted"
                    );
                    (resp.modes, resp.models)
                };
                // Mirror the Fresh path's `NewSessionResponse.modes/models`
                // read against `(Resume|Load)SessionResponse`. Both
                // share the same `modes` / `models` shape — collapsing
                // here keeps the projection in one spot.
                if let Some(modes) = &modes_state {
                    let advertised: Vec<crate::adapters::SessionModeInfo> = modes
                        .available_modes
                        .iter()
                        .map(|m| crate::adapters::SessionModeInfo {
                            id: m.id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_modes_meta.write().await = advertised;
                    *current_mode_meta.write().await = Some(modes.current_mode_id.0.to_string());
                }
                if let Some(models) = &models_state {
                    let advertised: Vec<crate::adapters::SessionModelInfo> = models
                        .available_models
                        .iter()
                        .map(|m| crate::adapters::SessionModelInfo {
                            id: m.model_id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_models_meta.write().await = advertised;
                    *current_model_meta.write().await = Some(models.current_model_id.0.to_string());
                }
                // Suspended sessions can resume with a half-finished
                // turn — pending tool call awaiting permission, agent
                // mid-stream, etc. The replay surfaces those states
                // in the transcript, but the agent server-side might
                // still treat the original turn as "in flight",
                // refusing fresh prompts until something resolves it.
                // Send a CancelNotification right after the load
                // accepts so any inherited in-flight state collapses
                // before the user types. Soft-fails: if the session
                // is already idle, the agent treats it as a no-op.
                if let Err(err) = connection.send_notification(CancelNotification::new(sid.clone())) {
                    debug!(
                        agent = %agent_id_notif,
                        session = %sid,
                        %err,
                        "acp::instance: post-load cancel notification failed (non-fatal)"
                    );
                }
                // Replay is a snapshot, not a live turn. The dispatch
                // loop hasn't yet drained the queued session/update
                // notifications (it can't — we're inside the request
                // future), so the role-transition splits + auto-turn
                // mints happen IN the dispatch loop after this future
                // resolves. By the time the dispatch loop catches up,
                // the LAST replay turn (the most recent agent
                // exchange) is still open with no natural closer —
                // ACP shipped no `TurnEnded` for it. Wait for the
                // dispatch loop to drain (one Vue tick + some buffer)
                // then close any remaining auto-turn. This is
                // explicit lifecycle ("load resolved, then dispatch
                // drains") rather than the previous wall-clock quiet
                // heuristic — replay-with-final-quiet-after-last-event
                // never fires the timer correctly under back-pressure.
                {
                    let turn_state = turn_state.clone();
                    let events_tx = events_tx_notif.clone();
                    let mirror = mirror_notif.clone();
                    let agent_id = agent_id_notif.clone();
                    let instance_id = instance_id_notif.clone();
                    let session_id = sid.0.to_string();
                    tokio::spawn(async move {
                        // 200ms is enough for the notification task to
                        // drain everything claude-agent-acp queued
                        // during the LoadSession future. Smaller than
                        // the previous 1500ms (which was a guess); the
                        // dispatch loop typically processes 500+ replay
                        // events in <50ms on local IPC. The role-
                        // transition splits each close their own
                        // auto-turn explicitly during the drain — only
                        // the LAST one waits for this close.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        close_auto_turn_if_open(
                            &turn_state,
                            &events_tx,
                            &mirror,
                            &agent_id,
                            &instance_id,
                            &session_id,
                            "replay_complete",
                        )
                        .await;
                    });
                }
                // Resumed sessions already saw the system prompt in their
                // original turn — re-injecting it on the first post-restore
                // submit would duplicate it in agent context. Drop the
                // pending injection on this path; the Fresh-bootstrap arm
                // keeps it for first prompts on new sessions.
                if first_message_prefix.is_some() {
                    debug!(
                        agent = %agent_id_notif,
                        session = %sid,
                        "acp::instance: dropping pending system-prompt injection (session restore)"
                    );
                    first_message_prefix = None;
                }
                let event = InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                };
                mirror_notif.apply(&event).await;
                let _ = events_tx_notif.send(event);
                meta_emitter.emit(&events_tx_notif, Some(sid.0.to_string())).await;
                Some(sid)
            }
            Bootstrap::ListOnly => {
                let event = InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: None,
                    state: InstanceState::Running,
                };
                mirror_notif.apply(&event).await;
                let _ = events_tx_notif.send(event);
                None
            }
        };

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        info!(agent = %agent_id_notif, "acp::instance: command channel closed, shutting down");
                        break;
                    };
                    match cmd {
                        // Detached: awaiting `send_request(...).block_task()` inline here
                        // blocks the select! from pumping `client_events_rx`, so every
                        // `SessionNotification` (and every `PermissionRequest`!) queues
                        // on the mpsc until the prompt resolves. The permission path
                        // blocks for up to 10min waiting on a UI reply — but the UI
                        // never sees the prompt because the event is stuck in that same
                        // mpsc. Spawn the request so the loop keeps draining.
                        InstanceCommand::Prompt { text, attachments, force_dispatch, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            // Auto-route: if the captain's prior prompt
                            // is still awaiting completion OR the queue
                            // already has waiters, append to the queue
                            // instead of spawning a parallel prompt-
                            // future. ACP serialises prompts on the
                            // wire, but two simultaneous TurnGuards
                            // would race on `turn_state.open` and strand
                            // the first turn's TurnEnded — so we
                            // serialise here. Non-prompt-driven turns
                            // (out-of-prompt agent activity wrappers
                            // from replay / post-cancel residue) don't
                            // gate — `prompt_in_flight()` returns
                            // false for them, so a captain's submit on
                            // top of replay output dispatches
                            // immediately and `take_if_unguarded` below
                            // closes the replay turn out of the way.
                            //
                            // `force_dispatch` skips the auto-route — set
                            // by `queue/dispatch` so popped items go
                            // on-wire immediately (the captain's "send
                            // now" intent).
                            let queue_route = !force_dispatch && (turn_state.read().await.prompt_in_flight() || !queue.is_empty());

                            if queue_route {
                                queue_next_seq = queue_next_seq.saturating_add(1);
                                let item = crate::adapters::queue::QueueItem {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    text,
                                    attachments,
                                    enqueued_seq: queue_next_seq,
                                    enqueued_at: now_epoch_ms() as i64,
                                };
                                queue.push_back(item);
                                publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                                let _ = reply.send(Ok(()));
                                continue;
                            }
                            // First-prompt system-prompt injection: wrap the prompt
                            // text in an Attachment-shaped wire resource, NOT
                            // concatenated with the user's text. The transcript
                            // surfaces only what the captain actually typed; the
                            // wire ships the system prompt as a markdown resource
                            // alongside any user attachments. Cleared after the
                            // first submit consumes it (one-shot per spawn).
                            let system_prompt_attachment = first_message_prefix.take().map(|prefix| Attachment {
                                slug: "system-prompt".into(),
                                path: std::path::PathBuf::from("system-prompt.md"),
                                body: prefix,
                                title: Some("system prompt".into()),
                                data: None,
                                mime: Some("text/markdown".into()),
                            });
                            // Captain prompt — close any unguarded turn
                            // first so the fresh `open` doesn't collide.
                            // `take_if_unguarded` is a no-op when no
                            // turn is open or when a prompt is in
                            // flight (the queue-route branch above
                            // already filtered that case).
                            close_auto_turn_if_open(
                                &turn_state,
                                &events_tx_notif,
                                &mirror_notif,
                                &agent_id_notif,
                                &instance_id_notif,
                                sid.0.as_ref(),
                                "superseded",
                            )
                            .await;
                            let turn_id = uuid::Uuid::new_v4().to_string();
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                turn = %turn_id,
                                text_len = text.len(),
                                attachments = attachments.len(),
                                system_prompt_injected = system_prompt_attachment.is_some(),
                                "acp::instance: turn start (session/prompt)"
                            );
                            let guard = TurnGuard::new(
                                turn_id.clone(),
                                agent_id_notif.clone(),
                                instance_id_notif.clone(),
                                sid.0.to_string(),
                                events_tx_notif.clone(),
                                mirror_notif.clone(),
                                turn_state.clone(),
                            )
                            .await;
                            // Daemon-authoritative user-prompt transcript item:
                            // emitted at submit time so the UI no longer mirrors
                            // optimistically. The system-prompt attachment, when
                            // present, is intentionally NOT included here — it's
                            // a wire-side prepend the agent sees, not something
                            // the captain typed.
                            let event = InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid.0.to_string(),
                                turn_id: Some(turn_id.clone()),
                                item: crate::adapters::TranscriptItem::UserPrompt {
                                    text: text.clone(),
                                    attachments: attachments.clone(),
                                },
                                // Placeholder; `publish` overwrites with the
                                // seq minted by `mirror.apply`.
                                seq: 0,
                                // User-prompt items are minted daemon-side
                                // from the captain's submit, not from a
                                // session/update notification — no `_meta`
                                // envelope to forward.
                                meta: None,
                            };
                            publish(&mirror_notif, &events_tx_notif, event).await;
                            // Wire blocks: [system_prompt?, ...user_attachments, user_text].
                            // Per-attachment ordering preserved through the chained iterator;
                            // `build_prompt_blocks` already lays attachments before text.
                            let wire_attachments: Vec<Attachment> = system_prompt_attachment
                                .into_iter()
                                .chain(attachments.iter().cloned())
                                .collect();
                            let blocks = build_prompt_blocks(&text, &wire_attachments);
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let session_log = sid.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let turn_id_done = turn_id.clone();
                            let meta_emitter = meta_emitter.clone();
                            tokio::spawn(async move {
                                // `guard` owns the open-turn slot for the lifetime of
                                // this future. On panic / drop / unwind it synthesises
                                // `TurnEnded { stop_reason: "cancelled" }` so the UI
                                // never gets stuck on a phantom in-flight turn.
                                let guard = guard;
                                let req = PromptRequest::new(sid.clone(), blocks);
                                trace!(
                                    target: "acp::wire",
                                    agent = %agent_log,
                                    session = %session_log,
                                    turn = %turn_id_done,
                                    payload = ?serde_json::to_value(&req).ok(),
                                    "acp::instance: session/prompt outgoing"
                                );
                                let res = conn.send_request(req).block_task().await;
                                let (stop_reason, error_msg, mapped) = match res {
                                    Ok(resp) => {
                                        info!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            stop_reason = ?resp.stop_reason,
                                            "acp::instance: turn stop (prompt resolved)"
                                        );
                                        let stop = serde_json::to_value(resp.stop_reason)
                                            .ok()
                                            .and_then(|v| v.as_str().map(str::to_owned));
                                        (stop, None, Ok(()))
                                    }
                                    Err(err) => {
                                        warn!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            %err,
                                            "acp::instance: turn ended with error"
                                        );
                                        let msg = err.to_string();
                                        (None, Some(msg.clone()), Err(msg))
                                    }
                                };
                                // Guard's `complete` returns true when this future
                                // still owned the slot. False = a concurrent Cancel
                                // already synthesised TurnEnded, so we skip the
                                // InstanceMeta refresh too (it piggy-backs on the
                                // emit and only fires once per logical close).
                                if !guard.complete(stop_reason, error_msg).await {
                                    let _ = reply.send(mapped);
                                    return;
                                }
                                // Refresh tick after every turn end so the
                                // header chrome re-syncs even when the agent
                                // didn't push a `current_mode_update` /
                                // `session_info_update` notification this turn.
                                meta_emitter.emit(&events_tx_done, Some(sid.0.to_string())).await;
                                let _ = reply.send(mapped);
                            });
                        }
                        InstanceCommand::Cancel { reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                "acp::instance: turn cancel (CancelNotification)"
                            );
                            // Take the open turn id BEFORE sending the notification
                            // so the prompt-future's late reply finds an empty slot
                            // and skips its own `TurnEnded` emit (see
                            // `still_owned_turn` above). Synthesize `TurnEnded
                            // (cancelled)` straight away so the chat surface stops
                            // grouping post-cancel emissions onto the cancelled
                            // block and the next user submit lands in a fresh turn.
                            let cancelled_turn_id = turn_state.write().await.take_current();
                            let res = connection
                                .send_notification(CancelNotification::new(sid.clone()))
                                .map_err(|e| e.to_string());

                            if let Some(turn_id) = cancelled_turn_id {
                                let event = InstanceEvent::TurnEnded {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid.0.to_string(),
                                    turn_id,
                                    stop_reason: Some("cancelled".to_string()),
                                    error: None,
                                    ended_at: now_epoch_ms(),
                                };
                                mirror_notif.apply(&event).await;
                                let _ = events_tx_notif.send(event);
                                // InstanceMeta refresh — same shape as the prompt-
                                // future path, kept in sync so the header chrome
                                // doesn't lag a stale mode / model after cancel.
                                meta_emitter.emit(&events_tx_notif, Some(sid.0.to_string())).await;
                            }
                            let _ = reply.send(res);
                        }
                        InstanceCommand::SetMode { mode_id, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                mode_id,
                                "acp::instance: session/set_mode requested"
                            );
                            let conn = connection.clone();
                            let session_log = sid.clone();
                            let current_mode = current_mode_meta.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let meta_emitter = meta_emitter.clone();
                            tokio::spawn(async move {
                                let req = SetSessionModeRequest::new(session_log.clone(), SessionModeId::from(std::sync::Arc::<str>::from(mode_id.clone())));
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                if res.is_ok() {
                                    *current_mode.write().await = Some(mode_id.clone());
                                    // Refresh InstanceMeta so the header
                                    // picks up the new mode without
                                    // waiting for an agent-pushed
                                    // current_mode_update.
                                    meta_emitter.emit(&events_tx_done, Some(session_log.0.to_string())).await;
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
                        }
                        InstanceCommand::SetConfigOption { config_id, value, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            let wire_config_id = match_provider_agent(cfg.provider).wire_config_option_id(&config_id);
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                config_id = %wire_config_id,
                                display_config_id = %config_id,
                                value,
                                "acp::instance: session/set_config_option requested"
                            );
                            let conn = connection.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let meta_emitter = meta_emitter.clone();
                            let session_log = sid.clone();
                            tokio::spawn(async move {
                                use agent_client_protocol::schema::{SessionConfigId, SessionConfigValueId, SetSessionConfigOptionRequest};
                                let req = SetSessionConfigOptionRequest::new(
                                    sid.clone(),
                                    SessionConfigId::from(std::sync::Arc::<str>::from(wire_config_id.as_str())),
                                    SessionConfigValueId::from(std::sync::Arc::<str>::from(value.as_str())),
                                );
                                let res = conn.send_request(req).block_task().await.map_err(|e| e.to_string());
                                if res.is_ok() {
                                    // Refresh InstanceMeta after a successful
                                    // config_option change — keeps the header
                                    // chrome consistent with set_mode /
                                    // set_model paths. The underlying mode /
                                    // model RwLocks are updated by the
                                    // `config_option_update` notification path,
                                    // so this emit picks up whatever values the
                                    // agent advertised post-change.
                                    meta_emitter.emit(&events_tx_done, Some(session_log.0.to_string())).await;
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
                        }
                        InstanceCommand::SetModel { model_id, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                model_id,
                                "acp::instance: session/set_model requested"
                            );
                            let conn = connection.clone();
                            let session_log = sid.clone();
                            let current_model_done = current_model_meta.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let meta_emitter = meta_emitter.clone();
                            tokio::spawn(async move {
                                let req = SetSessionModelRequest::new(session_log.clone(), ModelId::from(std::sync::Arc::<str>::from(model_id.clone())));
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                if res.is_ok() {
                                    *current_model_done.write().await = Some(model_id.clone());
                                    meta_emitter.emit(&events_tx_done, Some(session_log.0.to_string())).await;
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
                        }
                        // Detached for the same reason as Prompt: list_sessions can take
                        // seconds against a remote index, and blocking the select! starves
                        // event pumping.
                        InstanceCommand::ListSessions { cwd: filter_cwd, reply } => {
                            debug!(
                                agent = %agent_id_notif,
                                cwd_filter = ?filter_cwd,
                                "acp::instance: session/list requested"
                            );
                            let conn = connection.clone();
                            tokio::spawn(async move {
                                let mut req = ListSessionsRequest::new();
                                if let Some(c) = filter_cwd {
                                    req = req.cwd(c);
                                }
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                let _ = reply.send(res);
                            });
                        }
                        InstanceCommand::MetaSnapshot { reply } => {
                            // Direct read of the per-instance Arc-cached
                            // metadata. Fast — no agent roundtrip — but
                            // returns the freshest state the daemon has
                            // (updated on session/new, session/load,
                            // set_mode, set_model, every TurnEnded).
                            let snap = MetaSnapshot {
                                session_id: session_id.as_ref().map(|s| s.0.to_string()),
                                cwd: cwd_str.clone(),
                                current_mode_id: current_mode_meta.read().await.clone(),
                                current_model_id: current_model_meta.read().await.clone(),
                                available_modes: available_modes_meta.read().await.clone(),
                                available_models: available_models_meta.read().await.clone(),
                                mcps_count,
                            };
                            let _ = reply.send(snap);
                        }
                        InstanceCommand::QueueRemove { item_id, reply } => {
                            let before = queue.len();
                            queue.retain(|q| q.id != item_id);
                            let removed = queue.len() != before;
                            if removed {
                                publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                            }
                            let _ = reply.send(Ok(removed));
                        }
                        InstanceCommand::QueueMove { item_id, position, reply } => {
                            let from = match queue.iter().position(|q| q.id == item_id) {
                                Some(i) => i,
                                None => {
                                    let _ = reply.send(Ok(false));
                                    continue;
                                }
                            };
                            // Clamp target to `[0, len-1]` (item count stays
                            // the same; this is a reorder, not an insert).
                            let to = position.min(queue.len().saturating_sub(1));
                            if from == to {
                                let _ = reply.send(Ok(true));
                                continue;
                            }
                            // VecDeque has no `swap_remove` + `insert` at
                            // arbitrary positions cleanly — pop + reinsert.
                            if let Some(item) = queue.remove(from) {
                                queue.insert(to, item);
                                publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                                let _ = reply.send(Ok(true));
                            } else {
                                let _ = reply.send(Ok(false));
                            }
                        }
                        InstanceCommand::QueueClear { reply } => {
                            let dropped = queue.len() as u32;
                            if dropped > 0 {
                                queue.clear();
                                publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                            }
                            let _ = reply.send(Ok(dropped));
                        }
                        InstanceCommand::QueueList { reply } => {
                            let _ = reply.send(queue.iter().cloned().collect());
                        }
                        InstanceCommand::QueueEdit { item_id, text, attachments, reply } => {
                            let target = queue.iter_mut().find(|q| q.id == item_id);
                            match target {
                                Some(item) => {
                                    item.text = text;
                                    if let Some(atts) = attachments {
                                        item.attachments = atts;
                                    }
                                    // Capture the updated item BEFORE the
                                    // broadcast clone (else the borrow
                                    // checker complains about `queue`
                                    // being borrowed twice).
                                    let updated = item.clone();
                                    publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                                    let _ = reply.send(Ok(updated));
                                }
                                None => {
                                    let _ = reply.send(Err(format!("queue item not found: {item_id}")));
                                }
                            }
                        }
                        InstanceCommand::QueueDispatch { item_id, reply } => {
                            // Pop the named item (or the head when None).
                            // Pop happens BEFORE the prompt fires — a
                            // concurrent `prompts/cancel` aborts the
                            // turn but does NOT re-enqueue the popped
                            // item (captain explicit re-submits if they
                            // want it back).
                            let popped = match &item_id {
                                Some(id) => match queue.iter().position(|q| q.id == *id) {
                                    Some(i) => queue.remove(i),
                                    None => None,
                                },
                                None => queue.pop_front(),
                            };
                            let item = match popped {
                                Some(it) => it,
                                None => {
                                    // Queue empty / unknown id — reply with `item: None`,
                                    // `accepted: false`. No sentinel placeholder; frontends
                                    // type-narrow on `item` directly.
                                    let _ = reply.send(Ok(crate::adapters::queue::QueueDispatchResult {
                                        item: None,
                                        session_id: None,
                                        accepted: false,
                                    }));
                                    continue;
                                }
                            };
                            publish_queue_changed(&mirror_notif, &events_tx_notif, &agent_id_notif, &instance_id_notif, &queue).await;
                            // Forward the popped item to the same prompt
                            // path `InstanceCommand::Prompt` uses. We
                            // re-enqueue the synthesised Prompt onto our
                            // own command channel so the existing
                            // attachment / system-prompt-injection /
                            // TurnGuard machinery handles it without
                            // duplicating any of that logic here. The
                            // inner Prompt's reply fires when the prompt-
                            // future resolves (turn complete) — not what
                            // the captain's "did you accept this?" RPC
                            // wants. We reply IMMEDIATELY on mailbox-
                            // accept so the UI spinner resolves in ms;
                            // turn completion arrives via the regular
                            // `acp:turn-ended` event.
                            let (inner_reply, _inner_rx) = oneshot::channel::<Result<(), String>>();
                            let session_id_str = session_id.as_ref().map(|s| s.0.to_string());
                            let accepted = cmd_tx_self
                                .send(InstanceCommand::Prompt {
                                    text: item.text.clone(),
                                    attachments: item.attachments.clone(),
                                    // Captain explicit dispatch — bypass the
                                    // queue auto-route so the popped item
                                    // goes on-wire immediately.
                                    force_dispatch: true,
                                    reply: inner_reply,
                                })
                                .is_ok();
                            let _ = reply.send(Ok(crate::adapters::queue::QueueDispatchResult {
                                item: Some(item),
                                session_id: session_id_str,
                                accepted,
                            }));
                        }
                        InstanceCommand::Shutdown { reply } => {
                            info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                has_session = session_id.is_some(),
                                close_supported,
                                reason = "shutdown command received",
                                "acp::instance: shutting down instance"
                            );
                            if let Some(sid) = session_id.clone() {
                                if close_supported {
                                    // Graceful path: send `session/close`
                                    // and give the agent up to 500ms to
                                    // flush. The kill_on_drop fallback
                                    // (subprocess Drop) still fires
                                    // afterward for hard cleanup. ACP
                                    // gates this behind unstable_session_close
                                    // — fall through to the legacy cancel
                                    // path when the agent doesn't advertise
                                    // it.
                                    use agent_client_protocol::schema::CloseSessionRequest;
                                    let close_fut = connection.send_request(CloseSessionRequest::new(sid.clone())).block_task();
                                    match tokio::time::timeout(std::time::Duration::from_millis(500), close_fut).await {
                                        Ok(Ok(_)) => {
                                            debug!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                "acp::instance: session/close acked"
                                            );
                                        }
                                        Ok(Err(err)) => {
                                            warn!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                %err,
                                                "acp::instance: session/close failed; falling through to subprocess kill"
                                            );
                                        }
                                        Err(_elapsed) => {
                                            warn!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                "acp::instance: session/close timed out (500ms); falling through to subprocess kill"
                                            );
                                        }
                                    }
                                } else {
                                    // Legacy path: agents without
                                    // `session_capabilities.close` only
                                    // know `cancel` to flush in-flight
                                    // turns. The kill_on_drop fallback
                                    // takes over from here.
                                    let _ = connection.send_notification(CancelNotification::new(sid));
                                }
                            }
                            let _ = reply.send(());
                            break;
                        }
                    }
                }
                evt = client_events_rx.recv() => {
                    let Some(evt) = evt else { break };
                    match evt {
                        ClientEvent::Notification(SessionUpdateNotification { session_id: sid, update }) => {
                            let update_kind = update
                                .get("sessionUpdate")
                                .and_then(|v| v.as_str())
                                .unwrap_or("<unknown>")
                                .to_string();

                            debug!(
                                agent = %agent_id_notif,
                                session = %sid,
                                update_kind,
                                "acp::instance: session/update received"
                            );
                            // Trace-level raw payload so vendor wire-shape
                            // surprises (e.g. claude-agent-acp's empty
                            // `agent_thought_chunk` content with
                            // `display: omitted` thinking) surface in the
                            // log without code changes. Gate behind
                            // `RUST_LOG='hyprpilot::adapters=trace'` to
                            // avoid logging every chunk on the hot path
                            // by default. Captures BEFORE
                            // `map_session_update` consumes the value.
                            trace!(
                                target: "acp::wire",
                                agent = %agent_id_notif,
                                session = %sid,
                                update_kind,
                                payload = %update,
                                "acp::instance: session/update raw"
                            );
                            // Hold the write lock for the duration of the
                            // formatter pass — `map_session_update` mutates
                            // the cache (insert on `tool_call`, merge +
                            // terminal-state evict on `tool_call_update`).
                            // Out-of-actor readers take a read lock; they
                            // see either pre- or post-update state, never a
                            // half-merged delta.
                            let MappedSessionUpdate {
                                mapped,
                                meta,
                                message_id,
                            } = {
                                let mut guard = tool_call_cache.write().await;
                                map_session_update(update, &mut guard, provider_id_for_fmt.as_str())
                            };
                            // Out-of-prompt detection: if a transcript-shape
                            // update arrives without an open turn, mint an
                            // auto-turn id + emit TurnStarted so the chat
                            // groups the entries instead of scattering them
                            // into solo blocks. SessionInfo / CurrentMode /
                            // AvailableCommands updates DO NOT trigger an
                            // auto-turn — they're per-session metadata.
                            //
                            // **Role-transition split**: ACP has no turn-
                            // boundary primitive for `session/load` replay;
                            // the daemon's only signal that a new logical
                            // turn started is `user_message_chunk` arriving
                            // while the prior emitted role was `Agent`. When
                            // an active auto-turn sees that transition, we
                            // close it (emit TurnEnded) and fall through to
                            // mint a fresh auto-turn for the new exchange.
                            // Without this split, every replayed historical
                            // exchange collapses under ONE auto-turn id and
                            // the UI renders it as a single never-finalised
                            // mega-turn. nvim's `render.lua` did the same
                            // role-transition heuristic client-side; lifting
                            // it daemon-side means every frontend benefits
                            // from one implementation.
                            let needs_auto_turn = matches!(mapped, MappedUpdate::Transcript(_));
                            let incoming_role: Option<Role> = match &mapped {
                                MappedUpdate::Transcript(item) => Role::of(item),
                                _ => None,
                            };
                            if let Some(role) = incoming_role {
                                if turn_state.read().await.should_split_on(role) {
                                    close_auto_turn_if_open(
                                        &turn_state,
                                        &events_tx_notif,
                                        &mirror_notif,
                                        &agent_id_notif,
                                        &instance_id_notif,
                                        &sid,
                                        "role_transition",
                                    )
                                    .await;
                                }
                            }
                            let mut turn_id = turn_state.read().await.current().map(str::to_string);
                            if needs_auto_turn && turn_id.is_none() {
                                let auto = uuid::Uuid::new_v4().to_string();
                                info!(
                                    agent = %agent_id_notif,
                                    instance = %instance_id_notif,
                                    session = %sid,
                                    turn = %auto,
                                    "acp::instance: auto-turn start (out-of-prompt agent activity)"
                                );
                                turn_state.write().await.open(auto.clone());
                                // Auto-minted turns wrap replay /
                                // out-of-prompt agent activity — no
                                // meaningful wall-clock (the captain
                                // didn't kick this off, the daemon
                                // synthesised it). Stamp `started_at: 0`
                                // so the UI's "no real timing" gate
                                // hides the elapsed chip instead of
                                // rendering replay processing time as
                                // if it were the turn duration.
                                //
                                // No 2500ms-quiet-window timer: explicit
                                // close paths now drive termination
                                // (role-transition split for intra-
                                // replay boundaries; LoadSession-
                                // resolves + 200ms drain wait for the
                                // final replay turn; `submit_prompt`'s
                                // `close_auto_turn_if_open` for the
                                // captain-supersedes case).
                                let event = InstanceEvent::TurnStarted {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid.clone(),
                                    turn_id: auto.clone(),
                                    started_at: 0,
                                };
                                mirror_notif.apply(&event).await;
                                let _ = events_tx_notif.send(event);
                                turn_id = Some(auto);
                            }
                            let evt: Option<InstanceEvent> = match mapped {
                                MappedUpdate::Transcript(mut item) => {
                                    // Mark the current turn as having emitted
                                    // agent output — the prompt-future reads
                                    // this on completion to decide whether to
                                    // synthesize a "no output" error when the
                                    // vendor returns a null stop reason.
                                    // User-side echoes (`UserText`),
                                    // permission requests (own bus), and
                                    // unknown wire variants don't count;
                                    // they're not the agent "doing work."
                                    if matches!(
                                        item,
                                        crate::adapters::TranscriptItem::AgentText { .. }
                                            | crate::adapters::TranscriptItem::AgentThought { .. }
                                            | crate::adapters::TranscriptItem::AgentAttachment(_)
                                            | crate::adapters::TranscriptItem::ToolCall(_)
                                            | crate::adapters::TranscriptItem::ToolCallUpdate(_)
                                            | crate::adapters::TranscriptItem::Plan(_)
                                    ) {
                                        turn_state.write().await.note_agent_output();
                                    }

                                    // Markdown-paragraph lift. Each
                                    // `agent_message_chunk` /
                                    // `agent_thought_chunk` is a wire
                                    // fragment; frontends concatenate them
                                    // verbatim. Two signals decide the
                                    // prefix:
                                    //
                                    // 1. **messageId switch** — vendor's
                                    //    `messageId` (ACP `unstable_message_id`)
                                    //    changed between two chunks within
                                    //    one turn. Claude / Codex emit a
                                    //    fresh id per content block, and a
                                    //    tool call between text chunks
                                    //    produces two distinct ids. Force
                                    //    `\n\n` so the concat reads
                                    //    `"...prior sentence.\n\nNew sentence"`.
                                    // 2. **Soft-lift trailing-newline** —
                                    //    accumulated tail ends with a
                                    //    single `\n` and the next chunk
                                    //    doesn't begin with one. Prepend
                                    //    `\n` so the boundary reaches
                                    //    `\n\n` naturally. Also handles
                                    //    chunks that lead with a single
                                    //    `\n` themselves.
                                    //
                                    // Baking the prefix onto the outgoing
                                    // chunk means every frontend — Vue
                                    // desktop, Vue remote, hyprpilot.nvim,
                                    // ctl — sees concatenation-safe text
                                    // without having to re-implement the
                                    // lift. Never injects a break on a
                                    // non-newline / non-messageId-switch
                                    // boundary, so streaming token bursts
                                    // (`"Hello, "` + `"world"`) emit
                                    // verbatim instead of splitting into
                                    // bogus paragraphs. Per-turn state
                                    // (counter + last messageId) resets on
                                    // `open`.
                                    match &mut item {
                                        crate::adapters::TranscriptItem::AgentText { text } => {
                                            let prefix = turn_state
                                                .write()
                                                .await
                                                .note_agent_text(text, message_id.as_deref());

                                            if !prefix.is_empty() {
                                                let mut lifted = String::with_capacity(prefix.len() + text.len());

                                                lifted.push_str(prefix);
                                                lifted.push_str(text);
                                                *text = lifted;
                                            }
                                        }
                                        crate::adapters::TranscriptItem::AgentThought { text } => {
                                            let prefix = turn_state
                                                .write()
                                                .await
                                                .note_agent_thought(text, message_id.as_deref());

                                            if !prefix.is_empty() {
                                                let mut lifted = String::with_capacity(prefix.len() + text.len());

                                                lifted.push_str(prefix);
                                                lifted.push_str(text);
                                                *text = lifted;
                                            }
                                        }
                                        // Tool call / plan / attachment / etc. — flag the
                                        // turn so the NEXT text or thought chunk forces a
                                        // markdown paragraph break, even when the vendor
                                        // reuses the same messageId across the interrupt
                                        // (Claude does this regularly).
                                        _ => {
                                            turn_state.write().await.note_non_text_event();
                                        }
                                    }

                                    Some(InstanceEvent::Transcript {
                                        agent_id: agent_id_notif.clone(),
                                        instance_id: instance_id_notif.clone(),
                                        session_id: sid,
                                        turn_id,
                                        item,
                                        // Placeholder; `publish` overwrites
                                        // with the seq minted by `mirror.apply`.
                                        seq: 0,
                                        meta,
                                    })
                                }
                                MappedUpdate::SessionInfo { title, updated_at } => Some(InstanceEvent::SessionInfoUpdate {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid,
                                    title,
                                    updated_at,
                                }),
                                MappedUpdate::CurrentMode { current_mode_id } => {
                                    *current_mode_meta.write().await = Some(current_mode_id.clone());
                                    Some(InstanceEvent::CurrentModeUpdate {
                                        agent_id: agent_id_notif.clone(),
                                        instance_id: instance_id_notif.clone(),
                                        session_id: sid,
                                        current_mode_id,
                                    })
                                }
                                MappedUpdate::AvailableCommands { commands } => {
                                    if let Some(cache) = commands_cache.as_ref() {
                                        match cache.write() {
                                            Ok(mut guard) => {
                                                debug!(
                                                    instance = %instance_id_notif,
                                                    count = commands.len(),
                                                    "acp::instance: available_commands_update — refreshing autocomplete cache"
                                                );
                                                *guard = commands;
                                            }
                                            Err(err) => {
                                                tracing::warn!(%err, "acp::instance: commands_cache lock poisoned");
                                            }
                                        }
                                    }
                                    None
                                }
                                MappedUpdate::Usage { used, size, cost } => Some(InstanceEvent::UsageUpdate {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid,
                                    // Bind to the active turn at notification time —
                                    // UI uses this to attach the latest reading to
                                    // the right turn in the transcript.
                                    turn_id: turn_id.clone(),
                                    used,
                                    size,
                                    cost,
                                }),
                                MappedUpdate::ConfigOptions { categories } => {
                                    // claude-agent-acp can ride mode / model on the
                                    // configOptions channel instead of dedicated
                                    // current_mode_update / current_model_update notifications.
                                    // Mirror those flips into the per-instance RwLocks so the
                                    // next MetaEmitter::emit doesn't restate a stale id.
                                    for category in &categories {
                                        if let Some(value) = category.current_value.as_ref() {
                                            match category.id.as_str() {
                                                "mode" => {
                                                    *current_mode_meta.write().await = Some(value.clone());
                                                }
                                                "model" => {
                                                    *current_model_meta.write().await = Some(value.clone());
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    Some(InstanceEvent::ConfigOptionsUpdate {
                                        agent_id: agent_id_notif.clone(),
                                        instance_id: instance_id_notif.clone(),
                                        session_id: sid,
                                        categories,
                                    })
                                }
                            };
                            if let Some(evt) = evt {
                                // Split target by event topic so transcript
                                // chunks (10-30/sec during streaming) don't
                                // drown lifecycle / usage events at trace
                                // level. `acp::emit=trace` enables only the
                                // lifecycle stream; opt into the chunk
                                // firehose with `acp::emit::chunk=trace`.
                                if matches!(
                                    evt,
                                    InstanceEvent::Transcript { .. } | InstanceEvent::Terminal { .. }
                                ) {
                                    tracing::trace!(
                                        target: "acp::emit::chunk",
                                        instance = %instance_id_notif,
                                        topic = evt.topic(),
                                        "broadcasting InstanceEvent (chunk)",
                                    );
                                } else {
                                    tracing::trace!(
                                        target: "acp::emit",
                                        instance = %instance_id_notif,
                                        topic = evt.topic(),
                                        "broadcasting InstanceEvent",
                                    );
                                }
                                publish(&mirror_notif, &events_tx_notif, evt).await;
                                // Record the routed item's role under
                                // the current turn so the next item's
                                // role-transition check (see the
                                // `should_split_on` branch above) sees
                                // the prior role. No-op when no role
                                // applies (PermissionRequest, Unknown).
                                if let Some(role) = incoming_role {
                                    turn_state.write().await.note_role(role);
                                }
                            }
                        }
                        ClientEvent::PermissionRequested {
                            session_id: sid,
                            request_id,
                            tool,
                            identity,
                            kind,
                            args,
                            raw_input,
                            content,
                            options,
                        } => {
                            debug!(
                                agent = %agent_id_notif,
                                session = %sid,
                                request_id,
                                tool = %tool,
                                "acp::instance: fan out permission prompt to UI"
                            );
                            let turn_id = turn_state.read().await.current().map(str::to_string);
                            let formatted = {
                                use crate::tools::formatter::registry::FormatterContext;
                                let registry = crate::adapters::acp::formatter_registry();
                                // Permission-request path: the tool isn't running
                                // yet, so timing isn't meaningful. Pass zeros so
                                // formatters that key on `completed_at.is_some()`
                                // skip emitting Stat::Duration here.
                                let ctx = FormatterContext {
                                    wire_name: tool.as_str(),
                                    identity: &identity,
                                    kind: kind.as_str(),
                                    raw_input: raw_input.as_ref(),
                                    adapter: provider_id_for_fmt.as_str(),
                                    content: &content,
                                    started_at: 0,
                                    completed_at: None,
                                };
                                registry.dispatch(&ctx)
                            };
                            // Daemon-side canonical ordering: every
                            // frontend renders the same button
                            // arrangement regardless of vendor.
                            // Reorder THEN compute the option ids
                            // off the reordered list so the wire
                            // shape is fully consistent.
                            let options =
                                crate::adapters::permission::reorder_options(options);
                            let allow_option_id =
                                crate::adapters::permission::pick_allow_once_id(&options);
                            let reject_option_id =
                                crate::adapters::permission::pick_reject_option_id(&options);
                            let event = InstanceEvent::PermissionRequest {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                turn_id,
                                request_id,
                                tool,
                                identity,
                                kind,
                                args,
                                raw_input,
                                content,
                                options,
                                default_option_id: allow_option_id.clone(),
                                allow_option_id,
                                reject_option_id,
                                formatted,
                            };
                            publish(&mirror_notif, &events_tx_notif, event).await;
                        }
                    }
                }
            }
        }
        Ok::<(), agent_client_protocol::Error>(())
    };

    let builder = Client.builder().on_receive_notification(
        {
            let client = client.clone();
            move |notification: SessionUpdateNotification, _cx| {
                let client = client.clone();
                async move {
                    client.forward_notification(notification);
                    Ok(())
                }
            }
        },
        agent_client_protocol::on_receive_notification!(),
    );
    let builder = register_client_handler!(builder, client, request_permission);
    let builder = register_client_handler!(builder, client, read_text_file);
    let builder = register_client_handler!(builder, client, write_text_file);
    let builder = register_client_handler!(builder, client, create_terminal);
    let builder = register_client_handler!(builder, client, terminal_output);
    let builder = register_client_handler!(builder, client, wait_for_terminal_exit);
    let builder = register_client_handler!(builder, client, kill_terminal);
    let builder = register_client_handler!(builder, client, release_terminal);

    // Race the agent run against the child process exit. The ACP
    // crate's `send_request(...).block_task().await` doesn't propagate
    // a transport drop — when the child dies before responding to
    // `initialize`, the dispatch future stalls indefinitely. Watching
    // `child.wait()` in the same select! gives us a fast surface for
    // crashed agents (instead of waiting on a watchdog that doesn't
    // exist on this code path) and converts the failure into the
    // `InstanceState::Error` lifecycle event the rest of the system
    // already handles.
    let run_outcome: Result<(), anyhow::Error> = tokio::select! {
        outcome = builder.connect_with(transport, dispatch) => outcome.map_err(|err| anyhow::anyhow!("acp connection ended: {err}")),
        wait = child.wait() => match wait {
            Ok(status) if status.success() => {
                // Clean child exit before our shutdown handshake —
                // expected for `Bootstrap::ListOnly` actors and any
                // vendor that voluntarily exits 0 after their work
                // ends. Not an error; surface as a clean `Ended`.
                info!(agent = %agent_id, ?status, "acp::instance: child exited cleanly before disconnect");
                Ok(())
            }
            Ok(status) => {
                warn!(agent = %agent_id, ?status, "acp::instance: child exited with non-zero status before connection closed");
                Err(anyhow::anyhow!("agent process exited before disconnect: {status}"))
            }
            Err(err) => {
                warn!(agent = %agent_id, %err, "acp::instance: child wait failed mid-run");
                Err(anyhow::anyhow!("child wait failed: {err}"))
            }
        }
    };

    let final_state = match &run_outcome {
        Ok(_) => {
            info!(agent = %agent_id, "acp::instance: instance ended cleanly");
            InstanceState::Ended
        }
        Err(err) => {
            warn!(agent = %agent_id, %err, "acp::instance: instance ended with error");
            InstanceState::Error
        }
    };

    // Give the agent subprocess a brief window to exit cleanly after
    // the transport closes above. The `CancelNotification` we sent on
    // shutdown + the resulting stdin EOF are the standard ACP signals
    // to terminate. SIGKILL'ing zero-delay mid-cleanup makes vendor
    // SDKs (notably `@anthropic-ai/claude-agent-sdk` inside
    // claude-agent-acp) spew "Query closed before response received" on
    // stderr because they're tearing down a still-open Anthropic
    // streaming connection that's kept warm between turns. Wait up to
    // 5s for a clean exit, fall back to SIGKILL.
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => debug!(agent = %agent_id, ?status, "acp::instance: child exited cleanly"),
        Ok(Err(err)) => warn!(agent = %agent_id, %err, "acp::instance: child wait failed"),
        Err(_) => {
            warn!(
                agent = %agent_id,
                "acp::instance: child did not exit within 5s after stdin EOF, sending SIGKILL"
            );
            let _ = child.kill().await;
        }
    }
    let sid = session_id_slot.read().await.clone();
    if let Some(ref id) = sid {
        client.drain_terminals_for_session(id).await;
    }
    let event = InstanceEvent::State {
        agent_id,
        instance_id,
        session_id: sid.as_ref().map(|id| id.0.to_string()),
        state: final_state,
    };
    publish(&mirror, &events_tx, event).await;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::adapters::permission::DefaultPermissionController;
    use crate::config::{AgentConfig, AgentProvider};

    // ── TurnState markdown-paragraph lift ──────────────────────────────
    //
    // Tests for the per-turn lift counters. The pure helpers in
    // `acp::paragraph` are covered by their own unit tests; these
    // exercise the `TurnState` integration — counter init, mutation
    // through `note_agent_text` / `note_agent_thought`, reset on
    // `open`, and independence between the
    // text and thought streams.

    #[test]
    fn turn_state_lift_starts_at_zero_trailing() {
        let mut state = TurnState::default();
        // First chunk on a fresh turn — prior trailing is 0 → no lift.
        assert_eq!(state.note_agent_text("First chunk", None), "");
        assert_eq!(state.agent_text_trailing, 0);
    }

    #[test]
    fn turn_state_lifts_a_soft_newline_to_paragraph_break() {
        let mut state = TurnState::default();
        // Chunk 1 leaves trailing \n.
        assert_eq!(state.note_agent_text("Para 1.\n", None), "");
        assert_eq!(state.agent_text_trailing, 1);
        // Chunk 2 starts with non-newline → daemon prepends \n so the
        // boundary reaches \n\n in the concat.
        assert_eq!(state.note_agent_text("Para 2.", None), "\n");
        // After the lift, the chunk ends on non-newline → counter reset.
        assert_eq!(state.agent_text_trailing, 0);
    }

    #[test]
    fn turn_state_does_not_double_inject_when_prior_and_chunk_both_carry_a_newline() {
        let mut state = TurnState::default();
        state.note_agent_text("Para 1.\n", None);
        // Prior trailing `\n` + chunk's own leading `\n` already sum
        // to `\n\n` at the boundary — no lift, no wasted injection.
        // The combined wire shape is `Para 1.\n\nPara 2.`, which
        // renders as two paragraphs.
        assert_eq!(state.note_agent_text("\nPara 2.", None), "");
    }

    #[test]
    fn turn_state_lifts_chunk_self_newline_only_when_prior_contributes_nothing() {
        let mut state = TurnState::default();
        // Prior trailing is 0 (chunk ends on non-newline). Chunk's
        // own leading `\n` is just a soft break in markdown — lift
        // to `\n\n` so it reads as a paragraph break.
        state.note_agent_text("Para 1.", None);
        assert_eq!(state.note_agent_text("\nPara 2.", None), "\n");
    }

    #[test]
    fn turn_state_does_not_lift_when_chunk_already_starts_with_double_newline() {
        let mut state = TurnState::default();
        state.note_agent_text("Para 1.\n", None);
        // `\n\n` already carries its own paragraph break — no lift.
        assert_eq!(state.note_agent_text("\n\nPara 2.", None), "");
    }

    #[test]
    fn turn_state_does_not_lift_a_non_newline_boundary() {
        let mut state = TurnState::default();
        // Token streaming inside one paragraph — no lift between chunks.
        assert_eq!(state.note_agent_text("Hello, ", None), "");
        assert_eq!(state.note_agent_text("world", None), "");
        assert_eq!(state.note_agent_text("!", None), "");
    }

    #[test]
    fn turn_state_text_and_thought_counters_are_independent() {
        let mut state = TurnState::default();
        // Build trailing newline on the text axis.
        state.note_agent_text("Para 1.\n", None);
        assert_eq!(state.agent_text_trailing, 1);
        // Thought axis is untouched.
        assert_eq!(state.agent_thought_trailing, 0);
        // Thought chunk doesn't see the text-side trailing — first
        // thought is a fresh boundary, no lift.
        assert_eq!(state.note_agent_thought("First thought.", None), "");
        // And the text axis counter is still 1 — the thought
        // operation didn't bleed across.
        assert_eq!(state.agent_text_trailing, 1);
    }

    #[test]
    fn turn_state_open_resets_both_lift_counters() {
        let mut state = TurnState::default();
        state.note_agent_text("Tail 1.\n", None);
        state.note_agent_thought("Thought 1.\n", None);
        assert_eq!(state.agent_text_trailing, 1);
        assert_eq!(state.agent_thought_trailing, 1);
        // Opening a new turn — counters reset so the new turn's first
        // chunk doesn't carry forward the prior turn's tail.
        state.open("turn-2".to_string());
        assert_eq!(state.agent_text_trailing, 0);
        assert_eq!(state.agent_thought_trailing, 0);
        // First chunk of the new turn — no lift on the boundary.
        assert_eq!(state.note_agent_text("Hello", None), "");
    }

    #[test]
    fn turn_state_caps_trailing_at_two() {
        let mut state = TurnState::default();
        // A chunk ending on \n\n\n caps at 2.
        state.note_agent_text("Para 1.\n\n\n", None);
        assert_eq!(state.agent_text_trailing, 2);
        // Already paragraph-shaped → no further lift on next chunk.
        assert_eq!(state.note_agent_text("Para 2.", None), "");
    }

    #[test]
    fn turn_state_forces_paragraph_break_across_message_id_switch() {
        // Captain's screenshot bug: tool_use between two text chunks
        // produces two distinct messageIds. Prior text ends with `.`
        // (no trailing newline), incoming starts with a capital
        // (no leading whitespace). Soft-lift would correctly refuse
        // to inject (avoiding mid-sentence splits); messageId
        // boundary forces `\n\n` anyway.
        let mut state = TurnState::default();
        assert_eq!(
            state.note_agent_text("...so it hides whatever's behind.", Some("msg-1")),
            ""
        );
        assert_eq!(
            state.note_agent_text("Now bg is solid rgb(255, 255, 255).", Some("msg-2")),
            "\n\n"
        );
    }

    #[test]
    fn turn_state_same_message_id_keeps_soft_lift_path() {
        // Within one content block (same messageId), the vendor
        // streams tokens — bare token bursts must NOT be split into
        // bogus paragraphs.
        let mut state = TurnState::default();
        assert_eq!(state.note_agent_text("Hello, ", Some("msg-1")), "");
        assert_eq!(state.note_agent_text("world!", Some("msg-1")), "");
    }

    #[test]
    fn turn_state_forces_paragraph_break_across_thought_message_id_switch() {
        // Thought stream gets the same treatment — vendors emit fresh
        // messageIds per thought content block. Tool reasoning that
        // resumes after a tool call produces two distinct ids, and
        // the thinking card needs the same paragraph break or the
        // concatenated reasoning reads as one run-on.
        let mut state = TurnState::default();
        assert_eq!(state.note_agent_thought("Stepping through.", Some("th-1")), "");
        assert_eq!(
            state.note_agent_thought("Now checking the next case.", Some("th-2")),
            "\n\n"
        );
    }

    #[test]
    fn turn_state_text_and_thought_message_ids_are_independent() {
        // Text and thought streams track their own messageIds — a
        // text messageId switch must NOT trigger a thought-stream
        // paragraph break (or vice versa).
        let mut state = TurnState::default();
        state.note_agent_text("text-1", Some("msg-1"));
        // Switching the text messageId after writing a thought.
        state.note_agent_thought("thought-1", Some("th-1"));
        // Thought-stream chunk with a still-matching thought id —
        // soft-lift only, no forced break.
        assert_eq!(state.note_agent_thought(" continues", Some("th-1")), "");
    }

    #[test]
    fn turn_state_message_id_resets_on_open() {
        // A new turn opens; even if the next chunk's messageId matches
        // a stale id from the prior turn (vendor reuse — unlikely but
        // possible), we don't carry forward state.
        let mut state = TurnState::default();
        state.note_agent_text("Para 1.", Some("msg-1"));
        state.open("turn-2".into());
        // Fresh turn — no prior id, so the boundary check returns
        // false; soft-lift path runs and emits nothing for a clean
        // first chunk.
        assert_eq!(state.note_agent_text("Fresh start.", Some("msg-1")), "");
    }

    #[test]
    fn turn_state_forces_paragraph_break_after_tool_call_even_with_same_message_id() {
        // The regression PR #79 didn't catch: Claude often reuses the
        // SAME messageId across text → tool call → text. With only the
        // messageId-switch check, the resumed text would concat
        // directly onto the prior sentence. The
        // `non_text_event_since_last_text` flag forces a break.
        let mut state = TurnState::default();
        state.note_agent_text("...behind.", Some("msg-1"));
        // A tool call lands — dispatcher catch-all flags non-text event.
        state.note_non_text_event();
        // Next text chunk: same messageId, no leading newline.
        // Without the flag → soft-lift would do nothing → captain sees
        // "...behind.Now bg...". With the flag → paragraph_break_prefix
        // injects "\n\n" so the boundary renders as a paragraph break.
        assert_eq!(state.note_agent_text("Now bg is solid", Some("msg-1")), "\n\n");
    }

    #[test]
    fn turn_state_forces_paragraph_break_after_tool_call_with_no_message_id() {
        // Vendors that never emit messageId (or stopped emitting after
        // the tool) still get the paragraph break via the
        // non-text-event flag.
        let mut state = TurnState::default();
        state.note_agent_text("Pre-tool.", None);
        state.note_non_text_event();
        assert_eq!(state.note_agent_text("Post-tool.", None), "\n\n");
    }

    #[test]
    fn turn_state_non_text_event_flag_is_consumed_by_next_text_chunk() {
        // After the next text chunk uses the flag, a subsequent chunk
        // without an intervening non-text event should NOT get a
        // forced break — falls back to soft-lift (no break on clean
        // token concat).
        let mut state = TurnState::default();
        state.note_agent_text("...behind.", Some("msg-1"));
        state.note_non_text_event();
        // First chunk consumes the flag.
        assert_eq!(state.note_agent_text("Now bg is solid.", Some("msg-1")), "\n\n");
        // Second chunk: same messageId, no new non-text event since.
        // Should emit nothing (soft-lift; no trailing newline on
        // "...solid.", no leading newline on " More text.").
        assert_eq!(state.note_agent_text(" More text.", Some("msg-1")), "");
    }

    #[test]
    fn turn_state_does_not_inject_paragraph_break_inside_code_fence() {
        let mut state = TurnState::default();

        assert_eq!(state.note_agent_text("```rust\n", Some("msg-1")), "");
        state.note_non_text_event();

        assert_eq!(
            state.note_agent_text("let value = 1;\n", Some("msg-1")),
            "",
            "paragraph repair must not insert blank lines inside fenced code"
        );
        assert_eq!(state.note_agent_text("```\n", Some("msg-1")), "");
        state.note_non_text_event();
        assert_eq!(state.note_agent_text("Back to prose.", Some("msg-1")), "\n");
    }

    #[test]
    fn turn_state_non_text_event_clears_on_open() {
        let mut state = TurnState::default();
        state.note_non_text_event();
        state.open("turn-2".into());
        // Fresh turn — no flagged event, no prior text → soft-lift.
        assert_eq!(state.note_agent_text("Fresh start.", Some("msg-1")), "");
    }

    #[test]
    fn turn_state_text_and_thought_non_text_flags_are_independent() {
        // A non-text event flags BOTH streams, but consuming via text
        // does NOT consume the thought flag (and vice versa).
        let mut state = TurnState::default();
        state.note_agent_text("Text 1.", Some("text-1"));
        state.note_agent_thought("Thought 1.", Some("th-1"));
        state.note_non_text_event();
        // Text chunk consumes its flag.
        assert_eq!(state.note_agent_text("Text 2.", Some("text-1")), "\n\n");
        // Thought stream still has its flag set — should fire on next
        // thought chunk independently.
        assert_eq!(state.note_agent_thought("Thought 2.", Some("th-1")), "\n\n");
    }

    // ── existing tests ─────────────────────────────────────────────────

    #[test]
    fn strip_plan_step_header_drops_leading_atx_headers() {
        // Single leading header line, common case.
        assert_eq!(
            strip_plan_step_header("### tasks\nactual body content"),
            "actual body content"
        );
        // Multiple consecutive header lines (agent gets enthusiastic).
        assert_eq!(strip_plan_step_header("# title\n## subtitle\nbody"), "body");
        // Header buried mid-content stays put — only consecutive
        // leading lines are stripped.
        assert_eq!(
            strip_plan_step_header("real body\n## not a leading header"),
            "real body\n## not a leading header"
        );
        // Leading blank lines + header + blank line + body.
        assert_eq!(strip_plan_step_header("\n\n### tasks\n\nbody"), "body");
        // Trailing whitespace trimmed.
        assert_eq!(strip_plan_step_header("body  \n  "), "body");
    }

    #[test]
    fn strip_plan_step_header_preserves_non_header_content() {
        // `#1` is not an ATX header (no space between `#` and content).
        assert_eq!(strip_plan_step_header("#1 first item"), "#1 first item");
        // Empty input.
        assert_eq!(strip_plan_step_header(""), "");
        // No header at all.
        assert_eq!(strip_plan_step_header("just body text"), "just body text");
    }

    #[test]
    fn map_session_update_computes_plan_stats_and_strips_headers() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "### tasks\nset up branch", "status": "completed" },
                { "content": "implement feature", "status": "in_progress" },
                { "content": "write tests", "status": "pending" },
                { "content": "ship it", "status": "completed" },
            ],
        });
        let mapped = map_session_update(update, &mut cache, "claude-code");
        let MappedUpdate::Transcript(item) = mapped.mapped else {
            panic!("expected Transcript update");
        };
        let crate::adapters::TranscriptItem::Plan(plan) = item else {
            panic!("expected Plan transcript item");
        };
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(
            plan.steps[0].content, "set up branch",
            "header stripped from first step"
        );
        assert_eq!(plan.steps[1].content, "implement feature");
        assert_eq!(plan.stats.done, 2, "two completed");
        assert_eq!(plan.stats.total, 4, "four total");
    }

    #[test]
    fn map_session_update_sets_codex_mcp_tool_identity() {
        use serde_json::json;

        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tc-1",
            "title": "Tool: hyprpilot/read_skill",
            "kind": "other",
            "status": "running",
            "rawInput": {
                "server": "hyprpilot",
                "tool": "read_skill",
                "arguments": { "slug": "git-branch" }
            }
        });
        let mapped = map_session_update(update, &mut cache, "acp-codex").mapped;
        let MappedUpdate::Transcript(crate::adapters::TranscriptItem::ToolCall(record)) = mapped else {
            panic!("expected tool call");
        };

        assert_eq!(
            record.identity,
            ToolIdentity::Mcp {
                server: "hyprpilot".into(),
                leaf: "read_skill".into()
            }
        );
    }

    /// Pin every wire shape `chunk_text` extracts text from. Bare-text
    /// is the ACP-spec'd shape; `thinking` covers claude-agent-acp
    /// passing through Anthropic's reasoning blocks unchanged; the
    /// array form covers multi-block thought deltas.
    #[test]
    fn map_session_update_extracts_thought_text_from_every_known_shape() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let cases: &[(&str, serde_json::Value)] = &[
            (
                "hello",
                json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hello"}}),
            ),
            (
                "reasoning",
                json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "thinking", "thinking": "reasoning"}}),
            ),
            (
                "AB",
                json!({"sessionUpdate": "agent_thought_chunk", "content": [{"type": "thinking", "thinking": "A"}, {"type": "text", "text": "B"}]}),
            ),
        ];
        for (expected, update) in cases {
            let MappedSessionUpdate { mapped, .. } = map_session_update(update.clone(), &mut cache, "claude-code");
            match mapped {
                MappedUpdate::Transcript(crate::adapters::TranscriptItem::AgentThought { text }) => {
                    assert_eq!(&text, expected, "input: {update}");
                }
                _ => panic!("expected AgentThought for {update}"),
            }
        }
    }

    /// `messageId` (ACP `unstable_message_id`) round-trips through
    /// `map_session_update` for both `agent_message_chunk` and
    /// `agent_thought_chunk` — the emit site uses it to detect
    /// content-block boundaries and force a markdown paragraph break.
    #[test]
    fn map_session_update_extracts_message_id_from_chunk_payloads() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let cases: &[serde_json::Value] = &[
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "Hello" },
                "messageId": "msg-abc",
            }),
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "thinking", "thinking": "..." },
                "messageId": "msg-xyz",
            }),
        ];
        let expected = ["msg-abc", "msg-xyz"];
        for (i, update) in cases.iter().enumerate() {
            let MappedSessionUpdate { message_id, .. } = map_session_update(update.clone(), &mut cache, "claude-code");
            assert_eq!(
                message_id.as_deref(),
                Some(expected[i]),
                "messageId not extracted from {update}"
            );
        }
        // Updates without a messageId field yield None.
        let none_case = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "no id" },
        });
        let MappedSessionUpdate { message_id, .. } = map_session_update(none_case, &mut cache, "claude-code");
        assert!(message_id.is_none(), "missing messageId should yield None");
    }

    /// Empty / unknown content shapes should still flow through (UI
    /// renders the thinking card with no body) — no panic, no
    /// silent drop.
    /// PermissionRequest carries a `formatted` field so the transcript-
    /// path replay (snapshot hydration, cross-device mirror) renders the
    /// same description / fields / output the live `acp:permission-request`
    /// event carries. Without this, a permission row replayed from the
    /// daemon mirror would have an empty body while the same prompt
    /// arriving live shows command + args. The `started_at: 0` /
    /// `completed_at: None` path keeps `Stat::Duration` off the
    /// permission row, matching the live emit at instance.rs:~2949.
    #[test]
    fn map_session_update_permission_request_carries_formatted() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "permission_request",
            "requestId": "r-1",
            "tool": "Bash",
            "kind": "execute",
            "args": "ls /tmp",
            "rawInput": { "command": "ls /tmp" },
            "options": [],
        });
        let MappedSessionUpdate { mapped, .. } = map_session_update(update, &mut cache, "claude-code");
        match mapped {
            MappedUpdate::Transcript(crate::adapters::TranscriptItem::PermissionRequest(rec)) => {
                assert_eq!(rec.request_id, "r-1");
                assert_eq!(rec.tool, "Bash");
                // formatted must be populated — the formatter dispatched.
                // bash formatter emits a non-empty title; even a fall-through
                // formatter writes the wire name as title. Assert non-empty
                // so a regression that drops the dispatch trips here.
                assert!(
                    !rec.formatted.title.is_empty(),
                    "PermissionRequestRecord.formatted must be populated via the formatter registry"
                );
                // started_at: 0 / completed_at: None means no duration stat.
                let has_duration = rec
                    .formatted
                    .stats
                    .iter()
                    .any(|s| matches!(s, crate::tools::formatter::types::Stat::Duration { .. }));
                assert!(!has_duration, "permission row must not carry Stat::Duration");
            }
            _ => panic!("expected PermissionRequest variant"),
        }
    }

    /// `config_options_update` projects onto `MappedUpdate::ConfigOptions`
    /// with a typed `SessionConfigOptionCategory` per category. Pins
    /// T4 from the audit: the `mode` / `model` mirror dispatch
    /// (instance.rs:2945) is downstream of this mapping, and a
    /// regression that drops a `currentValue` or renames a category
    /// would silently break the per-instance mode/model mirror
    /// without surfacing in any other test.
    #[test]
    fn map_session_update_config_options_carries_mode_and_model_categories() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {
                    "id": "mode",
                    "name": "Mode",
                    "currentValue": "plan",
                    "options": [
                        { "value": "plan", "name": "Plan" },
                        { "value": "edit", "name": "Edit" }
                    ]
                },
                {
                    "id": "model",
                    "name": "Model",
                    "currentValue": "claude-sonnet-4-5",
                    "options": [
                        { "value": "claude-sonnet-4-5", "name": "Sonnet 4.5" }
                    ]
                }
            ]
        });
        let MappedSessionUpdate { mapped, .. } = map_session_update(update, &mut cache, "claude-code");
        match mapped {
            MappedUpdate::ConfigOptions { categories } => {
                assert_eq!(categories.len(), 2, "two categories — mode + model");
                let mode = categories.iter().find(|c| c.id == "mode").expect("mode category");
                assert_eq!(mode.current_value.as_deref(), Some("plan"));
                assert_eq!(mode.options.len(), 2);
                let model = categories.iter().find(|c| c.id == "model").expect("model category");
                assert_eq!(model.current_value.as_deref(), Some("claude-sonnet-4-5"));
            }
            _ => panic!("expected ConfigOptions variant"),
        }
    }

    /// Unknown category ids pass through with their fields intact —
    /// the actor's `mode`/`model` mirror dispatch (instance.rs:2945)
    /// only writes to the per-instance RwLocks on `id == "mode"` /
    /// `"model"`, so an `effort` or `thought_level` category just
    /// rides the wire untouched. Pins forward-compat: a new ACP
    /// category id doesn't crash the mapping.
    #[test]
    fn map_session_update_config_options_passes_unknown_category_through() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {
                    "id": "effort",
                    "name": "Effort",
                    "currentValue": "high",
                    "options": []
                }
            ]
        });
        let MappedSessionUpdate { mapped, .. } = map_session_update(update, &mut cache, "claude-code");
        match mapped {
            MappedUpdate::ConfigOptions { categories } => {
                assert_eq!(categories.len(), 1);
                assert_eq!(categories[0].id, "effort");
                assert_eq!(categories[0].current_value.as_deref(), Some("high"));
            }
            _ => panic!("expected ConfigOptions variant"),
        }
    }

    #[test]
    fn map_session_update_codex_config_options_normalizes_effort_id() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {
                    "id": "reasoning_effort",
                    "name": "Reasoning Effort",
                    "currentValue": "high",
                    "options": []
                }
            ]
        });
        let MappedSessionUpdate { mapped, .. } = map_session_update(update, &mut cache, "acp-codex");
        match mapped {
            MappedUpdate::ConfigOptions { categories } => {
                assert_eq!(categories.len(), 1);
                assert_eq!(categories[0].id, "effort");
                assert_eq!(categories[0].current_value.as_deref(), Some("high"));
            }
            _ => panic!("expected ConfigOptions variant"),
        }
    }

    #[test]
    fn map_session_update_thought_with_unknown_shape_yields_empty_text() {
        use serde_json::json;
        let mut cache = ToolCallCache::default();
        let update = json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "weird", "blob": "..."}});
        let MappedSessionUpdate { mapped, .. } = map_session_update(update, &mut cache, "claude-code");
        match mapped {
            MappedUpdate::Transcript(crate::adapters::TranscriptItem::AgentThought { text }) => {
                assert!(text.is_empty())
            }
            _ => panic!("expected AgentThought with empty text"),
        }
    }

    #[test]
    fn build_prompt_blocks_emits_only_text_when_no_attachments() {
        let blocks = build_prompt_blocks("hello", &[]);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_treats_skill_blob_as_plain_text_resource() {
        // The hydrator (`HyprpilotTokenHydrator::hydrate_skill`)
        // builds the markdown blob inline; by the time we hit
        // `build_prompt_blocks` the attachment is just a text resource
        // (slug = full URI, body = blob, mime = text/markdown). No
        // skill-specific dispatch here — verify the resource shape
        // rides through the regular `MimeCategory::Text` path.
        let att = Attachment {
            slug: "hyprpilot://skills/git-commit".into(),
            path: PathBuf::from("/tmp/skills/git-commit/SKILL.md"),
            body: "Attached skill `git-commit` (Git commit). Read via mcp__hyprpilot__read_skill.".into(),
            title: Some("Git commit".into()),
            data: None,
            mime: Some("text/markdown".into()),
        };
        let blocks = build_prompt_blocks("please commit", std::slice::from_ref(&att));
        assert_eq!(blocks.len(), 2, "one resource + one text");
        let ContentBlock::Resource(res) = &blocks[0] else {
            panic!("first block must be Resource");
        };
        let EmbeddedResourceResource::TextResourceContents(tr) = &res.resource else {
            panic!("resource must carry text contents");
        };
        assert_eq!(tr.uri, "file:///tmp/skills/git-commit/SKILL.md");
        assert_eq!(tr.mime_type.as_deref(), Some("text/markdown"));
        // Body rides through verbatim — `attachment_to_block` no
        // longer rewrites it.
        assert_eq!(
            tr.text,
            "Attached skill `git-commit` (Git commit). Read via mcp__hyprpilot__read_skill.",
        );
        match &blocks[1] {
            ContentBlock::Text(t) => assert_eq!(t.text, "please commit"),
            other => panic!("second block must be text, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_dispatches_image_audio_blob_purely_by_mime() {
        let img = Attachment {
            slug: "shot".into(),
            path: PathBuf::from("/tmp/shot.png"),
            body: String::new(),
            title: None,
            data: Some("BASE64IMG".into()),
            mime: Some("image/png".into()),
        };
        let audio = Attachment {
            slug: "clip".into(),
            path: PathBuf::from("/tmp/clip.wav"),
            body: String::new(),
            title: None,
            data: Some("BASE64AUDIO".into()),
            mime: Some("audio/wav".into()),
        };
        let pdf = Attachment {
            slug: "doc".into(),
            path: PathBuf::from("/tmp/doc.pdf"),
            body: String::new(),
            title: None,
            data: Some("BASE64PDF".into()),
            mime: Some("application/pdf".into()),
        };
        let yaml = Attachment {
            slug: "cfg".into(),
            path: PathBuf::from("/tmp/cfg.yaml"),
            body: "name: hyprpilot".into(),
            title: None,
            data: None,
            mime: Some("application/x-yaml".into()),
        };
        let blocks = build_prompt_blocks("text", &[img, audio, pdf, yaml]);
        assert_eq!(blocks.len(), 5, "4 attachments + 1 text");
        match &blocks[0] {
            ContentBlock::Image(i) => {
                assert_eq!(i.mime_type, "image/png");
                assert_eq!(i.data, "BASE64IMG");
            }
            other => panic!("expected image, got {other:?}"),
        }
        match &blocks[1] {
            ContentBlock::Audio(a) => {
                assert_eq!(a.mime_type, "audio/wav");
                assert_eq!(a.data, "BASE64AUDIO");
            }
            other => panic!("expected audio, got {other:?}"),
        }
        let ContentBlock::Resource(pdf_res) = &blocks[2] else {
            panic!("expected resource for PDF")
        };
        let EmbeddedResourceResource::BlobResourceContents(blob) = &pdf_res.resource else {
            panic!("PDF must encode as blob, not text")
        };
        assert_eq!(blob.blob, "BASE64PDF");
        assert_eq!(blob.mime_type.as_deref(), Some("application/pdf"));
        let ContentBlock::Resource(yaml_res) = &blocks[3] else {
            panic!("expected resource for yaml")
        };
        let EmbeddedResourceResource::TextResourceContents(tr) = &yaml_res.resource else {
            panic!("yaml must encode as text resource")
        };
        assert_eq!(tr.text, "name: hyprpilot");
        assert_eq!(tr.mime_type.as_deref(), Some("application/x-yaml"));
    }

    #[test]
    fn mime_category_classifies_known_types() {
        assert_eq!(mime_category("image/png"), MimeCategory::Image);
        assert_eq!(mime_category("image/svg+xml"), MimeCategory::Image);
        assert_eq!(mime_category("audio/mp3"), MimeCategory::Audio);
        assert_eq!(mime_category("text/plain"), MimeCategory::Text);
        assert_eq!(mime_category("text/markdown"), MimeCategory::Text);
        assert_eq!(mime_category("application/json"), MimeCategory::Text);
        assert_eq!(mime_category("application/x-yaml"), MimeCategory::Text);
        assert_eq!(mime_category("application/pdf"), MimeCategory::Blob);
        assert_eq!(mime_category("application/octet-stream"), MimeCategory::Blob);
    }

    #[test]
    fn build_prompt_blocks_preserves_attachment_order() {
        let a = Attachment {
            slug: "hyprpilot://skills/a".into(),
            path: PathBuf::from("/tmp/a/SKILL.md"),
            body: "blob-a".into(),
            title: None,
            data: None,
            mime: Some("text/markdown".into()),
        };
        let b = Attachment {
            slug: "hyprpilot://skills/b".into(),
            path: PathBuf::from("/tmp/b/SKILL.md"),
            body: "blob-b".into(),
            title: None,
            data: None,
            mime: Some("text/markdown".into()),
        };
        let blocks = build_prompt_blocks("text", &[a, b]);
        assert_eq!(blocks.len(), 3);
        // Bodies ride through verbatim now (hydrator owns the blob).
        // The order assertion is the load-bearing bit — attachments
        // must come before the user text and stay in their original
        // order.
        let ContentBlock::Resource(first) = &blocks[0] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr0) = &first.resource else {
            panic!()
        };
        assert_eq!(tr0.text, "blob-a");
        let ContentBlock::Resource(second) = &blocks[1] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr1) = &second.resource else {
            panic!()
        };
        assert_eq!(tr1.text, "blob-b");
    }

    fn dummy_resolved(id: &str) -> ResolvedInstance {
        ResolvedInstance {
            agent: AgentConfig {
                id: id.into(),
                provider: AgentProvider::AcpClaudeCode,
                command: "/bin/false".into(),
                args: Vec::new(),
                cwd: None,
                env: Default::default(),
                model: None,
            },
            profile_id: None,
            model: None,
            system_prompt: Vec::new(),
            mode: None,
        }
    }

    fn dummy_permissions() -> Arc<dyn PermissionController> {
        Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>
    }

    fn dummy_start_params(id: &str, events_tx: broadcast::Sender<InstanceEvent>) -> StartParams {
        StartParams {
            resolved: dummy_resolved(id),
            key: crate::adapters::InstanceKey::new_v4(),
            profile_id: None,
            events_tx,
            bootstrap: Bootstrap::Fresh,
            permissions: dummy_permissions(),
            mcps: None,
            skills: Arc::new(crate::skills::SkillsRegistry::new(Vec::new())),
            commands_cache: None,
            config_patches: Vec::new(),
        }
    }

    /// Regression: starting against a child that exits immediately
    /// pushes an `Error` lifecycle event rather than hanging forever.
    /// Smoke-tests the actor shell without depending on a real agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_child_yields_error_state() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(dummy_start_params("ded", tx));

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("starting event timely")
            .expect("starting event arrives");
        match first {
            InstanceEvent::State {
                state: InstanceState::Starting,
                ..
            } => {}
            other => panic!("expected Starting, got {other:?}"),
        }

        let err = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(err.is_ok(), "actor reached terminal state");

        drop(handle);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_against_dead_session_does_not_panic() {
        let (tx, _rx) = broadcast::channel(8);
        let handle = AcpInstance::start(dummy_start_params("ded-cancel", tx));

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = handle.cmd_tx.send(InstanceCommand::Cancel { reply: reply_tx });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), reply_rx).await;
    }

    /// Smoke: a `ListOnly` actor against a dead child still settles
    /// (the `initialize` roundtrip fails, which drives the actor to
    /// `Error` instead of panicking or hanging). The real list-only
    /// path is exercised end-to-end against the mock ACP agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_only_against_dead_child_settles() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(StartParams {
            bootstrap: Bootstrap::ListOnly,
            ..dummy_start_params("ded-list", tx)
        });

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }

    /// Regression for the "LLM responses don't show" bug: awaiting a
    /// long-running request inline inside the select! arm blocks the
    /// event-forwarding arm on the same loop, starving transcript +
    /// permission-request fanout. The fix detaches the request into
    /// its own `tokio::spawn` so the loop keeps polling
    /// `client_events_rx`. This test models the select!'s contract on
    /// pure channels (no real ACP connection needed).
    #[tokio::test(start_paused = true)]
    async fn select_loop_pumps_events_while_request_outstanding() {
        use tokio::sync::{mpsc, oneshot};

        enum Cmd {
            Request { reply: oneshot::Sender<()> },
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
        let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<&'static str>();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel::<&'static str>();

        let loop_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        let Some(cmd) = cmd else { break };
                        match cmd {
                            // Same shape as the fixed `Prompt` arm: spawn, do not await.
                            Cmd::Request { reply } => {
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                    let _ = reply.send(());
                                });
                            }
                        }
                    }
                    evt = evt_rx.recv() => {
                        let Some(evt) = evt else { break };
                        let _ = observed_tx.send(evt);
                    }
                }
            }
        });

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx.send(Cmd::Request { reply: reply_tx }).unwrap();
        evt_tx.send("mid-flight").unwrap();

        let observed = tokio::time::timeout(std::time::Duration::from_millis(50), observed_rx.recv())
            .await
            .expect("event forwarded while request outstanding")
            .expect("channel open");
        assert_eq!(observed, "mid-flight");

        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        let _ = reply_rx.await;
        drop(cmd_tx);
        drop(evt_tx);
        let _ = loop_handle.await;
    }

    /// `Bootstrap::Resume` against a child that dies before responding
    /// never leaks a partial session — the actor funnels through
    /// `InstanceState::Error`. The capability gate is a pre-connection
    /// check; integration coverage lives against the mock agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn resume_against_dead_child_reports_error() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(StartParams {
            bootstrap: Bootstrap::Resume("00000000-0000-0000-0000-000000000000".into()),
            ..dummy_start_params("ded-resume", tx)
        });

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }

    /// `TurnState::open` resets `output_observed` so a fresh turn
    /// always starts with no agent activity. Captures the invariant
    /// the `TurnGuard::complete` empty-turn heuristic relies on — if
    /// the flag persisted across turns, every turn after the first
    /// would skip the synthesized error.
    #[test]
    fn turn_state_open_resets_output_observed() {
        let mut state = TurnState::default();
        state.open("t-1".into());
        state.note_agent_output();
        assert!(state.output_observed());

        // Close the current turn, then open a fresh one.
        assert!(state.close_if_current("t-1"));
        state.open("t-2".into());
        assert!(
            !state.output_observed(),
            "open must reset output_observed; otherwise the empty-turn heuristic in TurnGuard::complete would skip the synthesized error on every turn after the first"
        );
    }

    /// `TurnGuard::complete` synthesizes an actionable `error` when
    /// the vendor returns `stop_reason: null` and no agent-output
    /// transcript items landed during the turn. Pins the contract the
    /// captain asked for: hyprpilot-nvim was rendering bare empty-
    /// turn closes as a generic "Internal error"; now the wire-side
    /// `error` field carries a specific message.
    #[tokio::test]
    async fn turn_guard_complete_synthesizes_error_on_empty_turn_with_null_stop_reason() {
        let mirror = std::sync::Arc::new(crate::adapters::InstanceMirror::new());
        let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<InstanceEvent>(16);
        let turn_state: SharedTurnState = std::sync::Arc::new(tokio::sync::RwLock::new(TurnState::default()));
        let guard = TurnGuard::new(
            "t-1".into(),
            "claude-code".into(),
            "i-1".into(),
            "s-1".into(),
            events_tx,
            mirror,
            turn_state,
        )
        .await;
        // Drain the TurnStarted emission so the next recv() picks up
        // TurnEnded specifically.
        let _ = events_rx.recv().await.expect("TurnStarted emitted");

        let emitted = guard.complete(None, None).await;
        assert!(emitted, "guard must own the slot at complete-time");

        let evt = events_rx.recv().await.expect("TurnEnded emitted");
        match evt {
            InstanceEvent::TurnEnded { stop_reason, error, .. } => {
                assert_eq!(stop_reason, None);
                let msg = error.expect("error synthesized on empty turn");
                assert!(
                    msg.contains("without emitting any output"),
                    "error message should describe the empty-turn case; got: {msg}"
                );
            }
            other => panic!("expected TurnEnded; got {other:?}"),
        }
    }

    /// Inverse of the empty-turn synthesis: when the dispatcher noted
    /// at least one agent-output transcript item, complete preserves
    /// the caller's `(stop_reason, error)` pair verbatim — no
    /// synthesis. Without this, a successful turn with a missing
    /// stop reason but real content would also get tagged as "empty."
    #[tokio::test]
    async fn turn_guard_complete_preserves_none_error_when_output_observed() {
        let mirror = std::sync::Arc::new(crate::adapters::InstanceMirror::new());
        let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<InstanceEvent>(16);
        let turn_state: SharedTurnState = std::sync::Arc::new(tokio::sync::RwLock::new(TurnState::default()));
        let guard = TurnGuard::new(
            "t-1".into(),
            "claude-code".into(),
            "i-1".into(),
            "s-1".into(),
            events_tx,
            mirror,
            turn_state.clone(),
        )
        .await;
        let _ = events_rx.recv().await.expect("TurnStarted emitted");

        // Simulate dispatcher noting an agent output for this turn.
        turn_state.write().await.note_agent_output();

        guard.complete(None, None).await;
        let evt = events_rx.recv().await.expect("TurnEnded emitted");
        match evt {
            InstanceEvent::TurnEnded { stop_reason, error, .. } => {
                assert_eq!(stop_reason, None);
                assert!(
                    error.is_none(),
                    "no synthesis when output was observed; got error={error:?}"
                );
            }
            other => panic!("expected TurnEnded; got {other:?}"),
        }
    }

    /// When the caller already supplies an explicit `error` (e.g. the
    /// transport returned an error), the empty-turn heuristic must
    /// NOT overwrite it — the caller's message is the real signal,
    /// the empty-turn synthesis is the fallback.
    #[tokio::test]
    async fn turn_guard_complete_keeps_callers_error_intact() {
        let mirror = std::sync::Arc::new(crate::adapters::InstanceMirror::new());
        let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<InstanceEvent>(16);
        let turn_state: SharedTurnState = std::sync::Arc::new(tokio::sync::RwLock::new(TurnState::default()));
        let guard = TurnGuard::new(
            "t-1".into(),
            "claude-code".into(),
            "i-1".into(),
            "s-1".into(),
            events_tx,
            mirror,
            turn_state,
        )
        .await;
        let _ = events_rx.recv().await.expect("TurnStarted emitted");

        guard.complete(None, Some("transport closed".into())).await;
        let evt = events_rx.recv().await.expect("TurnEnded emitted");
        match evt {
            InstanceEvent::TurnEnded { error, .. } => {
                assert_eq!(error.as_deref(), Some("transport closed"));
            }
            other => panic!("expected TurnEnded; got {other:?}"),
        }
    }
}
