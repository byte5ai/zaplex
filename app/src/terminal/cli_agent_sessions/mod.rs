pub mod event;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod hook_bridge;
pub mod listener;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod plugin_manager;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::blocklist::InputConfig;

use self::listener::CLIAgentSessionListener;
use super::CLIAgent;
use event::{CLIAgentEvent, CLIAgentEventType};

/// Status of a tracked CLI agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CLIAgentSessionStatus {
    InProgress,
    Success,
    Blocked { message: Option<String> },
}

impl CLIAgentSessionStatus {
    /// Whether this state requires a human response and may drive calm attention UI.
    pub fn needs_attention(&self) -> bool {
        matches!(self, CLIAgentSessionStatus::Blocked { .. })
    }

    pub fn to_conversation_status(&self) -> crate::ai::agent::conversation::ConversationStatus {
        use crate::ai::agent::conversation::ConversationStatus;
        match self {
            CLIAgentSessionStatus::InProgress => ConversationStatus::InProgress,
            CLIAgentSessionStatus::Success => ConversationStatus::Success,
            CLIAgentSessionStatus::Blocked { message } => ConversationStatus::Blocked {
                blocked_action: message.clone().unwrap_or_default(),
            },
        }
    }
}

/// Rich context accumulated from CLI agent session events.
#[derive(Debug, Clone, Default)]
pub struct CLIAgentSessionContext {
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub summary: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
}

/// State of the rich input editor for composing a prompt to send to a CLI agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentInputState {
    /// The rich input editor is not open.
    Closed,
    /// The rich input editor is open.
    Open {
        /// How this session was opened (for telemetry).
        entrypoint: CLIAgentInputEntrypoint,
        /// The input config that was active before opening rich input.
        previous_input_config: InputConfig,
        /// Whether the previous lock state was established while the input buffer was empty.
        previous_was_lock_set_with_empty_buffer: bool,
    },
}

/// Why the CLI agent rich input was closed (for telemetry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentRichInputCloseReason {
    /// User explicitly closed (Escape, Ctrl-G, footer button).
    Manual,
    /// Auto-closed due to agent status change (e.g. Blocked).
    AutoToggle,
    /// Auto-dismissed after submitting a prompt.
    Submit,
    /// Closed for another reason (chip removed, session ended, shared session sync).
    Other,
}

/// How a [`CLIAgentInputState`] was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentInputEntrypoint {
    /// User pressed Ctrl-G while a CLI agent was active.
    CtrlG,
    /// User clicked the rich input button in the CLI agent footer.
    FooterButton,
    /// Automatically opened when the CLI agent resumed work (left a blocked state)
    /// and the auto-show setting is enabled.
    AutoShow,
    /// Rich input was opened to mirror a shared-session participant's state.
    SharedSessionSync,
}

impl CLIAgentSessionContext {
    pub(crate) fn display_title(&self) -> Option<String> {
        self.latest_user_prompt().or_else(|| self.title_like_text())
    }

    pub(crate) fn latest_user_prompt(&self) -> Option<String> {
        self.query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_owned)
    }

    /// Returns summary text suitable as a fallback title when no user prompt is available.
    pub(crate) fn title_like_text(&self) -> Option<String> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
    }
}

/// A tracked CLI agent session.
#[derive(Debug, Clone)]
pub struct CLIAgentSession {
    pub agent: CLIAgent,
    pub status: CLIAgentSessionStatus,
    pub session_context: CLIAgentSessionContext,
    /// Rich input editor state.
    pub input_state: CLIAgentInputState,
    /// Whether status-driven auto-toggle is enabled for this session.
    pub should_auto_toggle_input: bool,
    /// Structured/legacy event listener, if an event source is connected.
    /// `None` for sessions created by command detection alone.
    /// Dropping this handle cleans up the listener's PTY event subscription.
    pub listener: Option<ModelHandle<CLIAgentSessionListener>>,
    /// The plugin version reported by the `SessionStart` event.
    /// `None` if the plugin predates version reporting or hasn't connected yet.
    pub plugin_version: Option<String>,
    /// `None` when the session is local.
    /// `Some("user@hostname")` when running over SSH (zaplexified or legacy).
    /// Used as a key for per-host plugin install failure tracking.
    pub remote_host: Option<String>,
    /// Draft text saved from the rich input composer when it was closed.
    /// Restored into the editor when the composer is reopened.
    pub draft_text: Option<String>,
    /// When the session was detected via a custom toolbar command pattern,
    /// the first word of the command (the binary/alias the user typed).
    /// Used to customize plugin instructions and force manual install mode.
    pub custom_command_prefix: Option<String>,
}

