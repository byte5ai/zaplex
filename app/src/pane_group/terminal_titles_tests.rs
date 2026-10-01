use super::resolve;

#[test]
fn different_hosts_and_directories_keep_short_titles() {
    let identities = vec![
        (vec![1], "a · api".into(), "a · /srv/api".into()),
        (vec![2], "b · api".into(), "b · /srv/api".into()),
        (vec![3], "a · logs".into(), "a · /srv/logs".into()),
    ];
    assert_eq!(resolve(&identities), vec!["a · api", "b · api", "a · logs"]);
}

#[test]
fn basename_collisions_expand_to_full_paths() {
    let identities = vec![
        (vec![1], "a · api".into(), "a · /one/api".into()),
        (vec![2], "a · api".into(), "a · /two/api".into()),
    ];
    assert_eq!(resolve(&identities), vec!["a · /one/api", "a · /two/api"]);
    assert_eq!(resolve(&identities[..1]), vec!["a · api"]);
}

#[test]
fn identical_paths_have_stable_session_suffixes_across_reordering() {
    let mut identities = vec![
        (vec![1], "a · api".into(), "a · /srv/api".into()),
        (vec![2], "a · api".into(), "a · /srv/api".into()),
    ];
    let original = resolve(&identities);
    assert_ne!(original[0], original[1]);
    assert!(original[0].starts_with("a · /srv/api · "));
    identities.reverse();
    let reordered = resolve(&identities);
    assert_eq!(original[0], reordered[1]);
    assert_eq!(original[1], reordered[0]);
}

#[test]
fn missing_directory_collisions_use_the_full_persistent_identity() {
    let mut first_session = vec![1; 16];
    let second_session = first_session.clone();
    first_session[15] = 2;
    let identities = vec![
        (first_session, "a · Terminal".into(), "a · Terminal".into()),
        (second_session, "a · Terminal".into(), "a · Terminal".into()),
    ];
    let titles = resolve(&identities);
    assert_ne!(titles[0], titles[1]);

    // Once metadata identifies another host, both panes regain their compact title.
    let mut updated = identities;
    updated[1].1 = "b · Terminal".into();
    updated[1].2 = "b · Terminal".into();
    assert_eq!(resolve(&updated), vec!["a · Terminal", "b · Terminal"]);
}
