use super::{validate_parsed_servers, ParsedTemplatableMCPServerResult};

fn parse_servers(json: &str) -> Vec<ParsedTemplatableMCPServerResult> {
    ParsedTemplatableMCPServerResult::from_user_json(json).expect("test MCP JSON must parse")
}

#[test]
fn clean_new_server_batch_is_returned_after_validation() {
    let servers = parse_servers(r#"{"clean":{"command":"/usr/bin/true"}}"#);
    let validated = validate_parsed_servers(servers, |_| Ok(()))
        .expect("clean MCP server must pass validation");

    assert_eq!(validated.len(), 1);
    assert_eq!(validated[0].templatable_mcp_server.name, "clean");
}

#[test]
fn secret_detection_rejects_the_entire_new_server_batch() {
    let servers = parse_servers(r#"{"secret":{"command":"/usr/bin/true"}}"#);
    let result = validate_parsed_servers(servers, |_| Err("secret detected".to_string()));

    assert!(result.is_err());
}

#[test]
fn multiple_new_servers_are_all_validated_before_the_batch_is_returned() {
    let servers = parse_servers(
        r#"{
            "clean": {"command":"/usr/bin/true"},
            "secret": {"command":"/usr/bin/false"}
        }"#,
    );
    let mut validated_names = Vec::new();
    let result = validate_parsed_servers(servers, |server| {
        validated_names.push(server.name.clone());
        if server.name == "secret" {
            Err("secret detected".to_string())
        } else {
            Ok(())
        }
    });

    assert!(result.is_err());
    assert_eq!(validated_names, ["clean", "secret"]);
}
