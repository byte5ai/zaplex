use super::*;
use zaplex_cockpit::TaskItem;

#[test]
fn current_task_prefers_in_progress_step() {
    let state = TaskState {
        tasks: vec![
            TaskItem {
                id: "one".into(),
                title: "Finished".into(),
                status: TaskStatus::Completed,
            },
            TaskItem {
                id: "two".into(),
                title: "Current".into(),
                status: TaskStatus::InProgress,
            },
            TaskItem {
                id: "three".into(),
                title: "Later".into(),
                status: TaskStatus::Pending,
            },
        ],
    };

    assert_eq!(current_task_title(&state), Some("Current"));
}

#[test]
fn current_task_falls_back_to_first_pending_step() {
    let state = TaskState {
        tasks: vec![
            TaskItem {
                id: "one".into(),
                title: "Next".into(),
                status: TaskStatus::Pending,
            },
            TaskItem {
                id: "two".into(),
                title: "After".into(),
                status: TaskStatus::Pending,
            },
        ],
    };
    assert_eq!(current_task_title(&state), Some("Next"));
}

#[test]
fn task_activity_uses_the_current_step_without_losing_recency() {
    let state = TaskState {
        tasks: vec![TaskItem {
            id: "one".into(),
            title: "Verify release gates".into(),
            status: TaskStatus::InProgress,
        }],
    };

    assert_eq!(
        task_activity_label(Some(&state), "2m ago"),
        "Verify release gates · 2m ago"
    );
    assert_eq!(task_activity_label(None, "2m ago"), "2m ago");
}

#[test]
fn removed_host_is_marked_and_cannot_attach_agents() {
    let host = HostNode {
        host: "devhost".to_string(),
        is_local: false,
        host_id: Some("daemon-dev".to_string()),
        availability: HostAvailability::Removed,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        // Deliberately retain a stale id in this presentation-level test: the
        // explicit state, not incidental id clearing, must close every route.
        registry_node_id: Some("node-dev".to_string()),
        needs_me: 1,
        projects: Vec::new(),
    };
    assert_eq!(
        host_display_label(
            &host,
            "removed from Connections",
            "Connections registry unavailable"
        ),
        "devhost — removed from Connections"
    );
    assert!(
        !host.is_available(),
        "removed daemon data is visible but cannot seed session click routes"
    );
}

#[test]
fn unverified_host_is_labeled_and_not_routable() {
    let host = HostNode {
        host: "devhost".to_string(),
        is_local: false,
        host_id: Some("daemon-dev".to_string()),
        availability: HostAvailability::Unverified,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        registry_node_id: Some("node-dev".to_string()),
        needs_me: 1,
        projects: Vec::new(),
    };
    assert_eq!(
        host_display_label(
            &host,
            "removed from Connections",
            "Connections registry unavailable"
        ),
        "devhost — Connections registry unavailable"
    );
    assert!(!host.is_available());
}

#[test]
fn expanded_containers_hide_counts() {
    assert_eq!(container_count_presentation(true, 7, 2), None);
}

#[test]
fn collapsed_counts_carry_hidden_attention_only() {
    assert_eq!(
        container_count_presentation(false, 7, 2),
        Some(ContainerCountPresentation {
            count: 7,
            attention: true,
        })
    );
    assert_eq!(
        container_count_presentation(false, 7, 0),
        Some(ContainerCountPresentation {
            count: 7,
            attention: false,
        })
    );
}

#[test]
fn agent_leaf_separates_provider_from_the_exact_optional_model() {
    assert_eq!(
        agent_leaf_presentation(Provider::Claude, "claude-opus-4-8-20260901"),
        AgentLeafPresentation {
            provider: "Claude",
            model: Some("claude-opus-4-8-20260901"),
        }
    );
    assert_eq!(
        agent_leaf_presentation(Provider::Codex, "  "),
        AgentLeafPresentation {
            provider: "Codex",
            model: None,
        }
    );
}

