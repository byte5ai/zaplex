use chrono::Utc;
use std::collections::BTreeMap;
use zaplex_cockpit::{Provider, SessionSnapshot, SessionState, WindowTotals};

use super::{
    matching_session_row, parse_hex_color, session_key, session_table_viewport_height,
    session_today_cost, TableRow, SESSION_TABLE_HEADER_HEIGHT, SESSION_TABLE_MAX_VISIBLE_ROWS,
    SESSION_TABLE_ROW_HEIGHT,
};

#[test]
fn remote_host_without_pricing_data_has_no_fabricated_cost() {
    let mut local_totals = BTreeMap::new();
    local_totals.insert(
        "same-session-id".to_string(),
        WindowTotals {
            cost_usd: 4.2,
            ..WindowTotals::default()
        },
    );

    assert_eq!(
        session_today_cost(false, "same-session-id", &local_totals),
        None,
        "a remote row must not inherit a same-id local transcript cost"
    );
    assert_eq!(
        session_today_cost(true, "same-session-id", &local_totals),
        Some(4.2)
    );
}

#[test]
fn session_table_body_clips_to_zone_card() {
    let capped = SESSION_TABLE_HEADER_HEIGHT
        + SESSION_TABLE_MAX_VISIBLE_ROWS as f32 * SESSION_TABLE_ROW_HEIGHT;
    assert_eq!(session_table_viewport_height(1), 62.0);
    assert_eq!(session_table_viewport_height(100), capped);
}

#[test]
fn cockpit_table_clips_to_card() {
    assert_eq!(
        session_table_viewport_height(SESSION_TABLE_MAX_VISIBLE_ROWS + 20),
        session_table_viewport_height(SESSION_TABLE_MAX_VISIBLE_ROWS)
    );
}

fn session(config_dir: Option<&str>, account_email: Option<&str>) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "copied-session".to_string(),
        cwd: "/work/project".to_string(),
        name: "job".to_string(),
        state: SessionState::Idle,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/work/project".to_string(),
        repo_root: "/work/project".to_string(),
        project_name: "project".to_string(),
        branch: None,
        worktree: None,
        config_dir: config_dir.map(str::to_string),
        account_email: account_email.map(str::to_string),
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: 0,
    }
}

fn row(session: SessionSnapshot) -> TableRow {
    TableRow::Session {
        session,
        host: None,
        host_id: None,
        is_local: true,
        today_cost: None,
    }
}

#[test]
fn row_menu_resolves_copied_id_to_exact_account() {
    let default = session(None, Some("shared@example.com"));
    let work = session(Some("/accounts/claude-work"), Some("shared@example.com"));
    let work_key = session_key(true, None, &work);
    let rows = vec![row(default), row(work)];

    let matched = matching_session_row(&rows, &work_key).expect("work-account row");
    assert_eq!(
        matched.session.config_dir.as_deref(),
        Some("/accounts/claude-work"),
        "the menu must not take the first same-id row from another account"
    );
}

#[test]
fn row_menu_refuses_duplicate_unknown_account_identity() {
    let first_unknown = session(None, None);
    let second_unknown = session(None, None);
    let ambiguous_key = session_key(true, None, &first_unknown);
    let rows = vec![row(first_unknown), row(second_unknown)];

    assert!(
        matching_session_row(&rows, &ambiguous_key).is_none(),
        "ambiguous legacy rows must fail closed instead of selecting the first account"
    );
}

#[test]
fn parses_six_digit_hex() {
    let c = parse_hex_color("#22C55E").expect("valid 6-digit hex");
    assert_eq!((c.r, c.g, c.b, c.a), (0x22, 0xC5, 0x5E, 255));
}

#[test]
fn parses_three_digit_shorthand() {
    // #f0a → ff 00 aa (each nibble doubled).
    let c = parse_hex_color("#f0a").expect("valid 3-digit hex");
    assert_eq!((c.r, c.g, c.b, c.a), (0xff, 0x00, 0xaa, 255));
}

#[test]
fn rejects_malformed_returns_none() {
    for bad in [
        "", "22C55E", "#", "#12", "#1234", "#12345", "#GGGGGG", "#12345Z",
    ] {
        assert!(parse_hex_color(bad).is_none(), "{bad:?} must not parse");
    }
}

/// The doc promised "never a panic" and did not deliver. `len()` counts
/// bytes; the slices index char boundaries. `#éa` measures 3 bytes, takes
/// the shorthand branch, and `&hex[0..1]` cuts the `é` in half — aborting
/// the app while it renders an account card, over a value someone typed into
/// instances.json by hand.
#[test]
fn a_non_ascii_colour_yields_no_tint_rather_than_taking_the_app_down() {
    // 3 bytes, 2 chars: exactly the shorthand branch's length check.
    assert_eq!(parse_hex_color("#éa"), None);
    // 6 bytes, 3 chars: the same trap on the long branch.
    assert_eq!(parse_hex_color("#ééé"), None);
    assert_eq!(parse_hex_color("#22C55é"), None);
    assert_eq!(parse_hex_color("#🎨🎨"), None);
}

/// Malformed-but-ASCII stays malformed — the guard must not start accepting
/// things it used to reject.
#[test]
fn ascii_rubbish_is_still_rejected() {
    for bad in [
        "#", "#12", "#1234", "#12345", "#1234567", "#GGGGGG", "22C55E", "",
    ] {
        assert_eq!(parse_hex_color(bad), None, "{bad:?} must not parse");
    }
}

/// …and the valid cases still work.
#[test]
fn the_guard_does_not_reject_real_colours() {
    assert!(parse_hex_color("#22C55E").is_some());
    assert!(parse_hex_color("#f0a").is_some());
    assert!(parse_hex_color("#FFFFFF").is_some());
}
