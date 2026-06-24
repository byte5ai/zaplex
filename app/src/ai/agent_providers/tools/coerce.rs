//! Tool parameter fault-tolerance layer.
//!
//! Some BYOP models (especially DeepSeek reasoner, certain OSS models) write booleans as `"true"`/`"false"`,
//! numbers as strings, and array/object as JSON.stringify in `arguments` of tool_calls.
//! `from_args` uses strict serde parsing, so such input gets rejected directly, showing as "tool occasional failure" in UI.
//!
//! This module is only invoked after first `from_args` failure: read `parameters()` schema,
//! coerce JSON Value strings back to target types based on schema-declared types. Covers:
//!
//! | schema type | model returns | corrected to |
//! |---|---|---|
//! | boolean | "true"/"True"/"1"/"yes" | true |
//! | boolean | "false"/"False"/"0"/"no" | false |
//! | integer | "42" / 42.0 | 42 |
//! | number | "3.14" | 3.14 |
//! | string | 42 / true | "42" / "true" |
//! | array | "[\"a\"]"(JSON string) | ["a"] |
//! | object | "{\"k\":1}"(JSON string) | {"k":1} |
//!
//! Fields that cannot be coerced retain original value, letting original parse error surface.

use serde_json::{Number, Value};

/// Attempt to correct args JSON based on schema. Return `Some(coerced_string)` if at least one
/// type conversion was performed; return `None` if input cannot be parsed as JSON or no fields need coercion.
pub fn coerce_args_against_schema(args_str: &str, schema: &Value) -> Option<String> {
    let mut value: Value = serde_json::from_str(args_str).ok()?;
    let mut changed = false;
    coerce_value(&mut value, schema, &mut changed);
    if !changed {
        return None;
    }
    serde_json::to_string(&value).ok()
}

fn coerce_value(value: &mut Value, schema: &Value, changed: &mut bool) {
    let Some(ty) = schema.get("type").and_then(|t| t.as_str()) else {
        // Schema has no type: try recursing into properties for object type, otherwise give up.
        if let Some(props) = schema.get("properties") {
            coerce_object(value, props, schema, changed);
        }
        return;
    };

    match ty {
        "object" => {
            // Model stringified entire object: parse one layer then continue.
            if let Some(s) = value.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_object() {
                        *value = parsed;
                        *changed = true;
                    }
                }
            }
            if let Some(props) = schema.get("properties") {
                coerce_object(value, props, schema, changed);
            }
        }
        "array" => {
            if let Some(s) = value.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_array() {
                        *value = parsed;
                        *changed = true;
                    }
                }
            }
            if let (Some(arr), Some(items_schema)) = (value.as_array_mut(), schema.get("items")) {
                for item in arr {
                    coerce_value(item, items_schema, changed);
                }
            }
        }
        "boolean" => {
            if let Some(s) = value.as_str() {
                match s.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => {
                        *value = Value::Bool(true);
                        *changed = true;
                    }
                    "false" | "0" | "no" => {
                        *value = Value::Bool(false);
                        *changed = true;
                    }
                    _ => {}
                }
            }
        }
        "integer" => {
            if let Some(s) = value.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    *value = Value::Number(n.into());
                    *changed = true;
                } else if let Ok(f) = s.parse::<f64>() {
                    if f.fract() == 0.0 && f.is_finite() {
                        if let Some(num) = Number::from_f64(f).and_then(|n| n.as_i64()) {
                            *value = Value::Number(num.into());
                            *changed = true;
                        }
                    }
                }
            } else if let Some(f) = value.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    let n = f as i64;
                    *value = Value::Number(n.into());
                    *changed = true;
                }
            }
        }
        "number" => {
            if let Some(s) = value.as_str() {
                if let Ok(f) = s.parse::<f64>() {
                    if let Some(num) = Number::from_f64(f) {
                        *value = Value::Number(num);
                        *changed = true;
                    }
                }
            }
        }
        "string" => match value {
            Value::Number(n) => {
                let s = n.to_string();
                *value = Value::String(s);
                *changed = true;
            }
            Value::Bool(b) => {
                *value = Value::String(b.to_string());
                *changed = true;
            }
            _ => {}
        },
        _ => {}
    }
}

fn coerce_object(value: &mut Value, props: &Value, parent_schema: &Value, changed: &mut bool) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(props_map) = props.as_object() else {
        return;
    };
    for (key, prop_schema) in props_map {
        if let Some(field) = obj.get_mut(key) {
            coerce_value(field, prop_schema, changed);
        }
    }
    // additionalProperties: schema may also describe fields not listed in properties.
    if let Some(additional) = parent_schema
        .get("additionalProperties")
        .filter(|v| v.is_object())
    {
        let known: std::collections::HashSet<&String> = props_map.keys().collect();
        // SAFETY: keys collected before mutating values. Iterate via owned copy of keys
        // to avoid double borrow.
        let extra_keys: Vec<String> = obj.keys().filter(|k| !known.contains(k)).cloned().collect();
        for k in extra_keys {
            if let Some(field) = obj.get_mut(&k) {
                coerce_value(field, additional, changed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "is_read_only": {"type": "boolean"},
                "uses_pager": {"type": "boolean"},
                "is_risky": {"type": "boolean"},
                "wait_until_complete": {"type": "boolean"}
            },
            "required": ["command"]
        })
    }

    #[test]
    fn boolean_strings_coerced() {
        let args =
            r#"{"command":"echo b","is_read_only":"true","is_risky":"False","uses_pager":"0"}"#;
        let out = coerce_args_against_schema(args, &shell_schema()).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["is_read_only"], json!(true));
        assert_eq!(v["is_risky"], json!(false));
        assert_eq!(v["uses_pager"], json!(false));
    }

    #[test]
    fn no_change_returns_none() {
        let args = r#"{"command":"echo b","is_read_only":true}"#;
        assert!(coerce_args_against_schema(args, &shell_schema()).is_none());
    }

    #[test]
    fn malformed_json_returns_none() {
        let args = r#"{not json"#;
        assert!(coerce_args_against_schema(args, &shell_schema()).is_none());
    }

    fn grep_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "queries": {"type": "array", "items": {"type": "string"}},
                "path": {"type": "string"}
            }
        })
    }

    #[test]
    fn array_string_coerced_to_array() {
        let args = r#"{"queries":"[\"mod menu\",\"foo\"]","path":"app/src/lib.rs"}"#;
        let out = coerce_args_against_schema(args, &grep_schema()).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["queries"], json!(["mod menu", "foo"]));
    }

    #[test]
    fn integer_string_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}}
        });
        let args = r#"{"count":"42"}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["count"], json!(42));
    }

    #[test]
    fn nested_array_items_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"flag": {"type": "boolean"}}
                    }
                }
            }
        });
        let args = r#"{"items":[{"flag":"true"},{"flag":"false"}]}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["flag"], json!(true));
        assert_eq!(v["items"][1]["flag"], json!(false));
    }

    #[test]
    fn number_to_string_field() {
        let schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}}
        });
        let args = r#"{"path":42}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["path"], json!("42"));
    }

    #[test]
    fn stringified_object_coerced() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {"enabled": {"type": "boolean"}}
                }
            }
        });
        let args = r#"{"config":"{\"enabled\":\"true\"}"}"#;
        let out = coerce_args_against_schema(args, &schema).expect("coerced");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["config"]["enabled"], json!(true));
    }
}
