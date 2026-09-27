use super::editor_context_menu_visibility;

#[test]
fn editor_context_menu_respects_selection_editability_and_password_safety() {
    let cases = [
        ((true, true, false), (true, true, true)),
        ((false, true, false), (false, false, true)),
        ((true, false, false), (false, true, false)),
        ((true, true, true), (false, false, true)),
        ((true, false, true), (false, false, false)),
    ];

    for ((has_selection, can_edit, is_password), expected) in cases {
        assert_eq!(
            editor_context_menu_visibility(has_selection, can_edit, is_password),
            expected
        );
    }
}

#[test]
fn settings_sections_round_trip_with_distinct_stable_keys() {
    use super::SettingsSection::*;
    let sections = [
        About,
        MCPServers,
        Appearance,
        Features,
        Keybindings,
        ZaplexDrive,
        Zaplexify,
        AI,
        WarpAgent,
        AgentProfiles,
        AgentMCPServers,
        Knowledge,
        ThirdPartyCLIAgents,
        Network,
        Code,
        EditorAndCodeReview,
        CloudSync,
    ];
    let mut keys = std::collections::HashSet::new();
    for section in sections {
        let key = section.persistence_key();
        assert!(keys.insert(key), "duplicate persisted key: {key}");
        assert_eq!(key.parse(), Ok(section));
    }
    assert_eq!(Appearance.persistence_key(), "Appearance");
    assert_eq!(AgentMCPServers.persistence_key(), "AgentMCPServers");
}

#[test]
fn settings_sections_restore_legacy_product_and_german_labels() {
    use super::SettingsSection::*;
    for (label, expected) in [
        ("Warpify", Zaplexify),
        ("ZapDrive", ZaplexDrive),
        ("Zap Drive", ZaplexDrive),
        ("Warp Drive", ZaplexDrive),
        ("Zap Agent", WarpAgent),
        ("Warp Agent", WarpAgent),
        ("Oz", WarpAgent),
        ("Über", About),
        ("KI", AI),
        ("MCP-Server", MCPServers),
        ("Erscheinungsbild", Appearance),
        ("Funktionen", Features),
        ("Tastenkürzel", Keybindings),
        ("Zaplex-Agent", WarpAgent),
        ("Profile", AgentProfiles),
        ("Wissen", Knowledge),
        ("Externe CLI-Agenten", ThirdPartyCLIAgents),
        ("Editor und Code-Review", EditorAndCodeReview),
        ("Netzwerk", Network),
        ("Cloud-Sync", CloudSync),
    ] {
        assert_eq!(label.parse(), Ok(expected), "legacy label: {label}");
    }
    assert!("unknown page".parse::<super::SettingsSection>().is_err());
}
