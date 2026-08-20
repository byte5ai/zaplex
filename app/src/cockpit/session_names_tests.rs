use std::fs;

use zaplex_cockpit::Provider;

use super::*;

fn local_route(session_id: &str) -> SessionRoute {
    SessionRoute {
        provider: Provider::Claude,
        session_id: session_id.to_string(),
        host: SessionHostRoute::Local,
        account: SessionAccountRoute::Local {
            config_dir: Some(PathBuf::from("/accounts/work")),
            account_email: Some("work@example.com".to_string()),
        },
        cwd: PathBuf::from("/work/zaplex"),
        pid: 42,
        process_fingerprint: Some("process-1".to_string()),
    }
}

#[test]
fn rename_round_trips_across_store_reload() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("names.json");
    let route = local_route("session-1");
    let mut store = SessionNameStore::load_from(file.clone());
    store.set_name(&route, "Deploy".to_string()).unwrap();

    let reloaded = SessionNameStore::load_from(file);
    assert_eq!(reloaded.name(&route), Some("Deploy"));
}

#[test]
fn complete_account_and_host_route_prevents_name_aliasing() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("names.json");
    let route = local_route("shared-session-id");
    let mut other = route.clone();
    other.account = SessionAccountRoute::Local {
        config_dir: Some(PathBuf::from("/accounts/personal")),
        account_email: Some("personal@example.com".to_string()),
    };
    let mut store = SessionNameStore::load_from(file);
    store.set_name(&route, "Work".to_string()).unwrap();

    assert_eq!(store.name(&route), Some("Work"));
    assert_eq!(store.name(&other), None);
}

#[test]
fn corrupt_store_is_never_overwritten_by_empty_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("names.json");
    fs::write(&file, "{broken").unwrap();
    let original = fs::read(&file).unwrap();
    let mut store = SessionNameStore::load_from(file.clone());

    assert!(store
        .set_name(&local_route("session-1"), "Deploy".to_string())
        .is_err());
    assert_eq!(store.name(&local_route("session-1")), None);
    assert_eq!(fs::read(file).unwrap(), original);
}

#[test]
fn undeclared_provider_has_no_authoritative_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let mut route = local_route("session-1");
    route.provider = Provider::Antigravity;
    let mut store = SessionNameStore::load_from(temp.path().join("names.json"));

    assert!(!SessionNameStore::supports(Provider::Antigravity));
    assert!(store.set_name(&route, "Unsupported".to_string()).is_err());
}
