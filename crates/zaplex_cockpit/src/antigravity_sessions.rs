//! Dormant Antigravity (`agy`) conversation discovery.
//!
//! Antigravity keeps the last conversation for each workspace in
//! `~/.gemini/antigravity-cli/cache/last_conversations.json` and one SQLite
//! database per conversation. The registry is enough to surface resumable
//! conversations without opening the databases or reading prompt/model output.
//!
//! Unlike Claude, Antigravity exposes no process registry here; unlike Codex,
//! its database stores state in protobuf blobs. This scanner therefore makes no
//! live-state claim: every discovered conversation is [`SessionState::Idle`].
//! Native terminal hooks remain responsible for live rich status.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::types::{Provider, SessionSnapshot, SessionState};

/// Locate Antigravity's application-data directory below a user's home.
pub fn data_dir(home: &Path) -> std::path::PathBuf {
    home.join(".gemini").join("antigravity-cli")
}

/// Discover the most recent resumable Antigravity conversation per workspace.
///
/// The registry is bounded naturally to one conversation per workspace and is
/// capped again by `limit`. Missing/malformed registries, unsafe ids, relative
/// workspaces, and conversations without a database are skipped.
pub fn idle_sessions(
    home: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> Vec<SessionSnapshot> {
    if limit == 0 {
        return Vec::new();
    }
    let state_dir = data_dir(home);
    let registry = state_dir.join("cache").join("last_conversations.json");
    let Ok(content) = std::fs::read_to_string(registry) else {
        return Vec::new();
    };
    let Ok(by_workspace) = serde_json::from_str::<HashMap<String, String>>(&content) else {
        return Vec::new();
    };
    let cutoff = now - max_age;
    let mut seen = HashSet::new();
    let mut sessions = Vec::new();

    for (cwd, session_id) in by_workspace {
        let cwd_path = Path::new(&cwd);
        if !cwd_path.is_absolute()
            || !is_safe_conversation_id(&session_id)
            || !seen.insert(session_id.clone())
        {
            continue;
        }
        let database = state_dir
            .join("conversations")
            .join(format!("{session_id}.db"));
        let Some(last_activity) = std::fs::metadata(database)
            .and_then(|metadata| metadata.modified())
            .ok()
            .map(DateTime::<Utc>::from)
            .filter(|mtime| *mtime >= cutoff)
        else {
            continue;
        };
        let project = crate::project::resolve_project(cwd_path);
        sessions.push(SessionSnapshot {
            session_id,
            cwd,
            name: String::new(),
            state: SessionState::Idle,
            provider: Provider::Antigravity,
            model: String::new(),
            effort: None,
            ctx_tokens: 0,
            project_root: project.root,
            repo_root: project.repo_root,
            project_name: project.name,
            branch: project.branch,
            worktree: project.worktree,
            config_dir: Some(state_dir.to_string_lossy().into_owned()),
            account_email: None,
            account_id: None,
            process_fingerprint: None,
            pty_session_id: None,
            pty_session_generation: None,
            pty_foreground: false,
            task_state: None,
            last_activity,
            pid: 0,
        });
    }
    sessions.sort_by(|left, right| right.last_activity.cmp(&left.last_activity));
    sessions.truncate(limit);
    sessions
}

fn is_safe_conversation_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
#[path = "antigravity_sessions_tests.rs"]
mod tests;
