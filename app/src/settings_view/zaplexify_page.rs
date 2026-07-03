use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use regex::Regex;
use settings::{Setting, ToggleableSetting};
use strum::IntoEnumIterator;
use warp_core::features::FeatureFlag;
use warpui::elements::{FormattedTextElement, HighlightedHyperlink};
use warpui::keymap::ContextPredicate;
use warpui::{
    elements::{Container, Flex, MouseStateHandle, ParentElement},
    presenter::ChildView,
    ui_components::{
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    Action, AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::terminal::zaplexify::settings::{
    EnableSshZaplexification, SshExtensionInstallMode, SshExtensionInstallModeSetting,
    UseSshTmuxWrapper, ZaplexifySettingsChangedEvent,
};
use crate::ui_components::blended_colors;
use crate::{
    appearance::Appearance,
    report_if_error, send_telemetry_from_ctx,
    server::telemetry::TelemetryEvent,
    terminal::zaplexify::settings::ZaplexifySettings,
    view_components::{SubmittableTextInput, SubmittableTextInputEvent},
};

use super::settings_page::{
    render_body_item, render_dropdown_item, render_page_title, AdditionalInfo, Category,
    LocalOnlyIconState, MatchData, PageType, SettingsPageEvent, SettingsWidget, ToggleState,
    HEADER_PADDING,
};
use super::SettingsSection;
use super::{
    flags,
    settings_page::{
        add_setting, render_alternating_color_list, SettingsPageMeta, SettingsPageViewHandle,
    },
    SettingsAction, ToggleSettingActionPair,
};
use crate::view_components::dropdown::{Dropdown, DropdownItem};

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    // Add all of the toggle settings from the Zaplexify Page that you want to show up on the Command Palette here.
    let mut toggle_binding_pairs = vec![];

    if FeatureFlag::SSHTmuxWrapper.is_enabled() {
        toggle_binding_pairs.push(ToggleSettingActionPair::new(
            &crate::t!("settings-zaplexify-ssh-tmux-toggle-binding-label"),
            builder(SettingsAction::ZaplexifyPageToggle(
                ZaplexifyPageAction::ToggleTmuxZaplexification,
            )),
            context,
            flags::SSH_TMUX_WRAPPER_CONTEXT_FLAG,
        ));
    }

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(toggle_binding_pairs, app);
}

const ITEM_VERTICAL_SPACING: f32 = 24.;
/// There's a built-in 10px margin below the text input.
const BUILT_IN_TEXT_INPUT_MARGIN: f32 = 10.;
const SPACE_AFTER_TEXT_INPUT: f32 = ITEM_VERTICAL_SPACING - BUILT_IN_TEXT_INPUT_MARGIN;

/// This page lets users configure when they get asked to zaplexify a session. Some shell commands
/// are recognized by default. Users can add new shell commands, or prevent the default ones from
/// asking. Users can also enable the SSH wrapper, and add hosts to a denylist.
/// This page is essentially the View for the SubshellSettings model, as well as the SshSettings
/// related to zaplexification.
pub struct ZaplexifyPageView {
    page: PageType<Self>,
    /// This needs to mirror the length of SubshellSettings::added_remove_button_states.
    remove_added_command_button_states: Vec<MouseStateHandle>,
    add_added_commands_editor: ViewHandle<SubmittableTextInput>,
    /// This needs to mirror the length of SubshellSettings::denylisted_remove_button_states.
    remove_denylisted_command_button_states: Vec<MouseStateHandle>,
    add_denylisted_commands_editor: ViewHandle<SubmittableTextInput>,

    remove_denylisted_ssh_button_states: Vec<MouseStateHandle>,
    add_denylisted_ssh_editor: ViewHandle<SubmittableTextInput>,

    ssh_extension_install_mode_dropdown: ViewHandle<Dropdown<ZaplexifyPageAction>>,
}

impl ZaplexifyPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let zaplexify_settings_handle = ZaplexifySettings::handle(ctx);

        ctx.observe(&zaplexify_settings_handle, Self::update_button_states);
        ctx.subscribe_to_model(&zaplexify_settings_handle, move |me, model, event, ctx| {
            me.update_button_states(model, ctx);
            if matches!(
                event,
                ZaplexifySettingsChangedEvent::SshExtensionInstallModeSetting { .. }
            ) {
                me.update_dropdown(ctx);
            }
            ctx.notify();
        });

        // Added commands can be specified by regex, while denied commands are strictly exact
        // match.
        let add_added_commands_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|regex| Regex::new(regex).is_ok());
            input.set_placeholder_text(crate::t!("settings-zaplexify-command-placeholder"), ctx);
            input
        });

        ctx.subscribe_to_view(
            &add_added_commands_editor,
            Self::handle_added_command_editor_event,
        );

        let add_denylisted_commands_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx);
            input.set_placeholder_text(crate::t!("settings-zaplexify-command-placeholder"), ctx);
            input
        });

        ctx.subscribe_to_view(
            &add_denylisted_commands_editor,
            Self::handle_denylisted_command_editor_event,
        );

        let add_denylisted_ssh_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx);
            input.set_placeholder_text(crate::t!("settings-zaplexify-host-placeholder"), ctx);
            input
        });

        ctx.subscribe_to_view(
            &add_denylisted_ssh_editor,
            Self::handle_denylisted_ssh_editor_event,
        );

        let ssh_extension_install_mode_dropdown =
            Self::create_ssh_extension_install_mode_dropdown(ctx);

        let mut instance = Self {
            page: Self::build_page(ctx),
            remove_added_command_button_states: Default::default(),
            add_added_commands_editor,
            remove_denylisted_command_button_states: Default::default(),
            add_denylisted_commands_editor,
            remove_denylisted_ssh_button_states: Default::default(),
            add_denylisted_ssh_editor,
            ssh_extension_install_mode_dropdown,
        };

        instance.update_button_states(zaplexify_settings_handle, ctx);
        instance
    }

    fn build_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let mut categories = vec![
            Category::new("", vec![Box::new(TitleWidget::default())]),
            Category::new(
                Box::leak(crate::t!("settings-zaplexify-section-subshells").into_boxed_str()),
                vec![Box::new(SubshellsWidget::default())],
            )
            .with_subtitle(Box::leak(
                crate::t!("settings-zaplexify-section-subshells-subtitle").into_boxed_str(),
            )),
        ];

        let zaplexify_settings = ZaplexifySettings::as_ref(ctx);
        if FeatureFlag::SSHTmuxWrapper.is_enabled()
            && zaplexify_settings
                .enable_ssh_zaplexification
                .is_supported_on_current_platform()
        {
            categories.push(
                Category::new(
                    Box::leak(crate::t!("settings-zaplexify-section-ssh").into_boxed_str()),
                    vec![Box::new(SSHWidget::default())],
                )
                .with_subtitle(Box::leak(
                    crate::t!("settings-zaplexify-section-ssh-subtitle").into_boxed_str(),
                )),
            );
        }
        PageType::new_categorized(categories, None)
    }

    /// This method ensures each command in the SubshellSettings has a matching button state for
    /// its delete button in the View.
    fn update_button_states(
        &mut self,
        zaplexify_settings_handle: ModelHandle<ZaplexifySettings>,
        ctx: &mut ViewContext<Self>,
    ) {
        let zaplexify_settings = zaplexify_settings_handle.as_ref(ctx);
        self.remove_denylisted_command_button_states = zaplexify_settings
            .subshell_command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();
        self.remove_added_command_button_states = zaplexify_settings
            .added_subshell_commands
            .iter()
            .map(|_| Default::default())
            .collect();
        self.remove_denylisted_ssh_button_states = zaplexify_settings
            .ssh_hosts_denylist
            .iter()
            .map(|_| Default::default())
            .collect();
        ctx.notify();
    }

    /// Syncs the install-mode dropdown selection with the current
    /// `ZaplexifySettings::ssh_extension_install_mode` value (e.g. after it
    /// was changed from the SSH remote server choice view).
    fn update_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let current_mode = *ZaplexifySettings::as_ref(ctx)
            .ssh_extension_install_mode
            .value();
        self.ssh_extension_install_mode_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    ZaplexifyPageAction::SetSshExtensionInstallMode(current_mode),
                    ctx,
                );
            });
    }

    fn handle_added_command_editor_event(
        &mut self,
        _handle: ViewHandle<SubmittableTextInput>,
        event: &SubmittableTextInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SubmittableTextInputEvent::Submit(new_command) => {
                ZaplexifySettings::handle(ctx).update(ctx, |zaplexify_settings, ctx| {
                    zaplexify_settings.add_subshell_command(new_command, ctx);
                });

                send_telemetry_from_ctx!(TelemetryEvent::AddAddedSubshellCommand, ctx);
            }
            SubmittableTextInputEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
        }
    }

    fn handle_denylisted_command_editor_event(
        &mut self,
        _handle: ViewHandle<SubmittableTextInput>,
        event: &SubmittableTextInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SubmittableTextInputEvent::Submit(new_command) => {
                ZaplexifySettings::handle(ctx).update(ctx, |zaplexify_settings, ctx| {
                    zaplexify_settings.denylist_subshell_command(new_command, ctx);
                });

                send_telemetry_from_ctx!(TelemetryEvent::AddDenylistedSubshellCommand, ctx);
            }
            SubmittableTextInputEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
        }
    }

    fn handle_denylisted_ssh_editor_event(
        &mut self,
        _handle: ViewHandle<SubmittableTextInput>,
        event: &SubmittableTextInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SubmittableTextInputEvent::Submit(new_command) => {
                ZaplexifySettings::handle(ctx).update(ctx, |zaplexify_settings, ctx| {
                    zaplexify_settings.denylist_ssh_host(new_command, ctx);
                });

                send_telemetry_from_ctx!(TelemetryEvent::AddDenylistedSshTmuxWrapperHost, ctx);
            }
            SubmittableTextInputEvent::Escape => ctx.emit(SettingsPageEvent::FocusModal),
        }
    }

    fn remove_denylisted_command(&self, index: usize, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::RemoveDenylistedSubshellCommand, ctx);
        ZaplexifySettings::handle(ctx).update(ctx, |zaplexify, ctx| {
            zaplexify.remove_denylisted_subshell_command(index, ctx)
        });
    }

    fn remove_added_command(&self, index: usize, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::RemoveAddedSubshellCommand, ctx);
        ZaplexifySettings::handle(ctx).update(ctx, |zaplexify, ctx| {
            zaplexify.remove_added_subshell_command(index, ctx)
        });
    }

    fn remove_denylisted_ssh_host(&self, index: usize, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::RemoveDenylistedSshTmuxWrapperHost, ctx);
        ZaplexifySettings::handle(ctx).update(ctx, |zaplexify, ctx| {
            zaplexify.remove_denylisted_ssh_host(index, ctx)
        });
    }
}

