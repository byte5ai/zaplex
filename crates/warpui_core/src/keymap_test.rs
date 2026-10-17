use super::*;
use crate::App;

#[test]
#[should_panic]
fn test_keystroke_invalid_shift_lowercase() {
    let _ = Keystroke::parse("ctrl-shift-p");
}

#[test]
#[should_panic]
fn test_keystroke_invalid_no_shift_uppercase() {
    let _ = Keystroke::parse("ctrl-P");
}

/// Regression guard for the "app won't even open" class: a malformed **Mac-only**
/// binding (`cmd-shift-o`, which must be `cmd-shift-O` because the OS delivers
/// shift+letter as the uppercase form) must be rejected even when this build is
/// *not* running on a Mac.
///
/// `with_mac_key_binding` leaves the trigger unset off-Mac, but it now still
/// validates the string, so this exact regression — which passed a Linux
/// `cargo check` yet panicked at startup on macOS during binding registration —
/// is caught by a Linux CI run. Without the cross-platform validation this test
/// would (wrongly) pass off-Mac, which is precisely the gap being closed.
#[test]
#[should_panic]
fn test_mac_only_binding_validated_on_all_platforms() {
    #[derive(Debug)]
    enum TestAction {
        Noop,
    }
    let _ =
        EditableBinding::new("test", "test", TestAction::Noop).with_mac_key_binding("cmd-shift-o");
}

/// The corrected, well-formed Mac binding must be accepted on every platform:
/// the cross-platform validation must not reject valid shift+uppercase bindings
/// (and must not panic off-Mac, where the trigger is simply left unset).
#[test]
fn test_valid_mac_only_binding_ok_on_all_platforms() {
    #[derive(Debug)]
    enum TestAction {
        Noop,
    }
    let _ =
        EditableBinding::new("test", "test", TestAction::Noop).with_mac_key_binding("cmd-shift-O");
}

/// `FixedBinding::new_per_platform` must likewise validate the variant this OS
/// does *not* use. Here the Mac variant is malformed while the linux/windows
/// variant (the one a Linux CI run actually selects) is fine, so only the
/// cross-platform check surfaces the bug.
#[test]
#[should_panic]
fn test_per_platform_off_platform_binding_validated() {
    use crate::keymap::macros::*;
    #[derive(Debug)]
    enum TestAction {
        Noop,
    }
    let _ = FixedBinding::new_per_platform(
        PerPlatformKeystroke {
            mac: "cmd-shift-o",
            linux_and_windows: "ctrl-shift-O",
        },
        TestAction::Noop,
        always!(),
    );
}

#[cfg(feature = "settings_value")]
mod settings_value_tests {
    use super::*;
    use settings_value::SettingsValue;

    #[test]
    fn test_option_keystroke_round_trip() {
        let some = Some(Keystroke::parse("alt-b").unwrap());
        let round_tripped = Option::<Keystroke>::from_file_value(&some.to_file_value()).unwrap();
        assert_eq!(round_tripped, some);

        let none: Option<Keystroke> = None;
        assert_eq!(none.to_file_value(), serde_json::Value::Null);
        let round_tripped = Option::<Keystroke>::from_file_value(&serde_json::Value::Null).unwrap();
        assert_eq!(round_tripped, none);
    }
}

#[test]
fn test_binding_description_resolve_dynamic_override_falls_back_to_custom_context() {
    App::test((), |app| async move {
        let resolved = app.read(|ctx| {
            BindingDescription::new("static")
                .with_custom_description(DescriptionContext::Custom("menu"), "menu-static")
                .with_dynamic_override(|_| None)
                .resolve(ctx, DescriptionContext::Custom("menu"))
                .into_owned()
        });
        assert_eq!(resolved, "menu-static");
    });
}
