use super::EditorChoice;
use settings_value::SettingsValue as _;

#[test]
fn legacy_editor_choice_survives_native_and_file_deserialization() {
    assert_eq!(
        serde_json::from_str::<EditorChoice>("\"Zap\"").unwrap(),
        EditorChoice::Zaplex,
    );
    assert_eq!(
        serde_json::from_str::<EditorChoice>("\"Zaplex\"").unwrap(),
        EditorChoice::Zaplex,
    );
    assert_eq!(
        EditorChoice::from_file_value(&serde_json::json!("zap")),
        Some(EditorChoice::Zaplex),
    );
    assert_eq!(EditorChoice::Zaplex.to_file_value(), serde_json::json!("zaplex"));
}

#[test]
fn editor_choice_file_variants_keep_their_existing_representation() {
    for value in [serde_json::json!("system_default"), serde_json::json!("zaplex"), serde_json::json!("env_editor")] {
        let parsed = EditorChoice::from_file_value(&value).unwrap();
        assert_eq!(parsed.to_file_value(), value);
    }
    assert_eq!(EditorChoice::from_file_value(&serde_json::json!("unknown")), None);
    assert_eq!(EditorChoice::from_file_value(&serde_json::json!({})), None);
}

#[test]
fn external_editor_choice_retains_nested_file_and_legacy_native_formats() {
    let choice = EditorChoice::ExternalEditor(super::super::Editor::VSCode);
    assert_eq!(EditorChoice::from_file_value(&choice.to_file_value()), Some(choice));
    assert_eq!(serde_json::from_str::<EditorChoice>("\"VSCode\"").unwrap(), choice);
    assert_eq!(
        serde_json::from_str::<EditorChoice>("null").unwrap(),
        EditorChoice::SystemDefault,
    );
}
