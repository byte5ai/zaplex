use super::shell_quote_arg;
use crate::terminal::shell::ShellType;

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
