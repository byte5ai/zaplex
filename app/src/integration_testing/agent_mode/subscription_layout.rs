use std::path::PathBuf;

use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use warpui::{App, SingletonEntity as _, WindowId};

use crate::{
    ai::{
        blocklist::agent_view::agent_input_footer::{
            subscription_action_position_id, subscription_identity_position_id,
        },
        subscription_agent::{
            AccountIdentity, AgentLifecycle, HostIdentity, InstallationIdentity, ModelCapability,
            SessionIdentity, SubscriptionAgent, SubscriptionSessionRegistry, SubscriptionTarget,
        },
    },
    integration_testing::view_getters::single_terminal_view_for_tab,
    BlocklistAIHistoryModel,
};

const ACTION_NAMES: &[&str] = &[
    "settings",
    "allow",
    "allow_for_session",
    "deny",
    "cancel",
    "retry",
    "resume",
    "restart",
    "end",
    "new_conversation",
    "back_to_shell",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionAgentLayoutState {
    NoAgentInstalled,
    NotSignedIn,
    SelectionRequired,
    Ready,
    Starting,
    Responding,
    RunningTool,
    WaitingForApproval,
    TurnCompleted,
    SessionEnded,
    RecoverableErrorWithSession,
    RecoverableErrorWithoutSession,
}

impl SubscriptionAgentLayoutState {
    pub const ALL: [Self; 12] = [
        Self::NoAgentInstalled,
        Self::NotSignedIn,
        Self::SelectionRequired,
        Self::Ready,
        Self::Starting,
        Self::Responding,
        Self::RunningTool,
        Self::WaitingForApproval,
        Self::TurnCompleted,
        Self::SessionEnded,
        Self::RecoverableErrorWithSession,
        Self::RecoverableErrorWithoutSession,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::NoAgentInstalled => "no_agent_installed",
            Self::NotSignedIn => "not_signed_in",
            Self::SelectionRequired => "selection_required",
            Self::Ready => "ready",
            Self::Starting => "starting",
            Self::Responding => "responding",
            Self::RunningTool => "running_tool",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::TurnCompleted => "turn_completed",
            Self::SessionEnded => "session_ended",
            Self::RecoverableErrorWithSession => "recoverable_error_with_session",
            Self::RecoverableErrorWithoutSession => "recoverable_error_without_session",
        }
    }

    pub fn expected_actions(self) -> &'static [&'static str] {
        match self {
            Self::NoAgentInstalled | Self::NotSignedIn => &["settings", "back_to_shell"],
            Self::SelectionRequired
            | Self::Ready
            | Self::Starting
            | Self::Responding
            | Self::RunningTool => &[],
            Self::WaitingForApproval => &["allow", "allow_for_session", "deny", "cancel"],
            Self::TurnCompleted | Self::RecoverableErrorWithSession => {
                &["resume", "restart", "end"]
            }
            Self::SessionEnded => &["new_conversation", "back_to_shell"],
            Self::RecoverableErrorWithoutSession => &["retry", "new_conversation", "back_to_shell"],
        }
    }

    fn lifecycle(self) -> AgentLifecycle {
        match self {
            Self::NoAgentInstalled => AgentLifecycle::NoAgentInstalled,
            Self::NotSignedIn => AgentLifecycle::NotSignedIn {
                agent: SubscriptionAgent::Codex,
            },
            Self::SelectionRequired => AgentLifecycle::SelectionRequired,
            Self::Ready => AgentLifecycle::Ready,
            Self::Starting => AgentLifecycle::Starting,
            Self::Responding => AgentLifecycle::Responding,
            Self::RunningTool => AgentLifecycle::RunningTool {
                name: "terminal command with a deliberately long descriptive name".to_string(),
            },
            Self::WaitingForApproval => AgentLifecycle::WaitingForApproval {
                request_id: "layout-evidence-approval".to_string(),
            },
            Self::TurnCompleted => AgentLifecycle::TurnCompleted {
                session: test_session(),
            },
            Self::SessionEnded => AgentLifecycle::SessionEnded,
            Self::RecoverableErrorWithSession => AgentLifecycle::RecoverableError {
                message: "The remote agent connection closed unexpectedly after acknowledging the previous output boundary. The recorded session remains resumable.".to_string(),
                session: Some(test_session()),
            },
            Self::RecoverableErrorWithoutSession => AgentLifecycle::RecoverableError {
                message: "Capability discovery timed out before the agent reported a session."
                    .to_string(),
                session: None,
            },
        }
    }

    fn target_is_known(self) -> bool {
        matches!(
            self,
            Self::Ready
                | Self::Starting
                | Self::Responding
                | Self::RunningTool
                | Self::WaitingForApproval
                | Self::TurnCompleted
                | Self::RecoverableErrorWithSession
        )
    }

    fn stored_session(self) -> Option<SessionIdentity> {
        matches!(
            self,
            Self::Responding
                | Self::RunningTool
                | Self::WaitingForApproval
                | Self::TurnCompleted
                | Self::RecoverableErrorWithSession
        )
        .then(test_session)
    }
}