#[test]
fn session_identity_avoids_only_exact_project_directory_duplication() {
    assert_eq!(project_directory_label("zaplex", "/work/zaplex"), "zaplex");
    assert_eq!(
        project_directory_label("zaplex", "/work/Zaplex"),
        "zaplex — Zaplex"
    );
}

#[test]
fn tree_status_is_glyph_only() {
    let cases = [
        (
            SessionState::Waiting,
            crate::t!("cockpit-task-peek-state-waiting"),
        ),
        (
            SessionState::Active,
            crate::t!("cockpit-task-peek-state-working"),
        ),
        (
            SessionState::Monitor,
            crate::t!("cockpit-task-peek-state-working"),
        ),
        (
            SessionState::Idle,
            crate::t!("cockpit-task-peek-state-idle"),
        ),
    ];

    for (state, expected_semantic_label) in cases {
        let presentation = session_glyph_presentation(state);
        assert_eq!(presentation.visible_label, session_glyph(state));
        assert!(
            !presentation.visible_label.chars().any(char::is_alphabetic),
            "the visible tree state must remain glyph-only"
        );
        assert_eq!(presentation.semantic_label, expected_semantic_label);
        assert!(
            !presentation.semantic_label.trim().is_empty(),
            "every state glyph needs a localized semantic description"
        );
    }
}

#[test]
fn waiting_pulse_is_fixed_and_capped_at_twice_the_core() {
    let start = waiting_pulse_frame(Duration::ZERO, true);
    let near_end = waiting_pulse_frame(Duration::from_millis(1599), true);

    assert!(start.repaint);
    assert!(near_end.repaint);
    assert!((88..=100).contains(&start.core_opacity));
    assert!((88..=100).contains(&near_end.core_opacity));
    assert!(near_end.ring_diameter <= WAITING_GLYPH_CORE_DIAMETER * 2.0);
    assert!(near_end.ring_diameter > WAITING_GLYPH_CORE_DIAMETER * 1.99);
    assert_eq!(WAITING_GLYPH_FOOTPRINT, GLYPH_COL_WIDTH);
}

#[test]
fn reduced_motion_uses_static_waiting_emphasis() {
    let frame = waiting_pulse_frame(Duration::from_secs(30), false);

    assert!(!frame.repaint);
    assert_eq!(frame.core_opacity, 100);
    assert_eq!(frame.ring_diameter, WAITING_GLYPH_CORE_DIAMETER * 1.45);
    assert!(frame.ring_opacity > 0);
}

#[test]
fn waiting_glyph_motion_respects_reduced_motion() {
    let animated = waiting_pulse_frame(Duration::from_millis(1599), true);
    assert!(animated.repaint);
    assert!(animated.ring_diameter <= WAITING_GLYPH_CORE_DIAMETER * 2.0);

    let reduced = waiting_pulse_frame(Duration::from_millis(1599), false);
    assert!(!reduced.repaint);
    assert_eq!(reduced.core_opacity, 100);
    assert_eq!(reduced.ring_diameter, WAITING_GLYPH_CORE_DIAMETER * 1.45);
}

#[test]
fn waiting_pulse_repaint_interval_is_battery_bounded() {
    assert!(WAITING_PULSE_REPAINT >= Duration::from_millis(100));
}

#[test]
fn waiting_pulse_animates_only_in_a_focused_window_without_reduced_motion() {
    assert!(waiting_pulse_should_animate(false, true));
    assert!(!waiting_pulse_should_animate(false, false));
    assert!(!waiting_pulse_should_animate(true, true));
}

#[test]
fn account_scan_error_is_not_rendered_as_zero_accounts() {
    assert_eq!(
        account_count_presentation(&zaplex_cockpit::ScanHealth::Pending, 0),
        None
    );
    assert_eq!(
        account_count_presentation(
            &zaplex_cockpit::ScanHealth::Degraded("account source unreadable".into()),
            0,
        ),
        None
    );
    assert_eq!(
        account_count_presentation(&zaplex_cockpit::ScanHealth::Loaded, 0),
        Some(0)
    );
    assert_eq!(
        account_count_presentation(
            &zaplex_cockpit::ScanHealth::Degraded("one source failed".into()),
            1,
        ),
        Some(1)
    );
}

