//! Tests for instances.json-style account overrides.

use super::*;
use crate::types::{Account, AccountStatus, Provider, UsageProvenance, WindowTotals};

fn account(key: &str) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider: Provider::Claude,
            key: key.into(),
            config_dir: format!("/tmp/{key}").into(),
            label: key.into(),
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: key.ends_with("default"),
        },
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        today_by_session: Default::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions: Vec::new(),
        status: AccountStatus::Live,
        provenance: UsageProvenance::Real,
    }
}

fn keys(v: &[AccountUsage]) -> Vec<String> {
    v.iter().map(|a| a.account.key.clone()).collect()
}

#[test]
fn empty_overrides_pass_accounts_through_unchanged() {
    let ov = AccountOverrides::parse("{}");
    assert!(ov.is_empty());
    let accts = vec![account("claude:a"), account("claude:default")];
    let out = ov.apply(accts);
    assert_eq!(keys(&out), vec!["claude:a", "claude:default"]);
}

#[test]
fn hidden_accounts_are_dropped() {
    let ov = AccountOverrides::parse(r#"{"claude:a": {"hidden": true}}"#);
    let out = ov.apply(vec![account("claude:a"), account("claude:b")]);
    assert_eq!(keys(&out), vec!["claude:b"]);
}

#[test]
fn label_override_is_applied() {
    let ov = AccountOverrides::parse(r#"{"claude:work": {"label": "Work — Max 20x"}}"#);
    let out = ov.apply(vec![account("claude:work")]);
    assert_eq!(out[0].account.label, "Work — Max 20x");
    assert_eq!(out[0].account.key, "claude:work", "key is never rewritten");
}

#[test]
fn order_sorts_ascending_with_unordered_last_stable() {
    // b→order 1, default→order 0; a and c are un-ordered and keep their
    // input relative order after the ordered ones.
    let ov = AccountOverrides::parse(
        r#"{"claude:b": {"order": 1}, "claude:default": {"order": 0}}"#,
    );
    let out = ov.apply(vec![
        account("claude:a"),
        account("claude:b"),
        account("claude:c"),
        account("claude:default"),
    ]);
    assert_eq!(
        keys(&out),
        vec!["claude:default", "claude:b", "claude:a", "claude:c"]
    );
}

#[test]
fn color_lookup_by_key() {
    let ov = AccountOverrides::parse(r##"{"claude:a": {"color": "#22C55E"}}"##);
    assert_eq!(ov.color_for("claude:a"), Some("#22C55E"));
    assert_eq!(ov.color_for("claude:b"), None);
}

#[test]
fn combined_override_hide_relabel_recolor_reorder() {
    let ov = AccountOverrides::parse(
        r##"{
            "claude:old": {"hidden": true},
            "claude:work": {"label": "Work", "color": "#FB923C", "order": 0}
        }"##,
    );
    let out = ov.apply(vec![
        account("claude:default"),
        account("claude:old"),
        account("claude:work"),
    ]);
    // old dropped; work pulled to front by order 0; default un-ordered after.
    assert_eq!(keys(&out), vec!["claude:work", "claude:default"]);
    assert_eq!(out[0].account.label, "Work");
    assert_eq!(ov.color_for("claude:work"), Some("#FB923C"));
}

#[test]
fn broken_json_yields_no_overrides_never_hides_accounts() {
    for bad in ["", "not json", "[1,2,3]", "{\"k\": \"not an object\"}"] {
        let ov = AccountOverrides::parse(bad);
        assert!(ov.is_empty(), "broken input {bad:?} must yield empty overrides");
        // And apply is a pass-through — accounts survive a broken file.
        let out = ov.apply(vec![account("claude:a")]);
        assert_eq!(keys(&out), vec!["claude:a"]);
    }
}

// ── Writing an alias back (A1) ──────────────────────────────────────────────

/// The one that matters: `instances.json` is claudeplex's file. A user's copy may
/// hold keys this crate has never heard of, and `AccountOverride` drops unknown
/// fields on parse — so writing that struct back would delete another tool's
/// data. The edit must touch one key and leave everything else standing.
#[test]
fn writing_an_alias_preserves_fields_this_crate_does_not_know() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    std::fs::write(
        &path,
        r##"{
  "claude:work": {
    "color": "#22C55E",
    "order": 2,
    "somethingClaudeplexAdded": {"nested": true},
    "futureField": 42
  },
  "codex:default": {"hidden": true}
}"##,
    )
    .unwrap();

    set_label_override(&path, "claude:work", Some("Arbeit")).unwrap();

    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let work = &v["claude:work"];
    assert_eq!(work["label"], "Arbeit", "the alias landed");
    assert_eq!(work["color"], "#22C55E", "our own known field survived");
    assert_eq!(work["order"], 2);
    assert_eq!(
        work["somethingClaudeplexAdded"]["nested"], true,
        "a field we have never heard of must survive a write"
    );
    assert_eq!(work["futureField"], 42);
    assert_eq!(
        v["codex:default"]["hidden"], true,
        "another account's entry is none of our business"
    );
}

