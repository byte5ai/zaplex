use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct ShellSshFixture {
    pub users: HashMap<String, String>,
    pub hostname: String,
    port: u16,
    known_hosts: String,
}

/// The selected SSH builder requires an explicitly provisioned, isolated fixture.
pub fn ssh_fixture() -> ShellSshFixture {
    let path = std::env::var("ZAPLEX_SHELL_SSH_FIXTURE")
        .expect("SSH tests require script/run-shell-ssh-fixtures; no external host fallback");
    let fixture: ShellSshFixture = serde_json::from_slice(
        &std::fs::read(path).expect("SSH fixture configuration must be readable"),
    )
    .expect("SSH fixture configuration must be valid");
    assert!(fixture.port > 0, "SSH fixture port must be assigned");
    assert!(
        fixture.users.values().all(|user| {
            !user.is_empty() && user.bytes().all(|byte| byte.is_ascii_alphanumeric())
        }),
        "SSH fixture usernames must be generated alphanumeric account names"
    );
    assert!(std::path::Path::new(&fixture.known_hosts).is_file());
    fixture
}

/// Produces the dedicated fixture user and loopback address for a remote shell.
pub fn user_host(shell: &str) -> String {
    let fixture = ssh_fixture();
    let user = fixture
        .users
        .get(shell)
        .expect("Fixture must provide this shell");
    format!("{user}@127.0.0.1")
}

/// Uses a real ProxyCommand so the startup-shell override regression stays covered.
pub fn ssh_command(shell: &str, should_use_ssh_wrapper: bool) -> String {
    let fixture = ssh_fixture();
    let command = if should_use_ssh_wrapper {
        "ssh"
    } else {
        "command ssh"
    };
    let user_host = user_host(shell);
    let known_hosts = fixture.known_hosts.replace('\'', "'\"'\"'");
    format!(
        "{command} -F /dev/null -p {} -o 'ProxyCommand=/usr/bin/nc %h %p' \
         -o StrictHostKeyChecking=yes -o 'UserKnownHostsFile={known_hosts}' \
         -o PreferredAuthentications=password -o PubkeyAuthentication=no \
         -o NumberOfPasswordPrompts=1 -o ConnectTimeout=10 {user_host}",
        fixture.port,
    )
}