impl CLIAgentSession {
    pub fn is_remote(&self) -> bool {
        self.remote_host.is_some()
    }

    /// Applies an event to this session, updating context and status.
    /// Returns the new status if it changed, or `None` if the event was irrelevant.
    fn apply_event(&mut self, event: &CLIAgentEvent) -> Option<CLIAgentSessionStatus> {
        self.session_context.cwd = event.cwd.clone().or(self.session_context.cwd.take());
        self.session_context.project = event
            .project
            .clone()
            .or(self.session_context.project.take());
        self.session_context.session_id = event
            .session_id
            .clone()
            .or(self.session_context.session_id.take());

        let new_status = match &event.event {
            CLIAgentEventType::PreToolUse => CLIAgentSessionStatus::InProgress,
            CLIAgentEventType::PromptSubmit => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = None;
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::ToolComplete => CLIAgentSessionStatus::InProgress,
            CLIAgentEventType::Stop => {
                if event.payload.query.is_some() {
                    self.session_context.query = event.payload.query.clone();
                }
                if event.payload.response.is_some() {
                    self.session_context.response = event.payload.response.clone();
                }
                CLIAgentSessionStatus::Success
            }
            CLIAgentEventType::PermissionRequest => {
                self.session_context.summary = event.payload.summary.clone();
                self.session_context.tool_name = event.payload.tool_name.clone();
                self.session_context.tool_input_preview = event.payload.tool_input_preview.clone();
                CLIAgentSessionStatus::Blocked {
                    message: event.payload.summary.clone(),
                }
            }
            CLIAgentEventType::QuestionAsked => CLIAgentSessionStatus::Blocked {
                message: event
                    .payload
                    .summary
                    .clone()
                    .or_else(|| Some("Waiting for your answer".to_owned())),
            },
            CLIAgentEventType::PermissionReplied => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                CLIAgentSessionStatus::InProgress
            }
            // IdlePrompt means the agent is sitting at its prompt waiting for input.
            // This should not affect status — otherwise it would override Success after a Stop event.
            CLIAgentEventType::IdlePrompt => return None,
            CLIAgentEventType::SessionStart => {
                self.plugin_version = event
                    .payload
                    .plugin_version
                    .clone()
                    .or(self.plugin_version.take());
                return None;
            }
            CLIAgentEventType::SessionEnd => return None,
            CLIAgentEventType::Unknown(_) => return None,
        };

        self.status = new_status.clone();
        Some(new_status)
    }
}

/// Account identity bound to a terminal independently of CLI session detection.
///
/// `config_dir` selects the account route used to start or resume the agent.
/// `account_email` is the durable identity coordinate used to distinguish
/// conversations whose provider-local session ids were copied between accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CLIAgentAccountIdentity {
    agent: CLIAgent,
    pub config_dir: Option<String>,
    pub account_email: Option<String>,
}

impl CLIAgentAccountIdentity {
    pub fn agent(&self) -> CLIAgent {
        self.agent
    }
}

/// Durable account route for restoring a provider-owned CLI agent session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersistedCLIAgentAccount {
    pub config_dir: Option<String>,
    pub email: Option<String>,
}

