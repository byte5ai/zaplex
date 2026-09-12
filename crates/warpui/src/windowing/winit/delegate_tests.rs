#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use super::spawn_file_opener;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
fn detached_file_opener_uses_reaping_child_handle() {
    spawn_file_opener("true", std::path::Path::new("ignored"))
        .expect("test file opener should start");
}
