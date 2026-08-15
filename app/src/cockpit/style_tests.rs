use super::*;

const WHITE: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};
const BLACK: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};
/// A representative near-black terminal background (the shipped dark theme
/// sits around here); the heat palette is validated against it.
const NEAR_BLACK: ColorU = ColorU {
    r: 20,
    g: 20,
    b: 20,
    a: 255,
};

const LEVELS: [HeatLevel; 5] = [
    HeatLevel::Ok,
    HeatLevel::Elevated,
    HeatLevel::High,
    HeatLevel::Critical,
    HeatLevel::Over,
];

#[test]
fn contrast_ratio_is_symmetric_and_bounded() {
    // Black on white is the canonical 21:1 maximum; a color against itself
    // is 1:1. Order must not matter.
    assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 0.05);
    assert!((contrast_ratio(WHITE, BLACK) - 21.0).abs() < 0.05);
    assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-9);
}

/// L1 contrast test: every heat band must clear the WCAG 3:1 graphical-object
/// threshold against BOTH theme backgrounds when picked via [`heat_coloru_on`]
/// — otherwise a status dot or meter fill washes out on one theme.
#[test]
fn heat_palette_legible_on_both_themes() {
    for level in LEVELS {
        let on_light = heat_coloru_on(level, WHITE);
        let cr_light = contrast_ratio(on_light, WHITE);
        assert!(
            cr_light >= 3.0,
            "{level:?} washes out on a light surface (contrast {cr_light:.2})"
        );

        let on_dark = heat_coloru_on(level, NEAR_BLACK);
        let cr_dark = contrast_ratio(on_dark, NEAR_BLACK);
        assert!(
            cr_dark >= 3.0,
            "{level:?} washes out on a dark surface (contrast {cr_dark:.2})"
        );
    }
}

/// The dark and light tables are genuinely different (the light variant is
/// the darker, more saturated tone) — a regression that collapsed them would
/// silently reintroduce the wash-out.
#[test]
fn heat_on_light_differs_from_dark_default() {
    for level in LEVELS {
        assert_ne!(heat_coloru_on(level, WHITE), heat_coloru(level));
        assert_eq!(heat_coloru_on(level, NEAR_BLACK), heat_coloru(level));
    }
}

/// E6: the attention accent is the ONE thing meant to be unmissable, so it
/// must survive both themes. It used to hand back the dark hue flat — the
/// waiting mark fading on a light background, which is the worst mark to
/// lose. WCAG calls 3:1 the floor for a non-text graphic; the accent clears
/// it on either surface.
#[test]
fn the_attention_accent_is_legible_on_both_themes() {
    for bg in [NEAR_BLACK, WHITE] {
        let c = heat_coloru_on(HeatLevel::Critical, bg);
        let ratio = contrast_ratio(c, bg);
        assert!(
            ratio >= 3.0,
            "attention amber on {bg:?} has contrast {ratio:.2} — below the 3:1 \
             floor for a graphic that must always be seen"
        );
    }
    // …and it is not the same hue on both: picking one would mean one theme
    // gets the worse of the two.
    assert_ne!(
        heat_coloru_on(HeatLevel::Critical, NEAR_BLACK),
        heat_coloru_on(HeatLevel::Critical, WHITE),
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

/// The same floor for every band, on both surfaces — a meter or dot that
/// only reads on one theme is half a signal.
#[test]
fn every_heat_band_clears_the_contrast_floor_on_both_themes() {
    for level in [
        HeatLevel::Ok,
        HeatLevel::Elevated,
        HeatLevel::High,
        HeatLevel::Critical,
        HeatLevel::Over,
    ] {
        for bg in [NEAR_BLACK, WHITE] {
            let ratio = contrast_ratio(heat_coloru_on(level, bg), bg);
            assert!(
                ratio >= 3.0,
                "{level:?} on {bg:?}: contrast {ratio:.2} < 3.0"
            );
        }
    }
}

#[test]
fn metric_column_width_does_not_change_with_label_length() {
    assert_eq!(
        session_metric_column_width("a"),
        session_metric_column_width(
            "a-very-long-worktree-label-that-must-shrink-before-the-metric-column"
        )
    );
    assert_eq!(session_metric_column_width("a"), METRIC_COL_WIDTH);
}
