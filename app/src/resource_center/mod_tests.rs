use std::collections::HashSet;

use settings_value::SettingsValue as _;

use super::{Tip, TipAction};

#[test]
fn persisted_welcome_tips_accept_both_drive_names_without_losing_other_tips() {
    let expected = HashSet::from([
        Tip::Action(TipAction::ZaplexDrive),
        Tip::Action(TipAction::SplitPane),
    ]);
    for drive in ["ZapDrive", "ZaplexDrive"] {
        let native = serde_json::json!([{"Action": drive}, {"Action": "SplitPane"}]);
        assert_eq!(
            serde_json::from_value::<HashSet<Tip>>(native).unwrap(),
            expected
        );
    }
    for drive in ["zap_drive", "zaplex_drive"] {
        let file = serde_json::json!([{"action": drive}, {"action": "split_pane"}]);
        assert_eq!(
            HashSet::<Tip>::from_file_value(&file),
            Some(expected.clone())
        );
    }
    assert_eq!(
        serde_json::to_value(TipAction::ZaplexDrive).unwrap(),
        "ZapDrive"
    );
    assert_eq!(TipAction::ZaplexDrive.to_file_value(), "zap_drive");
}

#[test]
fn welcome_tip_file_names_remain_stable() {
    for key in [
        "command_palette",
        "split_pane",
        "theme_picker",
        "history_search",
        "command_search",
        "ai_command_search",
        "save_new_launch_config",
        "warp_ai",
        "zap_drive",
        "changelog",
        "workflows",
    ] {
        let value = serde_json::json!(key);
        assert_eq!(
            TipAction::from_file_value(&value).unwrap().to_file_value(),
            value
        );
    }
    assert_eq!(
        TipAction::from_file_value(&serde_json::json!("unknown")),
        None
    );
    assert_eq!(TipAction::from_file_value(&serde_json::json!({})), None);
}
