//! Font-size validation preserves the effective setting and allows recovery.

use warp::integration_testing::{
    native_workspace as native,
    settings::font_validation as font,
    terminal::{
        execute_command, util::ExpectedExitStatus, wait_until_bootstrapped_single_pane_for_tab,
    },
};
use warpui::integration::TestStep;
use warpui::keymap::PerPlatformKeystroke;

use crate::Builder;

pub fn test_native_font_validation_evidence() -> Builder {
    font_validation("en")
}

pub fn test_native_font_validation_german_evidence() -> Builder {
    font_validation("de")
}

fn font_validation(locale: &'static str) -> Builder {
    warp::i18n::set_locale(locale);
    Builder::new()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(native::resize(1024., 640.))
        .with_step(native::theme(false))
        .with_step(execute_command(
            0,
            0,
            "ZAPLEX_PROOF=2231".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(font::open())
        .with_step(font::type_size("15"))
        .with_step(font::submit("15", 15., false).with_take_screenshot("font-valid-before.png"))
        .with_step(font::type_size("abc"))
        .with_step(font::submit("abc", 15., true).with_take_screenshot("font-invalid-text.png"))
        .with_step(font::type_size("0"))
        .with_step(font::submit("0", 15., true))
        .with_step(font::type_size("121"))
        .with_step(font::submit("121", 15., true).with_take_screenshot("font-invalid-range.png"))
        .with_step(font::cancel("15", 15.).with_take_screenshot("font-escape-restored.png"))
        .with_step(font::type_size("abc"))
        .with_step(font::submit("abc", 15., true))
        .with_step(font::type_size("15"))
        .with_step(
            font::submit("15", 15., false).with_take_screenshot("font-same-value-corrected.png"),
        )
        .with_step(font::type_size("13"))
        .with_step(font::submit("13", 13., false))
        .with_step(
            TestStep::new("Close only the Settings tab").with_per_platform_keystroke(
                PerPlatformKeystroke {
                    mac: "cmd-w",
                    linux_and_windows: "ctrl-shift-w",
                },
            ),
        )
        .with_step(
            execute_command(
                0,
                0,
                "test \"$ZAPLEX_PROOF\" = 2231 && printf 'FONT-RETURN-PASS\\n'".into(),
                ExpectedExitStatus::Success,
                "FONT-RETURN-PASS".to_string(),
            )
            .with_take_screenshot("font-shell-preserved.png"),
        )
}