impl Entity for ZaplexifyPageView {
    type Event = SettingsPageEvent;
}

fn build_sub_sub_title(title: String, appearance: &Appearance) -> Container {
    appearance
        .ui_builder()
        .span(title)
        .with_style(UiComponentStyles {
            font_size: Some(appearance.ui_font_body()),
            ..Default::default()
        })
        .build()
}

const SSH_EXTENSION_DROPDOWN_WIDTH: f32 = 250.;

impl ZaplexifyPageView {
    fn create_ssh_extension_install_mode_dropdown(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Dropdown<ZaplexifyPageAction>> {
        let items: Vec<DropdownItem<ZaplexifyPageAction>> = SshExtensionInstallMode::iter()
            .map(|mode| {
                DropdownItem::new(
                    mode.display_name(),
                    ZaplexifyPageAction::SetSshExtensionInstallMode(mode),
                )
            })
            .collect();

        let current_mode = *ZaplexifySettings::as_ref(ctx)
            .ssh_extension_install_mode
            .value();
        let enable_ssh_zaplexification = *ZaplexifySettings::as_ref(ctx)
            .enable_ssh_zaplexification
            .value();

        ctx.add_typed_action_view(move |ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(SSH_EXTENSION_DROPDOWN_WIDTH);
            dropdown.set_menu_width(SSH_EXTENSION_DROPDOWN_WIDTH, ctx);
            dropdown.add_items(items, ctx);
            dropdown.set_selected_by_action(
                ZaplexifyPageAction::SetSshExtensionInstallMode(current_mode),
                ctx,
            );
            if !enable_ssh_zaplexification {
                dropdown.set_disabled(ctx);
            }
            dropdown
        })
    }

