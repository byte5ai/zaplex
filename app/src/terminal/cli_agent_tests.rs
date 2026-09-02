use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Local;
use smol_str::SmolStr;
use warp_editor::render::model::LineCount;
use warp_util::path::EscapeChar;
use warpui::App;

use super::{
    agents_for_installation_scan, build_diff_hunk_prompt, build_review_prompt,
    build_selection_line_range_prompt, build_selection_substring_prompt, CLIAgent, UBER_TEAM_UID,
};
#[cfg(unix)]
use super::{cli_agent_search_dirs, resolve_executable_in_dirs};
use crate::ai::agent::{AgentReviewCommentBatch, DiffSetHunk};
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentOrigin, LineDiffContent,
};
use crate::server::ids::ServerId;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

/// Helper to build an alias map from pairs.
fn aliases(pairs: &[(&str, &str)]) -> HashMap<SmolStr, String> {
    pairs
        .iter()
        .map(|(k, v)| (SmolStr::new(k), v.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers for prompt-building tests
// ---------------------------------------------------------------------------

fn make_comment(
    content: &str,
    target: AttachedReviewCommentTarget,
    outdated: bool,
) -> AttachedReviewComment {
    AttachedReviewComment {
        id: Default::default(),
        content: content.to_string(),
        target,
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated,
        origin: CommentOrigin::Native,
    }
}

fn batch(comments: Vec<AttachedReviewComment>) -> AgentReviewCommentBatch {
    AgentReviewCommentBatch {
        comments,
        diff_set: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// build_review_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_review_prompt_current_line_is_1_indexed() {
    // LineCount 0 (0-indexed) should appear as L1 in the prompt.
    let comment = make_comment(
        "fix this",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: PathBuf::from("/repo/src/main.rs"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(0),
                line_range: LineCount::from(0)..LineCount::from(1),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(
        prompt.contains("/repo/src/main.rs L1"),
        "expected 1-indexed L1, got: {prompt}",
    );
    assert!(prompt.contains("fix this"));
}

#[test]
fn test_build_review_prompt_removed_line_is_1_indexed() {
    let comment = make_comment(
        "why was this deleted?",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: PathBuf::from("/repo/old.rs"),
            line: EditorLineLocation::Removed {
                line_number: LineCount::from(9),
                line_range: LineCount::from(9)..LineCount::from(10),
                index: 0,
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(
        prompt.contains("(deleted, was L10"),
        "expected 1-indexed L10, got: {prompt}",
    );
}

#[test]
fn test_build_review_prompt_collapsed_range_is_1_indexed_start() {
    let comment = make_comment(
        "check this hunk",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: PathBuf::from("/repo/lib.rs"),
            line: EditorLineLocation::Collapsed {
                line_range: LineCount::from(4)..LineCount::from(10),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    // line_range is [4, 10) 0-indexed -> L5-L10 (1-indexed, both ends inclusive)
    assert!(prompt.contains("L5-L10"), "expected L5-L10, got: {prompt}",);
}

#[test]
fn test_build_review_prompt_file_level_comment() {
    let comment = make_comment(
        "needs refactoring",
        AttachedReviewCommentTarget::File {
            absolute_file_path: PathBuf::from("/repo/src/utils.rs"),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("/repo/src/utils.rs: needs refactoring"));
    // Not a deleted file (empty diff_set), so no "deleted file" text.
    assert!(!prompt.contains("deleted file"));
}

#[test]
fn test_build_review_prompt_deleted_file_comment() {
    let comment = make_comment(
        "why remove this?",
        AttachedReviewCommentTarget::File {
            absolute_file_path: PathBuf::from("/repo/src/old.rs"),
        },
        false,
    );
    let mut review = batch(vec![comment]);
    review.diff_set.insert(
        "src/old.rs".to_string(),
        vec![DiffSetHunk {
            line_range: LineCount::from(0)..LineCount::from(5),
            diff_content: String::new(),
            lines_added: 0,
            lines_removed: 5,
        }],
    );
    let prompt = build_review_prompt(&review);
    assert!(
        prompt.contains("(deleted file"),
        "expected deleted file annotation, got: {prompt}",
    );
}

#[test]
fn test_build_review_prompt_general_comment() {
    let comment = make_comment(
        "overall looks good",
        AttachedReviewCommentTarget::General,
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("General: overall looks good"));
}

#[test]
fn test_build_review_prompt_skips_outdated_comments() {
    let active = make_comment("keep me", AttachedReviewCommentTarget::General, false);
    let outdated = make_comment("skip me", AttachedReviewCommentTarget::General, true);
    let prompt = build_review_prompt(&batch(vec![active, outdated]));
    assert!(prompt.contains("keep me"));
    assert!(!prompt.contains("skip me"));
}

#[test]
fn test_build_review_prompt_multiple_comments() {
    let c1 = make_comment(
        "first",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: PathBuf::from("/repo/a.rs"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(4),
                line_range: LineCount::from(4)..LineCount::from(5),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let c2 = make_comment("second", AttachedReviewCommentTarget::General, false);
    let prompt = build_review_prompt(&batch(vec![c1, c2]));
    assert!(prompt.contains("/repo/a.rs L5: first"));
    assert!(prompt.contains("General: second"));
}

#[test]
fn test_build_review_prompt_exports_internal_markdown_without_punctuation_escapes() {
    let comment = make_comment("Fix this\\.", AttachedReviewCommentTarget::General, false);
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("General: Fix this."));
    assert!(!prompt.contains("Fix this\\."));
}

// ---------------------------------------------------------------------------
// build_diff_hunk_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_diff_hunk_prompt_format() {
    let prompt = build_diff_hunk_prompt(Path::new("/repo/src/main.rs"), 10, 20, 3, 2);
    assert_eq!(
        prompt,
        "/repo/src/main.rs L10-L20 (+3 -2) -- run `git diff` to see the full context.",
    );
}

// ---------------------------------------------------------------------------
// build_selection_line_range_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_selection_line_range_prompt_format() {
    let result = build_selection_line_range_prompt("src/foo.rs", 5, 10);
    assert_eq!(result, "src/foo.rs L5-L10");
}

#[test]
fn test_build_selection_substring_prompt_format() {
    let result = build_selection_substring_prompt("src/foo.rs", 5, "let x = 42;");
    assert_eq!(result, "src/foo.rs L5: let x = 42;");
}

#[test]
fn test_detect_known_agents() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for (command, expected) in [
                ("claude", CLIAgent::Claude),
                ("gemini", CLIAgent::Gemini),
                ("codex", CLIAgent::Codex),
                ("deepseek", CLIAgent::DeepSeek),
                ("deepseek-tui", CLIAgent::DeepSeek),
                ("codewhale", CLIAgent::DeepSeek),
                ("codew", CLIAgent::DeepSeek),
                ("codewhale-tui", CLIAgent::DeepSeek),
                ("agy", CLIAgent::Antigravity),
                ("grok", CLIAgent::Grok),
                ("amp", CLIAgent::Amp),
                ("droid", CLIAgent::Droid),
                ("opencode", CLIAgent::OpenCode),
                ("copilot", CLIAgent::Copilot),
                ("agent", CLIAgent::CursorCli),
                ("goose", CLIAgent::Goose),
            ] {
                assert_eq!(
                    CLIAgent::detect(command, None, None, ctx),
                    Some(expected),
                    "failed to detect {command}",
                );
            }
        });
    });
}

#[test]
fn test_detect_with_arguments() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("claude --model opus", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("gemini chat", None, None, ctx),
                Some(CLIAgent::Gemini),
            );
        });
    });
}

