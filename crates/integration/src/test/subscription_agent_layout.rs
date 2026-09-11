use std::time::Duration;

use pathfinder_geometry::{rect::RectF, vector::vec2f};
use warp::integration_testing::{
    agent_mode::{
        enter_agent_view, set_subscription_agent_layout_state, subscription_agent_layout_snapshot,
        SubscriptionAgentLayoutSnapshot, SubscriptionAgentLayoutState,
    },
    step::new_step_with_default_assertions,
    terminal::wait_until_bootstrapped_single_pane_for_tab,
};
use warpui::{integration::AssertionOutcome, windowing::WindowManager, SingletonEntity as _};

use super::new_builder;
use crate::Builder;

const NORMAL_WIDTH: f32 = 1180.;
const NORMAL_HEIGHT: f32 = 640.;
const NARROW_WIDTH: f32 = 540.;
const NARROW_HEIGHT: f32 = 640.;
const GEOMETRY_EPSILON: f32 = 1.;

pub fn test_subscription_agent_conversation_layout_evidence() -> Builder {
    let mut builder = new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(enter_agent_view());

    for (width_name, width, height, screenshot_state, screenshot_name) in [
        (
            "normal",
            NORMAL_WIDTH,
            NORMAL_HEIGHT,
            SubscriptionAgentLayoutState::WaitingForApproval,
            "subscription-agent-normal-1180x640.png",
        ),
        (
            "narrow",
            NARROW_WIDTH,
            NARROW_HEIGHT,
            SubscriptionAgentLayoutState::SessionEnded,
            "subscription-agent-narrow-540x640.png",
        ),
    ] {
        builder = builder.with_step(resize_window(width_name, width, height));
        for state in SubscriptionAgentLayoutState::ALL {
            builder = builder.with_step(assert_layout_state(
                width_name,
                state,
                (state == screenshot_state).then_some(screenshot_name),
            ));
        }
    }

    builder
}

fn resize_window(
    width_name: &'static str,
    width: f32,
    height: f32,
) -> warpui::integration::TestStep {
    new_step_with_default_assertions(&format!("Resize Agent conversation to {width_name} width"))
        .with_action(move |app, window_id, _| {
            let origin = app
                .window_bounds(&window_id)
                .expect("Agent conversation window should have bounds")
                .origin();
            app.read(|ctx| {
                WindowManager::as_ref(ctx)
                    .set_window_bounds(window_id, RectF::new(origin, vec2f(width, height)));
            });
        })
        .add_named_assertion(
            "Window reaches the requested size",
            move |app, window_id| {
                let Some(bounds) = app.window_bounds(&window_id) else {
                    return AssertionOutcome::failure(
                        "Agent conversation window has no bounds".into(),
                    );
                };
                if (bounds.width() - width).abs() > GEOMETRY_EPSILON
                    || (bounds.height() - height).abs() > GEOMETRY_EPSILON
                {
                    return AssertionOutcome::failure(format!(
                        "Expected {width}x{height} window, got {}x{}",
                        bounds.width(),
                        bounds.height()
                    ));
                }
                AssertionOutcome::Success
            },
        )
}

fn assert_layout_state(
    width_name: &'static str,
    state: SubscriptionAgentLayoutState,
    screenshot_name: Option<&'static str>,
) -> warpui::integration::TestStep {
    let mut step = new_step_with_default_assertions(&format!(
        "Validate {width_name} Agent layout in {}",
        state.name()
    ))
    .with_action(move |app, window_id, _| {
        set_subscription_agent_layout_state(app, window_id, state);
    })
    .add_named_assertion(
        "Identity, composer, and primary actions remain reachable",
        move |app, window_id| {
            let Some(snapshot) = subscription_agent_layout_snapshot(app, window_id) else {
                return AssertionOutcome::failure(
                    "Agent conversation layout positions are not available yet".into(),
                );
            };
            match validate_layout_snapshot(width_name, state, &snapshot) {
                Ok(()) => AssertionOutcome::Success,
                Err(message) => AssertionOutcome::failure(message),
            }
        },
    );
    if let Some(screenshot_name) = screenshot_name {
        step = step
            .set_post_step_pause(Duration::from_millis(250))
            .with_take_screenshot(screenshot_name);
    }
    step
}

