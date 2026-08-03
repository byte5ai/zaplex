use warpui::{EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{CLIAgentEvent, CLIAgentSession, CLIAgentSessionsModel};
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::event::{CLIAgentEventPayload, CLIAgentEventType};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::CLIAgent;
#[cfg(not(target_family = "wasm"))]
use crate::warp_managed_paths_watcher::{
    repository_update_touches_path, WarpManagedPathsWatcher, WarpManagedPathsWatcherEvent,
};
#[cfg(not(target_family = "wasm"))]
use serde::Deserialize;
#[cfg(not(target_family = "wasm"))]
use std::collections::HashSet;
#[cfg(not(target_family = "wasm"))]
use std::fs::File;
#[cfg(not(target_family = "wasm"))]
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

#[cfg(not(target_family = "wasm"))]
const MAX_CODEWHALE_AUDIT_TOOL_NAME_BYTES: usize = 512;
#[cfg(not(target_family = "wasm"))]
const MAX_CODEWHALE_AUDIT_TOOL_ID_BYTES: usize = 512;

#[cfg(not(target_family = "wasm"))]
#[derive(Deserialize)]
struct CodeWhaleToolAuditRecord {
    event: String,
    tool_id: Option<String>,
    tool_name: Option<String>,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Debug, PartialEq, Eq)]
enum CodeWhaleAuditUpdate {
    ApprovalRequired {
        tool_id: String,
        tool_name: Option<String>,
    },
    ApprovalDecided {
        tool_id: String,
    },
}

#[cfg(not(target_family = "wasm"))]
struct CodeWhaleAuditRead {
    next_offset: u64,
    updates: Vec<CodeWhaleAuditUpdate>,
}

#[cfg(not(target_family = "wasm"))]
fn codewhale_audit_update(line: &[u8]) -> Option<CodeWhaleAuditUpdate> {
    let record = serde_json::from_slice::<CodeWhaleToolAuditRecord>(line).ok()?;
    let tool_id = record.tool_id.filter(|tool_id| {
        !tool_id.is_empty() && tool_id.len() <= MAX_CODEWHALE_AUDIT_TOOL_ID_BYTES
    })?;
    if record.event == "tool.approval_required" {
        let tool_name = record
            .tool_name
            .filter(|name| name.len() <= MAX_CODEWHALE_AUDIT_TOOL_NAME_BYTES);
        Some(CodeWhaleAuditUpdate::ApprovalRequired { tool_id, tool_name })
    } else if record.event == "tool.approval_decision" {
        Some(CodeWhaleAuditUpdate::ApprovalDecided { tool_id })
    } else {
        None
    }
}

#[cfg(not(target_family = "wasm"))]
fn codewhale_permission_event(
    event: CLIAgentEventType,
    tool_name: Option<String>,
) -> CLIAgentEvent {
    let summary = (event == CLIAgentEventType::PermissionRequest)
        .then(|| "CodeWhale is waiting for approval".to_owned());
    CLIAgentEvent {
        v: 1,
        agent: CLIAgent::DeepSeek,
        event,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload {
            summary,
            tool_name,
            ..Default::default()
        },
    }
}

