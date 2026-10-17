use super::*;

#[test]
fn try_new_unix_absolute() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(p.as_str(), "/home/user/project");
    assert!(p.is_unix());
}

#[test]
fn try_new_windows_absolute() {
    let p = StandardizedPath::try_new("C:\\Users\\user\\project").unwrap();
    assert_eq!(p.as_str(), "C:\\Users\\user\\project");
    assert!(p.is_windows());
}

#[test]
fn file_name() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    assert_eq!(p.file_name(), Some("file.rs"));
}

#[test]
fn extension() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    assert_eq!(p.extension(), Some("rs"));
}

#[test]
fn parent() {
    let p = StandardizedPath::try_new("/home/user/file.rs").unwrap();
    let parent = p.parent().unwrap();
    assert_eq!(parent.as_str(), "/home/user");
}

#[test]
fn starts_with() {
    let p = StandardizedPath::try_new("/home/user/project/src").unwrap();
    let base = StandardizedPath::try_new("/home/user/project").unwrap();
    assert!(p.starts_with(&base));
    let other = StandardizedPath::try_new("/other").unwrap();
    assert!(!p.starts_with(&other));
}

#[test]
fn join() {
    let p = StandardizedPath::try_new("/home/user").unwrap();
    let joined = p.join("project/src");
    assert_eq!(joined.as_str(), "/home/user/project/src");
}

#[test]
fn to_local_path() {
    // to_local_path returns Some only when encoding matches the OS.
    let existing = std::env::temp_dir();
    let p = StandardizedPath::from_local_canonicalized(&existing).unwrap();
    let local = p.to_local_path();
    assert!(local.is_some());
}

#[test]
fn display() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    assert_eq!(format!("{p}"), "/home/user/project");
}

#[test]
fn serde_roundtrip() {
    let p = StandardizedPath::try_new("/home/user/project").unwrap();
    let json = serde_json::to_string(&p).unwrap();
    assert_eq!(json, "\"/home/user/project\"");
    let deserialized: StandardizedPath = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, p);
}