fn validate_layout_snapshot(
    width_name: &str,
    state: SubscriptionAgentLayoutState,
    snapshot: &SubscriptionAgentLayoutSnapshot,
) -> Result<(), String> {
    let input = required_bounds(width_name, state, "input", snapshot.input)?;
    let composer = required_bounds(width_name, state, "composer", snapshot.composer)?;
    let footer = required_bounds(width_name, state, "footer", snapshot.footer)?;
    let identity = required_bounds(width_name, state, "identity", snapshot.identity)?;
    let viewport = RectF::new(vec2f(0., 0.), snapshot.window_size);

    for (name, bounds) in [
        ("input", input),
        ("composer", composer),
        ("footer", footer),
        ("identity", identity),
    ] {
        if !has_area(bounds) {
            return Err(format!(
                "{width_name}/{}: {name} has empty bounds {bounds:?}",
                state.name()
            ));
        }
        if !contains(viewport, bounds) {
            return Err(format!(
                "{width_name}/{}: {name} escapes viewport {viewport:?}: {bounds:?}",
                state.name()
            ));
        }
    }

    if !contains(input, composer) || !contains(input, footer) || !contains(footer, identity) {
        return Err(format!(
            "{width_name}/{}: composer/footer/identity left their native containers",
            state.name()
        ));
    }
    if overlaps(composer, footer) {
        return Err(format!(
            "{width_name}/{}: composer overlaps footer",
            state.name()
        ));
    }

    let mut expected_actions = state.expected_actions().to_vec();
    expected_actions.sort_unstable();
    let mut actual_actions = snapshot
        .actions
        .iter()
        .map(|action| action.name)
        .collect::<Vec<_>>();
    actual_actions.sort_unstable();
    if actual_actions != expected_actions {
        return Err(format!(
            "{width_name}/{}: expected actions {expected_actions:?}, found {actual_actions:?}",
            state.name()
        ));
    }

    for action in &snapshot.actions {
        if !has_area(action.bounds) || !contains(footer, action.bounds) {
            return Err(format!(
                "{width_name}/{}: action {} is not reachable inside footer: {:?}",
                state.name(),
                action.name,
                action.bounds
            ));
        }
        if overlaps(identity, action.bounds) {
            return Err(format!(
                "{width_name}/{}: identity overlaps action {}",
                state.name(),
                action.name
            ));
        }
    }
    for (index, action) in snapshot.actions.iter().enumerate() {
        for other in &snapshot.actions[index + 1..] {
            if overlaps(action.bounds, other.bounds) {
                return Err(format!(
                    "{width_name}/{}: actions {} and {} overlap",
                    state.name(),
                    action.name,
                    other.name
                ));
            }
        }
    }

    Ok(())
}

fn required_bounds(
    width_name: &str,
    state: SubscriptionAgentLayoutState,
    region: &str,
    bounds: Option<RectF>,
) -> Result<RectF, String> {
    bounds.ok_or_else(|| {
        format!(
            "{width_name}/{}: missing saved position for {region}",
            state.name()
        )
    })
}

fn has_area(bounds: RectF) -> bool {
    bounds.width() > 0. && bounds.height() > 0.
}

fn contains(outer: RectF, inner: RectF) -> bool {
    inner.min_x() >= outer.min_x() - GEOMETRY_EPSILON
        && inner.min_y() >= outer.min_y() - GEOMETRY_EPSILON
        && inner.max_x() <= outer.max_x() + GEOMETRY_EPSILON
        && inner.max_y() <= outer.max_y() + GEOMETRY_EPSILON
}

fn overlaps(left: RectF, right: RectF) -> bool {
    left.max_x().min(right.max_x()) - left.min_x().max(right.min_x()) > GEOMETRY_EPSILON
        && left.max_y().min(right.max_y()) - left.min_y().max(right.min_y()) > GEOMETRY_EPSILON
}