#[cfg(not(target_family = "wasm"))]
fn read_codewhale_audit_events(path: &Path, offset: u64) -> io::Result<CodeWhaleAuditRead> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let read_offset = if file_len < offset { 0 } else { offset };
    file.seek(SeekFrom::Start(read_offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let updates = bytes[..complete_len]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(codewhale_audit_update)
        .collect();
    Ok(CodeWhaleAuditRead {
        next_offset: read_offset + complete_len as u64,
        updates,
    })
}

#[cfg(not(target_family = "wasm"))]
fn codewhale_events_for_audit_updates(
    pending_approvals: &mut HashSet<String>,
    updates: Vec<CodeWhaleAuditUpdate>,
) -> Vec<CLIAgentEvent> {
    let decisions = updates
        .iter()
        .filter_map(|update| match update {
            CodeWhaleAuditUpdate::ApprovalDecided { tool_id } => Some(tool_id.clone()),
            CodeWhaleAuditUpdate::ApprovalRequired { .. } => None,
        })
        .collect::<HashSet<_>>();
    let mut events = Vec::new();
    for update in updates {
        match update {
            CodeWhaleAuditUpdate::ApprovalRequired { tool_id, tool_name } => {
                // A complete required+decided pair first observed in one
                // debounced file batch is already resolved. Publishing it
                // after a later Stop hook would resurrect stale Blocked state,
                // so only still-pending requests enter the model.
                if !decisions.contains(&tool_id) && pending_approvals.insert(tool_id) {
                    events.push(codewhale_permission_event(
                        CLIAgentEventType::PermissionRequest,
                        tool_name,
                    ));
                }
            }
            CodeWhaleAuditUpdate::ApprovalDecided { tool_id } => {
                if pending_approvals.remove(&tool_id) && pending_approvals.is_empty() {
                    events.push(codewhale_permission_event(
                        CLIAgentEventType::PermissionReplied,
                        None,
                    ));
                }
            }
        }
    }
    events
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListenerRegistrationAction {
    Register,
    Reuse,
    Reject,
}

pub(crate) fn listener_registration_action(
    existing_session: Option<(CLIAgent, bool)>,
    incoming_agent: CLIAgent,
) -> ListenerRegistrationAction {
    match existing_session {
        Some((existing_agent, true)) if existing_agent == incoming_agent => {
            ListenerRegistrationAction::Reuse
        }
        Some((_, true)) => ListenerRegistrationAction::Reject,
        Some((_, false)) | None => ListenerRegistrationAction::Register,
    }
}

/// Per-agent handler that filters and transforms parsed CLI agent events.
/// Each CLI agent can have a different implementation depending on which events
/// it cares about.
trait CLIAgentSessionHandler {
    /// Attempt to parse a raw `PluggableNotification` into a typed event.
    /// The default implementation delegates to the structured JSON parser
    /// (`parse_event`); agents with non-JSON notification formats (e.g. Codex
    /// OSC 9 plain text) should override this.
    fn try_parse(&self, title: Option<&str>, body: &str) -> Option<CLIAgentEvent> {
        parse_event(title, body)
    }

    /// Decide whether a parsed event should be forwarded to the sessions model.
    /// Returns the event (possibly transformed) if it should be processed.
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent>;

    /// Whether this handler provides meaningful, fine-grained status
    /// (e.g. in-progress / blocked / success) that should be shown in the UI.
    /// Handlers backed by the structured plugin protocol report rich status.
    /// A concrete session may still opt out when it has only legacy events.
    fn supports_rich_status(&self) -> bool {
        true
    }
}

/// Whether the listener for the given agent provides rich status.
/// Returns `false` for agents without a handler or whose handler opts out.
pub fn agent_supports_rich_status(agent: &CLIAgent) -> bool {
    create_handler(agent).is_some_and(|h| h.supports_rich_status())
}

/// Returns whether this concrete session has enough event context to render
/// fine-grained status in UI surfaces.
pub fn session_supports_rich_status(session: &CLIAgentSession) -> bool {
    if !agent_supports_rich_status(&session.agent) {
        return false;
    }

    // Codex, DeepSeek, Grok, and Antigravity have two listener paths:
    // - legacy OSC 9 completion notifications, registered from command detection,
    //   or command detection alone, with no session id or lifecycle events;
    // - structured OSC 777 hooks, which include the native hook session id.
    // Only the latter can drive rich status accurately.
    if matches!(
        session.agent,
        CLIAgent::Codex | CLIAgent::DeepSeek | CLIAgent::Grok | CLIAgent::Antigravity
    ) && session.session_context.session_id.is_none()
    {
        return false;
    }

    true
}

/// Returns `true` if the given CLI agent has a supported session handler.
pub fn is_agent_supported(agent: &CLIAgent) -> bool {
    matches!(
        agent,
        CLIAgent::Claude
            | CLIAgent::OpenCode
            | CLIAgent::Codex
            | CLIAgent::Gemini
            | CLIAgent::Auggie
            | CLIAgent::Pi
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::Grok
    )
}

/// Creates the appropriate handler for the given CLI agent.
fn create_handler(agent: &CLIAgent) -> Option<Box<dyn CLIAgentSessionHandler>> {
    match agent {
        // Auggie and Pi are supported via community-maintained plugins
        // (https://github.com/augmentmoogi/auggie-warp,
        // https://github.com/badlogic/pi-mono), which emit the same
        // structured OSC 777 events as the first-party Claude/OpenCode/Gemini
        // plugins. We don't ship install flows for them — we just listen.
        CLIAgent::Claude
        | CLIAgent::OpenCode
        | CLIAgent::Gemini
        | CLIAgent::Auggie
        | CLIAgent::Pi
        | CLIAgent::Antigravity => Some(Box::new(DefaultSessionListener)),
        CLIAgent::Codex => Some(Box::new(CodexSessionHandler)),
        CLIAgent::DeepSeek => Some(Box::new(DeepSeekSessionHandler)),
        CLIAgent::Grok => Some(Box::new(GrokSessionHandler)),
        CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Copilot
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::Unknown => None,
    }
}

/// Default handler shared by agents whose events need no special filtering
/// beyond skipping the initial `SessionStart`.
struct DefaultSessionListener;

impl CLIAgentSessionHandler for DefaultSessionListener {
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        // Skip session_start events (handled during listener construction)
        if event.event == CLIAgentEventType::SessionStart {
            return None;
        }

        Some(event)
    }
}

/// Codex-specific handler for structured hook events and legacy OSC 9
/// desktop notifications.
///
/// Codex sends notifications via OSC 9 (`\x1b]9;message\x07`) with
/// human-readable text. Since there's no way to distinguish notification types
/// from the raw text, only legacy OSC 9 notifications are treated as `Stop`.
/// The notification body becomes the event's `query` so it surfaces as the
/// notification title in the UI.
struct CodexSessionHandler;

impl CodexSessionHandler {
    /// Parse a plain-text OSC 9 notification body into a `CLIAgentEvent`.
    /// Returns `None` only for empty bodies.
    fn parse_osc9_text(body: &str) -> Option<CLIAgentEvent> {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }

        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Codex,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Some(body.to_owned()),
                ..Default::default()
            },
        })
    }
}

