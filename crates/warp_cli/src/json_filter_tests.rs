//! Tests for [`super::parse_jq_filter`] and the clap-flattened [`super::JsonOutput`].

use clap::Parser;

use super::*;

/// Tiny wrapper so we can exercise `JsonOutput` through clap in isolation.
#[derive(Debug, Parser)]
struct TestApp {
    #[clap(flatten)]
    json_filter: JsonOutput,
}

#[test]
fn parse_jq_filter_rejects_syntax_error() {
    let err = parse_jq_filter("@").expect_err("syntax error should be rejected");
    assert!(
        err.contains("`@`"),
        "error should quote the filter source with backticks, got: {err}"
    );
}

#[test]
fn clap_filter_is_none_by_default() {
    let app = TestApp::try_parse_from(["test"]).expect("no --jq parses");
    assert!(app.json_filter.filter.is_none());
}
