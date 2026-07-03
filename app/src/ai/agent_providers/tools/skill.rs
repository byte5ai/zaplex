//! `read_skill`: read Zaplex's Skill markdown template.
//!
//! Skills are user/project-defined, reusable workflows (`SKILL.md` file + optional metadata).
//! After the model reads a skill, it can advance tasks following the steps the user expects.
//! Zaplex maintains a `SkillManager` that indexes all available skills; they can be referenced
//! either by name (frontmatter `name` field), by absolute path, or by bundled ID.
//!
//! ## Input Contract
//!
//! The BYOP path exposes the `name` field, whose value comes from the system prompt `<available_skills><skill><name>`.
//! `from_args` places the name in the proto's `SkillReference::SkillPath` slot (no proto change),
//! and the `read_skill` executor reverses the lookup by name to the real SKILL.md absolute path on cache miss,
//! then reads the file. This fallback also handles the case where the model passes an absolute path directly
//! or uses the old bundled form `@warp-skill:<id>`.
//!
//! ## Usage Recommendations (write to description)
//!
//! The model can actively invoke this in the following scenarios:
//! - User mentions a skill name / file name / path
//! - Task matches a skill description (e.g., "do PR review" triggers `review` skill)

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use warp_multi_agent_api as api;

use super::OpenAiTool;

#[derive(Debug, Deserialize)]
struct Args {
    name: String,
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Skill name (must exactly match the <available_skills><skill><name> field in system prompt)."
            }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

fn from_args(args: &str) -> Result<api::message::tool_call::Tool> {
    use api::message::tool_call::read_skill::SkillReference;
    let parsed: Args = serde_json::from_str(args)?;
    // Reuse proto's `SkillPath` slot to carry the name (avoiding proto schema changes);
    // the executor side reverses the lookup by name to the real SKILL.md path on cache miss.
    Ok(api::message::tool_call::Tool::ReadSkill(
        api::message::tool_call::ReadSkill {
            skill_reference: Some(SkillReference::SkillPath(parsed.name)),
            name: String::new(),
        },
    ))
}

fn result_to_json(result: &api::message::tool_call_result::Result) -> Option<Value> {
    use api::message::tool_call_result::Result as R;
    use api::read_skill_result::Result as SR;
    let r = match result {
        R::ReadSkill(r) => r,
        _ => return None,
    };
    let value = match &r.result {
        Some(SR::Success(s)) => {
            // FileContent { file_path, content, line_range } is directly a single message,
            // not a oneof; no need to unwrap inner content.
            let (path, content) = s
                .content
                .as_ref()
                .map(|c| (c.file_path.clone(), c.content.clone()))
                .unwrap_or_default();
            json!({ "status": "ok", "path": path, "content": content })
        }
        Some(SR::Error(e)) => json!({ "status": "error", "message": e.message }),
        None => json!({ "status": "cancelled" }),
    };
    Some(value)
}

pub static READ_SKILL: OpenAiTool = OpenAiTool {
    name: "read_skill",
    description: include_str!("../prompts/tool_descriptions/read_skill.md"),
    parameters,
    from_args,
    result_to_json,
};
