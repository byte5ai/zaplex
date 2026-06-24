//! Prompt cache serialization stability test suite (corresponds to documentation P1-8 / P1-9 / P1-13).
//!
//! Anthropic documentation explicitly warns:
//! > Verify that the keys in your `tool_use` content blocks have stable
//! > ordering as some languages (for example, Swift, Go) randomize key order
//! > during JSON conversion, breaking caches
//!
//! This means any `serde_json::Value` output on the Rust side **must**:
//!   1. Be byte-equal across calls with the same input (deterministic)
//!   2. Not depend on `HashMap` iteration order
//!   3. Not depend on external state (timestamps, randomness, PID, etc.)
//!
//! This test suite is Zap's "anti-regression safeguard"—any future changes to the prompt
//! construction path that break byte-level stability will cause assertions here to fail.

use crate::ai::agent::{MCPContext, MCPServer};
use api::message;
use warp_multi_agent_api as api;

use super::chat_stream;
use super::tools;

// ---------------------------------------------------------------------------
// P1-8: tool schema field order stability
// ---------------------------------------------------------------------------

/// Call `(parameters)()` twice for each tool in `REGISTRY` and assert byte-equality.
///
/// Risk point: If embedded enums/oneofs within tool schemas use `HashMap<String, Schema>`
/// to produce a Value, the order becomes unstable. The `serde_json::Map` produced by
/// `json!({...})` literals preserves **insertion order** by default (`preserve_order` is
/// enabled by default in Cargo.toml), so the literal key order stays stable across calls.
/// This test guards that invariant.
#[test]
fn registry_tool_schemas_are_deterministic() {
    for tool in tools::REGISTRY {
        let s1 = (tool.parameters)();
        let s2 = (tool.parameters)();
        let j1 = serde_json::to_string(&s1).unwrap();
        let j2 = serde_json::to_string(&s2).unwrap();
        assert_eq!(
            j1, j2,
            "tool `{}` schema must be byte-equal across calls (prerequisite for prompt cache hits)",
            tool.name
        );
    }
}

/// Call each tool in `REGISTRY` 50 times repeatedly and assert all calls produce byte-equal output.
/// Prevents accidental HashMap iteration order drift (running only twice might coincidentally match).
#[test]
fn registry_tool_schemas_stable_under_repetition() {
    for tool in tools::REGISTRY {
        let baseline = serde_json::to_string(&(tool.parameters)()).unwrap();
        for i in 0..50 {
            let candidate = serde_json::to_string(&(tool.parameters)()).unwrap();
            assert_eq!(
                baseline, candidate,
                "tool `{}` call {i} output differs from baseline (possible HashMap order drift)",
                tool.name
            );
        }
    }
}

/// `tools::REGISTRY` has static order, but verify once: iterating multiple times within
/// the same process yields the same (name, description) sequence.
#[test]
fn registry_iteration_order_is_stable() {
    let names1: Vec<&str> = tools::REGISTRY.iter().map(|t| t.name).collect();
    let names2: Vec<&str> = tools::REGISTRY.iter().map(|t| t.name).collect();
    assert_eq!(names1, names2);
}

// ---------------------------------------------------------------------------
// P1-9: serialize_outgoing_tool_call history replay stability
// ---------------------------------------------------------------------------

/// Simulate a Grep tool call and verify that two serializations produce byte-equal output.
/// `serialize_outgoing_tool_call` reruns on each build_chat_request call,
/// converting prior-round ToolCalls to (name, args Value). Any HashMap or time-related
/// instability would invalidate the cache for the second half of the messages segment.
///
/// Grep was chosen because it has the simplest fields (`queries: Vec<String>`, `path: String`),
/// with no dependence on any implicit prost default fields.
#[test]
fn serialize_grep_tool_call_is_deterministic() {
    let tc = message::ToolCall {
        tool_call_id: "call-grep-1".to_owned(),
        tool: Some(message::tool_call::Tool::Grep(message::tool_call::Grep {
            queries: vec!["fn main".to_owned(), "Result<".to_owned()],
            path: "src/".to_owned(),
        })),
    };

    let (n1, v1) = chat_stream::serialize_outgoing_tool_call_for_test(&tc, None, "");
    let (n2, v2) = chat_stream::serialize_outgoing_tool_call_for_test(&tc, None, "");
    assert_eq!(n1, n2, "tool name must be consistent");
    let j1 = serde_json::to_string(&v1).unwrap();
    let j2 = serde_json::to_string(&v2).unwrap();
    assert_eq!(j1, j2, "same ToolCall must be byte-equal across serializations");
}

