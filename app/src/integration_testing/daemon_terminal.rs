//! Real Linux SSH/daemon acceptance. Only the CI-owned fixture account is touched.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use command::blocking::Command;
use remote_server::{
    manager::{ConnectedSessionDescriptor, RemoteServerManager},
    proto::SessionInfo,
};
use serde::Deserialize;
use warp_ssh_manager::{
    AuthType, HostKeyPreflight, ResolvedSshConnection, SessionResilience, SshRepository,
    SshServerInfo,
};
use warpui::{
    integration::{AssertionOutcome, TestStep},
    SingletonEntity,
};

use crate::{
    auth::AuthStateProvider,
    integration_testing::view_getters::{terminal_view, workspace_view},
    remote_server::auth_context::server_api_auth_context,
    remote_server::headless_connect,
    workspace::WorkspaceAction,
};

#[derive(Deserialize)]
struct Fixture {
    username: String,
    port: u16,
    key_path: String,
    known_hosts: String,
    fingerprint: String,
    remote_home: String,
    binary: String,
    binary_sha256: String,
}

fn fixture() -> Fixture {
    let path = std::env::var("ZAPLEX_DAEMON_ACCEPTANCE_CONFIG")
        .expect("CI fixture configuration is required");
    serde_json::from_slice(&std::fs::read(path).expect("read fixture configuration"))
        .expect("valid fixture configuration")
}

fn ssh_args(config: &Fixture) -> Vec<String> {
    vec![
        "-i".into(),
        config.key_path.clone(),
        "-p".into(),
        config.port.to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", config.known_hosts),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=5".into(),
        "-o".into(),
        "ServerAliveCountMax=2".into(),
    ]
}

