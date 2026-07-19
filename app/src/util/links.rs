use crate::channel::ChannelState;

// Upstream Warp's documentation site/Slack/privacy policy are no longer applicable to the Zaplex fork.
// These constants are retained as placeholder empty strings, to be filled once Zaplex's own channels are established.
// `ctx.open_url("")` is a harmless no-op at the UI call site.
pub const USER_DOCS_URL: &str = "";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/byte5ai/zaplex/issues";
pub const SLACK_URL: &str = "";
pub const PRIVACY_POLICY_URL: &str = "";

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
        for upstream in ["zerx-lab/warp", "warpdotdev/warp"] {
            assert!(
                !GITHUB_ISSUES_URL.contains(upstream),
                "GITHUB_ISSUES_URL still points at {upstream}"
            );
            assert!(
                !feedback_form_url().contains(upstream),
                "feedback_form_url still points at {upstream}"
            );
        }
    }
}
