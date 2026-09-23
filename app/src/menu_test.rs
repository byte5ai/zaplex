use super::{
    menu_item_main_axis_alignment, should_reverse_submenu_layout, split_submenu_primary_width,
    Menu, MenuAction, MenuItem, MenuItemFields, SelectAction, SubMenu, MENU_ITEM_VERTICAL_PADDING,
    SPLIT_SUBMENU_TRIGGER_WIDTH,
};

use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::{
    accessibility::ActionAccessibilityContent, elements::MainAxisAlignment, platform::WindowStyle,
    App, TypedActionView,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestAction {
    Root,
    ChildOne,
    ChildTwo,
}

fn test_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("submenu"),
            menu: SubMenu::new(vec![
                MenuItemFields::new("child one")
                    .with_on_select_action(TestAction::ChildOne)
                    .into_item(),
                MenuItemFields::new("child two")
                    .with_on_select_action(TestAction::ChildTwo)
                    .into_item(),
            ]),
        },
        MenuItemFields::new("root")
            .with_on_select_action(TestAction::Root)
            .into_item(),
    ]
}

fn two_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("first submenu"),
            menu: SubMenu::new(vec![MenuItemFields::new("first child")
                .with_on_select_action(TestAction::ChildOne)
                .into_item()]),
        },
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("second submenu"),
            menu: SubMenu::new(vec![MenuItemFields::new("second child")
                .with_on_select_action(TestAction::ChildTwo)
                .into_item()]),
        },
    ]
}

fn split_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![MenuItem::Submenu {
        fields: MenuItemFields::new_submenu("host")
            .with_on_select_action(TestAction::Root)
            .with_split_submenu_trigger("More actions for host"),
        menu: SubMenu::new(vec![
            MenuItemFields::new("child one")
                .with_on_select_action(TestAction::ChildOne)
                .into_item(),
            MenuItemFields::new("child two")
                .with_on_select_action(TestAction::ChildTwo)
                .into_item(),
        ]),
    }]
}

fn primary_disabled_split_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![MenuItem::Submenu {
        fields: MenuItemFields::new_submenu("removed host")
            .with_on_select_action(TestAction::Root)
            .with_split_submenu_trigger("More actions for removed host")
            .with_split_submenu_primary_disabled(true),
        menu: SubMenu::new(vec![MenuItemFields::new("remove")
            .with_on_select_action(TestAction::ChildOne)
            .into_item()]),
    }]
}

#[test]
fn test_menu_item_selectable() {
    assert!(MenuItemFields::<()>::new("normal").into_item().selectable());
    assert!(!MenuItemFields::<()>::new("disabled")
        .with_disabled(true)
        .into_item()
        .selectable());
    assert!(!MenuItem::<()>::Separator.selectable());
}

#[test]
fn test_next_and_previous_indexes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let items = vec![
            MenuItemFields::<()>::new("item1")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item2").into_item(),
            MenuItemFields::<()>::new("item3")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item4").into_item(),
            MenuItemFields::<()>::new("item5").into_item(),
        ];

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<()>::new();
            menu.set_items(items, ctx);
            menu
        });

        menu.update(&mut app, |menu, _ctx| {
            assert!(menu.selected_item().is_none());

            menu.menu
                .select_internal(SelectAction::Index { row: 1, item: 0 });
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Make sure we skip the disabled menu items
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            // Make sure we go around
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Makre sure we go around with Prev action too
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            // Makre sure we skip the disabled ones for previous as well
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );
        });
    })
}

#[test]
fn test_right_opens_selected_submenu_and_selects_first_child() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(0));
            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_up_and_down_navigate_the_active_submenu() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(1));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child two"
            );

            menu.handle_action(&MenuAction::Select(SelectAction::Previous), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_enter_uses_the_active_submenu_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        let mut selected_action = None;
        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            selected_action = menu
                .menu
                .selected_action_for_enter(menu.submenu_position_namespace, ctx);
        });

        assert_eq!(selected_action, Some(TestAction::ChildTwo));
    })
}