/// Grep `queries` is `Vec<String>`, order must be stable (Vec is inherently stable, but this is a defensive assertion).
/// This reflects a broader rule: any Vec field within a user ToolCall must preserve input parameter order.
#[test]
fn serialize_grep_preserves_queries_order() {
    let tc = message::ToolCall {
        tool_call_id: "call-grep-2".to_owned(),
        tool: Some(message::tool_call::Tool::Grep(message::tool_call::Grep {
            queries: vec!["zzz".to_owned(), "aaa".to_owned()],
            path: ".".to_owned(),
        })),
    };
    let (_, v) = chat_stream::serialize_outgoing_tool_call_for_test(&tc, None, "");
    let s = serde_json::to_string(&v).unwrap();
    let pos_z = s.find("zzz").expect("queries should contain zzz");
    let pos_a = s.find("aaa").expect("queries should contain aaa");
    assert!(pos_z < pos_a, "Vec order must follow input parameter order (zzz before aaa)");
}

/// MCP tool calls contain `prost_types::Struct`; verify serialization stability.
/// `prost_types::Struct.fields` uses `BTreeMap` internally, which is inherently stable;
/// this test confirms that behavior.
#[test]
fn serialize_mcp_tool_call_is_deterministic() {
    use prost_types::{value::Kind, Struct, Value as ProstValue};
    use std::collections::BTreeMap;

    let mut fields = BTreeMap::new();
    fields.insert(
        "key_z".to_owned(),
        ProstValue {
            kind: Some(Kind::StringValue("v_z".to_owned())),
        },
    );
    fields.insert(
        "key_a".to_owned(),
        ProstValue {
            kind: Some(Kind::NumberValue(42.0)),
        },
    );

    let server_id = "srv-uuid-1".to_owned();
    let tc = message::ToolCall {
        tool_call_id: "call-mcp-1".to_owned(),
        tool: Some(message::tool_call::Tool::CallMcpTool(
            message::tool_call::CallMcpTool {
                name: "echo".to_owned(),
                args: Some(Struct { fields }),
                server_id: server_id.clone(),
            },
        )),
    };

    // Construct an mcp_context so sanitize_server_name can look up server name
    let ctx = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![MCPServer {
            id: server_id.clone(),
            name: "my-server".to_owned(),
            description: String::new(),
            resources: vec![],
            tools: vec![],
        }],
    };

    let (n1, v1) = chat_stream::serialize_outgoing_tool_call_for_test(&tc, Some(&ctx), "");
    let (n2, v2) = chat_stream::serialize_outgoing_tool_call_for_test(&tc, Some(&ctx), "");
    assert_eq!(n1, n2);
    let j1 = serde_json::to_string(&v1).unwrap();
    let j2 = serde_json::to_string(&v2).unwrap();
    assert_eq!(j1, j2);
    // BTreeMap should output keys in lexicographic order (key_a before key_z)
    let pos_a = j1.find("key_a").expect("should contain key_a");
    let pos_z = j1.find("key_z").expect("should contain key_z");
    assert!(
        pos_a < pos_z,
        "prost_types::Struct should follow BTreeMap lexicographic key order"
    );
}

// ---------------------------------------------------------------------------
// Issue #245: carrier with invalid JSON args must fall back to empty object
// ---------------------------------------------------------------------------

/// When the model emits a tool call with invalid JSON escape sequences (e.g. `\e`, `\``),
/// the carrier message stores the raw string in server_message_data.
/// On the next turn, serialize_outgoing_tool_call must return a Value::Object (not
/// Value::String) so that genai serializes it as a JSON object for `arguments`,
/// not a doubly-wrapped JSON string that the provider would reject with "Invalid \escape".
#[test]
fn carrier_with_invalid_json_args_falls_back_to_empty_object() {
    use api::message;
    // Simulate a carrier message: tool = None, server_message_data = "fn_name\n<invalid_json>"
    let tc = message::ToolCall {
        tool_call_id: "call-invalid".to_owned(),
        tool: None,
    };
    let server_message_data = "shell\n{\"command\": \"echo \\epath\"}";

    let (fn_name, args_value) =
        chat_stream::serialize_outgoing_tool_call_for_test(&tc, None, server_message_data);

    assert_eq!(fn_name, "shell");
    // Must NOT be a String (which would cause double-wrapping and Invalid \escape on the wire)
    assert!(
        !args_value.is_string(),
        "args must be a JSON object/value, not a raw string (would cause Invalid \\escape)"
    );
    // Must be a valid JSON value that serde_json can serialize
    let serialized = serde_json::to_string(&args_value).expect("args_value must be serializable");
    // The serialized form must itself be valid JSON
    serde_json::from_str::<serde_json::Value>(&serialized)
        .expect("serialized args must be valid JSON");
}

