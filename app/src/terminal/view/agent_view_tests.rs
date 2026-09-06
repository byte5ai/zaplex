use super::*;
use crate::terminal::input::slash_commands::SlashCommandTrigger;

#[test]
fn empty_agent_slash_entry_starts_preflight_without_a_prompt() {
    let request = subscription_entry_preflight(
        AgentViewEntryOrigin::SlashCommand {
            trigger: SlashCommandTrigger::input(),
        },
        true,
        None,
    )
    .expect("/agent must preflight before enabling its composer");

    assert_eq!(request.pending_prompt, None);
}

#[test]
fn prompted_agent_slash_entry_defers_prompt_until_preflight_succeeds() {
    let request = subscription_entry_preflight(
        AgentViewEntryOrigin::SlashCommand {
            trigger: SlashCommandTrigger::input(),
        },
        true,
        Some("show uptime"),
    )
    .expect("/agent with a prompt must preflight before dispatch");

    assert_eq!(request.pending_prompt.as_deref(), Some("show uptime"));
}
