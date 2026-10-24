use super::parse_interactive_ssh_command;

#[test]
fn ssh_control_commands_never_request_interactive_bootstrap() {
    for command in [
        "ssh -S '/tmp/test socket' -O exit example.invalid",
        "ssh -Ocheck example.invalid",
        "ssh -vOcheck example.invalid",
        "ssh example.invalid -O stop",
        "ssh example.invalid -Oexit",
        "command ssh -O forward -L 1234:localhost:5678 example.invalid",
        "ssh -O unknown-control-command example.invalid",
    ] {
        assert!(
            parse_interactive_ssh_command(command).is_none(),
            "{command}"
        );
    }
}

#[test]
fn ssh_option_values_do_not_turn_interactive_sessions_into_control_commands() {
    for command in [
        "ssh example.invalid",
        "ssh -i -Oidentity example.invalid",
        "ssh -vi -Oidentity example.invalid",
        "ssh -i-Oidentity example.invalid",
        "ssh -o 'ProxyCommand=printf -Ocheck' example.invalid",
        "ssh -S /tmp/-Osocket example.invalid",
    ] {
        let parsed = parse_interactive_ssh_command(command).expect(command);
        assert_eq!(parsed.host.as_deref(), Some("example.invalid"), "{command}");
    }
}
