use std::ffi::OsString;
use std::path::Path;

use super::{admin_script_arguments, CREATE_SYMLINK_ADMIN_SCRIPT, REMOVE_FILE_ADMIN_SCRIPT};

#[test]
fn install_admin_script_passes_paths_as_separate_arguments() {
    let source = Path::new("/Applications/Zaplex \"$(touch injected)\".app/Contents/MacOS/zaplex");
    let target = Path::new("/usr/local/bin/zaplex 'preview'");

    assert_eq!(
        admin_script_arguments(CREATE_SYMLINK_ADMIN_SCRIPT, &[source, target]),
        vec![
            OsString::from("-e"),
            OsString::from(CREATE_SYMLINK_ADMIN_SCRIPT),
            OsString::from("--"),
            source.as_os_str().to_owned(),
            target.as_os_str().to_owned(),
        ]
    );
    assert!(CREATE_SYMLINK_ADMIN_SCRIPT.contains("quoted form of (item 1 of argv)"));
    assert!(CREATE_SYMLINK_ADMIN_SCRIPT.contains("quoted form of (item 2 of argv)"));
}

#[test]
fn uninstall_admin_script_uses_the_same_argument_boundary() {
    let target = Path::new("/usr/local/bin/zaplex \"$(touch injected)\"");

    assert_eq!(
        admin_script_arguments(REMOVE_FILE_ADMIN_SCRIPT, &[target]),
        vec![
            OsString::from("-e"),
            OsString::from(REMOVE_FILE_ADMIN_SCRIPT),
            OsString::from("--"),
            target.as_os_str().to_owned(),
        ]
    );
    assert!(REMOVE_FILE_ADMIN_SCRIPT.contains("quoted form of (item 1 of argv)"));
}
