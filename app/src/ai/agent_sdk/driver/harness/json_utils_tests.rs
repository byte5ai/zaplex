use super::write_json_file;
use serde::Serialize;

#[cfg(unix)]
#[test]
fn provider_config_replaces_existing_file_atomically_with_private_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("account.json");
    std::fs::write(&path, b"old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    write_json_file(&path, &serde_json::json!({"account": "high5"}), "serialize").unwrap();

    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value, serde_json::json!({"account": "high5"}));
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[derive(Debug)]
struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("intentional failure"))
    }
}

#[test]
fn serialization_failure_preserves_existing_provider_config() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("account.json");
    std::fs::write(&path, b"existing").unwrap();

    let result = write_json_file(&path, &FailingSerialize, "serialize");

    assert!(result.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"existing");
}