#[test]
fn empty_local_inventory_does_not_hide_scan_failures() {
    crate::i18n::init(Some("en"));
    assert_eq!(
        empty_inventory_message(AgentInventoryStatus::Unavailable, true),
        crate::t!("cockpit-host-inventory-unavailable"),
    );
    assert_ne!(
        empty_inventory_message(AgentInventoryStatus::Unavailable, true),
        empty_inventory_message(AgentInventoryStatus::Ready, true),
    );
    assert_eq!(
        empty_inventory_message(AgentInventoryStatus::Pending, true),
        crate::t!("cockpit-host-inventory-pending"),
    );
}

fn tree_agent(
    session_id: &str,
    cwd: &str,
    branch: Option<&str>,
    state: SessionState,
) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session_id.into(),
        cwd: cwd.into(),
        name: String::new(),
        state,
        provider: Provider::Claude,
        model: "claude-opus-5-5".into(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/work/proj".into(),
        repo_root: "/work/proj".into(),
        project_name: "proj".into(),
        branch: branch.map(str::to_string),
        worktree: None,
        config_dir: None,
        account_email: None,
        account_id: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: chrono::Utc::now(),
        pid: 0,
        awaiting_input: false,
        turn_id: None,
        attention: None,
    }
}

fn in_one_pty(mut agent: SessionSnapshot) -> SessionSnapshot {
    agent.pty_session_id = Some("pty-1".into());
    agent.pty_session_generation = Some(1);
    agent
}

/// `(depth, kind, label, focused)` of one projected row.
fn row_summary(row: &TreeRow<'_>) -> (usize, &'static str, String, bool) {
    match &row.kind {
        TreeRowKind::Project { label, focused, .. } => {
            (row.depth, "project", label.full.clone(), *focused)
        }
        TreeRowKind::SessionLeaf {
            label,
            agent,
            focused,
        } => (
            row.depth,
            "leaf",
            label
                .as_ref()
                .map_or_else(|| format!("agent:{}", agent.session_id), |l| l.full.clone()),
            *focused,
        ),
        TreeRowKind::SessionContainer { label, focused, .. } => (
            row.depth,
            "session",
            label.as_ref().map_or_else(String::new, |l| l.full.clone()),
            *focused,
        ),
        TreeRowKind::Agent { agent, focused } => {
            (row.depth, "agent", agent.session_id.clone(), *focused)
        }
    }
}

fn local_project_rows(
    project_name: &str,
    agents: &[SessionSnapshot],
    session_expanded: bool,
    focused_id: Option<&str>,
) -> Vec<(usize, &'static str, String, bool)> {
    let rows = project_tree_rows(
        project_name,
        group_project_sessions(false, Some("host-a"), agents),
        "host-a\u{1f}/work/proj".to_string(),
        true,
        |_| session_expanded,
        |agent: &SessionSnapshot| Some(agent.session_id.as_str()) == focused_id,
    );
    rows.iter().map(row_summary).collect()
}

#[test]
fn single_untitled_session_merges_into_project_row() {
    let agents = [tree_agent("a", "/work/proj", None, SessionState::Active)];
    assert_eq!(
        local_project_rows("proj", &agents, true, None),
        vec![(1, "leaf", "proj".to_string(), false)],
        "one untitled session with one agent is a single project row"
    );
}

#[test]
fn tree_never_repeats_parent_label_in_child_row() {
    let agents = [
        tree_agent("a", "/work/proj", None, SessionState::Active),
        tree_agent("b", "/work/proj", None, SessionState::Idle),
    ];
    let rows = local_project_rows("proj", &agents, true, None);
    assert_eq!(rows[0], (1, "project", "proj".to_string(), false));
    for (_, kind, label, _) in &rows[1..] {
        assert_eq!(*kind, "leaf");
        assert_ne!(label, "proj", "a child row never repeats its project label");
    }
    assert_eq!(rows.len(), 3);
}

