use crate::{
    ai::agent::conversation::AIConversationId,
    terminal::{
        model::{
            block::{AgentInteractionMetadata, SerializedAIMetadata, SerializedBlock},
            bootstrap::BootstrapStage,
            session::command_executor::testing::TestCommandExecutor,
            session::{Session, SessionId, SessionInfo},
            test_utils::TestBlockBuilder,
        },
        shell::ShellType,
        History,
    },
    test_util::{Stub, VirtualFS},
};
use futures::future::join_all;
use futures::Future;
use itertools::Itertools;
use warp_core::command::ExitCode;
use warpui::{App, ModelHandle};

use std::sync::Arc;

use super::{HistoryEntry, HistoryEvent};

impl History {
    /// Returns a Future that completes when `History` is initialized for all sessions with IDs in
    /// the given `session_ids` vector.
    pub async fn initialized_sessions(
        history_handle: &mut ModelHandle<History>,
        app: &mut App,
        session_ids: Vec<SessionId>,
    ) {
        let mut history_initialization_receivers = vec![];
        for session_id in session_ids {
            let is_session_initialized = history_handle.read(app, |history, _| {
                history.is_session_initialized(&session_id)
            });
            if !is_session_initialized {
                let (tx, rx) = async_channel::unbounded();
                let history_handle_clone = history_handle.clone();
                history_handle.update(app, move |_, ctx| {
                    ctx.subscribe_to_model(&history_handle_clone, move |_, event, _| {
                        let HistoryEvent::Initialized(event_id) = event;
                        if session_id == *event_id {
                            let _ = tx.try_send(());
                        }
                    });
                });
                history_initialization_receivers.push(rx);
            }
        }

        join_all(
            history_initialization_receivers
                .into_iter()
                .map(|rx| async move { rx.recv().await }),
        )
        .await;
    }
}

impl HistoryEntry {
    fn with_session_id<S: Into<String>>(session_id: SessionId, command: S) -> Self {
        Self {
            session_id: Some(session_id),
            exit_code: Some(ExitCode::from(0)),
            command: command.into(),
            workflow_id: None,
            workflow_command: None,
            pwd: None,
            start_ts: None,
            completed_ts: None,
            git_head: None,
            shell_host: None,
            is_agent_executed: false,
            is_for_restored_block: false,
        }
    }

    pub fn with_pwd_and_exit_code<S: Into<String>>(
        command: S,
        pwd: S,
        exit_code: impl Into<ExitCode>,
    ) -> Self {
        Self {
            session_id: None,
            exit_code: Some(exit_code.into()),
            command: command.into(),
            workflow_id: None,
            workflow_command: None,
            pwd: Some(pwd.into()),
            start_ts: None,
            completed_ts: None,
            git_head: None,
            shell_host: None,
            is_agent_executed: false,
            is_for_restored_block: false,
        }
    }
}

#[test]
fn history_entry_for_restored_block_preserves_agent_execution() {
    let mut block = TestBlockBuilder::new()
        .with_bootstrap_stage(BootstrapStage::RestoreBlocks)
        .build();
    block.set_agent_interaction_mode_for_requested_command(
        String::from("action-id").into(),
        None,
        AIConversationId::new(),
    );

    let entry = HistoryEntry::for_restored_block("ls".to_string(), &block);

    assert!(entry.is_agent_executed);
}

#[test]
fn history_entry_for_completed_block_preserves_agent_execution() {
    let ai_metadata = serde_json::to_string(&SerializedAIMetadata::from(
        AgentInteractionMetadata::new_hidden(
            String::from("action-id").into(),
            AIConversationId::new(),
        ),
    ))
    .unwrap();
    let block = SerializedBlock {
        ai_metadata: Some(ai_metadata),
        ..SerializedBlock::new_for_test("ls".as_bytes().to_vec(), vec![])
    };

    let entry = HistoryEntry::for_completed_block("ls".to_string(), &block);

    assert!(entry.is_agent_executed);
}

#[test]
fn history_entry_for_completed_block_does_not_treat_all_agent_interactions_as_agent_execution() {
    let ai_metadata = serde_json::to_string(&SerializedAIMetadata::from(
        AgentInteractionMetadata::new(None, AIConversationId::new(), None, None, false, false),
    ))
    .unwrap();
    let block = SerializedBlock {
        ai_metadata: Some(ai_metadata),
        ..SerializedBlock::new_for_test("ls".as_bytes().to_vec(), vec![])
    };

    let entry = HistoryEntry::for_completed_block("ls".to_string(), &block);

    assert!(!entry.is_agent_executed);
}

/// Initializes history for testing
/// initialization is complete.
///
/// `history_commands_fn` is an async function that should emulate the async operation of reading
/// command from a session's history file.
///
/// `session_commands_to_append` are commands that are appended to the in-memory session history
/// after commands have been read from the history file.
async fn initialize_history_for_testing<F>(
    history_handle: &mut ModelHandle<History>,
    session: Arc<Session>,
    history_commands_fn: F,
    session_commands_to_append: Vec<String>,
    app: &mut App,
) where
    F: 'static + Future<Output = Vec<String>> + Send,
{
    let session_id = session.id();
    history_handle.update(app, move |history, ctx| {
        history.init_session_with(session, history_commands_fn, ctx);
    });
    History::initialized_sessions(history_handle, app, vec![session_id]).await;
    history_handle.update(app, move |history, _| {
        history.append_commands(
            session_id,
            session_commands_to_append
                .into_iter()
                .map(|command| HistoryEntry::with_session_id(session_id, command))
                .collect_vec(),
        );
    });
}