#[test]
fn test_detect_with_leading_whitespace() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("  claude", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("\tclaude --help", None, None, ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_no_match() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(CLIAgent::detect("ls -la", None, None, ctx), None);
            assert_eq!(CLIAgent::detect("vim", None, None, ctx), None);
            assert_eq!(CLIAgent::detect("claude_wrapper", None, None, ctx), None);
        });
    });
}

#[test]
fn test_detect_with_alias() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("c", "claude")]);
            assert_eq!(
                CLIAgent::detect("c", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("c --help", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_alias_not_matching() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("c", "cat")]);
            assert_eq!(CLIAgent::detect("c", None, Some(&map), ctx), None);
        });
    });
}

#[test]
fn test_detect_alias_multi_word_value() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // Alias whose value starts with "gemini" but has extra words
            let map = aliases(&[("g", "gemini chat --verbose")]);
            assert_eq!(
                CLIAgent::detect("g", None, Some(&map), ctx),
                Some(CLIAgent::Gemini),
            );
        });
    });
}

#[test]
fn test_detect_with_env_var_prefix() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect(
                    "EXAMPLE=true opencode",
                    Some(EscapeChar::Backslash),
                    None,
                    ctx,
                ),
                Some(CLIAgent::OpenCode),
            );
        });
    });
}

