use super::*;
use crate::themes::default_themes::{dark_theme, light_theme};
use crate::ui_components::modal_frame;
use crate::WorkspaceAction;
use warp_core::ui::color::{
    blend::Blend,
    contrast::{high_enough_contrast, MinimumAllowedContrast},
};
use warp_core::ui::theme::WarpTheme;
use warpui::fonts::FamilyId;

fn modal_close_has_contrast(theme: WarpTheme) {
    let appearance = Appearance::new(
        theme,
        FamilyId(0),
        13.,
        Weight::Normal,
        FamilyId(1),
        1.4,
        FamilyId(0),
        FamilyId(0),
        12.,
        Default::default(),
    );
    let button = modal_frame::close_button(WorkspaceAction::ClosePanel);
    let modal_background = appearance.theme().background().into_solid();
    for hovered in [false, true] {
        let background = button.theme.background(hovered, &appearance);
        let surface = background.map_or(modal_background, |fill| {
            modal_background.blend(&fill.into_solid())
        });
        let icon = surface.blend(&button.theme.text_color(hovered, background, &appearance));
        assert!(
            high_enough_contrast(icon, surface, MinimumAllowedContrast::NonText),
            "modal close icon is unreadable in {:?} (hovered={hovered})",
            appearance.theme().name(),
        );
    }
}

#[test]
fn modal_close_icon_is_readable_on_light_surfaces() {
    modal_close_has_contrast(light_theme());
}

#[test]
fn modal_close_icon_is_readable_on_dark_surfaces() {
    modal_close_has_contrast(dark_theme());
}
