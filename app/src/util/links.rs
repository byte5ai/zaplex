use crate::channel::ChannelState;

// Upstream Warp's documentation site/Slack/privacy policy are no longer applicable to the Zaplex fork.
// These constants are retained as placeholder empty strings, to be filled once Zaplex's own channels are established.
// `ctx.open_url("")` is a harmless no-op at the UI call site.
pub const USER_DOCS_URL: &str = "";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/zerx-lab/warp/issues";
pub const SLACK_URL: &str = "";
pub const PRIVACY_POLICY_URL: &str = "";

pub fn feedback_form_url() -> String {
    let mut url = url::Url::parse("https://github.com/zerx-lab/warp/issues/new/choose")
        .expect("Should not fail to parse");
    if let Some(version) = ChannelState::app_version() {
        url.query_pairs_mut().append_pair("zap-version", version);
    }
    url.query_pairs_mut()
        .append_pair("os-version", &os_info::get().version().to_string());
    url.to_string()
}
