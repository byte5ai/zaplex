use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use warpui::{
    elements::Empty, platform::WindowStyle, App, AppContext, Element, Entity, ModelHandle,
    TypedActionView, View, ViewContext,
};

use super::{
    get_local_hostname, BootstrapSessionType, NoOpCommandExecutor, SessionId, SessionInfo,
    SessionType, Sessions, SessionsEvent,
};
use crate::remote_server::manager::RemoteServerManager;
use crate::terminal::model::ansi::{BootstrappedValue, InitShellValue, SSHValue};
use crate::terminal::model::terminal_model::SubshellInitializationInfo;
use crate::terminal::History;

struct TestView {
    events: Vec<SessionsEvent>,
}

impl Entity for TestView {
    type Event = usize;
}

impl View for TestView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

impl TestView {
    fn new(model: ModelHandle<Sessions>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&model, |me, _, event, _| {
            me.events.push(event.to_owned());
        });
        Self { events: Vec::new() }
    }
}

#[test]
fn test_set_env_var_emits_event() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
            let expected_session_id = session_id;
            let event = view.events.first().expect("checked length already");
            if let SessionsEvent::EnvironmentVariablesUpdated { session_id } = event {
                assert_eq!(*session_id, expected_session_id);
            } else {
                assert!(matches!(
                    event,
                    SessionsEvent::EnvironmentVariablesUpdated { .. }
                ));
            }
        });
    });
}

#[test]
fn test_set_env_var_emits_no_event_when_no_change() {
    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| Sessions::new_for_test());
        let session_id: SessionId = 0.into();
        let (_, view_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(model_handle.clone(), ctx)
        });
        view_handle.read(&app, |view, _ctx| {
            assert!(view.events.is_empty());
        });
        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });

        model_handle.update(&mut app, |sessions, ctx| {
            let new_vars = HashMap::from_iter([("foo".to_string(), "bar".to_string())]);
            sessions.set_env_vars_for_session(session_id, new_vars, ctx)
        });

        view_handle.read(&app, |view, _ctx| {
            assert_eq!(view.events.len(), 1);
        });
    });
}

#[test]
fn remote_history_defaults_expand_home_but_reported_paths_remain_literal() {
    use super::history_file_read_command;
    use crate::terminal::shell::ShellType;

    for shell in [ShellType::Bash, ShellType::Zsh, ShellType::Fish] {
        for path in shell.history_files() {
            assert!(path.starts_with("~/"));
            assert_eq!(
                history_file_read_command(&path, shell, true),
                format!("cat ~/'{}'", path.strip_prefix("~/").unwrap())
            );
            assert_eq!(
                history_file_read_command(&path, shell, false),
                format!("cat '{path}'")
            );
        }
        assert_eq!(
            history_file_read_command("/tmp/history; echo unwanted", shell, false),
            "cat '/tmp/history; echo unwanted'"
        );
    }
}

/// How a shell inside a terminal's PTY came to exist.
#[derive(Clone, Copy, Debug)]
enum ShellOrigin {
    /// The shell the PTY was spawned with.
    Root,
    /// A Zaplexified subshell, e.g. a container shell started from the root shell.
    Subshell,
    /// A nested `ssh` hop through the legacy SSH wrapper.
    LegacySsh,
    /// A nested `ssh` hop Zaplexified through tmux control mode.
    TmuxSsh,
}

/// The `SessionInfo` that `TerminalModel` derives from a shell's `InitShell` and
/// `Bootstrapped` hooks, for a shell reporting `hostname`.
fn bootstrapped_shell_info(
    session_id: SessionId,
    hostname: String,
    origin: ShellOrigin,
) -> SessionInfo {
    let is_subshell = matches!(origin, ShellOrigin::Subshell);
    let is_tmux_ssh = matches!(origin, ShellOrigin::TmuxSsh);
    let init_shell_value = InitShellValue {
        session_id,
        shell: "zsh".into(),
        is_subshell,
        user: "dev".into(),
        hostname,
        ..Default::default()
    };
    let subshell_info = is_subshell.then(|| SubshellInitializationInfo {
        spawning_command: "docker exec -it box zsh".into(),
        was_triggered_by_rc_file_snippet: false,
        env_var_collection_name: None,
        ssh_connection_info: None,
    });
    let legacy_ssh_session = matches!(origin, ShellOrigin::LegacySsh).then(|| SSHValue {
        socket_path: PathBuf::from("/tmp/zaplex-test-ssh-socket"),
        remote_shell: "zsh".into(),
    });
    SessionInfo::create_pending(
        crate::terminal::shell::ShellType::Zsh,
        init_shell_value,
        subshell_info,
        None,
        legacy_ssh_session,
        is_tmux_ssh,
        None,
    )
    .merge_from_bootstrapped_value(
        BootstrappedValue {
            shell: "zsh".into(),
            ..Default::default()
        },
        is_tmux_ssh,
    )
}

fn client_hostname() -> String {
    get_local_hostname().expect("the test machine reports a hostname")
}

/// A daemon host whose name never equals the client's, like `devhost` seen
/// from a client on `macbook.local`.
fn other_hostname() -> String {
    format!("{}-daemon-host", client_hostname())
}

fn daemon_connection() -> SessionId {
    SessionId::from((1u64 << 63) + 42)
}

/// A `Sessions` model as the terminal managers build it: daemon-backed panes
/// know their connection, classic panes do not.
fn add_sessions(app: &mut App, daemon_connection: Option<SessionId>) -> ModelHandle<Sessions> {
    app.add_singleton_model(|_| History::new(vec![]));
    app.add_singleton_model(RemoteServerManager::new);
    app.add_model(|_| {
        let mut sessions =
            Sessions::new_for_test().with_command_executor(Arc::new(NoOpCommandExecutor::new()));
        if let Some(connection) = daemon_connection {
            sessions.set_daemon_connection_session_id(connection);
        }
        sessions
    })
}

