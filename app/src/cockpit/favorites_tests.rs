use super::*;

#[test]
fn corrupt_store_is_never_overwritten() {
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
fn valid_store_is_replaced_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("favorites.json");
    let original = Favorites {
        items: vec![Favorite::new(
            FavoriteKind::Project,
            "project-1",
            "zaplex",
        )],
    };
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();

    let (mut favorites, state) = load_favorites_from(&path);
    favorites.add(Favorite::new(FavoriteKind::Host, "node-dev", "devhost"));
    save_favorites_to(&path, &favorites, state).unwrap();

    let persisted: Favorites =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted, favorites);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "the atomic temporary file must not remain beside the store"
    );
}
