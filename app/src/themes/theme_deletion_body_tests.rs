use super::remove_theme_image;
use std::{fs, path::Path};

#[test]
fn theme_deletion_only_removes_images_within_the_managed_directory() {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    let sibling = root.path().join("themes-other");
    fs::create_dir_all(themes.join("nested")).unwrap();
    fs::create_dir(&sibling).unwrap();
    let relative = themes.join("nested/relative.png");
    let absolute = themes.join("absolute.png");
    let external = sibling.join("original.png");
    for path in [&relative, &absolute, &external] {
        fs::write(path, b"original").unwrap();
    }
    remove_theme_image(&themes, Path::new("nested/relative.png")).unwrap();
    remove_theme_image(&themes, &absolute).unwrap();
    assert!(!relative.exists());
    assert!(!absolute.exists());

    remove_theme_image(&themes, &external).unwrap();
    remove_theme_image(&themes, Path::new("../themes-other/original.png")).unwrap();
    remove_theme_image(&themes, &themes).unwrap();
    assert_eq!(fs::read(&external).unwrap(), b"original");
    assert!(themes.is_dir());
    assert!(remove_theme_image(&themes, Path::new("missing.png")).is_err());
    assert!(remove_theme_image(&root.path().join("missing"), &external).is_err());
    assert_eq!(fs::read(&external).unwrap(), b"original");
}

#[cfg(unix)]
#[test]
fn theme_deletion_preserves_images_behind_outside_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    let outside = root.path().join("outside");
    fs::create_dir(&themes).unwrap();
    fs::create_dir(&outside).unwrap();
    let external = outside.join("original.png");
    fs::write(&external, b"original").unwrap();
    symlink(&external, themes.join("image.png")).unwrap();
    symlink(&outside, themes.join("linked-directory")).unwrap();

    remove_theme_image(&themes, Path::new("image.png")).unwrap();
    remove_theme_image(&themes, Path::new("linked-directory/original.png")).unwrap();
    assert_eq!(fs::read(&external).unwrap(), b"original");
    assert!(themes
        .join("image.png")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
}
