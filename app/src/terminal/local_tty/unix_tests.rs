use super::{get_pw_entry, get_pw_entry_once};

#[test]
fn get_pw_entry_once_reports_erange_without_initializing_output() {
    let mut buffer = [0; 1];
    let error = get_pw_entry_once(unsafe { libc::getuid() }, &mut buffer).unwrap_err();

    assert_eq!(error.raw_os_error(), Some(libc::ERANGE));
}

#[test]
fn get_pw_entry_returns_current_user_without_panicking() {
    let passwd = get_pw_entry().unwrap().unwrap();

    assert!(!passwd.name.is_empty());
    assert!(!passwd.dir.as_os_str().is_empty());
}