#[test]
fn test_detect_with_multiple_env_vars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect(
                    "FOO=1 BAR=2 opencode --flag",
                    Some(EscapeChar::Backslash),
                    None,
                    ctx,
                ),
                Some(CLIAgent::OpenCode),
            );
        });
    });
}

#[test]
fn test_detect_with_alias_and_env_var() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("oc", "EXAMPLE=1 opencode")]);
            assert_eq!(
                CLIAgent::detect("oc --flag", Some(EscapeChar::Backslash), Some(&map), ctx,),
                Some(CLIAgent::OpenCode),
            );
        });
    });
}

/// Creates a workspace containing a team with the given UID.
fn workspace_with_team_uid(uid: &str) -> Workspace {
    Workspace::from_local_cache(
        ServerId::from_string_lossy("test-workspace-uid-001").into(),
        "Test Workspace".to_string(),
        Some(vec![Team::from_local_cache(
            ServerId::from_string_lossy(uid),
            "Test Team".to_string(),
            None,
            None,
            None,
        )]),
    )
}

#[test]
fn test_detect_aifx_agent_run_claude_on_uber_team() {
    App::test((), |mut app| async move {
        let uber_workspace = workspace_with_team_uid(UBER_TEAM_UID);
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![uber_workspace], ctx));

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            // With extra args
            assert_eq!(
                CLIAgent::detect("aifx agent run claude --verbose", None, None, ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_aifx_agent_run_claude_via_alias_on_uber_team() {
    App::test((), |mut app| async move {
        let uber_workspace = workspace_with_team_uid(UBER_TEAM_UID);
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![uber_workspace], ctx));

        app.update(|ctx| {
            let map = aliases(&[("ai", "aifx agent run claude")]);
            assert_eq!(
                CLIAgent::detect("ai", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("ai --flag", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_aifx_agent_run_claude_not_on_uber_team() {
    App::test((), |mut app| async move {
        // Register UserWorkspaces with no Uber team membership
        app.add_singleton_model(UserWorkspaces::default_mock);

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                None,
            );
        });
    });
}

#[test]
fn test_serialized_name_round_trips_known_agents() {
    for agent in enum_iterator::all::<CLIAgent>() {
        let name = agent.to_serialized_name();
        if agent == CLIAgent::Unknown {
            assert_eq!(name, "Unknown");
        } else {
            assert!(!name.is_empty(), "empty serialized name for {agent:?}");
        }
        assert_eq!(
            CLIAgent::from_serialized_name(&name),
            agent,
            "round-trip failed for {agent:?} with serialized name {name:?}",
        );
    }
}

#[test]
fn test_from_serialized_name_falls_back_to_unknown() {
    assert_eq!(CLIAgent::from_serialized_name(""), CLIAgent::Unknown);
    assert_eq!(
        CLIAgent::from_serialized_name("nonexistent"),
        CLIAgent::Unknown
    );
}

#[test]
fn test_detect_aifx_agent_run_claude_wrong_team() {
    App::test((), |mut app| async move {
        let other_workspace = workspace_with_team_uid("some-other-team-uid-01");
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![other_workspace], ctx));

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                None,
            );
        });
    });
}

#[cfg(unix)]
#[test]
fn test_cli_agent_search_dirs_include_common_gui_app_paths() {
    let dirs: Vec<PathBuf> = cli_agent_search_dirs().collect();

    assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
}

#[cfg(unix)]
#[test]
fn test_cli_agent_search_dirs_include_home_managed_bins() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let dirs: Vec<PathBuf> = cli_agent_search_dirs().collect();

    assert!(dirs.contains(&home.join(".cargo/bin")));
    assert!(dirs.contains(&home.join(".bun/bin")));
    assert!(dirs.contains(&home.join(".grok/bin")));
    assert!(dirs.contains(&home.join(".local/bin")));
}