/// How the client classifies and routes a session after bootstrapping it.
#[derive(Debug)]
struct Routing {
    daemon_hosted: bool,
    session_type: SessionType,
    remote_server_session_id: SessionId,
}

fn bootstrap(app: &mut App, sessions: &ModelHandle<Sessions>, info: SessionInfo) -> Routing {
    let session_id = info.session_id;
    sessions.update(app, |sessions, ctx| {
        sessions.initialize_bootstrapped_session(info, String::new(), vec![], None, ctx);
        let session = sessions.get(session_id).expect("session was bootstrapped");
        Routing {
            daemon_hosted: session.is_daemon_hosted(),
            session_type: session.session_type(),
            remote_server_session_id: sessions.remote_server_session_id(session_id),
        }
    })
}

#[test]
fn daemon_root_shell_on_another_host_is_routed_through_its_connection() {
    App::test((), |mut app| async move {
        let connection = daemon_connection();
        let sessions = add_sessions(&mut app, Some(connection));
        let shell_session = SessionId::from(1_700_000_000_123u64);
        // The connection and the shell's bootstrap id are separate identity domains.
        assert_ne!(connection, shell_session);

        let info = bootstrapped_shell_info(shell_session, other_hostname(), ShellOrigin::Root);
        // The trigger: on a real remote host the root shell's hostname differs
        // from the client's, so it never bootstraps as `Local`.
        assert_eq!(info.session_type, BootstrapSessionType::ZaplexifiedRemote);
        // This gates the daemon's command executor and its SessionBootstrapped
        // registration on the connection.
        assert!(sessions.read(&app, |sessions, _| sessions.is_daemon_hosted_shell(&info)));

        let routing = bootstrap(&mut app, &sessions, info);
        assert!(routing.daemon_hosted, "{routing:?}");
        assert!(
            matches!(routing.session_type, SessionType::ZaplexifiedRemote { .. }),
            "{routing:?}"
        );
        assert_eq!(routing.remote_server_session_id, connection);
    });
}

#[test]
fn daemon_root_shell_sharing_the_client_hostname_stays_daemon_hosted() {
    App::test((), |mut app| async move {
        let connection = daemon_connection();
        let sessions = add_sessions(&mut app, Some(connection));
        let shell_session = SessionId::from(1_700_000_000_456u64);

        let info = bootstrapped_shell_info(shell_session, client_hostname(), ShellOrigin::Root);
        #[cfg(not(feature = "remote_tty"))]
        assert_eq!(info.session_type, BootstrapSessionType::Local);
        assert!(sessions.read(&app, |sessions, _| sessions.is_daemon_hosted_shell(&info)));

        let routing = bootstrap(&mut app, &sessions, info);
        assert!(routing.daemon_hosted, "{routing:?}");
        assert!(
            matches!(routing.session_type, SessionType::ZaplexifiedRemote { .. }),
            "{routing:?}"
        );
        assert_eq!(routing.remote_server_session_id, connection);
    });
}

#[test]
fn shells_nested_in_a_daemon_pty_keep_their_own_routing() {
    App::test((), |mut app| async move {
        let connection = daemon_connection();
        let sessions = add_sessions(&mut app, Some(connection));
        let nested = [
            ShellOrigin::Subshell,
            ShellOrigin::LegacySsh,
            ShellOrigin::TmuxSsh,
        ];
        let mut next_id = 1_700_000_001_000u64;
        for hostname in [other_hostname(), client_hostname()] {
            for origin in nested {
                next_id += 1;
                let session_id = SessionId::from(next_id);
                let info = bootstrapped_shell_info(session_id, hostname.clone(), origin);
                assert!(
                    !sessions.read(&app, |sessions, _| sessions.is_daemon_hosted_shell(&info)),
                    "{origin:?} on {hostname} classified as the daemon root shell"
                );
                let routing = bootstrap(&mut app, &sessions, info);
                assert!(
                    !routing.daemon_hosted,
                    "{origin:?} on {hostname}: {routing:?}"
                );
                assert_eq!(
                    routing.remote_server_session_id, session_id,
                    "{origin:?} on {hostname} borrowed the daemon connection"
                );
            }
        }
    });
}

#[test]
fn classic_pane_sessions_are_never_daemon_hosted() {
    App::test((), |mut app| async move {
        let sessions = add_sessions(&mut app, None);

        let local_id = SessionId::from(1_700_000_002_001u64);
        let local = bootstrapped_shell_info(local_id, client_hostname(), ShellOrigin::Root);
        assert!(!sessions.read(&app, |sessions, _| sessions.is_daemon_hosted_shell(&local)));
        let routing = bootstrap(&mut app, &sessions, local);
        assert!(!routing.daemon_hosted, "{routing:?}");
        #[cfg(not(feature = "remote_tty"))]
        assert_eq!(routing.session_type, SessionType::Local);
        assert_eq!(routing.remote_server_session_id, local_id);

        let ssh_id = SessionId::from(1_700_000_002_002u64);
        let ssh = bootstrapped_shell_info(ssh_id, other_hostname(), ShellOrigin::LegacySsh);
        assert_eq!(ssh.session_type, BootstrapSessionType::ZaplexifiedRemote);
        assert!(!sessions.read(&app, |sessions, _| sessions.is_daemon_hosted_shell(&ssh)));
        let routing = bootstrap(&mut app, &sessions, ssh);
        assert!(!routing.daemon_hosted, "{routing:?}");
        assert_eq!(
            routing.session_type,
            SessionType::ZaplexifiedRemote { host_id: None }
        );
        assert_eq!(routing.remote_server_session_id, ssh_id);
    });
}
