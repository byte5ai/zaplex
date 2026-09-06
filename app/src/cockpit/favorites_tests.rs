use super::*;

#[test]
fn corrupt_favorite_store_is_reported_and_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("favorites.json");
    let corrupt = b"{not valid favorites json";
    std::fs::write(&path, corrupt).unwrap();

    let (mut favorites, state) = load_favorites_from(&path);
    assert_eq!(state, FavoritesFileState::Protected);
    favorites.add(Favorite::new(FavoriteKind::Host, "node-dev", "devhost"));

    assert!(
        save_favorites_to(&path, &favorites, state).is_err(),
        "mutating an in-memory fallback must not clobber the corrupt source"
    );
    assert_eq!(std::fs::read(&path).unwrap(), corrupt);
}

#[test]
fn favorite_store_write_is_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("favorites.json");
    let original = Favorites::from_items(vec![Favorite::new(
        FavoriteKind::Project,
        "project-1",
        "zaplex",
    )]);
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();

    let (mut favorites, state) = load_favorites_from(&path);
    favorites.add(Favorite::new(FavoriteKind::Host, "node-dev", "devhost"));
    save_favorites_to(&path, &favorites, state).unwrap();

    let persisted: Favorites = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted, favorites);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "the atomic temporary file must not remain beside the store"
    );
}

#[test]
fn unknown_favorite_kind_survives_a_writable_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("favorites.json");
    let unknown = serde_json::json!({
        "kind": "workflow",
        "target": "release",
        "label": "Release",
        "future": {"color": "violet"}
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "items": [
                {"kind": "host", "target": "node-dev", "label": "devhost"},
                unknown.clone()
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let (mut favorites, state) = load_favorites_from(&path);
    assert_eq!(state, FavoritesFileState::Loaded);
    assert_eq!(favorites.items().len(), 1);
    assert_eq!(favorites.items()[0].target, "node-dev");

    favorites.add(Favorite::new(FavoriteKind::Host, "node-prod", "prodhost"));
    save_favorites_to(&path, &favorites, state).unwrap();

    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(raw["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record == &unknown));
    let reloaded: Favorites = serde_json::from_value(raw).unwrap();
    assert_eq!(reloaded.items().len(), 2);
}
