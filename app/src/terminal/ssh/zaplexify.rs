use asset_macro::bundled_asset;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::ui::theme::WarpTheme;
use warpui::assets::asset_cache::{AssetCache, AssetState};

use crate::ai::blocklist::inline_action::requested_action::RenderableAction;
use crate::appearance::Appearance;
use crate::terminal::shell::ShellType;
use crate::terminal::zaplexify;
use crate::terminal::zaplexify::render::SSH_DOCS_URL;
use crate::ui_components::icons::Icon as UiIcon;
use warpui::elements::{HighlightedHyperlink, Hoverable, Icon, MouseStateHandle};
use warpui::keymap::FixedBinding;
use warpui::AppContext;
use warpui::{
    elements::{Border, Container, CrossAxisAlignment, Flex, ParentElement},
    Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

#[derive(Debug, Clone)]
pub enum SshZaplexifyBlockEvent {
    ZaplexifySession,
    Cancel,
    Interrupt,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SshZaplexifyBlockAction {
    Interrupt,
    Focus,
}

pub struct SshZaplexifyBlock {
    block_mouse_state: MouseStateHandle,
    ssh_command: String,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        SshZaplexifyBlockAction::Interrupt,
        id!(SshZaplexifyBlock::ui_name()),
    )]);
}

impl SshZaplexifyBlock {
    #[allow(clippy::new_without_default)]
    pub fn new(ssh_command: String) -> Self {
        Self {
            block_mouse_state: Default::default(),
            ssh_command,
        }
    }

    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        ctx.notify();
    }
}

impl Entity for SshZaplexifyBlock {
    type Event = SshZaplexifyBlockEvent;
}

impl SshZaplexifyBlock {
    fn render_title_ui(&self, theme: &WarpTheme, appearance: &Appearance) -> Box<dyn Element> {
        let icon = Icon::new(UiIcon::Zap.into(), theme.active_ui_detail());
        zaplexify::render::header_row("Zaplexifying SSH Session...", icon, theme, appearance)
    }
}

pub fn zaplexify_description(
    app: &AppContext,
    hyperlink_index: &HighlightedHyperlink,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let description = FormattedText::new(vec![FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text(
            "Bring Zap's features to your remote session. Blocks, full text editing, auto-complete, Oz, and more. "
        ),
        FormattedTextFragment::hyperlink(crate::t!("common-learn-more"), SSH_DOCS_URL),
    ])]);
    zaplexify::render::build_description_row(description, theme, appearance, hyperlink_index.clone())
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, _, ctx| {
            ctx.open_url(&url.url);
        })
        .finish()
}

impl View for SshZaplexifyBlock {
    fn ui_name() -> &'static str {
        "SshZaplexifyBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        content.add_child(self.render_title_ui(theme, appearance));

        content.add_child(
            Container::new(
                RenderableAction::new(&self.ssh_command, app)
                    .with_background_color(theme.background().into_solid())
                    .render(app)
                    .finish(),
            )
            .with_margin_top(16.)
            .finish(),
        );

        Hoverable::new(self.block_mouse_state.clone(), |_| {
            Container::new(content.finish())
                .with_padding_top(10.)
                .with_background(theme.foreground().with_opacity(10))
                .with_border(Border::top(1.).with_border_fill(theme.outline()))
                .finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshZaplexifyBlockAction::Focus);
        })
        .finish()
    }
}

impl TypedActionView for SshZaplexifyBlock {
    type Action = SshZaplexifyBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshZaplexifyBlockAction::Interrupt => {
                ctx.emit(SshZaplexifyBlockEvent::Interrupt);
            }
            SshZaplexifyBlockAction::Focus => {
                self.focus(ctx);
            }
        }
    }
}

/// Convert the begin_zaplexify_ssh_session script into a string.
pub fn begin_zaplexify_ssh_session_command(app: &AppContext) -> String {
    let asset = bundled_asset!("bootstrap/unknown_init_subshell.sh");

    match AssetCache::as_ref(app).load_asset::<String>(asset) {
        AssetState::Loaded { data } => data.to_string().replace("HOOK_NAME", "InitSsh"),
        _ => panic!("ssh begin zaplexify script should be available as a string"),
    }
}

/// Convert the zaplexify_ssh_session script into a string.
pub fn zaplexify_ssh_session_command(
    uname: &str,
    shell_type: ShellType,
    app: &AppContext,
) -> Option<String> {
    let asset = match (uname, shell_type) {
        // Mac scripts must be less than 1020 characters due to macOS 15+ pty issue
        ("Darwin", ShellType::Zsh | ShellType::Bash) => {
            bundled_asset!("ssh/bash_zsh/zaplexify_ssh_session_mac.sh")
        }
        // Mac scripts must be less than 1020 characters due to macOS 15+ pty issue
        ("Darwin", ShellType::Fish) => bundled_asset!("ssh/fish/zaplexify_ssh_session_mac.sh"),
        (_, ShellType::Zsh | ShellType::Bash) => {
            bundled_asset!("ssh/bash_zsh/zaplexify_ssh_session.sh")
        }
        (_, ShellType::Fish) => bundled_asset!("ssh/fish/zaplexify_ssh_session.sh"),
        // PowerShell is not supported yet.
        (_, ShellType::PowerShell) => return None,
    };

    // Todo(Jack): look into avoiding an allocation here.
    match AssetCache::as_ref(app).load_asset::<String>(asset) {
        AssetState::Loaded { data } => Some(data.to_string()),
        _ => panic!("ssh zaplexify script should be available as a string"),
    }
}
#[cfg(test)]
#[path = "zaplexify_test.rs"]
mod tests;
