use std::fs;

use tempfile::TempDir;

use super::{
    install_at, managed_link_state, refresh_at, remove_at, ManagedLinkState, SkillInstallPaths,
    OPENAI_YAML, SKILL_MARKDOWN,
};

fn paths(root: &TempDir) -> SkillInstallPaths {
    SkillInstallPaths {
        canonical_dir: root.path().join("data/skills/zaplex"),
        link: root.path().join("home/.agents/skills/zaplex"),
    }
}

#[test]
fn install_refresh_and_remove_are_idempotent() {
    let root = TempDir::new().unwrap();
    let paths = paths(&root);

    install_at(&paths).unwrap();
    install_at(&paths).unwrap();
    assert_eq!(
        fs::read_to_string(paths.canonical_dir.join("SKILL.md")).unwrap(),
        SKILL_MARKDOWN
    );
    assert_eq!(
        fs::read_to_string(paths.canonical_dir.join("agents/openai.yaml")).unwrap(),
        OPENAI_YAML
    );
    assert_eq!(
        managed_link_state(&paths).unwrap(),
        ManagedLinkState::Managed
    );

    fs::write(paths.canonical_dir.join("SKILL.md"), "stale").unwrap();
    install_at(&paths).unwrap();
    assert_eq!(
        fs::read_to_string(paths.canonical_dir.join("SKILL.md")).unwrap(),
        SKILL_MARKDOWN
    );

    remove_at(&paths).unwrap();
    remove_at(&paths).unwrap();
    assert_eq!(
        managed_link_state(&paths).unwrap(),
        ManagedLinkState::Missing
    );
}

#[test]
fn install_and_remove_preserve_unrelated_skill_entries() {
    let root = TempDir::new().unwrap();
    let paths = paths(&root);
    fs::create_dir_all(&paths.link).unwrap();
    fs::write(paths.link.join("SKILL.md"), "user skill").unwrap();

    assert!(install_at(&paths).is_err());
    assert!(remove_at(&paths).is_err());
    assert_eq!(
        fs::read_to_string(paths.link.join("SKILL.md")).unwrap(),
        "user skill"
    );
}

#[test]
fn missing_install_is_not_created_by_launch_refresh() {
    let root = TempDir::new().unwrap();
    let paths = paths(&root);

    refresh_at(&paths).unwrap();
    assert_eq!(
        managed_link_state(&paths).unwrap(),
        ManagedLinkState::Missing
    );
    assert!(!paths.canonical_dir.exists());
    assert!(!paths.link.exists());
}
