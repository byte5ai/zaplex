//! Pane menu keyboard cancellation and focus transfer to the split menu.

use crate::Builder;
use std::time::Duration;
use warp::integration_testing::{
    native_workspace as native, pane_menu_acceptance as menu,
    terminal::{
        execute_command, util::ExpectedExitStatus, wait_until_bootstrapped_single_pane_for_tab,
    },
};

pub fn test_native_pane_menu_evidence() -> Builder {
    pane_menu("en")
}
pub fn test_native_pane_menu_german_evidence() -> Builder {
    pane_menu("de")
}

fn pane_menu(locale: &'static str) -> Builder {
    warp::i18n::set_locale(locale);
    let mut builder = Builder::new()
        .with_real_display()
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).set_timeout(Duration::from_secs(60)),
        )
        .with_step(execute_command(
            0,
            0,
            "ZAPLEX_MENU_PROOF=629".into(),
            ExpectedExitStatus::Success,
            (),
        ));
    for (light, theme) in [(false, "dark"), (true, "light")] {
        builder = builder.with_step(native::theme(light));
        for width in [1024, 760] {
            builder = builder
                .with_step(native::resize(width as f32, 640.))
                .with_step(
                    menu::open().with_take_screenshot(&format!("pane-menu-{theme}-{width}.png")),
                )
                .with_step(
                    menu::cancel()
                        .with_take_screenshot(&format!("pane-menu-canceled-{theme}-{width}.png")),
                )
                .with_step(menu::open())
                .with_step(
                    menu::choose_split()
                        .with_take_screenshot(&format!("pane-split-menu-{theme}-{width}.png")),
                )
                .with_step(menu::split_menu_layout())
                .with_step(menu::cancel())
                .with_step(
                    execute_command(
                        0,
                        0,
                        "test \"$ZAPLEX_MENU_PROOF\" = 629 && printf 'MENU-RETURN-PASS\\n'".into(),
                        ExpectedExitStatus::Success,
                        "MENU-RETURN-PASS".to_string(),
                    )
                    .with_take_screenshot(&format!("pane-menu-shell-{theme}-{width}.png")),
                );
        }
    }
    builder
}
