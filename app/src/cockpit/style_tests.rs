use super::*;

#[test]
fn cockpit_semantics_resolve_through_existing_theme_roles() {
    let appearance = Appearance::mock();
    let theme = appearance.theme();

    assert_eq!(attention_coloru(&appearance), theme.ui_warning_color());
    assert_eq!(
        status_dot_coloru(SessionState::Waiting, &appearance),
        theme.ui_warning_color()
    );
    assert_eq!(
        status_dot_coloru(SessionState::Active, &appearance),
        theme.ui_green_color()
    );
    assert_eq!(
        utilisation_coloru(NEARLY_FULL, &appearance),
        theme.ui_error_color()
    );
    assert_eq!(
        utilisation_coloru(NEARLY_FULL - 0.01, &appearance),
        theme.sub_text_color(theme.background()).into_solid()
    );
}

#[test]
fn attention_halo_keeps_the_accent_hue_at_static_low_alpha() {
    let appearance = Appearance::mock();
    let accent = attention_coloru(&appearance);
    let halo = attention_halo_coloru(&appearance);

    assert_eq!((halo.r, halo.g, halo.b), (accent.r, accent.g, accent.b));
    assert_eq!(halo.a, 38);
    assert!(halo.a < accent.a);
}

#[test]
fn style_has_no_unused_session_metric_width_contract() {
    let source = include_str!("style.rs");
    assert!(!source.contains("session_metric_column_width"));
    assert!(!source.contains("METRIC_COL_WIDTH"));

    for account_surface in [include_str!("panel.rs"), include_str!("pane.rs")] {
        assert!(!account_surface.contains("HEAT_BAR_WIDTH"));
        assert!(!account_surface.contains("with_width(90.0)"));
        assert!(!account_surface.contains("with_width(160.0)"));
    }
}

#[test]
fn provider_marks_are_distinct_and_never_the_accent_or_attention_role() {
    let appearance = Appearance::mock();
    let accent = appearance.theme().accent().into_solid();
    let attention = attention_coloru(&appearance);
    let claude = provider_mark_coloru(Provider::Claude, &appearance);
    let codex = provider_mark_coloru(Provider::Codex, &appearance);
    let antigravity = provider_mark_coloru(Provider::Antigravity, &appearance);

    assert_ne!(claude, codex);
    assert_ne!(claude, antigravity);
    assert_ne!(codex, antigravity);
    for mark in [claude, codex, antigravity] {
        assert_ne!(mark, accent);
        assert_ne!(mark, attention);
    }
    assert_ne!(
        provider_icon(Provider::Claude),
        provider_icon(Provider::Codex)
    );
}

#[test]
fn context_readout_starts_with_the_number_not_a_separator() {
    // With no model shown, a leading separator would dangle in the Context column.
    assert_eq!(ctx_pct_label(1, false), "1\u{202f}%");
    assert_eq!(ctx_pct_label(42, true), "42\u{202f}% ctx");
}