#[test]
fn multiple_agents_in_one_pty_render_child_rows() {
    let agents = [
        in_one_pty(tree_agent(
            "a",
            "/work/proj",
            Some("main"),
            SessionState::Active,
        )),
        in_one_pty(tree_agent(
            "b",
            "/work/proj",
            Some("main"),
            SessionState::Idle,
        )),
        tree_agent("c", "/work/proj", Some("feat/x"), SessionState::Idle),
    ];
    let rows = local_project_rows("proj", &agents, true, None);
    assert_eq!(rows[0], (1, "project", "proj".to_string(), false));
    let container = rows
        .iter()
        .position(|row| row.1 == "session")
        .expect("the multi-agent PTY keeps its own row");
    assert_eq!(rows[container], (2, "session", "main".to_string(), false));
    assert_eq!(
        rows.iter()
            .filter(|row| row.0 == 3 && row.1 == "agent")
            .count(),
        2
    );

    let collapsed = local_project_rows("proj", &agents, false, None);
    assert!(
        collapsed.iter().all(|row| row.1 != "agent"),
        "a collapsed multi-agent session hides its agent rows"
    );

    // A collapsed session hiding the focused agent carries the highlight
    // itself; expanded, only the agent row does.
    let collapsed_focused = local_project_rows("proj", &agents, false, Some("b"));
    assert!(collapsed_focused
        .iter()
        .any(|row| row.1 == "session" && row.3));
    let expanded_focused = local_project_rows("proj", &agents, true, Some("b"));
    let marked: Vec<_> = expanded_focused.iter().filter(|row| row.3).collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].1, "agent");
}

#[test]
fn single_multi_agent_session_merges_into_project_row() {
    let agents = [
        in_one_pty(tree_agent(
            "a",
            "/work/proj",
            Some("main"),
            SessionState::Active,
        )),
        in_one_pty(tree_agent(
            "b",
            "/work/proj",
            Some("main"),
            SessionState::Idle,
        )),
    ];
    let rows = local_project_rows("proj", &agents, true, None);
    assert_eq!(rows[0], (1, "project", "proj / main".to_string(), false));
    assert_eq!(rows.len(), 3);
    assert!(rows[1..].iter().all(|row| row.0 == 2 && row.1 == "agent"));
}

#[test]
fn single_titled_session_merges_with_dimmed_title() {
    let agents = [tree_agent(
        "a",
        "/work/proj",
        Some("main"),
        SessionState::Active,
    )];
    assert_eq!(
        local_project_rows("proj", &agents, true, None),
        vec![(1, "leaf", "proj / main".to_string(), false)]
    );
    let label = merged_label("proj", Some("main"));
    assert_eq!(label.parts.len(), 2);
    assert_eq!(
        (
            label.parts[0].text.as_str(),
            label.parts[0].tone,
            label.parts[0].fit
        ),
        ("proj", PartTone::Title, PartFit::Holds),
        "the project name leads and keeps the larger share"
    );
    assert_eq!(
        (
            label.parts[1].text.as_str(),
            label.parts[1].tone,
            label.parts[1].gap_before
        ),
        ("main", PartTone::Dim, true),
        "the session title follows dimmed after air, without a glyph"
    );
}

#[test]
fn long_titles_shorten_in_the_middle_and_keep_their_end() {
    let label = TreeLabel::plain("feat/checkout-redesign-step-2");
    let parts: Vec<_> = label
        .parts
        .iter()
        .map(|part| (part.text.as_str(), part.fit))
        .collect();
    assert_eq!(
        parts,
        vec![
            ("feat/checkout-redesign", PartFit::Shrinks),
            ("-step-2", PartFit::Fixed),
        ]
    );

    // Short titles, and long ones without a short last segment, end-clip.
    assert_eq!(TreeLabel::plain("main").parts.len(), 1);
    assert_eq!(
        TreeLabel::plain("averyveryverylongbranchnamewithoutseparators")
            .parts
            .len(),
        1
    );
}