#[cfg(unix)]
#[test]
fn cli_resolution_uses_gui_safe_search_dirs() {
    use std::os::unix::fs::PermissionsExt as _;

    let process_path = tempfile::tempdir().unwrap();
    let gui_safe_path = tempfile::tempdir().unwrap();
    let executable = gui_safe_path.path().join("codex");
    std::fs::write(&executable, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        resolve_executable_in_dirs(
            "codex",
            &[
                process_path.path().to_owned(),
                gui_safe_path.path().to_owned(),
            ],
        ),
        Some(executable),
    );
}

#[cfg(unix)]
#[test]
fn cli_resolution_rejects_non_executable_files() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let non_executable = directory.path().join("codex");
    std::fs::write(&non_executable, "not executable\n").unwrap();
    std::fs::set_permissions(&non_executable, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(
        resolve_executable_in_dirs("codex", &[directory.path().to_owned()]),
        None,
    );
}

#[test]
fn fork_command_per_provider() {
    // Verified against the CLIs on 2026-07-03 (fork/worktree design §2).
    assert_eq!(
        CLIAgent::Claude.fork_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("claude --resume 0198c8f3-aaaa-bbbb-cccc-1234567890ab --fork-session".to_string())
    );
    assert_eq!(
        CLIAgent::Codex.fork_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("codex fork 0198c8f3-aaaa-bbbb-cccc-1234567890ab".to_string())
    );
    assert_eq!(
        CLIAgent::Grok.fork_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("grok --resume 0198c8f3-aaaa-bbbb-cccc-1234567890ab --fork-session".to_string())
    );
    // No known fork mechanism → None, surfaces stay disabled (no fake fork).
    assert_eq!(CLIAgent::Gemini.fork_command("x"), None);
    assert_eq!(CLIAgent::Antigravity.fork_command("x"), None);
    assert_eq!(CLIAgent::Unknown.fork_command("x"), None);
}

#[test]
fn fork_command_quotes_hostile_session_ids() {
    let cmd = CLIAgent::Claude
        .fork_command("evil; rm -rf /")
        .expect("claude forks");
    assert_eq!(cmd, "claude --resume 'evil; rm -rf /' --fork-session");
    let cmd = CLIAgent::Grok
        .fork_command("evil; rm -rf /")
        .expect("grok forks");
    assert_eq!(cmd, "grok --resume 'evil; rm -rf /' --fork-session");
}

#[test]
fn fork_command_pinned_prepends_inline_env_for_non_default_accounts() {
    let dir = PathBuf::from("/home/u/claude-work dir");
    assert_eq!(
        CLIAgent::Claude.fork_command_pinned("abc", Some(&dir)),
        Some(
            "CLAUDE_CONFIG_DIR='/home/u/claude-work dir' claude --resume abc --fork-session"
                .to_string()
        )
    );
    assert_eq!(
        CLIAgent::Codex.fork_command_pinned("abc", Some(Path::new("/home/u/.codex-alt"))),
        Some("CODEX_HOME=/home/u/.codex-alt codex fork abc".to_string())
    );
    assert_eq!(
        CLIAgent::Grok.fork_command_pinned("abc", Some(Path::new("/home/u/grok work"))),
        Some("GROK_HOME='/home/u/grok work' grok --resume abc --fork-session".to_string())
    );
    // Default account (None) → bare fork command, no env prefix.
    assert_eq!(
        CLIAgent::Claude.fork_command_pinned("abc", None),
        Some("claude --resume abc --fork-session".to_string())
    );
}