#[derive(Clone, Debug)]
pub struct SubscriptionAgentActionLayoutPosition {
    pub name: &'static str,
    pub bounds: RectF,
}

#[derive(Clone, Debug)]
pub struct SubscriptionAgentLayoutSnapshot {
    pub window_size: Vector2F,
    pub input: Option<RectF>,
    pub composer: Option<RectF>,
    pub footer: Option<RectF>,
    pub identity: Option<RectF>,
    pub actions: Vec<SubscriptionAgentActionLayoutPosition>,
}

pub fn set_subscription_agent_layout_state(
    app: &mut App,
    window_id: WindowId,
    state: SubscriptionAgentLayoutState,
) {
    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
    let terminal_view_id = terminal_view.id();
    let conversation_id = BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .active_conversation(terminal_view_id)
            .expect("Agent View should have an active conversation")
            .id()
            .to_string()
    });
    let registry =
        SubscriptionSessionRegistry::handle(app).read(app, |registry, _| registry.clone());

    registry.remove(&conversation_id);
    if state.target_is_known() {
        let target = test_target();
        registry.set_target(conversation_id.clone(), target.clone());
        if let Some(session) = state.stored_session() {
            registry.store(conversation_id.clone(), target, session);
        }
    }
    registry.set_lifecycle(conversation_id, state.lifecycle());

    let input = terminal_view.read(app, |terminal_view, _| terminal_view.input().clone());
    let footer = input.read(app, |input, _| input.agent_input_footer().clone());
    footer.update(app, |_, ctx| ctx.notify());
    input.update(app, |_, ctx| ctx.notify());
    terminal_view.update(app, |_, ctx| ctx.notify());
}

pub fn subscription_agent_layout_snapshot(
    app: &mut App,
    window_id: WindowId,
) -> Option<SubscriptionAgentLayoutSnapshot> {
    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
    let terminal_view_id = terminal_view.id();
    let conversation_id = BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .active_conversation(terminal_view_id)
            .map(|conversation| conversation.id().to_string())
    })?;
    let input = terminal_view.read(app, |terminal_view, _| terminal_view.input().clone());
    let (input_id, composer_id, footer_id) = input.read(app, |input, _| {
        (
            input.save_position_id(),
            input.subscription_composer_save_position_id(),
            input.prompt_save_position_id(),
        )
    });
    let window_size = app.window_bounds(&window_id)?.size();
    let presenter = app.presenter(window_id)?;
    let presenter = presenter.borrow();
    let positions = presenter.position_cache();

    Some(SubscriptionAgentLayoutSnapshot {
        window_size,
        input: positions.get_position(&input_id),
        composer: positions.get_position(&composer_id),
        footer: positions.get_position(&footer_id),
        identity: positions.get_position(&subscription_identity_position_id(&conversation_id)),
        actions: ACTION_NAMES
            .iter()
            .copied()
            .filter_map(|name| {
                positions
                    .get_position(&subscription_action_position_id(&conversation_id, name))
                    .map(|bounds| SubscriptionAgentActionLayoutPosition { name, bounds })
            })
            .collect(),
    })
}

fn test_session() -> SessionIdentity {
    SessionIdentity::Codex("0199-layout-evidence-session-identifier".to_string())
}

fn test_target() -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent: SubscriptionAgent::Codex,
            host: HostIdentity {
                id: "remote-development-host".to_string(),
                display_name: "Remote development host with a long display name".to_string(),
            },
            account: AccountIdentity {
                id: "engineering-account".to_string(),
                display_name: "Engineering account with a long display name".to_string(),
                provider_account_id: Some("provider-account-layout-evidence".to_string()),
                config_dir: Some(PathBuf::from("/Users/test/.codex-engineering")),
            },
            executable: PathBuf::from("/usr/local/bin/codex"),
            version: "layout-evidence".to_string(),
        },
        working_directory: PathBuf::from(
            "/Users/test/projects/a-very-long-monorepo-name/packages/application",
        ),
        model: ModelCapability {
            id: "gpt-5".to_string(),
            display_name: "GPT-5 with a long display name".to_string(),
            description: None,
            resolved_model: Some("gpt-5-2026-08-07".to_string()),
            is_default: true,
            supported_efforts: Vec::new(),
            default_effort: Some("high".to_string()),
            context_window: Some(400_000),
        },
        effort: Some("high".to_string()),
    }
}
