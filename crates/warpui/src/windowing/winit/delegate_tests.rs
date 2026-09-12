#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use super::wsl_url_handler_command;

#[test]
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn wsl_url_handler_preserves_shell_metacharacters_as_one_argument() {
    let url = "https://example.com/search?a=1&b=2|three^four<five>(six)";

    assert_eq!(
        wsl_url_handler_command(url),
        ("rundll32.exe", ["url.dll,FileProtocolHandler", url])
    );
}
