//! Unit tests for `mcp.rs`.
//!
//! Covers P0-3 prompt cache optimization: `build_mcp_tool_defs` must be **lexicographically stable**,
//! producing byte-equal tool lists across multiple calls for the same `MCPContext` on the same
//! request, otherwise Anthropic detects changes to the tools field → all cache layers are invalidated.
//!
//! Note: `rmcp::model::Tool` and `rmcp::model::Resource` (= `Annotated<RawResource>`)
//! come from the upstream vendor crate; we only use their public constructor paths
//! (`Tool::new` / `RawResource::new`).

use rmcp::model::{AnnotateAble, RawResource, Tool};
use serde_json::json;
use std::sync::Arc;

use crate::ai::agent::{MCPContext, MCPServer};

use super::{build_mcp_tool_defs, function_name};

/// Construct an `rmcp::model::Tool` with minimal input schema.
fn mk_tool(name: &'static str, desc: &'static str) -> Tool {
    let schema: serde_json::Map<String, serde_json::Value> = json!({
        "type": "object",
        "properties": {
            "x": { "type": "string" }
        }
    })
    .as_object()
    .unwrap()
    .clone();
    // `Tool::new` accepts Arc<JsonObject>; we pass Map directly (implements Into<Arc<JsonObject>>).
    Tool::new(name, desc, Arc::new(schema))
}

/// Construct MCPServer. Tool and resource order is preserved as passed (simulating
/// potentially out-of-order inputs from upstream based on HashMap iteration order).
fn mk_server(
    id: &str,
    name: &str,
    tools: Vec<Tool>,
    resources: Vec<rmcp::model::Resource>,
) -> MCPServer {
    MCPServer {
        id: id.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        resources,
        tools,
    }
}

fn mk_resource(uri: &str, name: &str) -> rmcp::model::Resource {
    // RawResource → Annotated<RawResource> (without annotation).
    // The safe conversion entry point provided by upstream is `AnnotateAble::no_annotation`.
    RawResource::new(uri, name).no_annotation()
}

/// Same ctx, build twice: the (name, description, schema) tuples must be byte-equal.
/// This is the minimum bar for prompt cache hits — any instability causes all Anthropic cache to be invalidated.
#[test]
fn build_mcp_tool_defs_is_stable_across_calls() {
    let ctx = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![
            mk_server(
                "id-b",
                "server-b",
                vec![mk_tool("zeta", "z"), mk_tool("alpha", "a")],
                vec![],
            ),
            mk_server(
                "id-a",
                "server-a",
                vec![mk_tool("beta", "b"), mk_tool("gamma", "g")],
                vec![],
            ),
        ],
    };
    let r1 = build_mcp_tool_defs(&ctx);
    let r2 = build_mcp_tool_defs(&ctx);
    assert_eq!(r1, r2, "build_mcp_tool_defs must produce deterministic output");
}

/// When input servers / tools are out of order, output is sorted by function_name lexicographically.
/// This is the core P0-3 assertion: across requests, if upstream ctx.servers order differs
/// (due to HashMap iteration, etc.), output is still byte-equal.
#[test]
fn build_mcp_tool_defs_outputs_lexicographic_order() {
    let ctx = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![
            mk_server(
                "id-b",
                "server-b",
                // Out of order: zeta before alpha
                vec![mk_tool("zeta", "z"), mk_tool("alpha", "a")],
                vec![],
            ),
            mk_server(
                "id-a",
                "server-a",
                vec![mk_tool("beta", "b"), mk_tool("gamma", "g")],
                vec![],
            ),
        ],
    };
    let out = build_mcp_tool_defs(&ctx);
    let names: Vec<&str> = out.iter().map(|(n, _, _)| n.as_str()).collect();
    // After sorting by function_name: server-a/beta < server-a/gamma < server-b/alpha < server-b/zeta
    let expected = [
        function_name(&mk_server("id-a", "server-a", vec![], vec![]), "beta"),
        function_name(&mk_server("id-a", "server-a", vec![], vec![]), "gamma"),
        function_name(&mk_server("id-b", "server-b", vec![], vec![]), "alpha"),
        function_name(&mk_server("id-b", "server-b", vec![], vec![]), "zeta"),
    ];
    assert_eq!(
        names,
        expected.iter().map(|s| s.as_str()).collect::<Vec<_>>()
    );
}

/// Across requests, even if servers argument order differs (simulating HashMap re-shuffling),
/// output remains byte-equal.
#[test]
fn build_mcp_tool_defs_invariant_under_servers_permutation() {
    let server_a = mk_server(
        "id-a",
        "server-a",
        vec![mk_tool("beta", "b"), mk_tool("gamma", "g")],
        vec![],
    );
    let server_b = mk_server(
        "id-b",
        "server-b",
        vec![mk_tool("zeta", "z"), mk_tool("alpha", "a")],
        vec![],
    );
    let ctx1 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![server_a.clone(), server_b.clone()],
    };
    let ctx2 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![server_b, server_a],
    };
    assert_eq!(build_mcp_tool_defs(&ctx1), build_mcp_tool_defs(&ctx2));
}

/// When any server exposes resources, available_uris in the read_resource description
/// must also be lexicographically stable, and read_resource must always be at the array end.
#[test]
fn read_resource_description_is_stable_and_sorted() {
    let ctx1 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![mk_server(
            "id-a",
            "srv",
            vec![mk_tool("t", "")],
            vec![
                mk_resource("file:///z.txt", "Z"),
                mk_resource("file:///a.txt", "A"),
            ],
        )],
    };
    // Same ctx but with resources order swapped
    let ctx2 = MCPContext {
        #[allow(deprecated)]
        resources: vec![],
        #[allow(deprecated)]
        tools: vec![],
        servers: vec![mk_server(
            "id-a",
            "srv",
            vec![mk_tool("t", "")],
            vec![
                mk_resource("file:///a.txt", "A"),
                mk_resource("file:///z.txt", "Z"),
            ],
        )],
    };
    let r1 = build_mcp_tool_defs(&ctx1);
    let r2 = build_mcp_tool_defs(&ctx2);
    assert_eq!(r1, r2, "read_resource description must be byte-equal");

    let last = r1.last().expect("should at least contain read_resource");
    assert_eq!(last.0, "mcp_read_resource");
    // After sorting, a.txt comes before z.txt
    let pos_a = last.1.find("a.txt").expect("should contain a.txt");
    let pos_z = last.1.find("z.txt").expect("should contain z.txt");
    assert!(pos_a < pos_z, "available_uris must be sorted lexicographically");
}
