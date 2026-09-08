use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

#[test]
fn daemon_bootstrap_delivery_is_single_and_shell_specific() {
    assert_eq!(
        daemon_bootstrap_delivery(Some(ShellType::Bash)),
        DaemonBootstrapDelivery::OrderedPty
    );
    assert_eq!(
        daemon_bootstrap_delivery(Some(ShellType::Zsh)),
        DaemonBootstrapDelivery::OrderedPty
    );
    assert_eq!(
        daemon_bootstrap_delivery(Some(ShellType::Fish)),
        DaemonBootstrapDelivery::GuardedFile
    );
    assert_eq!(
        daemon_bootstrap_delivery(Some(ShellType::PowerShell)),
        DaemonBootstrapDelivery::GuardedFile
    );
    assert_eq!(
        daemon_bootstrap_delivery(None),
        DaemonBootstrapDelivery::NoIntegration
    );
}

#[test]
fn test_asset_provider_cannot_poison_real_bootstrap_body() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
    assert!(
        decode_script(&script_for_shell(ShellType::Fish, &crate::ASSETS))
            .starts_with("if test \"$ZAPLEX_BOOTSTRAPPED\" != 1\n"),
        "a test provider must not populate the production asset cache"
    );
}

#[test]
fn fish_bootstrap_body_is_delivered_idempotently() {
    let script = bundled_script("bundled/bootstrap/fish.sh");
    let lines = significant_lines(&script);

    assert_eq!(
        lines.first().copied(),
        Some(r#"if test "$ZAPLEX_BOOTSTRAPPED" != 1"#),
        "the real fish body must fail closed before any bootstrap side effect"
    );
    assert_eq!(
        lines.last().copied(),
        Some("end"),
        "the fish idempotency guard must enclose the complete body"
    );

    let init = bundled_script("bundled/bootstrap/fish_init_shell.sh");
    let forget = init
        .find("set -e ZAPLEX_DAEMON_BOOTSTRAP_FILE")
        .expect("fish init must consume the one-shot body route");
    let source = init
        .find(r#"source "$_zaplex_daemon_bootstrap_file""#)
        .expect("fish init must source the daemon body");
    assert!(
        forget < source,
        "fish must consume the route before sourcing, so a failing body cannot replay"
    );
}

#[test]
fn bash_and_fish_hex_encoding_does_not_append_a_newline() {
    let bash = bundled_script("bundled/bootstrap/bash_body.sh");
    assert!(
        bash.contains("printf '%s' \"$1\" | command -p od -An -v -tx1 | command -p tr -d ' \\n'")
    );

    let fish = bundled_script("bundled/bootstrap/fish.sh");
    assert!(fish.contains("printf '%s' \"$argv\" | od -An -v -tx1 | command tr -d ' \\n'"));
}

#[test]
fn fish_preexec_kills_only_recorded_generator_pids_for_user_commands() {
    let fish = bundled_script("bundled/bootstrap/fish.sh");

    assert!(fish.contains(
        "if not string match -q \"warp_run_generator_command*\" -- (string trim -- $argv[1])"
    ));
    assert!(fish.contains("kill -9 $pid >/dev/null 2>/dev/null"));
    assert!(!fish.contains("kill -9 $pids >/dev/null 2>/dev/null"));
}

#[test]
fn pwsh_bootstrap_body_is_delivered_idempotently() {
    let script = bundled_script("bundled/bootstrap/pwsh.ps1");
    let lines = significant_lines(&script);
    let param_index = lines
        .iter()
        .position(|line| *line == "param()")
        .expect("the real PowerShell body must keep param() first");

    assert_eq!(
        lines.get(param_index + 1).copied(),
        Some("if ($global:ZAPLEX_BOOTSTRAPPED -ne 1) {"),
        "the real PowerShell body must fail closed before any bootstrap side effect"
    );
    assert_eq!(
        lines.last().copied(),
        Some("}"),
        "the PowerShell idempotency guard must enclose the complete body"
    );

    let init = bundled_script("bundled/bootstrap/pwsh_init_shell.ps1");
    let forget = init
        .find("Remove-Item -Path env:ZAPLEX_DAEMON_BOOTSTRAP_FILE")
        .expect("PowerShell init must consume the one-shot body route");
    let source = init
        .find(". $daemonBootstrapFile")
        .expect("PowerShell init must source the daemon body");
    assert!(
        forget < source,
        "PowerShell must consume the route before sourcing, so a failing body cannot replay"
    );
}

fn bundled_script(path: &str) -> String {
    String::from_utf8(
        crate::ASSETS
            .get(path)
            .expect("bundled bootstrap asset")
            .into_owned(),
    )
    .expect("bootstrap assets are UTF-8")
}

fn significant_lines(script: &str) -> Vec<&str> {
    script
        .trim_start_matches(BYTE_ORDER_MARK)
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("[Diagnostics.CodeAnalysis.SuppressMessageAttribute")
        })
        .collect()
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
