use super::plugin_manager_for;
use crate::terminal::CLIAgent;

#[test]
fn returns_manager_for_claude() {
    assert!(plugin_manager_for(CLIAgent::Claude).is_some());
}

#[test]
fn returns_manager_for_opencode() {
    let _oc_guard = crate::features::FeatureFlag::OpenCodeNotifications.override_enabled(true);
    let _hoa_guard = crate::features::FeatureFlag::HOANotifications.override_enabled(true);
    assert!(plugin_manager_for(CLIAgent::OpenCode).is_some());
}

#[test]
fn returns_manager_for_codex() {
    let _codex_guard = crate::features::FeatureFlag::CodexNotifications.override_enabled(true);
    let _hoa_guard = crate::features::FeatureFlag::HOANotifications.override_enabled(true);
    assert!(plugin_manager_for(CLIAgent::Codex).is_some());
}

#[test]
fn returns_manager_for_gemini() {
    let _gemini_guard = crate::features::FeatureFlag::GeminiNotifications.override_enabled(true);
    let _hoa_guard = crate::features::FeatureFlag::HOANotifications.override_enabled(true);
    assert!(plugin_manager_for(CLIAgent::Gemini).is_some());
}

#[test]
fn returns_none_for_unsupported_agents() {
    assert!(plugin_manager_for(CLIAgent::Amp).is_none());
    assert!(plugin_manager_for(CLIAgent::Droid).is_none());
    assert!(plugin_manager_for(CLIAgent::Copilot).is_none());
    assert!(plugin_manager_for(CLIAgent::Unknown).is_none());
}
