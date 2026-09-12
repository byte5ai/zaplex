use warpui::{assets::asset_cache::AssetSource, App};

use crate::{
    terminal::ssh::util::convert_script_to_one_line,
    test_util::settings::initialize_settings_for_tests, Assets,
};

use super::*;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
}

/// We write a couple extra bytes at the beginning of a script to clear the line, so we limit the
/// script to 1020 bytes here.
fn assert_script_is_short_enough_mac(script: &str, script_name: &str, convert_to_one_line: bool) {
    let script = if convert_to_one_line {
        convert_script_to_one_line(script)
    } else {
        script.to_string()
    };
    assert!(
        script.len() <= 1020,
        "{} script too long: {} bytes",
        script_name,
        script.len()
    );
}

fn get_script(asset_source: AssetSource, ctx: &AppContext) -> String {
    match AssetCache::as_ref(ctx).load_asset::<String>(asset_source) {
        AssetState::Loaded { data } => data.to_string(),
        _ => panic!("install tmux script should be available as a string"),
    }
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
/// See [assert_script_is_short_enough_mac] for more information.
fn test_mac_zaplexification_script_size() {
    App::test(Assets, |mut app| async move {
        initialize_app(&mut app);

        app.read(|ctx| {
            assert_script_is_short_enough_mac(
                &begin_zaplexify_ssh_session_command(ctx),
                "unknown_init_subshell.sh",
                false,
            );

            assert_script_is_short_enough_mac(
                &get_script(
                    bundled_asset!("ssh/bash_zsh/install_tmux_and_zaplexify_brew.sh"),
                    ctx,
                ),
                "install_tmux_and_zaplexify_brew.sh",
                false,
            );
            assert_script_is_short_enough_mac(
                &get_script(
                    bundled_asset!("ssh/fish/install_tmux_and_zaplexify_brew.sh"),
                    ctx,
                ),
                "fish/install_tmux_and_zaplexify_brew.sh",
                false,
            );

            assert_script_is_short_enough_mac(
                &zaplexify_ssh_session_command("Darwin", ShellType::Zsh, ctx)
                    .expect("Should get Darwin zsh script"),
                "zsh zaplexify",
                true,
            );
            assert_script_is_short_enough_mac(
                &zaplexify_ssh_session_command("Darwin", ShellType::Bash, ctx)
                    .expect("Should get Darwin bash script"),
                "bash zaplexify",
                true,
            );
            assert_script_is_short_enough_mac(
                &zaplexify_ssh_session_command("Darwin", ShellType::Fish, ctx)
                    .expect("Should get Darwin fish script"),
                "fish zaplexify",
                true,
            )
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn zaplexify_scripts_prefer_managed_tmux_and_reject_missing_candidates() {
    App::test(Assets, |mut app| async move {
        initialize_app(&mut app);

        app.read(|ctx| {
            let bash_mac = zaplexify_ssh_session_command("Darwin", ShellType::Bash, ctx)
                .expect("Darwin bash script");
            assert!(bash_mac.contains("if _find \"$TMUX\";then"));

            for (uname, script_name) in [("Darwin", "Darwin fish"), ("Linux", "Linux fish")] {
                let fish = zaplexify_ssh_session_command(uname, ShellType::Fish, ctx)
                    .expect("fish script");
                let managed = fish
                    .find(".warp/tmux/execute_tmux.sh")
                    .expect("managed tmux check");
                let system = fish.find("else if _is tmux").expect("system tmux fallback");
                assert!(
                    managed < system,
                    "{script_name} must prefer the managed tmux binary"
                );
            }

            let fish_mac = zaplexify_ssh_session_command("Darwin", ShellType::Fish, ctx)
                .expect("Darwin fish script");
            assert!(fish_mac.contains("if _is $TMUX"));
        });
    });
}
