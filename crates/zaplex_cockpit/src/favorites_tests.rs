use super::*;

fn host(id: &str) -> Favorite {
    Favorite::new(FavoriteKind::Host, id, format!("host-{id}"))
}

#[test]
fn add_is_idempotent_by_kind_and_target() {
    let mut favs = Favorites::default();
    assert!(favs.add(host("n1")));
    assert!(!favs.add(host("n1"))); // same (kind,target) → not re-added
    assert_eq!(favs.len(), 1);
    // Same target but a different kind is a distinct favorite.
    assert!(favs.add(Favorite::new(FavoriteKind::Project, "n1", "proj")));
    assert_eq!(favs.len(), 2);
}

#[test]
fn re_add_refreshes_label_in_place_without_reordering() {
    let mut favs = Favorites::default();
    favs.add(host("a"));
    favs.add(host("b"));
    // Re-add "a" with a new label: label updates, order stays [a, b].
    assert!(!favs.add(Favorite::new(FavoriteKind::Host, "a", "renamed")));
    assert_eq!(favs.items()[0].display_label(), "renamed");
    assert_eq!(favs.items()[1].target, "b");
}

#[test]
fn toggle_adds_then_removes() {
    let mut favs = Favorites::default();
    assert!(favs.toggle(host("x"))); // now a favorite
    assert!(favs.contains(FavoriteKind::Host, "x"));
    assert!(!favs.toggle(host("x"))); // toggled off
    assert!(!favs.contains(FavoriteKind::Host, "x"));
    assert!(favs.is_empty());
}

#[test]
fn remove_reports_membership() {
    let mut favs = Favorites::default();
    favs.add(host("x"));
    assert!(favs.remove(FavoriteKind::Host, "x"));
    assert!(!favs.remove(FavoriteKind::Host, "x")); // already gone
}

#[test]
fn display_label_falls_back_to_target() {
    let bare = Favorite::new(FavoriteKind::Session, "sess-1", "");
    assert_eq!(bare.display_label(), "sess-1");
}

#[test]
fn move_item_reorders_and_clamps() {
    let mut favs = Favorites::default();
    favs.add(host("a"));
    favs.add(host("b"));
    favs.add(host("c"));
    favs.move_item(2, 0); // c to front
    assert_eq!(
        favs.items()
            .iter()
            .map(|f| f.target.clone())
            .collect::<Vec<_>>(),
        vec!["c", "a", "b"]
    );
    favs.move_item(0, 99); // clamp to last
    assert_eq!(
        favs.items()
            .iter()
            .map(|f| f.target.clone())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    favs.move_item(5, 0); // out-of-range from → no-op
    assert_eq!(favs.len(), 3);
}

#[test]
fn json_round_trips() {
    let mut favs = Favorites::default();
    favs.add(Favorite::new(FavoriteKind::Host, "n1", "devhost"));
    favs.add(Favorite::new(
        FavoriteKind::GithubFlow,
        "quick_issue",
        "Quick issue",
    ));
    let json = serde_json::to_string(&favs).unwrap();
    let back: Favorites = serde_json::from_str(&json).unwrap();
    assert_eq!(favs, back);
}

#[test]
fn missing_label_deserializes_to_empty() {
    let json = r#"{"items":[{"kind":"host","target":"n1"}]}"#;
    let favs: Favorites = serde_json::from_str(json).unwrap();
    assert_eq!(favs.items()[0].label, "");
    assert_eq!(favs.items()[0].display_label(), "n1");
}

#[test]
fn favorite_menu_migration_preserves_non_host_favorite_data() {
    let mut favorites = Favorites::default();
    favorites.add(Favorite::new(FavoriteKind::Host, "host-1", "devhost"));
    favorites.add(Favorite::new(FavoriteKind::Project, "project-1", "zaplex"));
    favorites.add(Favorite::new(FavoriteKind::Session, "session-1", "review"));
    favorites.add(Favorite::new(FavoriteKind::Launch, "launch-1", "Release"));
    let before = favorites.clone();

    let host_rows: Vec<&Favorite> = favorites.host_menu_items().collect();

    assert_eq!(host_rows.len(), 1);
    assert_eq!(host_rows[0].kind, FavoriteKind::Host);
    assert_eq!(
        favorites, before,
        "filtering the host menu must not mutate or discard hidden favorite records"
    );
    let round_trip: Favorites =
        serde_json::from_str(&serde_json::to_string(&favorites).unwrap()).unwrap();
    assert_eq!(round_trip, before);
}

#[test]
fn plus_menu_lists_only_favorite_hosts() {
    let favorites = Favorites::from_items(vec![
        Favorite::new(FavoriteKind::Project, "project", "Project"),
        Favorite::new(FavoriteKind::Host, "host-a", "A"),
        Favorite::new(FavoriteKind::Session, "session", "Session"),
        Favorite::new(FavoriteKind::Host, "host-b", "B"),
        Favorite::new(FavoriteKind::Launch, "launch", "Launch"),
    ]);

    let host_targets: Vec<&str> = favorites
        .host_menu_items()
        .map(|favorite| favorite.target.as_str())
        .collect();

    assert_eq!(host_targets, vec!["host-a", "host-b"]);
    assert_eq!(
        favorites.items().len(),
        5,
        "hidden records remain persisted"
    );
}

#[test]
fn unknown_favorite_kind_is_opaque_and_survives_mutation() {
    let unknown = serde_json::json!({
        "kind": "workflow",
        "target": "release",
        "label": "Release",
        "future": {"color": "violet"}
    });
    let mut favorites: Favorites = serde_json::from_value(serde_json::json!({
        "items": [
            {"kind": "host", "target": "node-dev", "label": "devhost"},
            unknown.clone()
        ]
    }))
    .unwrap();

    assert_eq!(favorites.items().len(), 1);
    favorites.add(host("node-prod"));

    let raw = serde_json::to_value(&favorites).unwrap();
    assert!(raw["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record == &unknown));
    let reloaded: Favorites = serde_json::from_value(raw).unwrap();
    assert_eq!(reloaded.items().len(), 2);
}