    /// Renders a title, a list of items that can be removed, and an input field to add new items.
    fn build_input_list<
        ListItem: Display,
        SettingsPageAction: Action + Clone,
        F: Fn(usize) -> SettingsPageAction,
        T: View,
    >(
        &self,
        title: String,
        patterns: &[ListItem],
        mouse_states: &[MouseStateHandle],
        create_action: F,
        handle: &ViewHandle<T>,
        appearance: &Appearance,
    ) -> Container {
        let mut column = Flex::column();
        let mut title = build_sub_sub_title(title, appearance);

        if !patterns.is_empty() {
            title = title.with_padding_bottom(BUILT_IN_TEXT_INPUT_MARGIN);
        }

        column.add_child(title.finish());

        render_alternating_color_list(
            &mut column,
            patterns,
            mouse_states,
            create_action,
            appearance,
        );

        Container::new(
            column
                .with_child(
                    Container::new(ChildView::new(handle).finish())
                        .with_margin_bottom(SPACE_AFTER_TEXT_INPUT)
                        .finish(),
                )
                .finish(),
        )
    }
}

impl View for ZaplexifyPageView {
    fn ui_name() -> &'static str {
        "ZaplexifyPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ZaplexifyPageAction {
    RemoveAddedCommand(usize),
    RemoveDenylistedCommand(usize),
    RemoveDenylistedSshHost(usize),
    /// If disabled, auto-Zaplexification and the SSH Zaplexification prompt will be disabled.
    ToggleTmuxZaplexification,
    ToggleSshZaplexification,
    /// Set the SSH extension installation mode (always ask / always install / always skip).
    SetSshExtensionInstallMode(SshExtensionInstallMode),
    OpenUrl(String),
}