/// Stage the real package binary at the exact Integration-channel install path.
/// The remote process itself retains its normal Oss channel and daemon namespace.
pub fn stage_binary() {
    let config = fixture();
    assert!(config.username.starts_with("zpd"));
    assert!(config.remote_home.starts_with("/tmp/zpd-"));
    assert!(config
        .remote_home
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)));
    let relative = remote_server::setup::remote_server_binary();
    let relative = relative
        .strip_prefix("~/")
        .expect("home-relative remote install path");
    assert!(relative
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)));
    let destination = PathBuf::from(&config.remote_home).join(relative);
    let host = format!("{}@127.0.0.1", config.username);
    let output = Command::new("ssh")
        .args(ssh_args(&config))
        .arg(&host)
        .arg(format!(
            "mkdir -p '{}'",
            destination.parent().expect("binary parent").display()
        ))
        .output()
        .expect("stage directory over SSH");
    assert!(
        output.status.success(),
        "SSH mkdir: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("scp")
        .args([
            "-i",
            &config.key_path,
            "-P",
            &config.port.to_string(),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={}", config.known_hosts),
        ])
        .arg(&config.binary)
        .arg(format!("{host}:{}", destination.display()))
        .output()
        .expect("stage real binary over SFTP");
    assert!(
        output.status.success(),
        "SCP binary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("ssh")
        .args(ssh_args(&config))
        .arg(host)
        .arg(format!(
            "chmod 700 '{}' && sha256sum '{}' && '{}' --version",
            destination.display(),
            destination.display(),
            destination.display()
        ))
        .output()
        .expect("verify staged binary");
    assert!(
        output.status.success(),
        "Remote binary: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next(),
        Some(config.binary_sha256.as_str()),
        "Staged daemon bytes must match the CI binary"
    );
}

#[derive(Default)]
pub struct State {
    connection: Option<ResolvedSshConnection>,
    descriptor: Option<ConnectedSessionDescriptor>,
    session: Option<SessionInfo>,
    failure: Option<String>,
    prepared: bool,
    inventory_done: bool,
}

pub type SharedState = Arc<Mutex<State>>;

pub fn prepare_host_key(state: SharedState) -> TestStep {
    let prepared = Arc::clone(&state);
    TestStep::new("Verify the fixture host key and open the real persistent SSH connection")
        .set_timeout(Duration::from_secs(45))
        .with_action(move |app, window_id, _| {
            let config = fixture();
            let mut server = SshServerInfo::new_default(String::new());
            server.host = "127.0.0.1".into();
            server.port = config.port;
            server.username = config.username;
            server.auth_type = AuthType::Key;
            server.key_path = Some(config.key_path);
            server.session_resilience = SessionResilience::PersistOnly;
            server.node_id = warp_ssh_manager::with_conn(|database| {
                Ok(SshRepository::create_server(database, None, "CI isolated daemon", &server)?.id)
            })
            .expect("save isolated SSH host");
            let connection = warp_ssh_manager::with_conn(|database| {
                Ok(SshRepository::resolve_server_connection(database, &server)?)
            })
            .expect("resolve isolated SSH host");
            state.lock().expect("fixture state").connection = Some(connection);
            let workspace = workspace_view(app, window_id);
            let state = Arc::clone(&state);
            workspace.update(app, |_, ctx| {
                ctx.spawn(
                    async move {
                        match warp_ssh_manager::preflight_host_key(&server).await? {
                            HostKeyPreflight::ConfirmationRequired(key) => {
                                if key.fingerprint != config.fingerprint {
                                    return Err(
                                        "Fixture fingerprint does not match the generated key"
                                            .to_string(),
                                    );
                                }
                                headless_connect::confirm_host_key(&server, &key)?;
                                Ok(())
                            }
                            HostKeyPreflight::Verified => {
                                Err("Expected a fresh isolated known-hosts store".into())
                            }
                            HostKeyPreflight::Changed => Err("Fixture host key changed".into()),
                        }
                    },
                    move |_, result, _| {
                        let mut state = state.lock().expect("fixture state");
                        match result {
                            Ok(()) => state.prepared = true,
                            Err(error) => state.failure = Some(error),
                        }
                    },
                );
            });
        })
        .add_named_assertion(
            "Authenticated fixture connection was requested",
            move |_, _| {
                let state = prepared.lock().expect("fixture state");
                if let Some(error) = &state.failure {
                    return AssertionOutcome::immediate_failure(error.clone());
                }
                if !state.prepared {
                    return AssertionOutcome::failure("Host-key preflight is pending".into());
                }
                AssertionOutcome::Success
            },
        )
}

pub fn ready(state: SharedState) -> TestStep {
    TestStep::new("Wait for the real daemon protocol and remote editor readiness")
        .set_timeout(Duration::from_secs(60))
        .add_named_assertion(
            "No classic SSH fallback; daemon session-host and editor are ready",
            move |app, window_id| {
                if workspace_view(app, window_id).read(app, |view, _| view.tab_count()) != 2 {
                    return AssertionOutcome::failure("Waiting for the remote tab".into());
                }
                terminal_view(app, window_id, 1, 0).read(app, |view, ctx| {
                    let Some(session_id) = view.active_block_session_id() else {
                        return AssertionOutcome::failure("No active remote session".into());
                    };
                    let Some(descriptor) =
                        RemoteServerManager::as_ref(ctx).connected_session_descriptor(session_id)
                    else {
                        return AssertionOutcome::failure(
                            "No exact connected daemon descriptor; classic SSH is not acceptance"
                                .into(),
                        );
                    };
                    let mut state = state.lock().expect("fixture state");
                    let node_id = &state
                        .connection
                        .as_ref()
                        .expect("saved connection")
                        .server
                        .node_id;
                    if descriptor.registry_node_id.as_ref() != Some(node_id)
                        || !descriptor.is_current_runtime
                        || !descriptor.features.iter().any(|feature| {
                            feature == zaplex_remote_session::types::FEATURE_SESSION_HOST
                        })
                        || !view.remote_input_is_ready()
                    {
                        return AssertionOutcome::failure(
                            "Daemon route or input readiness is not established".into(),
                        );
                    }
                    if let Some(prior) = &state.descriptor {
                        if prior.host_id != descriptor.host_id
                            || prior.daemon_runtime != descriptor.daemon_runtime
                        {
                            return AssertionOutcome::immediate_failure(
                                "Reattach changed daemon host/runtime".into(),
                            );
                        }
                    }
                    state.descriptor = Some(descriptor);
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn inventory(state: SharedState, reattached: bool) -> TestStep {
    let result = Arc::clone(&state);
    TestStep::new("Verify the daemon owns exactly the same live PTY generation")
        .set_timeout(Duration::from_secs(30))
        .with_action(move |app, window_id, _| {
            let client = {
                let mut state = state.lock().expect("fixture state");
                state.inventory_done = false;
                Arc::clone(&state.descriptor.as_ref().expect("connected daemon").client)
            };
            let state = Arc::clone(&state);
            workspace_view(app, window_id).update(app, |_, ctx| {
                ctx.spawn(
                    async move {
                        client
                            .list_sessions_with_timeout(Duration::from_secs(20))
                            .await
                    },
                    move |_, response, _| {
                        let mut state = state.lock().expect("fixture state");
                        match response {
                            Ok(list)
                                if list.sessions.len() == 1
                                    && list.sessions[0].alive
                                    && list.sessions[0].generation != 0 =>
                            {
                                let current = &list.sessions[0];
                                if let Some(prior) = &state.session {
                                    if prior.session_id != current.session_id
                                        || prior.generation != current.generation
                                    {
                                        state.failure =
                                            Some("The original PTY generation was replaced".into());
                                        return;
                                    }
                                } else if reattached {
                                    state.failure = Some("Missing pre-detach PTY identity".into());
                                    return;
                                }
                                state.session = Some(current.clone());
                                state.inventory_done = true;
                            }
                            Ok(list) => {
                                state.failure = Some(format!(
                                    "Expected one live daemon PTY, received {:?}",
                                    list.sessions
                                ))
                            }
                            Err(error) => {
                                state.failure =
                                    Some(format!("Real daemon inventory failed: {error}"))
                            }
                        }
                    },
                );
            });
        })
        .add_named_assertion(
            "Live daemon inventory preserves PTY identity",
            move |_, _| {
                let state = result.lock().expect("fixture state");
                if let Some(error) = &state.failure {
                    return AssertionOutcome::immediate_failure(error.clone());
                }
                if !state.inventory_done {
                    return AssertionOutcome::failure("Inventory response pending".into());
                }
                AssertionOutcome::Success
            },
        )
}

pub fn detach(state: SharedState) -> TestStep {
    TestStep::new("Close the remote tab through the production detach path")
        .with_action(|app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::CloseActiveTab,
            );
        })
        .add_named_assertion(
            "Local tab remains; old transport is deregistered",
            move |app, window_id| {
                if workspace_view(app, window_id).read(app, |view, _| view.tab_count()) != 1 {
                    return AssertionOutcome::failure("Remote tab did not detach".into());
                }
                let session_id = state
                    .lock()
                    .expect("fixture state")
                    .descriptor
                    .as_ref()
                    .expect("original connection")
                    .session_id;
                if app.read(|ctx| {
                    RemoteServerManager::as_ref(ctx)
                        .session(session_id)
                        .is_some()
                }) {
                    return AssertionOutcome::failure(
                        "Old transport remains registered; undo hiding is not detach".into(),
                    );
                }
                AssertionOutcome::Success
            },
        )
}

pub fn reattach(state: SharedState) -> TestStep {
    TestStep::new("Adopt the existing daemon PTY using its exact host/runtime/generation")
        .with_action(move |app, window_id, _| {
            let (connection, session_id, generation, host_id, runtime) = {
                let state = state.lock().expect("fixture state");
                let session = state.session.as_ref().expect("original PTY");
                let descriptor = state.descriptor.as_ref().expect("original daemon");
                (
                    state.connection.clone().expect("saved connection"),
                    session.session_id.clone(),
                    session.generation,
                    descriptor.host_id.to_string(),
                    descriptor.daemon_runtime.clone(),
                )
            };
            workspace_view(app, window_id).update(app, |workspace, ctx| {
                workspace.adopt_daemon_session(
                    connection,
                    session_id,
                    generation,
                    None,
                    Some(host_id),
                    Some(runtime),
                    None,
                    ctx,
                );
            });
        })
}

pub fn open(state: SharedState) -> TestStep {
    TestStep::new("Open the saved host through the production workspace action").with_action(
        move |app, window_id, _| {
            let node_id = state
                .lock()
                .expect("fixture state")
                .connection
                .as_ref()
                .expect("saved host")
                .server
                .node_id
                .clone();
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::OpenSshTerminalByNode { node_id },
            );
        },
    )
}

pub fn write_identity_evidence(state: SharedState) -> TestStep {
    TestStep::new("Record the accepted real daemon and PTY identity").with_action(move |_, _, _| {
        let state = state.lock().expect("fixture state");
        let daemon = state.descriptor.as_ref().expect("connected daemon");
        let session = state.session.as_ref().expect("live PTY");
        let evidence = serde_json::json!({
            "scope": "Linux isolated OpenSSH fixture; real daemon/PTY, not macOS or a user host",
            "host_id": daemon.host_id.to_string(), "registry_node_id": daemon.registry_node_id,
            "daemon_runtime": {
                "filename": daemon.daemon_runtime.runtime_filename(),
                "server_version": daemon.daemon_runtime.server_version()
            },
            "pty_id": session.session_id, "pty_generation": session.generation,
            "cwd": session.cwd, "features": daemon.features,
            "tab_completion": "cd pro -> cd projects/ before history seed",
            "ghost_text": "jects/ from executed remote command history",
            "reattach": "same live PTY generation and preserved shell variable after CloseActiveTab"
        });
        let path = PathBuf::from(
            std::env::var("ZAPLEX_DAEMON_ACCEPTANCE_OUTPUT").expect("CI evidence directory"),
        )
        .join("observations.json");
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&evidence).expect("serialize observations"),
        )
        .expect("write observations");
    })
}

pub fn terminal_input() -> TestStep {
    TestStep::new("Select terminal input mode for the real shell completion keys").with_action(
        |app, window_id, _| {
            terminal_view(app, window_id, 1, 0).update(app, |view, ctx| {
                view.input()
                    .update(ctx, |input, ctx| input.set_input_mode_terminal(false, ctx));
            });
        },
    )
}

/// Closing the last remote pane destroys its SSH/proxy transport. Inventory
/// must reconnect through the same production path used by the host panel.
pub fn detached_inventory(state: SharedState) -> TestStep {
    let result = Arc::clone(&state);
    TestStep::new("Open fresh inventory after the old transport closes")
        .set_timeout(Duration::from_secs(40))
        .with_action(move |app, window_id, _| {
            let server = {
                let mut state = state.lock().expect("fixture state");
                state.inventory_done = false;
                state
                    .connection
                    .as_ref()
                    .expect("saved connection")
                    .server
                    .clone()
            };
            let state = Arc::clone(&state);
            workspace_view(app, window_id).update(app, |_, ctx| {
                let auth = Arc::new(server_api_auth_context(
                    AuthStateProvider::as_ref(ctx).get().clone(),
                ));
                let socket = headless_connect::control_socket_path(&server);
                let executor = ctx.background_executor().clone();
                ctx.spawn(
                    headless_connect::list_daemon_sessions(server, socket, auth, executor),
                    move |_, response, _| {
                        let mut state = state.lock().expect("fixture state");
                        match response {
                            Ok(inventory) if inventory.sessions.len() == 1 => {
                                let route = &inventory.sessions[0];
                                let prior = state.session.as_ref().expect("pre-detach PTY");
                                let daemon = state.descriptor.as_ref().expect("pre-detach daemon");
                                if !route.session.alive
                                    || route.session.session_id != prior.session_id
                                    || route.session.generation != prior.generation
                                    || route.route.is_some()
                                    || route.host_id.as_deref() != Some(daemon.host_id.as_str())
                                    || route.daemon_runtime.as_ref() != Some(&daemon.daemon_runtime)
                                {
                                    state.failure = Some(
                                        "Fresh inventory changed host/runtime/PTY identity".into(),
                                    );
                                } else {
                                    state.inventory_done = true;
                                }
                            }
                            Ok(inventory) => {
                                state.failure = Some(format!(
                                    "Expected one surviving PTY after detach, got {}",
                                    inventory.sessions.len()
                                ))
                            }
                            Err(error) => {
                                state.failure =
                                    Some(format!("Fresh detached inventory failed: {error}"))
                            }
                        }
                    },
                );
            });
        })
        .add_named_assertion(
            "A new inventory transport sees the original detached PTY",
            move |_, _| {
                let state = result.lock().expect("fixture state");
                if let Some(error) = &state.failure {
                    return AssertionOutcome::immediate_failure(error.clone());
                }
                if !state.inventory_done {
                    return AssertionOutcome::failure("Fresh inventory pending".into());
                }
                AssertionOutcome::Success
            },
        )
}

pub fn projects_path() -> String {
    format!("{}/acceptance/projects", fixture().remote_home)
}