#[test]
fn resume_command_per_provider_continues_in_place() {
    // Adopt-in-place: same session, no `--fork-session` (verified 2026-07-05).
    assert_eq!(
        CLIAgent::Claude.resume_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("claude --resume 0198c8f3-aaaa-bbbb-cccc-1234567890ab".to_string())
    );
    assert_eq!(
        CLIAgent::Codex.resume_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("codex resume 0198c8f3-aaaa-bbbb-cccc-1234567890ab".to_string())
    );
    assert_eq!(
        CLIAgent::Grok.resume_command("0198c8f3-aaaa-bbbb-cccc-1234567890ab"),
        Some("grok --resume 0198c8f3-aaaa-bbbb-cccc-1234567890ab".to_string())
    );
    // No known resume mechanism → None, surfaces stay disabled.
    assert_eq!(CLIAgent::Gemini.resume_command("x"), None);
    assert_eq!(
        CLIAgent::Antigravity.resume_command("x"),
        Some("agy --conversation x".into())
    );
    assert_eq!(CLIAgent::Unknown.resume_command("x"), None);
}

#[test]
fn resume_command_quotes_hostile_session_ids() {
    let cmd = CLIAgent::Claude
        .resume_command("evil; rm -rf /")
        .expect("claude resumes");
    assert_eq!(cmd, "claude --resume 'evil; rm -rf /'");
    let cmd = CLIAgent::Grok
        .resume_command("evil; rm -rf /")
        .expect("grok resumes");
    assert_eq!(cmd, "grok --resume 'evil; rm -rf /'");
}

#[test]
fn resume_command_pinned_prepends_inline_env_for_non_default_accounts() {
    let dir = PathBuf::from("/home/u/claude-work dir");
    assert_eq!(
        CLIAgent::Claude.resume_command_pinned("abc", Some(&dir)),
        Some("CLAUDE_CONFIG_DIR='/home/u/claude-work dir' claude --resume abc".to_string())
    );
    assert_eq!(
        CLIAgent::Codex.resume_command_pinned("abc", Some(Path::new("/home/u/.codex-alt"))),
        Some("CODEX_HOME=/home/u/.codex-alt codex resume abc".to_string())
    );
    assert_eq!(
        CLIAgent::Grok.resume_command_pinned("abc", Some(Path::new("/home/u/grok work"))),
        Some("GROK_HOME='/home/u/grok work' grok --resume abc".to_string())
    );
    // Default account (None) → bare resume command, no env prefix.
    assert_eq!(
        CLIAgent::Claude.resume_command_pinned("abc", None),
        Some("claude --resume abc".to_string())
    );
}

#[test]
fn routed_resume_preserves_account_model_effort_and_scrubs_api_keys() {
    assert_eq!(
        CLIAgent::Claude.resume_command_routed_with(
            "claude-session",
            Some(Path::new("/home/u/claude work")),
            Some("opus"),
            Some("high"),
        ),
        Some(
            "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; \
             CLAUDE_CONFIG_DIR='/home/u/claude work' claude --model opus \
             --resume claude-session"
                .to_string()
        )
    );
    assert_eq!(
        CLIAgent::Codex.resume_command_routed_with(
            "codex-session",
            Some(Path::new("/home/u/.codex-alt")),
            Some("gpt-5.6-sol"),
            Some("high"),
        ),
        Some(
            "unset OPENAI_API_KEY; CODEX_HOME=/home/u/.codex-alt codex \
             --model gpt-5.6-sol -c 'model_reasoning_effort=\"high\"' \
             resume codex-session"
                .to_string()
        )
    );
}

