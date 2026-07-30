use base64::engine::general_purpose::STANDARD as BASE64;
use warp_core::features::FeatureFlag;

use crate::terminal::model::TerminalModel;

/// Builds a single-chunk kitty graphics APC message.
fn kitty_apc(control_data: &str, payload: &[u8]) -> String {
    format!(
        "\x1b_G{};{}\x1b\\",
        control_data,
        base64::Engine::encode(&BASE64, payload)
    )
}

/// A one pixel, 24-bit RGB image, which is the smallest payload that passes
/// kitty's RGB size validation.
fn one_pixel_rgb() -> &'static [u8] {
    &[0xff, 0x00, 0x00]
}

/// A terminal with a running command, so that graphics land in a block's output
/// grid. Blocks that haven't started executing route to their header grid, which
/// doesn't handle kitty actions at all.
fn kitty_terminal() -> TerminalModel {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.simulate_cmd("kitty");
    terminal
}

fn reply_for(control_data: &str, payload: &[u8]) -> String {
    let mut terminal = kitty_terminal();
    let written = terminal.process_bytes_capturing(kitty_apc(control_data, payload).as_str());
    String::from_utf8_lossy(&written).into_owned()
}

#[test]
fn zero_size_transmit_and_display_does_not_panic() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let reply = reply_for("a=T,i=1,f=24,s=0,v=0", &[]);

    // The action is a no-op, but it must still be acknowledged rather than panic.
    assert!(reply.contains("i=1;OK"), "unexpected reply: {reply:?}");
}

#[test]
fn zero_size_display_of_stored_image_does_not_panic() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let mut terminal = kitty_terminal();
    terminal.process_bytes(kitty_apc("a=t,i=1,f=24,s=0,v=0", &[]).as_str());
    let written = terminal.process_bytes_capturing(kitty_apc("a=p,i=1", &[]).as_str());

    let reply = String::from_utf8_lossy(&written);
    assert!(reply.contains("i=1;OK"), "unexpected reply: {reply:?}");
}

#[test]
fn query_reply_is_sent_despite_quiet_mode() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let reply = reply_for("a=q,i=1,q=1,f=24,s=1,v=1", one_pixel_rgb());

    assert!(reply.contains("i=1;OK"), "unexpected reply: {reply:?}");
}

#[test]
fn unknown_image_id_error_reply_uses_enoent() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let reply = reply_for("a=p,i=999", &[]);

    assert!(
        reply.contains("i=999;ENOENT:"),
        "unexpected reply: {reply:?}"
    );
}

#[test]
fn ok_reply_echoes_image_and_placement_ids() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let reply = reply_for("a=T,i=7,p=3,f=24,s=1,v=1", one_pixel_rgb());

    assert!(reply.contains("i=7,p=3;OK"), "unexpected reply: {reply:?}");
}

#[test]
fn quiet_mode_one_suppresses_ok_but_not_errors() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let ok_reply = reply_for("a=T,i=1,q=1,f=24,s=1,v=1", one_pixel_rgb());
    assert!(ok_reply.is_empty(), "unexpected reply: {ok_reply:?}");

    let error_reply = reply_for("a=p,i=999,q=1", &[]);
    assert!(
        error_reply.contains("ENOENT:"),
        "unexpected reply: {error_reply:?}"
    );
}

#[test]
fn quiet_mode_two_suppresses_errors() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);

    let error_reply = reply_for("a=p,i=999,q=2", &[]);
    assert!(error_reply.is_empty(), "unexpected reply: {error_reply:?}");

    let ok_reply = reply_for("a=T,i=1,q=2,f=24,s=1,v=1", one_pixel_rgb());
    assert!(ok_reply.is_empty(), "unexpected reply: {ok_reply:?}");
}