// ---------------------------------------------------------------------------
// P1-13: build_tools_array overall stability (coordinated with P0-3 MCP ordering)
// ---------------------------------------------------------------------------

/// End-to-end assertion: given the same `(REGISTRY + mcp_context)`, concatenating
/// the tools array twice produces byte-equal strings. This covers the critical stability
/// constraint for the tools array in prompts (Anthropic docs: changes to tool definitions
/// invalidate all cache).
///
/// We don't call `build_tools_array(params: &RequestParams)` directly because `RequestParams`
/// has too many fields with high construction overhead; instead, we replicate the core
/// concatenation logic for REGISTRY and MCP parts.
#[test]
fn full_tools_array_serialization_is_stable() {
    let assemble = || -> String {
        let mut buf = String::new();
        // Built-in tools (REGISTRY iteration order is static)
        for t in tools::REGISTRY {
            buf.push_str(t.name);
            buf.push('|');
            buf.push_str(t.description);
            buf.push('|');
            let schema = (t.parameters)();
            buf.push_str(&serde_json::to_string(&schema).unwrap());
            buf.push('\n');
        }
        // MCP tools (already sorted in build_mcp_tool_defs, empty if no ctx)
        buf
    };
    let a = assemble();
    let b = assemble();
    assert_eq!(a.len(), b.len());
    assert_eq!(a, b, "tools array serialization result must be byte-equal across calls");
}

/// End-to-end concatenation stability with MCP server (coordinates with P0-3 sorting guarantee).
#[test]
fn full_tools_array_with_mcp_is_stable() {
    use rmcp::model::{AnnotateAble, RawResource, Tool as McpTool};
    use serde_json::json;
    use std::sync::Arc;

    let schema_obj = json!({
        "type": "object",
        "properties": { "x": { "type": "string" } }
    })
    .as_object()
    .unwrap()
    .clone();

    let server_a = MCPServer {
        id: "id-a".to_owned(),
        name: "server-a".to_owned(),
        description: String::new(),
        resources: vec![RawResource::new("file:///x.txt", "X").no_annotation()],
        tools: vec![
            McpTool::new("zeta", "Z desc", Arc::new(schema_obj.clone())),
            McpTool::new("alpha", "A desc", Arc::new(schema_obj.clone())),
        ],
    };
    let ctx1 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![server_a.clone()],
    };
    // Reconstruct with the same ctx (servers Vec order is identical):
    let ctx2 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![server_a],
    };

    let assemble = |ctx: &MCPContext| -> String {
        let mut buf = String::new();
        for t in tools::REGISTRY {
            buf.push_str(t.name);
            buf.push('|');
            buf.push_str(t.description);
            buf.push('|');
            let schema = (t.parameters)();
            buf.push_str(&serde_json::to_string(&schema).unwrap());
            buf.push('\n');
        }
        for (name, desc, schema) in tools::mcp::build_mcp_tool_defs(ctx) {
            buf.push_str(&name);
            buf.push('|');
            buf.push_str(&desc);
            buf.push('|');
            buf.push_str(&serde_json::to_string(&schema).unwrap());
            buf.push('\n');
        }
        buf
    };

    let a = assemble(&ctx1);
    let b = assemble(&ctx2);
    assert_eq!(a, b, "tools array with MCP must be byte-equal across calls");
    // Verify MCP tools are in lexicographic order by function_name (alpha before zeta)
    let pos_alpha = a.find("mcp__server-a__alpha").expect("should contain alpha");
    let pos_zeta = a.find("mcp__server-a__zeta").expect("should contain zeta");
    assert!(pos_alpha < pos_zeta, "P0-3 ordering guarantee: alpha < zeta");
}
