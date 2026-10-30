use super::{attention_inbox_binding, ATTENTION_INBOX_DEFAULTS};
use crate::util::bindings::{custom_tag_to_keystroke, open_repository_keystroke, CustomAction};
use warpui::keymap::{Keystroke, Trigger};
use warpui::platform::OperatingSystem;
use warpui::App;

#[test]
fn attention_inbox_default_does_not_shadow_repository_shortcuts() {
    crate::i18n::init(Some("en"));
    for (platform, inbox_default) in [
        (OperatingSystem::Mac, ATTENTION_INBOX_DEFAULTS.mac),
        (
            OperatingSystem::Linux,
            ATTENTION_INBOX_DEFAULTS.linux_and_windows,
        ),
        (
            OperatingSystem::Windows,
            ATTENTION_INBOX_DEFAULTS.linux_and_windows,
        ),
    ] {
        let repository_shortcut = open_repository_keystroke(platform)
            .expect("the repository picker must keep its keyboard shortcut");
        assert_ne!(
            Keystroke::parse(inbox_default)
                .expect("the inbox default must parse on every platform"),
            repository_shortcut,
            "the attention inbox shadows the repository picker on {platform:?}",
        );
    }
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_editable_bindings([attention_inbox_binding()]);
            let inbox = ctx
                .get_binding_by_name("workspace:open_attention_inbox")
                .expect("the attention inbox must remain a registered editable binding");
            let Trigger::Keystrokes(keys) = inbox.trigger else {
                panic!("the attention inbox must have a keyboard shortcut");
            };
            assert_eq!(keys.len(), 1);
            assert_ne!(
                keys[0],
                custom_tag_to_keystroke(CustomAction::OpenRepository.into()).unwrap(),
                "the registered inbox shortcut shadows the native repository menu",
            );
        });
    });
}