impl TypedActionView for ZaplexifyPageView {
    type Action = ZaplexifyPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use ZaplexifyPageAction::*;
        match action {
            RemoveDenylistedCommand(index) => self.remove_denylisted_command(*index, ctx),
            RemoveAddedCommand(index) => self.remove_added_command(*index, ctx),
            ToggleSshZaplexification => {
                ZaplexifySettings::handle(ctx).update(ctx, |ssh_settings, ctx| {
                    report_if_error!(ssh_settings
                        .enable_ssh_zaplexification
                        .toggle_and_save_value(ctx));
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ToggleSshZaplexification {
                            enabled: *ssh_settings.enable_ssh_zaplexification.value(),
                        },
                        ctx
                    );
                });
                let enabled = *ZaplexifySettings::as_ref(ctx)
                    .enable_ssh_zaplexification
                    .value();
                self.ssh_extension_install_mode_dropdown
                    .update(ctx, |dropdown, ctx| {
                        if enabled {
                            dropdown.set_enabled(ctx);
                        } else {
                            dropdown.set_disabled(ctx);
                        }
                    });
            }
            ToggleTmuxZaplexification => {
                ZaplexifySettings::handle(ctx).update(ctx, |ssh_settings, ctx| {
                    report_if_error!(ssh_settings.use_ssh_tmux_wrapper.toggle_and_save_value(ctx));
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ToggleSshTmuxWrapper {
                            enabled: *ssh_settings.use_ssh_tmux_wrapper.value(),
                        },
                        ctx
                    );
                });
            }
            SetSshExtensionInstallMode(mode) => {
                ZaplexifySettings::handle(ctx).update(ctx, |zaplexify_settings, ctx| {
                    report_if_error!(zaplexify_settings
                        .ssh_extension_install_mode
                        .set_value(*mode, ctx));
                    send_telemetry_from_ctx!(
                        TelemetryEvent::SetSshExtensionInstallMode {
                            mode: mode.telemetry_name(),
                        },
                        ctx
                    );
                });
            }
            ZaplexifyPageAction::RemoveDenylistedSshHost(index) => {
                self.remove_denylisted_ssh_host(*index, ctx);
            }
            OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
        }
    }
}

impl SettingsPageMeta for ZaplexifyPageView {
    fn section() -> SettingsSection {
        SettingsSection::Zaplexify
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<ZaplexifyPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ZaplexifyPageView>) -> Self {
        SettingsPageViewHandle::Zaplexify(view_handle)
    }
}

#[derive(Default)]
struct TitleWidget;

