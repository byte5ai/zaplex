use std::path::Path;

use super::{atomic_replace_command, atomic_replace_operand};

#[test]
fn atomic_replace_command_shell_quotes_both_paths() {
    let command = atomic_replace_command(
        Path::new("stage; touch injected"),
        Path::new("target's name"),
    )
    .unwrap();

    assert_eq!(
        command,
        "/bin/sh -c 'if [ -d \"$2\" ]; then echo \"destination is a directory\" >&2; exit 73; fi; exec /bin/mv -f \"$1\" \"$2\"' zaplex-atomic-replace './stage; touch injected' './target'\\''s name'"
    );
}

#[test]
fn atomic_replace_operand_disarms_leading_option_marker() {
    assert_eq!(
        atomic_replace_operand(Path::new("-dangerous-option")).unwrap(),
        "./-dangerous-option"
    );
}

#[test]
fn atomic_replace_operand_keeps_absolute_path() {
    assert_eq!(
        atomic_replace_operand(Path::new("/tmp/remote-file")).unwrap(),
        "/tmp/remote-file"
    );
}
