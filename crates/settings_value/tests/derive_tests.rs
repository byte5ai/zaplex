use serde::{Deserialize, Serialize};
use serde_json::json;
use settings_value::SettingsValue;

// -- struct-level #[serde(default)] -------------------------------------------

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, SettingsValue)]
#[serde(default)]
struct StructWithContainerDefault {
    name: String,
    count: u32,
}

#[test]
fn container_default_empty_object_uses_all_defaults() {
    let json = json!({});
    let result = StructWithContainerDefault::from_file_value(&json).unwrap();
    assert_eq!(result, StructWithContainerDefault::default());
}