impl CLIAgentSessionHandler for CodexSessionHandler {
    /// Codex sends plain-text OSC 9 notifications (title = `None`) instead of
    /// the structured OSC 777 JSON used by Claude Code / OpenCode.
    fn try_parse(&self, title: Option<&str>, body: &str) -> Option<CLIAgentEvent> {
        // If the notification carries the structured sentinel, try the normal
        // JSON parser first (future-proofing in case Codex adds plugin
        // support later).
        if let Some(parsed) = parse_event(title, body) {
            return Some(parsed);
        }
        // OSC 9 notifications have no title.
        if title.is_some() {
            return None;
        }
        Self::parse_osc9_text(body)
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        (event.event != CLIAgentEventType::SessionStart).then_some(event)
    }
}

/// DeepSeek-TUI handler: listens for structured OSC 777 events and legacy
/// OSC 9 plain-text notifications.
/// DeepSeek-TUI emits `\x1b]9;deepseek: turn complete\x07` (optionally with
/// elapsed time and cost) when a turn finishes. Those legacy notifications are
/// treated as `Stop` events. Rich status is only available when DeepSeek hooks
/// emit structured OSC 777 events with a session id.
struct DeepSeekSessionHandler;

impl DeepSeekSessionHandler {
    fn notification_title_from_body(body: &str) -> Option<String> {
        let title = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| !line.starts_with("deepseek: turn complete"))
            .collect::<Vec<_>>()
            .join("\n");

        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    }
}

impl CLIAgentSessionHandler for DeepSeekSessionHandler {
    /// DeepSeek-TUI uses OSC 9 with no title (same channel as Codex).
    fn try_parse(&self, title: Option<&str>, body: &str) -> Option<CLIAgentEvent> {
        // Future-proof: try structured JSON first in case a plugin is added later.
        if let Some(parsed) = parse_event(title, body) {
            return Some(parsed);
        }
        // OSC 9 notifications have no title.
        if title.is_some() {
            return None;
        }
        let body = body.trim();
        if body.is_empty() {
            return None;
        }
        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::DeepSeek,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Self::notification_title_from_body(body),
                response: Some(body.to_owned()),
                ..Default::default()
            },
        })
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        (event.event != CLIAgentEventType::SessionStart).then_some(event)
    }

    fn supports_rich_status(&self) -> bool {
        true
    }
}

/// Grok handler for structured hook events and its legacy Warp OSC 9
/// notifications. Plain OSC 9 can prove turn completion but cannot distinguish
/// approval requests, so rich status requires a structured event with a
/// session id.
struct GrokSessionHandler;

impl GrokSessionHandler {
    fn parse_osc9_text(body: &str) -> Option<CLIAgentEvent> {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }
        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Grok,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Some(body.to_owned()),
                ..Default::default()
            },
        })
    }
}

