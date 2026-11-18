use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use env_logger::{Target, WriteStyle};
use log::{Level, Log, Record};

use super::format_for_terminal_output;

#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn terminal_line(level: Level, write_style: WriteStyle) -> String {
    let buffer = SharedBuffer::default();
    let logger = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .write_style(write_style)
        .target(Target::Pipe(Box::new(buffer.clone())))
        .format(format_for_terminal_output)
        .build();
    logger.log(
        &Record::builder()
            .level(level)
            .target("zaplex::test")
            .args(format_args!("message"))
            .build(),
    );
    String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn terminal_output_colors_info_blue_and_debug_green() {
    let info = terminal_line(Level::Info, WriteStyle::Always);
    assert!(info.contains("\u{1b}[34m[INFO]\u{1b}[0m"), "{info:?}");
    let debug = terminal_line(Level::Debug, WriteStyle::Always);
    assert!(debug.contains("\u{1b}[32m[DEBUG]\u{1b}[0m"), "{debug:?}");
    let tail = if cfg!(debug_assertions) {
        " \u{1b}[2m[zaplex::test] \u{1b}[0mmessage\n"
    } else {
        " message\n"
    };
    assert!(debug.ends_with(tail), "{debug:?}");
}

#[test]
fn terminal_output_without_styles_has_no_escape_codes() {
    let line = terminal_line(Level::Info, WriteStyle::Never);
    assert!(!line.contains('\u{1b}'), "{line:?}");
    assert!(line.contains(" [INFO] "), "{line:?}");
    assert!(line.ends_with(" message\n"), "{line:?}");
}