#[test]
fn test_split_submenu_enter_runs_primary_action_without_opening_child() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(split_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            let action = menu
                .menu
                .selected_action_for_enter(menu.submenu_position_namespace, ctx);

            assert_eq!(action, Some(TestAction::Root));
            assert!(!menu.menu.has_visible_submenu_at_depth(0));
            assert!(menu
                .menu
                .selected_submenu()
                .unwrap()
                .selected_item()
                .is_none());
        });
    })
}

#[test]
fn test_split_submenu_preserves_identity_width_and_full_text_metadata() {
    let items = split_submenu_items();
    let MenuItem::Submenu { fields, .. } = &items[0] else {
        panic!("expected split submenu");
    };

    assert_eq!(SPLIT_SUBMENU_TRIGGER_WIDTH, 28.);
    assert_eq!(split_submenu_primary_width(200.), 172.);
    assert_eq!(split_submenu_primary_width(20.), 0.);
    assert_eq!(
        menu_item_main_axis_alignment(true),
        MainAxisAlignment::Start
    );
    assert_eq!(
        menu_item_main_axis_alignment(false),
        MainAxisAlignment::SpaceEvenly
    );
    assert!(fields.ellipsizes_label());
    assert_eq!(fields.get_a11y_text(), "host");
}

#[test]
fn test_split_submenu_trigger_has_stable_hover_identity_without_a_chevron() {
    let items = split_submenu_items();
    let MenuItem::Submenu { fields, .. } = &items[0] else {
        panic!("expected split submenu");
    };

    let first_render_fields = fields.split_submenu_trigger_fields().unwrap();
    let second_render_fields = fields.split_submenu_trigger_fields().unwrap();

    assert!(Arc::ptr_eq(
        &first_render_fields.mouse_state,
        &second_render_fields.mouse_state
    ));
    assert!(first_render_fields.has_submenu);
    assert!(!first_render_fields.render_submenu_chevron);
    assert_eq!(
        first_render_fields.vertical_padding_override,
        Some(MENU_ITEM_VERTICAL_PADDING)
    );
    assert_eq!(first_render_fields.horizontal_padding_override, Some(0.));
    assert_eq!(first_render_fields.tooltip(), Some("More actions for host"));

    let standard_submenu = MenuItemFields::<TestAction>::new_submenu("standard submenu");
    assert!(standard_submenu.render_submenu_chevron);
}

#[test]
fn test_primary_disabled_split_submenu_keeps_child_keyboard_accessible() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(primary_disabled_split_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            let MenuItem::Submenu { fields, .. } = &menu.items()[0] else {
                panic!("expected split submenu");
            };
            assert!(!fields.is_disabled());
            assert!(fields.is_split_submenu_primary_disabled());
            assert!(menu.items()[0].selectable());

            menu.set_selected_by_index(0, ctx);
            assert_eq!(
                menu.menu
                    .selected_action_for_enter(menu.submenu_position_namespace, ctx),
                None
            );

            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            assert!(menu.menu.last_open_submenu_succeeded);
            assert_eq!(
                menu.menu.selected_submenu().unwrap().selected_index(),
                Some(0)
            );
        });
    })
}

#[test]
fn test_split_submenu_accessibility_tracks_primary_trigger_and_nested_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(split_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            assert_eq!(menu.menu.selected_accessibility_label(), "host Selected");

            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 1 },
                    position: Default::default(),
                },
                ctx,
            );
            assert_eq!(menu.menu.selected_accessibility_label(), "host Expanded");

            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            assert_eq!(
                menu.menu.selected_accessibility_label(),
                "child one Selected"
            );

            menu.handle_action(&MenuAction::Escape, ctx);
            assert_eq!(menu.menu.escape_accessibility_label(), "Submenu Closed");
            assert_eq!(menu.menu.selected_accessibility_label(), "host Selected");
        });
    })
}