impl TitleWidget {
    fn render_top_of_page(&self, appearance: &Appearance, _app: &AppContext) -> Box<dyn Element> {
        // No "Learn more" link yet: there is no Zaplexify docs page, and an empty
        // href renders a broken/no-op hyperlink. The description stands alone as a
        // complete sentence; restore the link once a real docs URL exists.
        let zaplexify_description =
            vec![FormattedTextFragment::plain_text(crate::t!(
                "settings-zaplexify-description-prefix"
            ))];

        let zaplexify_description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(zaplexify_description)]),
            appearance.ui_font_body(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            HighlightedHyperlink::default(),
        )
        .with_heading_to_font_size_multipliers(appearance.heading_font_size_multipliers().clone())
        .finish();

        Flex::column()
            .with_child(render_page_title(
                &crate::t!("settings-zaplexify-page-title"),
                appearance,
            ))
            .with_child(zaplexify_description)
            .finish()
    }
}

impl SettingsWidget for TitleWidget {
    type View = ZaplexifyPageView;

    fn search_terms(&self) -> &str {
        "ssh subshell zaplexify session"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(self.render_top_of_page(appearance, app))
            .with_margin_bottom(ITEM_VERTICAL_SPACING)
            .finish()
    }
}

#[derive(Default)]
struct SubshellsWidget {}

impl SubshellsWidget {
    fn render_subshells_section(
        &self,
        view: &ZaplexifyPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();

        let zaplexify_settings = ZaplexifySettings::as_ref(app);

        column.add_child(
            view.build_input_list(
                crate::t!("settings-zaplexify-added-commands"),
                &zaplexify_settings.added_subshell_commands,
                &view.remove_added_command_button_states,
                ZaplexifyPageAction::RemoveAddedCommand,
                &view.add_added_commands_editor,
                appearance,
            )
            .finish(),
        );

        column.add_child(
            view.build_input_list(
                crate::t!("settings-zaplexify-denylisted-commands"),
                &zaplexify_settings.subshell_command_denylist,
                &view.remove_denylisted_command_button_states,
                ZaplexifyPageAction::RemoveDenylistedCommand,
                &view.add_denylisted_commands_editor,
                appearance,
            )
            .with_margin_bottom(-BUILT_IN_TEXT_INPUT_MARGIN)
            .finish(),
        );

        column.finish()
    }
}

impl SettingsWidget for SubshellsWidget {
    type View = ZaplexifyPageView;

    fn search_terms(&self) -> &str {
        "zaplexify subshell"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(self.render_subshells_section(view, appearance, app))
            .with_margin_bottom(ITEM_VERTICAL_SPACING)
            .finish()
    }
}

#[derive(Default)]
struct SSHWidget {
    tmux_zaplexification_switch_state: SwitchStateHandle,
    enable_ssh_zaplexification_switch_state: SwitchStateHandle,
    additional_info_mouse_state: MouseStateHandle,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl SettingsWidget for SSHWidget {
    type View = ZaplexifyPageView;

    fn search_terms(&self) -> &str {
        "zaplexify ssh"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();
        let ui_builder = appearance.ui_builder();
        let description_text_color = appearance
            .theme()
            .sub_text_color(appearance.theme().surface_2());

        let enable_ssh_zaplexification = *ZaplexifySettings::as_ref(app)
            .enable_ssh_zaplexification
            .value();

        let should_prompt_ssh_tmux_wrapper =
            *ZaplexifySettings::as_ref(app).use_ssh_tmux_wrapper.value();

        add_setting(
            &mut column,
            &ZaplexifySettings::as_ref(app).enable_ssh_zaplexification,
            move || {
                render_body_item::<ZaplexifyPageAction>(
                    crate::t!("settings-zaplexify-enable-ssh"),
                    None,
                    LocalOnlyIconState::for_setting(
                        EnableSshZaplexification::storage_key(),
                        EnableSshZaplexification::sync_to_cloud(),
                        &mut self.local_only_icon_tooltip_states.borrow_mut(),
                        app,
                    ),
                    ToggleState::Enabled,
                    appearance,
                    ui_builder
                        .switch(self.enable_ssh_zaplexification_switch_state.clone())
                        .check(enable_ssh_zaplexification)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(ZaplexifyPageAction::ToggleSshZaplexification);
                        })
                        .finish(),
                    None,
                )
            },
        );

