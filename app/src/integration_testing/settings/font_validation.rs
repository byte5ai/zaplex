//! Real editor input and visible validation for terminal font settings.

use settings::Setting;
use warpui::elements::get_rich_content_position_id;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::keymap::PerPlatformKeystroke;
use warpui::{App, SingletonEntity, ViewHandle, WindowId};

use crate::integration_testing::{native_workspace, view_getters::workspace_view};
use crate::settings::FontSettings;
use crate::settings_view::{AppearanceSettingsPageView, SettingsSection};
use crate::workspace::WorkspaceAction;

fn appearance_page(app: &App, window_id: WindowId) -> ViewHandle<AppearanceSettingsPageView> {
    app.views_of_type::<AppearanceSettingsPageView>(window_id)
        .and_then(|views| views.first().cloned())
        .expect("appearance settings page")
}

pub fn open() -> TestStep {
    TestStep::new("Find terminal font settings through the Settings search")
        .with_action(|app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::ShowSettingsPageWithSearch {
                    search_query: "font".into(),
                    section: Some(SettingsSection::Appearance),
                },
            );
        })
        .add_named_assertion("Font size editor is visible", |app, window_id| {
            let id = appearance_page(app, window_id).read(app, |page, _| {
                get_rich_content_position_id(&page.terminal_font_size_editor_for_test().id())
            });
            if native_workspace::bounds(app, window_id, &id).is_none() {
                return AssertionOutcome::failure("Font size editor is not visible".into());
            }
            AssertionOutcome::Success
        })
}

pub fn type_size(value: &'static str) -> TestStep {
    TestStep::new(&format!("Type terminal font size {value}"))
        .with_click_on_saved_position_fn(|app, window_id| {
            appearance_page(app, window_id).read(app, |page, _| {
                get_rich_content_position_id(&page.terminal_font_size_editor_for_test().id())
            })
        })
        .with_per_platform_keystroke(PerPlatformKeystroke {
            mac: "cmd-a",
            linux_and_windows: "ctrl-a",
        })
        .with_input_string(value, None)
}

pub fn submit(value: &'static str, effective_size: f32, invalid: bool) -> TestStep {
    TestStep::new("Submit the size and verify the effective setting and visible feedback")
        .with_keystrokes(&["enter"])
        .add_named_assertion(
            "Input, setting and validation agree",
            move |app, window_id| verify(app, window_id, value, effective_size, invalid),
        )
}

pub fn cancel(value: &'static str, effective_size: f32) -> TestStep {
    TestStep::new("Escape restores the effective font size and removes the validation error")
        .with_keystrokes(&["escape"])
        .add_named_assertion("The invalid draft was discarded", move |app, window_id| {
            verify(app, window_id, value, effective_size, false)
        })
}

fn verify(
    app: &App,
    window_id: WindowId,
    value: &str,
    effective_size: f32,
    invalid: bool,
) -> AssertionOutcome {
    let (text, actual_size) = appearance_page(app, window_id).read(app, |page, ctx| {
        (
            page.terminal_font_size_editor_for_test()
                .as_ref(ctx)
                .buffer_text(ctx),
            *FontSettings::as_ref(ctx).monospace_font_size.value(),
        )
    });
    let error_is_visible =
        native_workspace::bounds(app, window_id, "settings-terminal-font-size-error").is_some();
    if text != value || actual_size != effective_size || error_is_visible != invalid {
        return AssertionOutcome::failure(format!(
            "Expected input {value}, effective size {effective_size}, error {invalid}; got {text}, {actual_size}, {error_is_visible}"
        ));
    }
    AssertionOutcome::Success
}