/// An empty alias means "no alias", not an account named "". And an entry left
/// with nothing in it is removed rather than kept as bookkeeping.
#[test]
fn clearing_an_alias_removes_it_and_tidies_up_after_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    std::fs::write(&path, r#"{"claude:work": {"label": "Arbeit", "order": 1}}"#).unwrap();

    set_label_override(&path, "claude:work", Some("   ")).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(v["claude:work"].get("label").is_none(), "blank is not a name");
    assert_eq!(v["claude:work"]["order"], 1, "the rest of the entry stays");

    // Now clear the last thing it held: the entry itself goes.
    set_label_override(&path, "claude:work", None).unwrap();
    std::fs::write(&path, r#"{"claude:only": {"label": "X"}}"#).unwrap();
    set_label_override(&path, "claude:only", None).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        v.as_object().unwrap().is_empty(),
        "an entry with nothing left in it leaves no trace"
    );
}

/// A file that is not a JSON object is something we failed to understand.
/// Overwriting it would destroy it; refusing loses only the alias.
#[test]
fn a_file_we_do_not_understand_is_refused_not_clobbered() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    for garbage in [r#"["not", "an", "object"]"#, "this is not json at all"] {
        std::fs::write(&path, garbage).unwrap();
        assert!(
            set_label_override(&path, "claude:work", Some("X")).is_err(),
            "must refuse: {garbage}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            garbage,
            "and must leave the file exactly as it found it"
        );
    }
}

/// No file yet is the normal first case, not an error.
#[test]
fn a_missing_file_starts_an_empty_one() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested").join("instances.json");
    set_label_override(&path, "claude:default", Some("Privat")).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["claude:default"]["label"], "Privat");
}

/// What is written must read back through the parser that consumes it — a write
/// path and a read path that disagree would be worse than no alias at all.
#[test]
fn what_we_write_is_what_the_parser_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    set_label_override(&path, "claude:work", Some("Arbeit")).unwrap();
    let parsed = AccountOverrides::parse(&std::fs::read_to_string(&path).unwrap());
    let relabelled = parsed.apply(vec![account("claude:work")]);
    assert_eq!(relabelled[0].account.label, "Arbeit");
}

/// A symlinked instances.json is a deliberate act — the user pointed it at a
/// dotfiles repo, a shared config, somewhere. Renaming onto the link would
/// silently replace it with a regular copy and disconnect them from wherever
/// they meant it to live.
#[test]
#[cfg(unix)]
fn a_symlinked_file_is_followed_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("dotfiles").join("instances.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, r#"{"claude:work": {"order": 1}}"#).unwrap();
    let link = tmp.path().join("instances.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    set_label_override(&link, "claude:work", Some("Arbeit")).unwrap();

    assert!(
        std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
        "the link must still be a link"
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
    assert_eq!(v["claude:work"]["label"], "Arbeit", "and the target got the edit");
    assert_eq!(v["claude:work"]["order"], 1);
}

/// The file may hold secrets-adjacent config, so a 0600 file must not come back
/// 0644 because the rename swapped in a fresh inode under our umask.
#[test]
#[cfg(unix)]
fn the_original_permissions_survive_the_write() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    set_label_override(&path, "claude:work", Some("Arbeit")).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "0600 in, 0600 out");
}

fn assert_failed_write_preserves_last_good_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instances.json");
    let last_good = r#"{"claude:work":{"label":"Last good"}}"#;
    std::fs::write(&path, last_good).unwrap();

    // Block the deterministic per-process scratch path. The write must fail
    // before the atomic rename and leave the destination byte-for-byte intact.
    let scratch = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::create_dir(&scratch).unwrap();

    assert!(set_label_override(&path, "claude:work", Some("New value")).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), last_good);
}

#[test]
fn failed_settings_write_preserves_last_good_file() {
    assert_failed_write_preserves_last_good_file();
}

#[test]
fn cockpit_persistence_failure_preserves_last_good_state() {
    assert_failed_write_preserves_last_good_file();
}
