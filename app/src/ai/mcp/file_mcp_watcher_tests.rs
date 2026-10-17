use super::{parse_mcp_config_file, MCPProvider};
use std::env;

fn cleanup_env_vars(vars: &[&str]) {
    for var in vars {
        env::remove_var(var);
    }
}

#[test]
fn invalid_config_is_not_reported_as_an_empty_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join(".mcp.json");
    std::fs::write(&config_path, "{ incomplete").unwrap();

    let result =
        futures_lite::future::block_on(parse_mcp_config_file(&config_path, MCPProvider::Zaplex));

    assert!(result.is_err());
}
