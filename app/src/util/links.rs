use crate::channel::ChannelState;

// Upstream Warp's documentation site/Slack/privacy policy are no longer applicable to the Zaplex fork.
// These constants are retained as placeholder empty strings, to be filled once Zaplex's own channels are established.
// A destination that does not exist yet stays empty; the `has_*()` predicates below let every UI surface
// omit the control entirely rather than render a dead button that opens `""`.
pub const USER_DOCS_URL: &str = "";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/byte5ai/zaplex/issues";
// Where a test user can grab a build by hand when the in-app auto-update fails.
pub const GITHUB_RELEASES_URL: &str = "https://github.com/byte5ai/zaplex/releases";
pub const SLACK_URL: &str = "";
pub const PRIVACY_POLICY_URL: &str = "";

/// Whether Zaplex has a user-documentation destination yet. When `false`, surfaces
/// must omit their "Documentation" control instead of opening an empty URL.
pub fn has_user_docs() -> bool {
    !USER_DOCS_URL.is_empty()
}

/// Whether Zaplex has a community-chat destination yet. When `false`, surfaces
/// must omit their "Slack" control instead of opening an empty URL.
pub fn has_slack() -> bool {
    !SLACK_URL.is_empty()
}

/// Whether Zaplex has a privacy-policy destination yet. When `false`, surfaces
/// must omit their "Privacy Policy" control instead of opening an empty URL.
pub fn has_privacy_policy() -> bool {
    !PRIVACY_POLICY_URL.is_empty()
}

pub fn feedback_form_url() -> String {
    let mut url = url::Url::parse("https://github.com/byte5ai/zaplex/issues/new/choose")
        .expect("Should not fail to parse");
    if let Some(version) = ChannelState::app_version() {
        url.query_pairs_mut().append_pair("zap-version", version);
    }
    url.query_pairs_mut()
        .append_pair("os-version", &os_info::get().version().to_string());
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// User-facing "report an issue" / feedback links must point at zaplex's own
    /// tracker, never the upstream fork a test user would otherwise land on.
    #[test]
    fn issue_links_point_at_zaplex_not_upstream() {
        assert!(
            GITHUB_ISSUES_URL.contains("byte5ai/zaplex"),
            "GITHUB_ISSUES_URL leaks upstream: {GITHUB_ISSUES_URL}"
        );
        assert!(
            GITHUB_RELEASES_URL.contains("byte5ai/zaplex"),
            "GITHUB_RELEASES_URL leaks upstream: {GITHUB_RELEASES_URL}"
        );
        for upstream in ["zerx-lab/warp", "warpdotdev/warp"] {
            assert!(
                !GITHUB_ISSUES_URL.contains(upstream),
                "GITHUB_ISSUES_URL still points at {upstream}"
            );
            assert!(
                !GITHUB_RELEASES_URL.contains(upstream),
                "GITHUB_RELEASES_URL still points at {upstream}"
            );
            assert!(
                !feedback_form_url().contains(upstream),
                "feedback_form_url still points at {upstream}"
            );
        }
    }

    /// A placeholder (empty) destination must report as "not present" so every UI
    /// surface omits its control instead of rendering a dead button. If a real URL
    /// is filled in later, the predicate flips to `true` and the control returns.
    #[test]
    fn absent_destinations_report_as_missing() {
        assert_eq!(has_user_docs(), !USER_DOCS_URL.is_empty());
        assert_eq!(has_slack(), !SLACK_URL.is_empty());
        assert_eq!(has_privacy_policy(), !PRIVACY_POLICY_URL.is_empty());
    }
}
