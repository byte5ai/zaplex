use super::*;

#[test]
fn tmux_inventory_preserves_session_names_as_data() {
    let sessions = parse_tmux_sessions(
        b"release candidate; touch /tmp/never\t3\t1\nplain\t1\t0\n",
        MultiplexerKind::Tmux,
    )
    .unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].target, "release candidate; touch /tmp/never");
    assert_eq!(sessions[0].name, sessions[0].target);
    assert_eq!(sessions[0].windows, 3);
    assert_eq!(sessions[0].attached_clients, 1);
    assert_eq!(sessions[0].kind, MultiplexerKind::Tmux as i32);
}

#[test]
fn tmux_inventory_rejects_malformed_or_ambiguous_rows() {
    for input in [
        b"missing-counts\n".as_slice(),
        b"\t1\t0\n".as_slice(),
        b"name\tnot-a-number\t0\n".as_slice(),
        b"name\t1\tnot-a-number\n".as_slice(),
        b"name\t1\t0\textra\n".as_slice(),
        b"name\xff\t1\t0\n".as_slice(),
    ] {
        assert!(parse_tmux_sessions(input, MultiplexerKind::Tmux).is_err());
    }
}

#[test]
fn byobu_screen_inventory_keeps_exact_target_and_clean_display_name() {
    let sessions = parse_byobu_screen_sessions(
        b"There are screens on:\n\t1234.ops\t(Detached)\n\t5678.release\t(Attached)\n2 Sockets in /run/screen/S-user.\n",
    )
    .unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].target, "1234.ops");
    assert_eq!(sessions[0].name, "ops");
    assert_eq!(sessions[0].attached_clients, 0);
    assert_eq!(sessions[1].target, "5678.release");
    assert_eq!(sessions[1].name, "release");
    assert_eq!(sessions[1].attached_clients, 1);
    assert!(sessions
        .iter()
        .all(|session| session.kind == MultiplexerKind::ByobuScreen as i32));
}

#[test]
fn byobu_screen_inventory_rejects_unrecognized_success_output() {
    assert!(parse_byobu_screen_sessions(
        b"There are screens on:\nnot-a-session\n.pidless\n1 Socket in /run/screen/S-user.\n",
    )
    .is_err());
}

#[test]
fn backend_file_accepts_only_known_literal_values() {
    assert_eq!(
        parse_byobu_backend("BYOBU_BACKEND=screen\n").as_deref(),
        Some("screen")
    );
    assert_eq!(
        parse_byobu_backend("BYOBU_BACKEND=tmux\n").as_deref(),
        Some("tmux")
    );
    assert_eq!(
        parse_byobu_backend("BYOBU_BACKEND=screen; touch /tmp/never\n"),
        None
    );
}
