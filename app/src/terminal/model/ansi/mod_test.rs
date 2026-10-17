use hex;
use warpui::color::ColorU;

use super::*;
use crate::terminal::model::index::VisibleRow;
use crate::terminal::model::selection::ScrollDelta;
use std::{io, io::Write};

const HEX_ENCODED_JSON_DCS_START: &[u8] = &[0x1b, 0x50, 0x24, 0x64];
const UNENCODED_JSON_DCS_START: &[u8] = &[0x1b, 0x50, 0x24, 0x66];
const DCS_END: &[u8] = &[0x9c];

struct MockHandler {
    index: CharsetIndex,
    charset: StandardCharset,
    attr: Option<Attr>,
    identity_reported: bool,
    d_proto_hooks: Vec<DProtoHook>,
    pluggable_notifications: Vec<(Option<String>, String)>,
}

impl Handler for MockHandler {
    fn terminal_attribute(&mut self, attr: Attr) {
        self.attr = Some(attr);
    }

    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        self.index = index;
        self.charset = charset;
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.index = index;
    }

    fn identify_terminal<W: io::Write>(&mut self, _: &mut W, _intermediate: Option<char>) {
        self.identity_reported = true;
    }

    fn report_xtversion<W: io::Write>(&mut self, _: &mut W) {}

    fn reset_state(&mut self) {
        *self = Self::default();
    }

    fn set_title(&mut self, _: Option<String>) {}

    fn set_cursor_style(&mut self, _: Option<super::CursorStyle>) {}

    fn set_cursor_shape(&mut self, _shape: super::CursorShape) {}

    fn input(&mut self, _c: char) {}

    fn goto(&mut self, _: VisibleRow, _: usize) {}

    fn goto_line(&mut self, _: VisibleRow) {}

    fn goto_col(&mut self, _: usize) {}

    fn insert_blank(&mut self, _: usize) {}

    fn move_up(&mut self, _: usize) {}

    fn move_down(&mut self, _: usize) {}

    fn device_status<W: io::Write>(&mut self, _: &mut W, _: usize) {}

    fn move_forward(&mut self, _: usize) {}

    fn move_backward(&mut self, _: usize) {}

    fn move_down_and_cr(&mut self, _: usize) {}

    fn move_up_and_cr(&mut self, _: usize) {}

    fn put_tab(&mut self, _count: u16) {}

    fn backspace(&mut self) {}

    fn carriage_return(&mut self) {}

    fn linefeed(&mut self) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn bell(&mut self) {}

    fn substitute(&mut self) {}

    fn newline(&mut self) {}

    fn set_horizontal_tabstop(&mut self) {}

    fn scroll_up(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn scroll_down(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn insert_blank_lines(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn delete_lines(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn erase_chars(&mut self, _: usize) {}

    fn delete_chars(&mut self, _: usize) {}

    fn move_backward_tabs(&mut self, _count: u16) {}

    fn move_forward_tabs(&mut self, _count: u16) {}

    fn save_cursor_position(&mut self) {}

    fn restore_cursor_position(&mut self) {}

    fn clear_line(&mut self, _mode: super::LineClearMode) {}

    fn clear_screen(&mut self, _mode: super::ClearMode) {}

    fn clear_tabs(&mut self, _mode: super::TabulationClearMode) {}

    fn reverse_index(&mut self) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn set_mode(&mut self, _mode: super::Mode) {}

    fn unset_mode(&mut self, _: super::Mode) {}

    fn set_scrolling_region(&mut self, _top: usize, _bottom: Option<usize>) {}

    fn set_keypad_application_mode(&mut self) {}

    fn unset_keypad_application_mode(&mut self) {}

    fn set_color(&mut self, _: usize, _: ColorU) {}

    fn dynamic_color_sequence<W: io::Write>(&mut self, _: &mut W, _: u8, _: usize, _: &str) {}

    fn reset_color(&mut self, _: usize) {}

    fn clipboard_store(&mut self, _: u8, _: &[u8]) {}

    fn clipboard_load(&mut self, _: u8, _: &str) {}

    fn decaln(&mut self) {}

    fn push_title(&mut self) {}

    fn pop_title(&mut self) {}

    fn text_area_size_pixels<W: io::Write>(&mut self, _: &mut W) {}

    fn text_area_size_chars<W: io::Write>(&mut self, _: &mut W) {}

    fn command_finished(&mut self, data: CommandFinishedValue) {
        self.d_proto_hooks
            .push(DProtoHook::CommandFinished { value: data });
    }

    fn precmd(&mut self, data: PrecmdValue) {
        self.d_proto_hooks.push(DProtoHook::Precmd { value: data });
    }

    fn preexec(&mut self, data: PreexecValue) {
        self.d_proto_hooks.push(DProtoHook::Preexec { value: data });
    }

    fn bootstrapped(&mut self, data: BootstrappedValue) {
        self.d_proto_hooks.push(DProtoHook::Bootstrapped {
            value: Box::new(data),
        });
    }

    fn pre_interactive_ssh_session(&mut self, data: PreInteractiveSSHSessionValue) {
        self.d_proto_hooks
            .push(DProtoHook::PreInteractiveSSHSession { value: data })
    }

    fn ssh(&mut self, data: SSHValue) {
        self.d_proto_hooks.push(DProtoHook::SSH { value: data });
    }

    fn init_shell(&mut self, data: InitShellValue) {
        self.d_proto_hooks
            .push(DProtoHook::InitShell { value: data });
    }

    fn clear(&mut self, data: ClearValue) {
        self.d_proto_hooks.push(DProtoHook::Clear { value: data });
    }

    fn input_buffer(&mut self, data: super::InputBufferValue) {
        self.d_proto_hooks
            .push(DProtoHook::InputBuffer { value: data })
    }

    fn init_subshell(&mut self, data: InitSubshellValue) {
        self.d_proto_hooks
            .push(DProtoHook::InitSubshell { value: data })
    }

    fn init_ssh(&mut self, data: InitSshValue) {
        self.d_proto_hooks.push(DProtoHook::InitSsh { value: data })
    }

    fn sourced_rc_file(&mut self, data: SourcedRcFileForWarpValue) {
        self.d_proto_hooks
            .push(DProtoHook::SourcedRcFileForWarp { value: data })
    }

    fn pluggable_notification(&mut self, title: Option<String>, body: String) {
        self.pluggable_notifications.push((title, body));
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        _mode: KeyboardModes,
        _apply: KeyboardModesApplyBehavior,
    ) {
    }

    fn push_keyboard_enhancement_flags(&mut self, _mode: KeyboardModes) {}

    fn pop_keyboard_enhancement_flags(&mut self, _count: u16) {}

    fn query_keyboard_enhancement_flags<W: io::Write>(&mut self, _: &mut W) {}
}

impl Default for MockHandler {
    fn default() -> MockHandler {
        MockHandler {
            index: CharsetIndex::G0,
            charset: StandardCharset::Ascii,
            attr: None,
            identity_reported: false,
            d_proto_hooks: Vec::new(),
            pluggable_notifications: Vec::new(),
        }
    }
}

fn hex_encoded_dcs_string(dcs_payload: &str) -> Vec<u8> {
    let encoded_dcs_string = hex::encode(dcs_payload).into_bytes();
    [HEX_ENCODED_JSON_DCS_START, &encoded_dcs_string, DCS_END].concat()
}

fn parse_bytes(bytes: &[u8]) -> (Processor, MockHandler) {
    let mut parser = Processor::new();
    let mut handler = MockHandler::default();

    parser.parse_bytes(&mut handler, bytes, &mut io::sink());

    (parser, handler)
}

#[test]
fn parse_truecolor_attr() {
    static BYTES: &[u8] = &[
        0x1b, b'[', b'3', b'8', b';', b'2', b';', b'1', b'2', b'8', b';', b'6', b'6', b';', b'2',
        b'5', b'5', b'm',
    ];

    let (_, handler) = parse_bytes(BYTES);

    let spec = ColorU::new(128, 66, 255, 0xff);

    assert_eq!(handler.attr, Some(Attr::Foreground(Color::Spec(spec))));
}

/// No exactly a test; useful for debugging.
#[test]
fn parse_zsh_startup() {
    static BYTES: &[u8] = &[
        0x1b, b'[', b'1', b'm', 0x1b, b'[', b'7', b'm', b'%', 0x1b, b'[', b'2', b'7', b'm', 0x1b,
        b'[', b'1', b'm', 0x1b, b'[', b'0', b'm', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b'\r', b' ', b'\r',
        b'\r', 0x1b, b'[', b'0', b'm', 0x1b, b'[', b'2', b'7', b'm', 0x1b, b'[', b'2', b'4', b'm',
        0x1b, b'[', b'J', b'j', b'w', b'i', b'l', b'm', b'@', b'j', b'w', b'i', b'l', b'm', b'-',
        b'd', b'e', b's', b'k', b' ', 0x1b, b'[', b'0', b'1', b';', b'3', b'2', b'm', 0xe2, 0x9e,
        0x9c, b' ', 0x1b, b'[', b'0', b'1', b';', b'3', b'2', b'm', b' ', 0x1b, b'[', b'3', b'6',
        b'm', b'~', b'/', b'c', b'o', b'd', b'e',
    ];

    parse_bytes(BYTES);
}

#[test]
fn parse_valid_rgb_colors() {
    assert_eq!(
        xparse_color(b"rgb:f/e/d"),
        Some(ColorU::new(0xff, 0xee, 0xdd, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:11/aa/ff"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:f/ed1/cb23"),
        Some(ColorU::new(0xff, 0xec, 0xca, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:ffff/0/0"),
        Some(ColorU::new(0xff, 0x0, 0x0, 0xff))
    );
}

#[test]
fn parse_valid_legacy_rgb_colors() {
    assert_eq!(
        xparse_color(b"#1af"),
        Some(ColorU::new(0x10, 0xa0, 0xf0, 0xff))
    );
    assert_eq!(
        xparse_color(b"#11aaff"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"#110aa0ff0"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"#1100aa00ff00"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
}

#[test]
fn parse_invalid_rgb_colors() {
    assert_eq!(xparse_color(b"rgb:0//"), None);
    assert_eq!(xparse_color(b"rgb://///"), None);
}

#[test]
fn parse_invalid_legacy_rgb_colors() {
    assert_eq!(xparse_color(b"#"), None);
    assert_eq!(xparse_color(b"#f"), None);
}

#[test]
fn named_color_to_ansi_escape_valid() {
    assert!(matches!(NamedColor::Red.to_ansi_fg_escape_code(), Ok(31)));
    assert!(matches!(NamedColor::Red.to_ansi_bg_escape_code(), Ok(41)));
    assert!(matches!(
        NamedColor::BrightGreen.to_ansi_fg_escape_code(),
        Ok(92)
    ));
    assert!(matches!(
        NamedColor::BrightBlue.to_ansi_bg_escape_code(),
        Ok(104)
    ));
}

#[test]
fn named_color_to_ansi_escape_invalid() {
    assert!(NamedColor::Background.to_ansi_fg_escape_code().is_err());
    assert!(NamedColor::Foreground.to_ansi_bg_escape_code().is_err());
    assert!(NamedColor::Cursor.to_ansi_bg_escape_code().is_err());
}

#[test]
fn parse_sourced_rc_file_hook() {
    let rc_file_hook = r#"{"hook": "SourcedRcFileForWarp", "value": { "shell": "zsh" }}"#;
    let bytes = [
        UNENCODED_JSON_DCS_START,
        &Vec::from(rc_file_hook.as_bytes()),
        DCS_END,
    ]
    .concat();

    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SourcedRcFileForWarp { value } => assert_eq!(
            *value,
            SourcedRcFileForWarpValue {
                shell: "zsh".to_owned(),
                uname: None,
                tmux: None,
            }
        ),
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_sourced_rc_file_hook_with_uname() {
    let rc_file_hook =
        r#"{"hook": "SourcedRcFileForWarp", "value": { "shell": "zsh", "uname": "Darwin" }}"#;
    let bytes = [
        UNENCODED_JSON_DCS_START,
        &Vec::from(rc_file_hook.as_bytes()),
        DCS_END,
    ]
    .concat();

    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SourcedRcFileForWarp { value } => assert_eq!(
            *value,
            SourcedRcFileForWarpValue {
                shell: "zsh".to_owned(),
                uname: Some("Darwin".to_owned()),
                tmux: None,
            }
        ),
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_osc9_notification_with_st_terminator() {
    let bytes: &[u8] = b"\x1b]9;Message with ST terminator\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(*title, None);
    assert_eq!(body, "Message with ST terminator");
}

#[test]
fn parse_osc777_notification_empty_title() {
    let bytes: &[u8] = b"\x1b]777;notify;;Just the body\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(*title, None);
    assert_eq!(body, "Just the body");
}

#[test]
fn tmux_pane_writer_formats_bytes_as_send_keys() {
    // Test that TmuxPaneWriter correctly converts writes to tmux send-keys format
    let mut output = Vec::new();
    {
        let mut writer = super::TmuxPaneWriter::new(&mut output, 123);
        // Write a cursor position response (ESC[1;1R)
        writer.write_all(b"\x1b[1;1R").unwrap();
    }

    let output_str = String::from_utf8(output).unwrap();
    // The output should be a send-keys command with hex bytes
    // Format: send-keys -Ht %{pane_id} {hex} {hex}...\n
    assert!(output_str.starts_with("send-keys -Ht %123"));
    assert!(output_str.contains("1B")); // ESC = 0x1B
    assert!(output_str.ends_with('\n'));
}

#[test]
fn tmux_pane_writer_empty_write_returns_zero() {
    let mut output = Vec::new();
    let mut writer = super::TmuxPaneWriter::new(&mut output, 42);
    let result = writer.write(&[]).unwrap();

    assert_eq!(result, 0);
    assert!(output.is_empty());
}

#[test]
fn tmux_pane_writer_returns_original_byte_count() {
    let mut output = Vec::new();
    let mut writer = super::TmuxPaneWriter::new(&mut output, 42);
    let input = b"test";
    let result = writer.write(input).unwrap();

    assert_eq!(result, 4);
    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.starts_with("send-keys -Ht %42"));
    assert!(output_str.ends_with('\n'));
}