impl CLIAgentSessionHandler for GrokSessionHandler {
    fn try_parse(&self, title: Option<&str>, body: &str) -> Option<CLIAgentEvent> {
        if let Some(parsed) = parse_event(title, body) {
            return Some(parsed);
        }
        if title.is_some() {
            return None;
        }
        Self::parse_osc9_text(body)
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        (event.event != CLIAgentEventType::SessionStart).then_some(event)
    }
}

/// Per-agent listener that subscribes to PTY events and forwards them to the
/// sessions model. Stored on [`super::CLIAgentSession`] so its lifetime is
/// tied to the session; dropping the handle cleans up the subscription.
pub struct CLIAgentSessionListener {
    terminal_view_id: EntityId,
    inner: Box<dyn CLIAgentSessionHandler>,
    #[cfg(not(target_family = "wasm"))]
    codewhale_audit_log_path: Option<PathBuf>,
    #[cfg(not(target_family = "wasm"))]
    codewhale_audit_offset: u64,
    #[cfg(not(target_family = "wasm"))]
    codewhale_pending_approvals: HashSet<String>,
}

impl warpui::Entity for CLIAgentSessionListener {
    type Event = ();
}

impl CLIAgentSessionListener {
    pub fn new(
        terminal_view_id: EntityId,
        agent: CLIAgent,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_with_codewhale_audit_log(
            terminal_view_id,
            agent,
            model_event_dispatcher,
            None,
            ctx,
        )
    }

    pub fn new_with_codewhale_audit_log(
        terminal_view_id: EntityId,
        agent: CLIAgent,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        codewhale_audit_log_path: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let handler =
            create_handler(&agent).expect("is_agent_supported must be checked before calling new");

        // Subscribe to subsequent OSC events from this terminal's PTY.
        // Parsing is delegated to the handler's `try_parse`; the handler's
        // `handle_event` then filters/transforms the result.
        ctx.subscribe_to_model(model_event_dispatcher, move |me, event, ctx| {
            if let ModelEvent::PluggableNotification { title, body } = event {
                let Some(parsed) = me.inner.try_parse(title.as_deref(), body) else {
                    return;
                };
                if let Some(event) = me.inner.handle_event(parsed) {
                    #[cfg(not(target_family = "wasm"))]
                    if matches!(
                        event.event,
                        CLIAgentEventType::Stop | CLIAgentEventType::SessionEnd
                    ) {
                        me.codewhale_pending_approvals.clear();
                    }
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
                        sessions_model.update_from_event(me.terminal_view_id, &event, ctx);
                    });
                }
            }
        });

        #[cfg(not(target_family = "wasm"))]
        if agent == CLIAgent::DeepSeek {
            if let Some(path) = codewhale_audit_log_path.as_ref() {
                let path = path.clone();
                ctx.subscribe_to_model(
                    &WarpManagedPathsWatcher::handle(ctx),
                    move |listener, event, ctx| match event {
                        WarpManagedPathsWatcherEvent::FilesChanged(update)
                            if repository_update_touches_path(update, &path) =>
                        {
                            listener.consume_codewhale_audit_events(ctx);
                        }
                        WarpManagedPathsWatcherEvent::FilesChanged(_) => {}
                    },
                );
            }
        }
        #[cfg(target_family = "wasm")]
        let _ = codewhale_audit_log_path;

        Self {
            terminal_view_id,
            inner: handler,
            #[cfg(not(target_family = "wasm"))]
            codewhale_audit_log_path,
            #[cfg(not(target_family = "wasm"))]
            codewhale_audit_offset: 0,
            #[cfg(not(target_family = "wasm"))]
            codewhale_pending_approvals: HashSet::new(),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn consume_codewhale_audit_events(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(path) = self.codewhale_audit_log_path.as_deref() else {
            return;
        };
        let batch = match read_codewhale_audit_events(path, self.codewhale_audit_offset) {
            Ok(batch) => batch,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => {
                log::warn!(
                    "Failed to read CodeWhale audit log {}: {error}",
                    path.display()
                );
                return;
            }
        };
        self.codewhale_audit_offset = batch.next_offset;
        if batch.updates.is_empty() {
            return;
        }
        let events = codewhale_events_for_audit_updates(
            &mut self.codewhale_pending_approvals,
            batch.updates,
        );
        if events.is_empty() {
            return;
        }
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
            for event in events {
                sessions_model.update_from_event(self.terminal_view_id, &event, ctx);
            }
        });
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
