use super::{subscription_directory_from_input, subscription_directory_input_is_valid};
use std::path::PathBuf;

#[test]
fn remote_directory_input_accepts_only_absolute_posix_paths_or_host_home() {
    assert_eq!(
        subscription_directory_from_input("  /srv/project with spaces  "),
        Some(PathBuf::from("/srv/project with spaces"))
    );
    assert_eq!(
        subscription_directory_from_input("  .  "),
        Some(PathBuf::from("."))
    );
    assert_eq!(subscription_directory_from_input("../relative"), None);
    assert_eq!(subscription_directory_from_input("   "), None);
    assert!(subscription_directory_input_is_valid(" /srv/project "));
    assert!(subscription_directory_input_is_valid("."));
    assert!(!subscription_directory_input_is_valid("relative/path"));
    assert!(!subscription_directory_input_is_valid(""));
}
