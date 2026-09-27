use crate::integration_testing::view_getters::{command_palette_view, workspace_view};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use warpui::async_assert;
use warpui::integration::AssertionCallback;

/// Asserts that the command palette is currently open.
pub fn assert_command_palette_is_open() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);

        workspace.read(app, |workspace, _| {
            async_assert!(
                workspace.is_palette_open(),
                "Expected palette to be open, but it was closed"
            )
        })
    })
}

/// Asserts that the command palette is currently closed.
pub fn assert_command_palette_is_closed() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);

        workspace.read(app, |workspace, _| {
            async_assert!(
                !workspace.is_palette_open(),
                "Expected palette to be closed, but it was open"
            )
        })
    })
}

/// Asserts that the command palette currently has at least one search result.
pub fn assert_command_palette_has_results() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let palette = command_palette_view(app, window_id);

        palette.read(app, |palette, ctx| {
            async_assert!(
                palette.search_results(ctx).next().is_some(),
                "Expected command palette to have results, but it was empty"
            )
        })
    })
}

/// Checks the live creation results after querying "Create a new".
/// A positive Personal result ensures an empty or pending result list cannot pass.
pub fn assert_personal_creation_without_team_actions(
    personal_binding: &'static str,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let palette = command_palette_view(app, window_id);
        palette.read(app, |palette, ctx| {
            let bindings: Vec<String> = palette
                .search_results(ctx)
                .filter_map(|result| {
                    if let CommandPaletteItemAction::AcceptBinding { binding } = result.accept_result() {
                        Some(binding.name.clone())
                    } else {
                        None
                    }
                })
                .collect();
            async_assert!(
                bindings.iter().any(|name| name == personal_binding)
                    && !bindings.iter().any(|name| matches!(
                        name.as_str(),
                        "workspace:create_team_workflow" | "workspace:create_team_folder"
                    )),
                "Expected {personal_binding} and no retired Team creation actions; got {bindings:?}"
            )
        })
    })
}

/// Used alongside a positive result assertion to verify a context-gated action is absent.
pub fn assert_command_palette_binding_not_offered(binding_name: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let palette = command_palette_view(app, window_id);
        palette.read(app, |palette, ctx| {
            async_assert!(
                !palette.search_results(ctx).any(|result| {
                    matches!(result.accept_result(), CommandPaletteItemAction::AcceptBinding { binding }
                        if binding.name == binding_name)
                }),
                "Context-gated binding {binding_name} must not be offered"
            )
        })
    })
}