#[test]
fn test_split_submenu_right_opens_child_and_escape_returns_to_parent() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(split_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert!(menu.menu.has_visible_submenu_at_depth(0));
            assert_eq!(
                menu.menu.selected_submenu().unwrap().selected_index(),
                Some(0)
            );

            menu.handle_action(&MenuAction::Escape, ctx);

            assert_eq!(menu.selected_index(), Some(0));
            assert_eq!(menu.menu.selected_item_index, Some(0));
            assert!(!menu.menu.has_visible_submenu_at_depth(0));
            assert!(menu
                .menu
                .selected_submenu()
                .unwrap()
                .selected_item()
                .is_none());
        });
    })
}

#[test]
fn test_split_submenu_trailing_target_opens_without_selecting_primary_action() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(split_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 1 },
                    position: Default::default(),
                },
                ctx,
            );

            assert_eq!(menu.selected_index(), Some(0));
            assert_eq!(menu.menu.selected_item_index, Some(1));
            assert!(menu.menu.has_visible_submenu_at_depth(0));
            assert!(menu
                .menu
                .selected_submenu()
                .unwrap()
                .selected_item()
                .is_none());

            let action = menu
                .menu
                .selected_action_for_enter(menu.submenu_position_namespace, ctx);
            assert_eq!(action, None);
            assert_eq!(
                menu.menu.selected_submenu().unwrap().selected_index(),
                Some(0)
            );
        });
    })
}

#[test]
fn test_submenu_edge_placement_reverses_only_when_right_side_overflows() {
    assert!(!should_reverse_submenu_layout(100., 800., 200., 2));
    assert!(should_reverse_submenu_layout(550., 800., 200., 2));
    assert!(!should_reverse_submenu_layout(790., 800., 200., 1));
}

#[test]
fn test_right_is_a_noop_for_leaf_items() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(1, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(1));
            assert!(!menu.menu.last_open_submenu_succeeded);
            assert!(matches!(
                menu.action_accessibility_contents(&MenuAction::OpenSubmenu, ctx),
                ActionAccessibilityContent::Empty
            ));
            assert!(menu.menu.selected_submenu().is_none());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "root"
            );
        });
    })
}

#[test]
fn menu_accessibility_copy_uses_fluent_keys() {
    let source = include_str!("menu.rs");
    for literal in [
        "{item} Selected",
        "Submenu Expanded",
        "Submenu Closed",
        "Menu Closed",
        "Action Selected",
        "Press the right key",
    ] {
        assert!(!source.contains(literal), "hard-coded a11y text: {literal}");
    }
    for key in [
        "menu-a11y-item-selected",
        "menu-a11y-submenu-expanded-label",
        "menu-a11y-select-instructions",
        "menu-a11y-select-submenu-instructions",
        "menu-a11y-submenu-expanded",
        "menu-a11y-open-submenu-instructions",
        "menu-a11y-submenu-closed",
        "menu-a11y-close-submenu-instructions",
        "menu-a11y-submenu-escape-instructions",
        "menu-a11y-menu-closed",
        "menu-a11y-menu-escape-instructions",
        "menu-a11y-action-selected",
        "menu-a11y-action-instructions",
    ] {
        assert!(source.contains(key), "missing Fluent use: {key}");
    }
}

#[test]
fn test_stale_submenu_parent_unhover_does_not_clear_new_hover_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(two_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 1, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(&MenuAction::UnhoverSubmenuParent(0, 0), ctx);

            assert_eq!(menu.selected_index(), Some(1));
            assert_eq!(
                menu.selected_item()
                    .unwrap()
                    .submenu_fields()
                    .unwrap()
                    .label(),
                "second submenu"
            );
        });
    })
}

#[test]
fn test_submenu_parent_unhover_keeps_submenu_open() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(&MenuAction::UnhoverSubmenuParent(0, 0), ctx);

            assert_eq!(menu.selected_index(), Some(0));
            assert!(menu.menu.selected_submenu().is_some());
        });
    })
}

