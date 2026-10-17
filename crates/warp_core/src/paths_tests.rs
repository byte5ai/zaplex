use dirs::home_dir;

use super::*;

#[test]
fn test_cache_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(cache_dir(), home_dir.join("Library/Application Support/dev.zaplex.Zaplex"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(cache_dir(), home_dir.join(".cache/zap"));
        } else if #[cfg(windows)] {
            assert_eq!(cache_dir(), home_dir.join("AppData\\Local\\zaplex\\Zaplex\\cache"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_oss_secure_state_dir_is_disabled() {
    // ChannelState defaults to Channel::Oss. Zaplex should not probe the official Zaplex App Group,
    // otherwise macOS will identify it as accessing other app data and show a permissions
    // dialog on every startup.
    assert_eq!(secure_state_dir(), None);
}
