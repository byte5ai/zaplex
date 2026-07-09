use std::path::PathBuf;

use crate::{
    app_state::{
        AppState, BranchSnapshot, LeafContents, LeafSnapshot, NotebookPaneSnapshot, PaneFlex,
        PaneNodeSnapshot, SplitDirection, TabSnapshot, TerminalPaneSnapshot, WindowSnapshot,
    },
    drive::ZaplexDriveObjectSettings,
    tab::SelectedTabColor,
};

use super::{LaunchConfig, PaneMode, PaneTemplateType};

fn single_tab_snapshot(root: PaneNodeSnapshot) -> AppState {
    AppState {
        windows: vec![WindowSnapshot {
            tabs: vec![TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root,
                left_panel: None,
                right_panel: None,
            }],
            active_tab_index: 0,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
        }],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

fn multi_tab_snapshot(active_tab_index: usize, tabs: Vec<TabSnapshot>) -> AppState {
    AppState {
        windows: vec![WindowSnapshot {
            tabs,
            active_tab_index,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
        }],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

#[test]
fn test_config_from_snapshot_flattens_single_pane() {
    // If only one pane of the branch can be saved into a launch configuration, it should
    // be flattened to a single leaf.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::NotebookObject {
                        notebook_id: None,
                        settings: ZaplexDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        template.windows[0].tabs[0].layout,
        PaneTemplateType::PaneTemplate {
            is_focused: Some(true),
            cwd: PathBuf::from("/some/dir"),
            commands: vec![],
            pane_mode: PaneMode::Terminal,
            shell: None,
        },
    )
}

#[test]
fn test_config_from_snapshot_filters_panes() {
    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/path/to/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::NotebookObject {
                        notebook_id: None,
                        settings: ZaplexDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        template.windows[0].tabs[0].layout,
        PaneTemplateType::PaneBranchTemplate {
            split_direction: SplitDirection::Vertical.into(),
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(true),
                    cwd: PathBuf::from("/path/to/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(false),
                    cwd: PathBuf::from("/some/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ]
        }
    )
}

#[test]
fn test_config_from_snapshot_filters_tabs() {
    // If no panes of a tab are valid, it's filtered out entirely.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![(
            PaneFlex(1.),
            PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Notebook(NotebookPaneSnapshot::NotebookObject {
                    notebook_id: None,
                    settings: ZaplexDriveObjectSettings::default(),
                }),
            }),
        )],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert!(template.windows[0].tabs.is_empty())
}

#[test]
fn test_config_with_active_tab_index() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None
            };
            3
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, Some(1))
}

#[test]
fn test_config_with_active_tab_index_and_filtered_tabs() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(
                                NotebookPaneSnapshot::NotebookObject {
                                    notebook_id: None,
                                    settings: ZaplexDriveObjectSettings::default(),
                                },
                            ),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, Some(0))
}

#[test]
fn test_config_with_active_tab_being_filtered() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(
                                NotebookPaneSnapshot::NotebookObject {
                                    notebook_id: None,
                                    settings: ZaplexDriveObjectSettings::default(),
                                },
                            ),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, None)
}

#[test]
fn test_config_captures_ssh_host_reference() {
    // A registered SSH host pane is captured as a host *reference* (node_id
    // only), not dropped and not carrying connection data (#103).
    let state = single_tab_snapshot(PaneNodeSnapshot::Leaf(LeafSnapshot {
        is_focused: true,
        custom_vertical_tabs_title: None,
        contents: LeafContents::SshServer {
            node_id: "node-devhost".into(),
        },
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        template.windows[0].tabs[0].layout,
        PaneTemplateType::SshHost {
            node_id: "node-devhost".into(),
        },
    );
}

#[test]
fn test_ssh_host_reference_serde_round_trips_and_does_not_collide() {
    // The untagged SshHost variant survives a YAML round-trip and does not
    // shadow (or get shadowed by) the PaneTemplate variant.
    let host = PaneTemplateType::SshHost {
        node_id: "node-1".into(),
    };
    let yaml = serde_yaml::to_string(&host).unwrap();
    assert_eq!(
        serde_yaml::from_str::<PaneTemplateType>(&yaml).unwrap(),
        host
    );

    // A `{node_id}` blob resolves to SshHost, a `{cwd,…}` blob to PaneTemplate.
    let template = PaneTemplateType::PaneTemplate {
        cwd: PathBuf::from("/tmp"),
        commands: vec![],
        is_focused: None,
        pane_mode: PaneMode::Terminal,
        shell: None,
    };
    let template_yaml = serde_yaml::to_string(&template).unwrap();
    assert_eq!(
        serde_yaml::from_str::<PaneTemplateType>(&template_yaml).unwrap(),
        template
    );
}
