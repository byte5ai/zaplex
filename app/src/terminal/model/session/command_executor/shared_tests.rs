use super::{shell_escape_single_quotes, shell_quote_arg};
use crate::terminal::shell::ShellType;

#[test]
fn paths_without_quotes_are_preserved_for_supported_shells() {
    for shell_type in [
        ShellType::Bash,
        ShellType::Zsh,
        ShellType::Fish,
        ShellType::PowerShell,
    ] {
        assert_eq!(
            shell_quote_arg("/home/user/history file", shell_type),
            "'/home/user/history file'"
        );
    }
}

#[test]
fn single_quotes_are_escaped_for_bash_and_zsh() {
    for shell_type in [ShellType::Bash, ShellType::Zsh] {
        assert_eq!(
            shell_quote_arg("/tmp/it's history", shell_type),
            r#"'/tmp/it'"'"'s history'"#
        );
    }
}

#[test]
fn single_quotes_are_escaped_for_fish() {
    assert_eq!(
        shell_quote_arg("/tmp/it's history", ShellType::Fish),
        r"'/tmp/it\'s history'"
    );
}

#[test]
fn single_quotes_are_escaped_for_powershell() {
    assert_eq!(
        shell_quote_arg("C:\\Users\\it's history", ShellType::PowerShell),
        "'C:\\Users\\it''s history'"
    );
}

#[test]
fn injection_payload_remains_inside_the_quoted_argument() {
    let payload = "/tmp/history'; touch /tmp/zaplex-injected; echo '";

    assert_eq!(
        shell_quote_arg(payload, ShellType::Bash),
        r#"'/tmp/history'"'"'; touch /tmp/zaplex-injected; echo '"'"''"#
    );
    assert_eq!(
        shell_quote_arg(payload, ShellType::Fish),
        r"'/tmp/history\'; touch /tmp/zaplex-injected; echo \''"
    );
    assert_eq!(
        shell_quote_arg(payload, ShellType::PowerShell),
        "'/tmp/history''; touch /tmp/zaplex-injected; echo '''"
    );
}

#[test]
fn escaping_does_not_add_or_remove_shell_metacharacters() {
    let value = "$HOME/$(touch /tmp/zaplex-injected);&|<>`";

    for shell_type in [
        ShellType::Bash,
        ShellType::Zsh,
        ShellType::Fish,
        ShellType::PowerShell,
    ] {
        let quoted = shell_quote_arg(value, shell_type);
        assert_eq!(&quoted[1..quoted.len() - 1], value);
        assert_eq!(shell_escape_single_quotes(value, shell_type), value);
    }
}
