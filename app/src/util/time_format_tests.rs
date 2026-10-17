use super::*;

#[test]
fn test_human_readable_precise_duration() {
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(3)),
        "3 ms".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(10)),
        "10 ms".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(3141)),
        "3.14 sec".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(19961)),
        "20.0 sec".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::seconds(61)),
        "1.02 min".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::minutes(930)),
        "15.5 hours".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::hours(46)),
        "1.92 days".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::weeks(2)),
        ">1 week".to_owned()
    );
}

#[test]
fn test_human_readable_approx_duration() {
    crate::i18n::init(Some("en"));
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), false),
        "just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), false),
        "just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), true),
        "Just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), true),
        "Just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(90), false),
        "\u{2068}1\u{2069} min ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(100), false),
        "1 hour ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(130), false),
        "\u{2068}2\u{2069} hours ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::days(4), false),
        "\u{2068}4\u{2069} days ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(1), false),
        "1 week ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(15), false),
        "\u{2068}3\u{2069} months ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(520), false),
        "\u{2068}9\u{2069} years ago".to_owned()
    );
}
