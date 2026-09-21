use super::terminal_identity;

#[test]
fn terminal_identity_uses_project_basename_and_preserves_full_path() {
    let identity = terminal_identity("prod@example", Some("/srv/projects/zaplex"), "zsh");

    assert_eq!(identity.short, "prod@example · zaplex");
    assert_eq!(identity.full, "prod@example · /srv/projects/zaplex");
}

#[test]
fn terminal_identity_has_an_honest_fallback() {
    let identity = terminal_identity("Local", None, "fish");

    assert_eq!(identity.short, "Local · fish");
    assert_eq!(identity.full, "Local · fish");
}

#[test]
fn terminal_identity_accepts_windows_paths() {
    let identity = terminal_identity("build-host", Some(r"C:\work\zaplex\"), "pwsh");

    assert_eq!(identity.short, "build-host · zaplex");
    assert_eq!(identity.full, r"build-host · C:\work\zaplex\");
}
