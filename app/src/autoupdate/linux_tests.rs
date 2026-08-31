use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

use anyhow::bail;

use super::{
    appimage::{atomically_replace_appimage_with_hook, temporary_appimage_for},
    OSS_APPIMAGE_ASSET_NAME,
};

#[test]
fn oss_appimage_asset_name_matches_linux_bundle() {
    assert_eq!(OSS_APPIMAGE_ASSET_NAME, "Zaplex-x86_64.AppImage");
}

#[test]
fn pre_rename_failure_preserves_old_appimage_and_cleans_staging_file() {
    let directory = tempfile::tempdir().unwrap();
    let appimage_path = directory.path().join("Zaplex.AppImage");
    fs::write(&appimage_path, b"old appimage").unwrap();
    fs::set_permissions(&appimage_path, fs::Permissions::from_mode(0o755)).unwrap();

    let mut staged = temporary_appimage_for(&appimage_path).unwrap();
    staged.write_all(b"new appimage").unwrap();
    let staged_path = staged.path().to_path_buf();

    let error = atomically_replace_appimage_with_hook(
        staged,
        &appimage_path,
        |prepared_path, target_path| {
            assert_eq!(prepared_path, staged_path);
            assert_eq!(target_path, appimage_path);
            assert_eq!(fs::read(prepared_path).unwrap(), b"new appimage");
            assert_eq!(
                fs::metadata(prepared_path).unwrap().permissions().mode() & 0o777,
                0o755
            );
            bail!("injected pre-rename failure")
        },
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("injected pre-rename failure"));
    assert_eq!(fs::read(&appimage_path).unwrap(), b"old appimage");
    assert_eq!(
        fs::metadata(&appimage_path).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!staged_path.exists());
}

#[test]
fn successful_replacement_renames_the_prepared_inode() {
    let directory = tempfile::tempdir().unwrap();
    let appimage_path = directory.path().join("Zaplex.AppImage");
    fs::write(&appimage_path, b"old appimage").unwrap();
    fs::set_permissions(&appimage_path, fs::Permissions::from_mode(0o755)).unwrap();
    let old_inode = fs::metadata(&appimage_path).unwrap().ino();

    let mut staged = temporary_appimage_for(&appimage_path).unwrap();
    staged.write_all(b"new appimage").unwrap();
    let staged_path = staged.path().to_path_buf();
    let staged_metadata = fs::metadata(&staged_path).unwrap();
    let staged_inode = staged_metadata.ino();
    let staged_device = staged_metadata.dev();

    atomically_replace_appimage_with_hook(staged, &appimage_path, |_, _| Ok(())).unwrap();

    let installed_metadata = fs::metadata(&appimage_path).unwrap();
    assert_eq!(fs::read(&appimage_path).unwrap(), b"new appimage");
    assert_eq!(installed_metadata.permissions().mode() & 0o777, 0o755);
    assert_eq!(installed_metadata.dev(), staged_device);
    assert_eq!(installed_metadata.ino(), staged_inode);
    assert_ne!(installed_metadata.ino(), old_inode);
    assert!(!staged_path.exists());
}

#[test]
fn temporary_appimage_is_created_in_the_target_directory() {
    let directory = tempfile::tempdir().unwrap();
    let appimage_path = directory.path().join("Zaplex.AppImage");

    let staged = temporary_appimage_for(&appimage_path).unwrap();

    assert_eq!(staged.path().parent(), Some(directory.path()));
    assert_eq!(
        fs::metadata(staged.path()).unwrap().dev(),
        fs::metadata(directory.path()).unwrap().dev()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn replacement_uses_the_target_mount_when_system_temp_is_on_another_mount() {
    let system_temp = tempfile::tempdir_in("/tmp")
        .expect("the Linux integration environment must provide a writable /tmp");
    let target_directory = tempfile::tempdir_in("/dev/shm")
        .expect("the Linux integration environment must provide a writable /dev/shm");
    assert_ne!(
        fs::metadata(system_temp.path()).unwrap().dev(),
        fs::metadata(target_directory.path()).unwrap().dev(),
        "the integration test requires /tmp and /dev/shm on separate mounts"
    );

    let appimage_path = target_directory.path().join("Zaplex.AppImage");
    fs::write(&appimage_path, b"old appimage").unwrap();
    fs::set_permissions(&appimage_path, fs::Permissions::from_mode(0o755)).unwrap();
    let mut staged = temporary_appimage_for(&appimage_path).unwrap();
    staged.write_all(b"new appimage").unwrap();

    assert_eq!(
        fs::metadata(staged.path()).unwrap().dev(),
        fs::metadata(target_directory.path()).unwrap().dev()
    );
    atomically_replace_appimage_with_hook(staged, &appimage_path, |_, _| Ok(())).unwrap();

    assert_eq!(fs::read(appimage_path).unwrap(), b"new appimage");
}

#[test]
fn tempfile_creation_error_names_the_target_and_parent() {
    let directory = tempfile::tempdir().unwrap();
    let invalid_parent = directory.path().join("not-a-directory");
    fs::write(&invalid_parent, b"file").unwrap();
    let appimage_path = invalid_parent.join("Zaplex.AppImage");

    let error = temporary_appimage_for(&appimage_path).unwrap_err();
    let message = format!("{error:#}");

    assert!(message.contains(&invalid_parent.display().to_string()));
    assert!(message.contains(&appimage_path.display().to_string()));
}
