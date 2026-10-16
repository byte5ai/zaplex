use fluent::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

use super::*;

/// Renders one message of the German catalog the way the app does, without
/// the bidi isolates. The global loader holds one locale per process, so
/// German text is read from the catalog itself.
fn german(id: &str, args: Option<&FluentArgs>) -> String {
    // Unrelated entries the parser rejects only drop out; a broken message
    // under test then fails below as missing.
    let resource = FluentResource::try_new(include_str!("../../i18n/de/warp.ftl").to_owned())
        .unwrap_or_else(|(resource, _)| resource);
    let german: LanguageIdentifier = "de".parse().expect("valid language identifier");
    let mut bundle = FluentBundle::new(vec![german]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("the German catalog has unique message ids");
    let pattern = bundle
        .get_message(id)
        .and_then(|message| message.value())
        .unwrap_or_else(|| panic!("the German catalog has no value for {id}"));
    let mut errors = Vec::new();
    let text = bundle
        .format_pattern(pattern, args, &mut errors)
        .into_owned();
    assert!(errors.is_empty(), "formatting {id} failed: {errors:?}");
    text
}

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

/// Each value reaches its own message with its numbers in the right slots: a
/// swapped argument or unit would misstate a countdown by hours or days.
#[test]
fn countdowns_ages_and_states_are_spelled_from_the_catalog() {
    crate::i18n::init(Some("en"));
    assert_eq!(
        reset_text(ResetCountdown::HoursMinutes {
            hours: 2,
            minutes: 13
        }),
        "\u{2068}2\u{2069}h\u{2068}13\u{2069}m"
    );
    assert_eq!(
        reset_text(ResetCountdown::DaysHours { days: 4, hours: 1 }),
        "\u{2068}4\u{2069}d\u{2068}1\u{2069}h"
    );
    assert_eq!(age_text(RelativeAge::Hours(3)), "\u{2068}3\u{2069}h");
    assert_eq!(age_text(RelativeAge::Days(2)), "\u{2068}2\u{2069}d");
    let words = [
        state_text(StateWord::Waiting),
        state_text(StateWord::Working),
        state_text(StateWord::Idle),
    ];
    assert!(
        words[0] != words[1] && words[1] != words[2] && words[0] != words[2],
        "every state keeps its own word: {words:?}"
    );
}

/// #516: the session table's status word is the same German word as its
/// filter chip in the same table (one name per state).
#[test]
fn german_status_words_match_the_table_filters() {
    for (state, filter) in [
        (
            "cockpit-table-state-waiting",
            "cockpit-table-filter-waiting",
        ),
        ("cockpit-table-state-working", "cockpit-table-filter-active"),
        ("cockpit-table-state-idle", "cockpit-table-filter-idle"),
    ] {
        assert_eq!(german(state, None), german(filter, None), "{state}");
    }
}