#[test]
fn single_agent_sessions_are_leaves_without_agent_rows() {
    let agents = [
        tree_agent("a", "/work/proj", Some("main"), SessionState::Active),
        tree_agent("b", "/work/proj", Some("feat/x"), SessionState::Idle),
    ];
    let rows = local_project_rows("proj", &agents, true, None);
    assert_eq!(rows.len(), 3);
    assert!(rows[1..].iter().all(|row| row.0 == 2 && row.1 == "leaf"));
}

#[test]
fn similar_session_names_show_distinguishing_suffix() {
    let titles = [
        "vault-curator-inbox-2026-10-06-0300",
        "vault-curator-inbox-2026-10-06-0900",
        "vault-curator-inbox-2026-10-06-1500",
    ];
    let cut = shared_prefix_cut(&titles);
    let label = split_title(titles[0], cut);
    let parts: Vec<_> = label
        .parts
        .iter()
        .map(|part| (part.text.as_str(), part.tone, part.fit))
        .collect();
    assert_eq!(
        parts,
        vec![
            ("vault-curator-…", PartTone::Dim, PartFit::Shrinks),
            ("10-06-0300", PartTone::Title, PartFit::Fixed),
        ]
    );
    assert_eq!(label.full, titles[0]);
    for title in titles {
        let tail = split_title(title, cut).parts.last().unwrap().text.clone();
        assert!(tail.chars().count() >= MIN_DISTINCT_TAIL_CHARS);
    }

    // Short or unrelated names keep their whole title.
    assert_eq!(shared_prefix_cut(&["feat/a", "feat/b"]), None);
    assert_eq!(shared_prefix_cut(&["main", "main"]), None);
    assert_eq!(shared_prefix_cut(&["main", "release"]), None);
    assert_eq!(shared_prefix_cut(&["only-one-title-here"]), None);
}

#[test]
fn idle_session_title_uses_muted_role() {
    assert_eq!(title_tone(SessionState::Idle), TitleTone::Quiet);
    assert_eq!(title_tone(SessionState::Active), TitleTone::Active);
    assert_eq!(title_tone(SessionState::Monitor), TitleTone::Active);
    assert_eq!(title_tone(SessionState::Waiting), TitleTone::Active);
}

#[test]
fn focused_session_row_has_stable_highlight() {
    let agents = [
        tree_agent("a", "/work/proj", Some("main"), SessionState::Active),
        tree_agent("b", "/work/proj", Some("feat/x"), SessionState::Idle),
    ];
    let rows = local_project_rows("proj", &agents, true, Some("b"));
    let focused: Vec<_> = rows.iter().filter(|row| row.3).collect();
    assert_eq!(
        focused.len(),
        1,
        "exactly the focused pane's session is marked"
    );
    assert_eq!(focused[0].2, "feat/x");
    assert!(local_project_rows("proj", &agents, true, None)
        .iter()
        .all(|row| !row.3));
}

#[test]
fn prefix_dimming_ignores_unrelated_siblings() {
    let titles = [
        "main",
        "vault-curator-inbox-2026-10-06-0300",
        "vault-curator-inbox-2026-10-06-0900",
    ];
    let cuts = shared_prefix_cuts(&titles);
    assert_eq!(
        cuts[0], None,
        "a title without a similar sibling stays whole"
    );
    assert_eq!(
        split_title(titles[1], cuts[1]).parts.last().unwrap().text,
        "10-06-0300"
    );
    assert_eq!(
        split_title(titles[2], cuts[2]).parts.last().unwrap().text,
        "10-06-0900"
    );
}

#[test]
fn directory_title_never_repeats_the_project_name() {
    let agents = [tree_agent(
        "a",
        "/work/proj/tools",
        None,
        SessionState::Active,
    )];
    assert_eq!(
        local_project_rows("proj", &agents, true, None),
        vec![(1, "leaf", "proj / tools".to_string(), false)],
        "a distinct directory is the session title on its own"
    );
}
