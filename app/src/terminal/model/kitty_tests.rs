#[cfg(all(feature = "local_fs", target_os = "linux"))]
#[test]
fn shared_memory_read_releases_fd_and_mapping() {
    use std::io::Write as _;
    use std::os::fd::FromRawFd as _;

    use nix::fcntl::OFlag;
    use nix::sys::mman::{shm_open, shm_unlink};
    use nix::sys::stat::Mode;

    use super::{
        KittyControlData, KittyImage, KittyMessage, KittyPixelDataFormat, KittyTransmissionMedium,
    };

    let object_name = format!(
        "/zaplex-kitty-test-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    );
    let image_bytes = b"test-image-payload";
    let fd = shm_open(
        object_name.as_str(),
        OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR,
        Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(image_bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let message = KittyMessage {
        control_data: KittyControlData {
            pixel_data_format: KittyPixelDataFormat::Png,
            transmission_medium: KittyTransmissionMedium::SharedMemoryObject,
            ..KittyControlData::default()
        },
        payload: object_name.as_bytes().to_vec(),
    };
    let image = KittyImage::try_from(message).unwrap();

    assert_eq!(image.data, image_bytes);
    assert!(!open_file_descriptors_contain(&object_name));
    assert!(!std::fs::read_to_string("/proc/self/maps")
        .unwrap()
        .contains(&object_name));

    let _ = shm_unlink(object_name.as_str());
}

#[cfg(all(feature = "local_fs", target_os = "linux"))]
fn open_file_descriptors_contain(object_name: &str) -> bool {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .any(|target| target.to_string_lossy().contains(object_name))
}