#[test]
fn routed_resume_quotes_session_id_and_rejects_unsupported_providers() {
    assert_eq!(
        CLIAgent::Claude.resume_command_routed_with(
            "id with spaces;$(touch /tmp/nope)",
            None,
            None,
            None,
        ),
        Some(
            "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; \
             claude --resume 'id with spaces;$(touch /tmp/nope)'"
                .to_string()
        )
    );
    assert_eq!(
        CLIAgent::Claude.resume_command_routed_with("   ", None, None, None),
        None
    );
    assert_eq!(
        CLIAgent::Unknown.resume_command_routed_with("session", None, None, None),
        None
    );
}

#[test]
fn launch_command_routed_scrubs_and_pins_claude() {
    // Default account: scrub the API key env, no config-dir pin, bare `claude`.
    assert_eq!(
        CLIAgent::Claude.launch_command_routed(None),
        "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; claude"
    );
    // Pinned account: scrub + CLAUDE_CONFIG_DIR before the command.
    assert_eq!(
        CLIAgent::Claude.launch_command_routed(Some(Path::new("/home/u/.claude-work"))),
        "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; CLAUDE_CONFIG_DIR=/home/u/.claude-work claude"
    );
}

#[test]
fn launch_command_routed_handles_codex_and_bare_agents() {
    assert_eq!(
        CLIAgent::Codex.launch_command_routed(Some(Path::new("/home/u/.codex"))),
        "unset OPENAI_API_KEY; CODEX_HOME=/home/u/.codex codex"
    );
    // An agent with no subscription/config-dir model launches bare (no scrub/pin).
    assert_eq!(CLIAgent::Gemini.launch_command_routed(None), "gemini");
    assert_eq!(
        CLIAgent::Gemini.launch_command_routed(Some(Path::new("/x"))),
        "gemini"
    );
    assert_eq!(CLIAgent::Antigravity.launch_command_routed(None), "agy");
    assert_eq!(
        CLIAgent::Grok.launch_command_routed(Some(Path::new("/home/u/.grok"))),
        "GROK_HOME=/home/u/.grok grok"
    );
}

#[test]
fn launch_command_routed_with_model_claude() {
    // Claude gets `--model <model>`; effort is NOT a Claude CLI flag, so it must
    // never appear on the command line even when supplied.
    assert_eq!(
        CLIAgent::Claude.launch_command_routed_with(None, Some("opus"), None),
        "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; claude --model opus"
    );
    assert_eq!(
        CLIAgent::Claude.launch_command_routed_with(
            Some(Path::new("/home/u/.claude-work")),
            Some("haiku"),
            Some("low"),
        ),
        "unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN; \
         CLAUDE_CONFIG_DIR=/home/u/.claude-work claude --model haiku"
    );
}

