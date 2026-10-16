use super::*;

#[test]
fn amounts_round_to_whole_cents() {
    let parts = |usd| {
        let (whole, cents) = dollars_and_cents(usd);
        format!("{whole}|{cents}")
    };
    assert_eq!(parts(4.2), "4|20");
    assert_eq!(parts(0.999), "1|00");
    // 19.005 is stored as 19.00499…: it rounds down, as the stored value says.
    assert_eq!(parts(19.005), "19|00");
    assert_eq!(parts(1234.567), "1234|57");
    assert_eq!(parts(-1.0), "0|00");
}

/// #515: a partly priced sum keeps its amount and says what it leaves out.
#[test]
fn cost_text_keeps_the_priced_part_of_a_partial_sum() {
    crate::i18n::init(Some("en"));
    let partial = CostFigure::Partial {
        usd: 4.2,
        unpriced_models: vec!["claude-next".to_string(), "gpt-5.5".to_string()],
    };
    let amount = cost_amount_text(&partial);
    assert_eq!(amount, cost_text(&CostFigure::Estimate { usd: 4.2 }));
    let hint = cost_hint_text(&partial).expect("a partial sum carries a visible hint");
    assert_eq!(cost_text(&partial), format!("{amount} · {hint}"));
    assert!(unpriced_models_text(&partial)
        .expect("a partial sum names its unpriced models")
        .contains("claude-next, gpt-5.5"));
    assert_eq!(cost_hint_text(&CostFigure::Estimate { usd: 4.2 }), None);
    assert_eq!(
        unpriced_models_text(&CostFigure::Estimate { usd: 4.2 }),
        None
    );
}

/// #515: nothing priced shows the localized label, never a fictional amount.
#[test]
fn unpriced_label_is_localized() {
    crate::i18n::init(Some("en"));
    let unpriced = CostFigure::Unpriced {
        unpriced_models: vec!["gpt-5.5".to_string()],
    };
    assert_eq!(cost_text(&unpriced), crate::t!("cockpit-cost-unpriced"));
    assert_ne!(
        cost_text(&unpriced),
        cost_text(&CostFigure::Estimate { usd: 0.0 })
    );
    assert_eq!(cost_hint_text(&unpriced), None);
    assert!(unpriced_models_text(&unpriced)
        .expect("an unpriced sum names its models")
        .contains("gpt-5.5"));
}