#[test]
fn test_leaf_hover_clears_previously_selected_submenu_parent() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            assert!(menu.menu.selected_submenu().is_some());

            menu.handle_action(
                &MenuAction::HoverSubmenuLeafNode {
                    depth: 0,
                    row_index: 1,
                    position: Default::default(),
                    select: true,
                },
                ctx,
            );

            assert_eq!(menu.selected_index(), Some(1));
            assert!(menu.menu.selected_submenu().is_none());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "root"
            );
        });
    })
}

#[test]
fn test_leaf_mouse_in_tracking_does_not_clear_open_submenu() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(
                &MenuAction::HoverSubmenuLeafNode {
                    depth: 0,
                    row_index: 1,
                    position: Default::default(),
                    select: false,
                },
                ctx,
            );

            assert_eq!(menu.selected_index(), Some(0));
            assert!(menu.menu.selected_submenu().is_some());
        });
    })
}

fn two_submenu_context_menu_items() -> Vec<MenuItem<TestAction>> {
    vec![
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("upload"),
            menu: SubMenu::new(vec![MenuItemFields::new("upload file")
                .with_on_select_action(TestAction::ChildOne)
                .into_item()]),
        },
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("other"),
            menu: SubMenu::new(vec![
                MenuItemFields::new("rename")
                    .with_on_select_action(TestAction::ChildOne)
                    .into_item(),
                MenuItemFields::new("copy name")
                    .with_on_select_action(TestAction::ChildTwo)
                    .into_item(),
            ]),
        },
    ]
}

#[test]
fn test_switching_submenu_parent_clears_nested_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(two_submenu_context_menu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 1, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(
                &MenuAction::HoverSubmenuLeafNode {
                    depth: 1,
                    row_index: 1,
                    position: Default::default(),
                    select: true,
                },
                ctx,
            );
            assert_eq!(
                menu.menu
                    .selected_submenu()
                    .and_then(|submenu| submenu.selected_index()),
                Some(1)
            );

            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 1, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );

            assert_eq!(
                menu.menu
                    .selected_submenu()
                    .and_then(|submenu| submenu.selected_index()),
                None
            );
        });
    })
}

#[test]
fn test_nested_submenu_leaf_hover_is_handled_at_child_depth() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.handle_action(
                &MenuAction::HoverSubmenuWithChildren {
                    depth: 0,
                    selection: SelectAction::Index { row: 0, item: 0 },
                    position: Default::default(),
                },
                ctx,
            );
            assert!(menu.menu.selected_submenu().is_some());

            menu.handle_action(
                &MenuAction::HoverSubmenuLeafNode {
                    depth: 1,
                    row_index: 0,
                    position: Default::default(),
                    select: true,
                },
                ctx,
            );

            assert_eq!(menu.selected_index(), Some(0));
            assert_eq!(
                menu.menu
                    .selected_submenu()
                    .and_then(|submenu| submenu.selected_index()),
                Some(0)
            );
            let child_label = menu
                .menu
                .selected_submenu()
                .and_then(|submenu| submenu.selected_item())
                .and_then(|item| item.fields().map(|fields| fields.label().to_string()));
            assert_eq!(child_label.as_deref(), Some("child one"));
        });
    })
}

#[test]
fn test_submenu_position_ids_are_scoped_per_menu_instance() {
    let first_menu = Menu::<()>::new();
    let second_menu = Menu::<()>::new();

    assert_ne!(
        first_menu.submenu_save_position_id_for_tests(0, 0),
        second_menu.submenu_save_position_id_for_tests(0, 0)
    );
}

#[test]
fn test_submenu_position_ids_are_scoped_per_row() {
    let menu = Menu::<()>::new();

    assert_ne!(
        menu.submenu_save_position_id_for_tests(0, 0),
        menu.submenu_save_position_id_for_tests(0, 1)
    );
}
