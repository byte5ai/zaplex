use std::fs;
use std::time::SystemTime;

use chrono::{Duration, Utc};
use serde_json::json;

use super::*;

fn write_conversation(home: &Path, cwd: &str, id: &str, modified: DateTime<Utc>) {
    let state_dir = data_dir(home);
    fs::create_dir_all(state_dir.join("cache")).unwrap();
    fs::create_dir_all(state_dir.join("conversations")).unwrap();
    let registry = state_dir.join("cache").join("last_conversations.json");
    let mut entries = fs::read_to_string(&registry)
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).ok()
        })
        .unwrap_or_default();
    entries.insert(cwd.to_string(), json!(id));
    fs::write(registry, serde_json::to_vec(&entries).unwrap()).unwrap();
    fs::File::create(state_dir.join("conversations").join(format!("{id}.db")))
        .unwrap()
        .set_modified(SystemTime::from(modified))
        .unwrap();
}

#[test]
fn discovers_bounded_idle_sessions_without_reading_conversation_content() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    write_conversation(
        tmp.path(),
        "/work/older",
        "11111111-1111-1111-1111-111111111111",
        now - Duration::hours(2),
    );
    write_conversation(
        tmp.path(),
        "/work/newer",
        "22222222-2222-2222-2222-222222222222",
        now - Duration::hours(1),
    );

    let sessions = idle_sessions(tmp.path(), now, Duration::days(7), 1);
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].session_id,
        "22222222-2222-2222-2222-222222222222"
    );
    assert_eq!(sessions[0].cwd, "/work/newer");
    assert_eq!(sessions[0].provider, Provider::Antigravity);
    assert_eq!(sessions[0].state, SessionState::Idle);
    assert_eq!(sessions[0].task_state, None);
    assert_eq!(sessions[0].pid, 0);
}

#[test]
fn skips_stale_missing_relative_and_unsafe_registry_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    write_conversation(
        tmp.path(),
        "/work/stale",
        "11111111-1111-1111-1111-111111111111",
        now - Duration::days(8),
    );
    write_conversation(
        tmp.path(),
        "relative/work",
        "22222222-2222-2222-2222-222222222222",
        now,
    );

    let registry = data_dir(tmp.path())
        .join("cache")
        .join("last_conversations.json");
    let mut entries = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(
        &fs::read(&registry).unwrap(),
    )
    .unwrap();
    entries.insert("/work/unsafe".into(), json!("../../outside"));
    entries.insert(
        "/work/missing".into(),
        json!("33333333-3333-3333-3333-333333333333"),
    );
    fs::write(registry, serde_json::to_vec(&entries).unwrap()).unwrap();

    assert!(idle_sessions(tmp.path(), now, Duration::days(7), 50).is_empty());
}
