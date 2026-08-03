use super::*;

#[test]
fn gemini_query_resolves_to_antigravity() {
    assert!(match_agent(CLIAgent::Antigravity, "gemini").is_some());
    assert!(match_agent(CLIAgent::Antigravity, "Antigravity").is_some());
    assert!(match_agent(CLIAgent::Antigravity, "agy").is_some());
    assert!(match_agent(CLIAgent::Grok, "gemini").is_none());
}

#[test]
fn first_class_agent_results_run_their_canonical_commands() {
    for (agent, command) in [(CLIAgent::Antigravity, "agy"), (CLIAgent::Grok, "grok")] {
        let item = CliAgentSearchItem { agent, score: 1.0 };
        assert_eq!(item.priority_tier(), 1);
        match item.accept_result() {
            CommandSearchItemAction::AcceptHistory(accepted) => {
                assert_eq!(accepted.command, command)
            }
            other => panic!("expected an accepted {command} result, got {other:?}"),
        }
        match item.execute_result() {
            CommandSearchItemAction::ExecuteHistory(actual) => {
                assert_eq!(actual, command)
            }
            other => panic!("expected an executable {command} result, got {other:?}"),
        }
    }
}

#[test]
fn grok_is_searchable_by_name_and_command() {
    assert!(match_agent(CLIAgent::Grok, "Grok").is_some());
    assert!(match_agent(CLIAgent::Grok, "gro").is_some());
    assert!(match_agent(CLIAgent::Grok, "agy").is_none());
}
