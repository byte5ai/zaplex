use anyhow::Result;
use objc2_foundation::NSBundle;

/// Apple Developer Team ID used for code-signature validation.
///
/// Distribution builds derive this value from the imported Developer ID
/// certificate and expose it through `ZAPLEX_APPLE_TEAM_ID`. The fallback
/// preserves inherited non-OSS development channels.
pub const APPLE_TEAM_ID: &str = match option_env!("ZAPLEX_APPLE_TEAM_ID") {
    Some(team_id) => team_id,
    None => "2BBY89MBSN",
};

/// Get the path to the macOS `.app` bundle.
pub fn get_bundle_path() -> Result<String> {
    let bundle = NSBundle::mainBundle();
    let path = bundle.bundlePath();
    Ok(path.to_string())
}
