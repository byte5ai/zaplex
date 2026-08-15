use warpui::{
    elements::{MouseStateHandle, Text},
    Element,
};

use crate::{appearance::Appearance, terminal::view::TerminalAction};

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerContent,
    InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};

#[derive(Clone, Copy, Debug)]
pub enum CLIAgentRestoreBannerAction {
    Resume,
    KeepShell,
}

#[derive(Default)]
pub struct CLIAgentRestoreBannerMouseStates {
    pub resume: MouseStateHandle,
    pub keep_shell: MouseStateHandle,
}

pub struct CLIAgentRestoreBannerState {
    pub banner_id: super::super::InlineBannerId,
    pub provider: String,
    pub session_id: String,
    pub cwd: String,
    pub account: Option<String>,
    pub mouse_states: CLIAgentRestoreBannerMouseStates,
}

pub fn render_cli_agent_restore_banner(
    state: &CLIAgentRestoreBannerState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let text_color = appearance.theme().active_ui_text_color().into_solid();
    let short_session_id: String = state.session_id.chars().take(12).collect();
    let account = state
        .account
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| crate::t!("terminal-cli-agent-restore-default-account"));
    let route = Text::new(
        crate::t!(
            "terminal-cli-agent-restore-route",
            account = account,
            cwd = state.cwd.as_str()
        ),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.0,
    )
    .with_color(appearance.theme().nonactive_ui_text_color().into_solid())
    .soft_wrap(true);
    render_inline_block_list_banner(
        InlineBannerStyle::CallToAction,
        appearance,
        InlineBannerContent {
            title: crate::t!(
                "terminal-cli-agent-restore-title",
                provider = state.provider.as_str(),
                session = short_session_id.as_str()
            ),
            content: Some(vec![route]),
            buttons: vec![
                InlineBannerTextButton {
                    text: crate::t!("terminal-cli-agent-restore-resume"),
                    text_color,
                    button_state: InlineBannerButtonState {
                        on_click_event: TerminalAction::CLIAgentRestoreBanner(
                            CLIAgentRestoreBannerAction::Resume,
                        ),
                        mouse_state_handle: state.mouse_states.resume.clone(),
                    },
                    font: Default::default(),
                    position_id: None,
                    variant: InlineBannerTextButtonVariant::Primary,
                },
                InlineBannerTextButton {
                    text: crate::t!("terminal-cli-agent-restore-keep-shell"),
                    text_color,
                    button_state: InlineBannerButtonState {
                        on_click_event: TerminalAction::CLIAgentRestoreBanner(
                            CLIAgentRestoreBannerAction::KeepShell,
                        ),
                        mouse_state_handle: state.mouse_states.keep_shell.clone(),
                    },
                    font: Default::default(),
                    position_id: None,
                    variant: InlineBannerTextButtonVariant::Secondary,
                },
            ],
            vertical_align_title_content: true,
            ..Default::default()
        },
    )
}