#[test]
fn launch_command_routed_with_model_and_effort_codex() {
    // Codex gets `--model <model>` plus the reasoning-effort config override.
    // `-c key=value` is parsed as TOML by Codex, so the value must itself be a
    // TOML-quoted string — a bare `high` is not valid TOML.
    let cmd = CLIAgent::Codex.launch_command_routed_with(
        Some(Path::new("/home/u/.codex")),
        Some("gpt-5-codex"),
        Some("high"),
    );
    assert_eq!(
        cmd,
        "unset OPENAI_API_KEY; CODEX_HOME=/home/u/.codex codex \
         --model gpt-5-codex -c 'model_reasoning_effort=\"high\"'"
    );
    // The effort value is TOML-double-quoted inside the shell-quoted token.
    assert!(cmd.contains(r#"model_reasoning_effort="high""#));
    // Effort alone (no model) still emits the effort override. The `key=value`
    // token is shell-quoted (the `=` triggers quoting) — harmless and safe.
    assert_eq!(
        CLIAgent::Codex.launch_command_routed_with(None, None, Some("medium")),
        "unset OPENAI_API_KEY; codex -c 'model_reasoning_effort=\"medium\"'"
    );
}

#[test]
fn selected_codex_effort_reaches_cli_arguments() {
    let launch = CLIAgent::Codex.routed_launch(None, None, Some("high"));
    assert_eq!(launch.program, "codex");
    assert_eq!(
        launch.args,
        vec![
            "-c".to_string(),
            "model_reasoning_effort=\"high\"".to_string(),
        ]
    );
    assert_eq!(
        CLIAgent::Codex.launch_command_routed_with(None, None, Some("high")),
        "unset OPENAI_API_KEY; codex -c 'model_reasoning_effort=\"high\"'"
    );
}

#[test]
fn launch_command_routed_with_model_and_effort_grok() {
    let launch = CLIAgent::Grok.routed_launch(
        Some(Path::new("/home/u/grok work")),
        Some("grok-4"),
        Some("high"),
    );
    assert_eq!(launch.program, "grok");
    assert_eq!(
        launch.args,
        vec![
            "--model".to_string(),
            "grok-4".to_string(),
            "--effort".to_string(),
            "high".to_string(),
        ]
    );
    assert_eq!(
        launch.environment,
        vec![("GROK_HOME", "/home/u/grok work".to_string())]
    );
    assert!(launch.unset_environment.is_empty());
    assert_eq!(
        CLIAgent::Grok.launch_command_routed_with(
            Some(Path::new("/home/u/grok work")),
            Some("grok-4"),
            Some("high"),
        ),
        "GROK_HOME='/home/u/grok work' grok --model grok-4 --effort high"
    );
}

#[test]
fn launch_command_routed_with_none_is_verbatim_today() {
    // None/None must be byte-for-byte identical to the pre-existing bare launch.
    assert_eq!(
        CLIAgent::Claude.launch_command_routed_with(None, None, None),
        CLIAgent::Claude.launch_command_routed(None),
    );
    assert_eq!(
        CLIAgent::Codex.launch_command_routed_with(Some(Path::new("/home/u/.codex")), None, None),
        CLIAgent::Codex.launch_command_routed(Some(Path::new("/home/u/.codex"))),
    );
    // Bare agents ignore model/effort entirely.
    assert_eq!(
        CLIAgent::Gemini.launch_command_routed_with(None, Some("pro"), Some("high")),
        "gemini"
    );
    assert_eq!(
        CLIAgent::Antigravity.launch_command_routed_with(
            None,
            Some("unverified-model"),
            Some("unverified-effort"),
        ),
        "agy"
    );
}

#[test]
fn new_launch_surfaces_retire_gemini_but_keep_antigravity() {
    assert!(!CLIAgent::Gemini.is_available_for_new_launch());
    assert!(CLIAgent::Antigravity.is_available_for_new_launch());
    assert!(CLIAgent::Grok.is_available_for_new_launch());
    assert!(!CLIAgent::Unknown.is_available_for_new_launch());
}

#[test]
fn installation_scan_targets_agy_not_standalone_gemini() {
    let agents = agents_for_installation_scan().collect::<Vec<_>>();
    assert!(agents.contains(&CLIAgent::Antigravity));
    assert!(agents.contains(&CLIAgent::Grok));
    assert!(!agents.contains(&CLIAgent::Gemini));
}

#[test]
fn gemini_is_only_a_command_search_alias_for_antigravity() {
    assert_eq!(CLIAgent::Antigravity.command_search_aliases(), &["gemini"]);
    assert!(CLIAgent::Gemini.command_search_aliases().is_empty());
    assert!(CLIAgent::Grok.command_search_aliases().is_empty());
}

#[test]
fn antigravity_resume_quotes_the_conversation_id_and_does_not_invent_fork_support() {
    assert_eq!(
        CLIAgent::Antigravity.resume_command("id with spaces;$(touch /tmp/nope)"),
        Some("agy --conversation 'id with spaces;$(touch /tmp/nope)'".into())
    );
    assert_eq!(CLIAgent::Antigravity.fork_command("conversation"), None);
}