        if FeatureFlag::SshRemoteServer.is_enabled() {
            let label_color_override = if !enable_ssh_zaplexification {
                Some(appearance.theme().disabled_ui_text_color())
            } else {
                None
            };
            add_setting(
                &mut column,
                &ZaplexifySettings::as_ref(app).ssh_extension_install_mode,
                move || {
                    let install_ssh_label = crate::t!("settings-zaplexify-install-ssh-extension");
                    let install_ssh_desc =
                        crate::t!("settings-zaplexify-install-ssh-extension-description");
                    Container::new(render_dropdown_item(
                        appearance,
                        &install_ssh_label,
                        Some(&install_ssh_desc),
                        None,
                        LocalOnlyIconState::for_setting(
                            SshExtensionInstallModeSetting::storage_key(),
                            SshExtensionInstallModeSetting::sync_to_cloud(),
                            &mut self.local_only_icon_tooltip_states.borrow_mut(),
                            app,
                        ),
                        label_color_override,
                        &view.ssh_extension_install_mode_dropdown,
                    ))
                    .with_padding_bottom(HEADER_PADDING)
                    .finish()
                },
            );
        }

        add_setting(
            &mut column,
            &ZaplexifySettings::as_ref(app).use_ssh_tmux_wrapper,
            move || {
                let mut column = Flex::column();

                column.add_child(render_body_item::<ZaplexifyPageAction>(
                    crate::t!("settings-zaplexify-use-tmux"),
                    Some(AdditionalInfo {
                        mouse_state: self.additional_info_mouse_state.clone(),
                        on_click_action: Some(ZaplexifyPageAction::OpenUrl(
                            "".into(),
                        )),
                        secondary_text: None,
                        tooltip_override_text: None,
                    }),
                    LocalOnlyIconState::for_setting(
                        UseSshTmuxWrapper::storage_key(),
                        UseSshTmuxWrapper::sync_to_cloud(),
                        &mut self.local_only_icon_tooltip_states.borrow_mut(),
                        app,
                    ),
                    enable_ssh_zaplexification.into(),
                    appearance,
                    ui_builder
                        .switch(self.tmux_zaplexification_switch_state.clone())
                        .check(should_prompt_ssh_tmux_wrapper)
                        .with_disabled(!enable_ssh_zaplexification)
                        .build()
                        .on_click(move |ctx, _, _| {
                            if !enable_ssh_zaplexification {
                                return;
                            }

                            ctx.dispatch_typed_action(ZaplexifyPageAction::ToggleTmuxZaplexification);
                        })
                        .finish(),
                    None,
                ));

                column.add_child(
                    ui_builder
                        .paragraph(crate::t!("settings-zaplexify-tmux-description"))
                        .with_style(UiComponentStyles {
                            font_color: Some(description_text_color.into_solid()),
                            margin: Some(
                                Coords::default()
                                    .top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                                    .bottom(styles::DESCRIPTION_LINE_MARGIN_BOTTOM),
                            ),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );

                if enable_ssh_zaplexification && should_prompt_ssh_tmux_wrapper {
                    let zaplexify_settings = ZaplexifySettings::as_ref(app);
                    column.add_child(
                        view.build_input_list(
                            crate::t!("settings-zaplexify-denylisted-hosts"),
                            &zaplexify_settings.ssh_hosts_denylist,
                            &view.remove_denylisted_ssh_button_states,
                            ZaplexifyPageAction::RemoveDenylistedSshHost,
                            &view.add_denylisted_ssh_editor,
                            appearance,
                        )
                        .finish(),
                    );
                } else {
                    // Add margin to hint the user should scroll to see more.
                    column.add_child(
                        Container::new(Flex::column().finish())
                            .with_margin_bottom(styles::MINIMUM_SCROLL_OFFSET_AFTER_SSH)
                            .finish(),
                    );
                }

                column.finish()
            },
        );

        column.finish()
    }
}

mod styles {
    // Apply a negative margin to the description text so it appears closer to the main
    // settings option text.
    pub const DESCRIPTION_NEGATIVE_MARGIN_OFFSET: f32 = -8.;

    /// The space after a description.
    pub const DESCRIPTION_LINE_MARGIN_BOTTOM: f32 = 18.;

    /// Because we hide the SSH settings if the SSH wrapper is disabled, we need to add a margin
    /// to the bottom to make it clear that toggling this item will reveal more settings,
    /// even at smaller window sizes. We picked an offset that cuts off the first item
    /// to imply the user should scroll to see more.
    pub const MINIMUM_SCROLL_OFFSET_AFTER_SSH: f32 = 40.;
}