#[test]
fn test_append_commands() {
    VirtualFS::test("history_append_command", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    ls
                    pwd
                    warp --listen --ports=8080,8081
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec![
                    "pwd".to_owned(),
                    "ls".to_owned(),
                    "pwd".to_owned(),
                    "git status".to_owned(),
                ],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("warp --listen --ports=8080,8081"),
                        &HistoryEntry::with_session_id(session.id(), "ls"),
                        &HistoryEntry::with_session_id(session.id(), "pwd"),
                        &HistoryEntry::with_session_id(session.id(), "git status"),
                    ]
                );
            });
        });
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_append_multiple_sessions() {
    VirtualFS::test("append_multiple_sessions", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    cd warp
                    cargo run --bin dev
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec![
                    "ls target/".to_owned(),
                    "cargo clean".to_owned(),
                    "ls target/".to_owned(),
                ],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("cargo run --bin dev"),
                        &HistoryEntry::with_session_id(session.id(), "cargo clean"),
                        &HistoryEntry::with_session_id(session.id(), "ls target/"),
                    ]
                );
            });

            let second_session = Arc::new(Session::new(
                SessionInfo::new_for_test().with_id(1),
                Arc::new(TestCommandExecutor::default()),
            ));

            initialize_history_for_testing(
                &mut history_handle,
                second_session.clone(),
                async move { Vec::new() },
                vec!["ls target/".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(
                    history.commands(second_session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("cargo run --bin dev"),
                        &HistoryEntry::with_session_id(session.id(), "cargo clean"),
                        &HistoryEntry::with_session_id(second_session.id(), "ls target/"),
                    ]
                );
            });
        });
    });
}

#[test]
fn test_len() {
    VirtualFS::test("history_len", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![Stub::FileWithContentToBeTrimmed(
                ".bash_history",
                r#"
                    cd warp
                    cargo run --bin dev
                    touch
                "#,
            )]);

            let mut history_handle = app.add_model(|_| History::default());
            let file = Some(dirs.tests().join(".bash_history").display().to_string());
            let session = Arc::new(Session::new(
                SessionInfo::new_for_test()
                    .with_histfile(file)
                    .with_shell_type(ShellType::Bash),
                Arc::new(TestCommandExecutor::default()),
            ));

            let session_clone = session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                session.clone(),
                async move { session_clone.read_history(false).await },
                vec!["ls".to_owned(), "echo 'hello'".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(session.id()), 5);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    session.id(),
                    vec![HistoryEntry::with_session_id(
                        session.id(),
                        "cargo run --bin dev".to_string(),
                    )],
                );
            });

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(session.id()), 5);

                assert_eq!(
                    history.commands(session.id()).unwrap_or_default(),
                    vec![
                        &HistoryEntry::command_only("cd warp"),
                        &HistoryEntry::command_only("touch"),
                        &HistoryEntry::with_session_id(session.id(), "ls"),
                        &HistoryEntry::with_session_id(session.id(), "echo 'hello'"),
                        &HistoryEntry::with_session_id(session.id(), "cargo run --bin dev")
                    ]
                );
            });
        });
    });
}

#[test]
fn test_multiple_shells() {
    VirtualFS::test("multiple_shells", |dirs, mut sandbox| {
        App::test((), |mut app| async move {
            sandbox.with_files(vec![
                Stub::FileWithContentToBeTrimmed(
                    ".bash_history",
                    r#"
                    bash-cmd1
                    bash-cmd2
                "#,
                ),
                Stub::FileWithContentToBeTrimmed(
                    ".zsh_history",
                    r#"
                    zsh-cmd1
                    zsh-cmd2
                "#,
                ),
            ]);

            let mut history_handle = app.add_model(|_| History::default());

            let root = dirs.tests();
            let (bash_file, zsh_file) = (
                Some(root.join(".bash_history").display().to_string()),
                Some(root.join(".zsh_history").display().to_string()),
            );

            let (bash_session, zsh_session) = (
                Arc::new(Session::new(
                    SessionInfo::new_for_test()
                        .with_id(0)
                        .with_histfile(bash_file)
                        .with_shell_type(ShellType::Bash),
                    Arc::new(TestCommandExecutor::default()),
                )),
                Arc::new(Session::new(
                    SessionInfo::new_for_test()
                        .with_id(1)
                        .with_histfile(zsh_file)
                        .with_shell_type(ShellType::Zsh),
                    Arc::new(TestCommandExecutor::default()),
                )),
            );

            let bash_session_clone = bash_session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                bash_session.clone(),
                async move { bash_session_clone.read_history(false).await },
                vec!["bash-cmd3".to_owned()],
                &mut app,
            )
            .await;

            let zsh_session_clone = zsh_session.clone();
            initialize_history_for_testing(
                &mut history_handle,
                zsh_session.clone(),
                async move { zsh_session_clone.read_history(false).await },
                vec!["zsh-cmd3".to_owned()],
                &mut app,
            )
            .await;

            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 3);
                assert_eq!(history.len(zsh_session.id()), 3);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    bash_session.id(),
                    vec![HistoryEntry::with_session_id(
                        bash_session.id(),
                        "bash-cmd4".to_string(),
                    )],
                );
            });
            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 4);
                assert_eq!(history.len(zsh_session.id()), 3);
            });

            history_handle.update(&mut app, |history, _ctx| {
                history.append_commands(
                    zsh_session.id(),
                    vec![HistoryEntry::with_session_id(
                        zsh_session.id(),
                        "zsh-cmd4".to_string(),
                    )],
                );
            });
            history_handle.read(&app, |history, _ctx| {
                assert_eq!(history.len(bash_session.id()), 4);
                assert_eq!(history.len(zsh_session.id()), 4);
            });
        });
    });
}