/// Provider-owned session coordinates persisted with a local terminal pane.
///
/// This intentionally excludes remote sessions: the remote daemon owns their
/// process and PTY lifetime, while this binding exists to resume a local CLI
/// process after the app has restored its shell pane.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersistedCLIAgentBinding {
    pub provider: CLIAgent,
    pub session_id: String,
    pub cwd: String,
    pub account: Option<PersistedCLIAgentAccount>,
}

impl PersistedCLIAgentBinding {
    /// Builds the provider's pinned in-place resume command.
    ///
    /// Persisted data is treated as untrusted input: empty or control-bearing
    /// coordinates fail closed instead of reaching a shell command line.
    pub fn resume_command(&self) -> Option<String> {
        if !is_valid_persisted_coordinate(&self.session_id)
            || !is_valid_persisted_coordinate(&self.cwd)
            || self
                .account
                .as_ref()
                .and_then(|account| account.config_dir.as_deref())
                .is_some_and(|config_dir| !is_valid_persisted_coordinate(config_dir))
        {
            return None;
        }

        let config_dir = self
            .account
            .as_ref()
            .and_then(|account| account.config_dir.as_deref())
            .map(Path::new);
        self.provider
            .resume_command_pinned(&self.session_id, config_dir)
    }
}

fn is_valid_persisted_coordinate(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

/// Events emitted by `CLIAgentSessionsModel` for subscribers (e.g., `AgentNotificationsModel`).
#[allow(dead_code)] // `agent` fields on Started/InputSessionChanged/Ended are used for logging and future subscribers.
#[derive(Debug, Clone)]
pub enum CLIAgentSessionsModelEvent {
    Started {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    StatusChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        status: CLIAgentSessionStatus,
        session_context: Box<CLIAgentSessionContext>,
    },
    InputSessionChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        /// The input state BEFORE this change. When transitioning from
        /// `Open` → `Closed`, contains the saved input config to restore.
        previous_input_state: CLIAgentInputState,
        /// The input state AFTER this change.
        new_input_state: CLIAgentInputState,
    },
    Ended {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    /// The agent session has been updated. Subscribers may use this as a trigger for best-effort
    /// saving of state derived from the agent's session.
    SessionUpdated {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
}

impl CLIAgentSessionsModelEvent {
    pub fn terminal_view_id(&self) -> EntityId {
        match self {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id, ..
            } => *terminal_view_id,
        }
    }
}

/// Singleton model that tracks pane-scoped CLI agent state and plugin-enriched session context.
pub struct CLIAgentSessionsModel {
    sessions: HashMap<EntityId, CLIAgentSession>,
    account_identities: HashMap<EntityId, CLIAgentAccountIdentity>,
    pending_restores: HashMap<EntityId, PersistedCLIAgentBinding>,
    /// Tracks (agent, remote_host) pairs where an auto plugin operation (install or update) has failed.
    /// Shared across all views so failure in one tab is reflected everywhere.
    plugin_auto_failures: HashSet<(CLIAgent, Option<String>)>,
}

impl Entity for CLIAgentSessionsModel {
    type Event = CLIAgentSessionsModelEvent;
}

impl SingletonEntity for CLIAgentSessionsModel {}

impl CLIAgentSessionsModel {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            account_identities: HashMap::new(),
            pending_restores: HashMap::new(),
            plugin_auto_failures: HashSet::new(),
        }
    }

    pub fn session(&self, terminal_view_id: EntityId) -> Option<&CLIAgentSession> {
        self.sessions.get(&terminal_view_id)
    }

    pub fn terminal_needs_attention(&self, terminal_view_id: EntityId) -> bool {
        self.sessions
            .get(&terminal_view_id)
            .is_some_and(|session| session.status.needs_attention())
    }

    /// Returns whether a live session has proven that the agent's structured
    /// status channel is active. This is intentionally stronger than checking
    /// whether a hook file exists: every managed native hook supplies a stable
    /// session identity, while legacy command detection does not.
    pub fn has_rich_status_session_for_agent(&self, agent: CLIAgent) -> bool {
        self.sessions.values().any(|session| {
            session.agent == agent
                && session.session_context.session_id.is_some()
                && listener::session_supports_rich_status(session)
        })
    }

    pub fn account_identity(&self, terminal_view_id: EntityId) -> Option<&CLIAgentAccountIdentity> {
        self.account_identities.get(&terminal_view_id)
    }

    /// Captures the complete local resume identity for persistence.
    pub fn local_binding_for_restore(
        &self,
        terminal_view_id: EntityId,
        fallback_cwd: Option<&str>,
    ) -> Option<PersistedCLIAgentBinding> {
        let Some(session) = self.sessions.get(&terminal_view_id) else {
            return self.pending_restores.get(&terminal_view_id).cloned();
        };
        if session.is_remote() {
            return None;
        }

        let Some(session_id) = session.session_context.session_id.clone() else {
            return self.pending_restores.get(&terminal_view_id).cloned();
        };
        let cwd = session
            .session_context
            .cwd
            .as_deref()
            .filter(|cwd| is_valid_persisted_coordinate(cwd))
            .or_else(|| fallback_cwd.filter(|cwd| is_valid_persisted_coordinate(cwd)))?
            .to_owned();

        let account = match self.account_identities.get(&terminal_view_id) {
            Some(identity) if identity.agent == session.agent => (identity.config_dir.is_some()
                || identity.account_email.is_some())
            .then(|| PersistedCLIAgentAccount {
                config_dir: identity.config_dir.clone(),
                email: identity.account_email.clone(),
            }),
            Some(_) => return None,
            None => None,
        };

        let binding = PersistedCLIAgentBinding {
            provider: session.agent,
            session_id,
            cwd,
            account,
        };
        binding.resume_command()?;
        Some(binding)
    }

    pub fn register_pending_restore(
        &mut self,
        terminal_view_id: EntityId,
        binding: PersistedCLIAgentBinding,
    ) -> bool {
        if binding.resume_command().is_none() {
            return false;
        }
        self.pending_restores.insert(terminal_view_id, binding);
        true
    }

    pub fn pending_restore(&self, terminal_view_id: EntityId) -> Option<&PersistedCLIAgentBinding> {
        self.pending_restores.get(&terminal_view_id)
    }

    pub fn take_pending_restore(
        &mut self,
        terminal_view_id: EntityId,
    ) -> Option<PersistedCLIAgentBinding> {
        self.pending_restores.remove(&terminal_view_id)
    }

    /// Prepares the one shared resume path used by prompt and automatic
    /// restoration. The pending binding stays registered until native session
    /// detection proves that the provider process actually resumed.
    pub fn prepare_pending_restore(&mut self, terminal_view_id: EntityId) -> Option<String> {
        let binding = self.pending_restores.get(&terminal_view_id)?.clone();
        let command = binding.resume_command()?;
        let (config_dir, account_email) = binding
            .account
            .as_ref()
            .map(|account| (account.config_dir.clone(), account.email.clone()))
            .unwrap_or((None, None));
        self.bind_account_identity(
            terminal_view_id,
            binding.provider,
            config_dir,
            account_email,
        );
        Some(command)
    }

    /// Binds the account selected for a started, resumed, forked, or adopted
    /// agent before command detection can report its CLI session.
    pub fn bind_account_identity(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        config_dir: Option<String>,
        account_email: Option<String>,
    ) {
        self.account_identities.insert(
            terminal_view_id,
            CLIAgentAccountIdentity {
                agent,
                config_dir,
                account_email,
            },
        );
    }

    pub fn unbind_account_identity(&mut self, terminal_view_id: EntityId) {
        self.account_identities.remove(&terminal_view_id);
    }

    pub fn account_identity_matches(
        &self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        config_dir: Option<&str>,
        account_email: Option<&str>,
    ) -> bool {
        match self.account_identities.get(&terminal_view_id) {
            Some(identity) => {
                identity.agent == agent
                    && identity.config_dir.as_deref() == config_dir
                    && identity.account_email.as_deref() == account_email
            }
            None => config_dir.is_none() && account_email.is_none(),
        }
    }

    /// Finds the terminal currently hosting one provider-owned agent session.
    /// Session ids are only meaningful within their provider, so both values
    /// must match. Remote panes are intentionally included: callers use this to
    /// focus an already-open session instead of starting a duplicate resume.
    /// `matches_terminal` supplies the host check because the same conversation
    /// id may exist locally and on several remote hosts after a copy.
    pub fn terminal_view_id_for_agent_session_matching(
        &self,
        agent: CLIAgent,
        session_id: &str,
        config_dir: Option<&str>,
        account_email: Option<&str>,
        mut matches_terminal: impl FnMut(EntityId, &CLIAgentSession) -> bool,
    ) -> Option<EntityId> {
        let mut matches = self
            .sessions
            .iter()
            .filter_map(|(terminal_view_id, session)| {
                (session.agent == agent
                    && session.session_context.session_id.as_deref() == Some(session_id)
                    && self.account_identity_matches(
                        *terminal_view_id,
                        agent,
                        config_dir,
                        account_email,
                    )
                    && matches_terminal(*terminal_view_id, session))
                .then_some(*terminal_view_id)
            });
        let matched = matches.next()?;
        matches.next().is_none().then_some(matched)
    }

    /// Returns `true` if the rich input editor is currently open for this terminal.
    pub fn is_input_open(&self, terminal_view_id: EntityId) -> bool {
        self.sessions
            .get(&terminal_view_id)
            .is_some_and(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
    }

    /// Registers an event listener on the session for this terminal.
    ///
    /// If a session for the same agent already exists (e.g. created earlier by
    /// command detection), it is upgraded with the listener and plugin context.
    /// Otherwise a new session is created.
    ///
    /// The optional `cwd` / `project` / `session_id` fields supply initial
    /// context when available (e.g. from a `SessionStart` event). Passing
    /// `None` for all three is fine — happens when the plugin is installed
    /// mid-session and there is no start event to extract context from.
    #[allow(clippy::too_many_arguments)]
    pub fn register_listener(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        cwd: Option<String>,
        project: Option<String>,
        session_id: Option<String>,
        plugin_version: Option<String>,
        remote_host: Option<String>,
        should_auto_toggle_input: bool,
        listener: ModelHandle<CLIAgentSessionListener>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .get_mut(&terminal_view_id)
            .filter(|s| s.agent == agent)
        {
            // Upgrade existing session with plugin context.
            session.status = CLIAgentSessionStatus::InProgress;
            session.listener = Some(listener);
            session.plugin_version = plugin_version;
            session.remote_host = remote_host;
            session.should_auto_toggle_input = should_auto_toggle_input;
            session.session_context.cwd = cwd.or(session.session_context.cwd.take());
            session.session_context.project = project.or(session.session_context.project.take());
            session.session_context.session_id =
                session_id.or(session.session_context.session_id.take());
            return;
        }

        self.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext {
                    cwd,
                    project,
                    session_id,
                    ..Default::default()
                },
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input,
                listener: Some(listener),
                plugin_version,
                remote_host,
                draft_text: None,
                custom_command_prefix: None,
            },
            ctx,
        );
    }

    pub fn remove_session(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        self.account_identities.remove(&terminal_view_id);
        self.pending_restores.remove(&terminal_view_id);
        if let Some(session) = self.sessions.remove(&terminal_view_id) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: session.agent,
            });
        }
    }

    /// Ends the current local CLI process while retaining its durable session
    /// coordinates for a later app restore or explicit resume in this pane.
    pub fn end_session_preserving_restore(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let binding = self.local_binding_for_restore(terminal_view_id, None);
        self.account_identities.remove(&terminal_view_id);
        if let Some(session) = self.sessions.remove(&terminal_view_id) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: session.agent,
            });
        }
        match binding {
            Some(binding) => {
                self.pending_restores.insert(terminal_view_id, binding);
            }
            None => {
                self.pending_restores.remove(&terminal_view_id);
            }
        }
    }

    /// Updates the session's status and context from a parsed CLI agent event.
    pub fn update_from_event(
        &mut self,
        terminal_view_id: EntityId,
        event: &CLIAgentEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        let event_type = &event.event;
        if let Some(new_status) = session.apply_event(event) {
            let agent = session.agent;
            ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status: new_status,
                session_context: Box::new(session.session_context.clone()),
            });
        }

        if matches!(
            event_type,
            CLIAgentEventType::SessionStart
                | CLIAgentEventType::PreToolUse
                | CLIAgentEventType::PromptSubmit
                | CLIAgentEventType::ToolComplete
        ) {
            ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id,
                agent: session.agent,
            });
        }
    }

    pub fn open_input(
        &mut self,
        terminal_view_id: EntityId,
        entrypoint: CLIAgentInputEntrypoint,
        previous_input_config: InputConfig,
        previous_was_lock_set_with_empty_buffer: bool,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Open {
            entrypoint,
            previous_input_config,
            previous_was_lock_set_with_empty_buffer,
        };
        session.should_auto_toggle_input = should_auto_toggle_input;

        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: session.input_state,
        });
    }

    pub fn close_input(
        &mut self,
        terminal_view_id: EntityId,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if session.input_state == CLIAgentInputState::Closed {
            return;
        }

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Closed;
        session.should_auto_toggle_input = should_auto_toggle_input;
        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: CLIAgentInputState::Closed,
        });
    }

    pub fn set_session(
        &mut self,
        terminal_view_id: EntityId,
        session: CLIAgentSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let agent = session.agent;
        if self
            .pending_restores
            .get(&terminal_view_id)
            .is_some_and(|binding| {
                binding.provider != agent || session.session_context.session_id.is_some()
            })
        {
            self.pending_restores.remove(&terminal_view_id);
        }
        if self
            .account_identities
            .get(&terminal_view_id)
            .is_some_and(|identity| identity.agent != agent)
        {
            self.account_identities.remove(&terminal_view_id);
        }
        // Close any open rich input before replacing, so subscribers can
        // restore input config before the session ends.
        self.close_input(terminal_view_id, false, ctx);
        if let Some(old) = self.sessions.insert(terminal_view_id, session) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: old.agent,
            });
        }

        ctx.emit(CLIAgentSessionsModelEvent::Started {
            terminal_view_id,
            agent,
        });
    }

    /// Records that an auto plugin operation (install or update) failed for the given agent/host.
    /// `remote_host` is `None` for local sessions, `Some("user@hostname")` for remote.
    #[cfg(not(target_family = "wasm"))]
    pub fn record_plugin_auto_failure(&mut self, agent: CLIAgent, remote_host: Option<String>) {
        self.plugin_auto_failures.insert((agent, remote_host));
    }

    /// Saves draft text from the rich input composer for the given terminal.
    /// Stores `None` for empty or whitespace-only text.
    pub fn set_draft(&mut self, terminal_view_id: EntityId, text: String) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = if text.trim().is_empty() {
                None
            } else {
                Some(text)
            };
        }
    }

    /// Clears any saved draft text for the given terminal.
    pub fn clear_draft(&mut self, terminal_view_id: EntityId) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = None;
        }
    }

    /// Returns and clears the draft text for the given terminal, if any.
    pub fn take_draft(&mut self, terminal_view_id: EntityId) -> Option<String> {
        self.sessions
            .get_mut(&terminal_view_id)
            .and_then(|s| s.draft_text.take())
    }

    /// Whether an auto plugin operation has previously failed for this agent on this host.
    pub fn has_plugin_auto_failed(&self, agent: CLIAgent, remote_host: &Option<String>) -> bool {
        self.plugin_auto_failures
            .contains(&(agent, remote_host.clone()))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
