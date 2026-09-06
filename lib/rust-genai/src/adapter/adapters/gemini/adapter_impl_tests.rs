use super::*;
use crate::chat::{ChatMessage, ToolResponse};

fn function_response<'a>(parts: &'a GeminiChatRequestParts) -> &'a Value {
	&parts.contents[1]["parts"][0]["functionResponse"]
}

#[test]
fn synthetic_call_id_is_not_used_as_function_name() {
	let model = ModelIden::new(AdapterKind::Gemini, "gemini-test");
	let call = ToolCall {
		call_id: "call#read_files#0".to_string(),
		fn_name: "read_files".to_string(),
		fn_arguments: json!({"paths": ["a.rs"]}),
		thought_signatures: None,
	};
	let response = ToolResponse::new("call#read_files#0", r#"{"ok":true}"#).with_fn_name("read_files");
	let request = ChatRequest::new(vec![ChatMessage::from(vec![call]), ChatMessage::from(response)]);

	let parts = GeminiAdapter::into_gemini_request_parts(&model, request).expect("serialize Gemini request");
	let response = function_response(&parts);
	assert_eq!(response["name"], "read_files");
	assert_eq!(response["response"]["name"], "read_files");
	assert!(response.get("id").is_none());
}

#[test]
fn legacy_synthetic_call_id_recovers_function_name_without_new_metadata() {
	let model = ModelIden::new(AdapterKind::Gemini, "gemini-test");
	let request = ChatRequest::new(vec![ChatMessage::from(ToolResponse::new(
		"call#read_files#7",
		"legacy result",
	))]);

	let parts = GeminiAdapter::into_gemini_request_parts(&model, request).expect("serialize legacy response");
	let response = &parts.contents[0]["parts"][0]["functionResponse"];
	assert_eq!(response["name"], "read_files");
	assert!(response.get("id").is_none());
}

#[test]
fn native_function_call_id_round_trips_into_function_response() {
	let model = ModelIden::new(AdapterKind::Gemini, "gemini-test");
	let parsed = GeminiAdapter::body_to_gemini_chat_response(
		&model,
		json!({
			"candidates": [{
				"content": {"parts": [{
					"functionCall": {"id": "native-123", "name": "read_files", "args": {}}
				}]}
			}],
			"usageMetadata": {}
		}),
	)
	.expect("parse Gemini response");
	let call = parsed
		.content
		.into_iter()
		.find_map(|content| match content {
			GeminiChatContent::ToolCall(call) => Some(call),
			GeminiChatContent::Text(_)
			| GeminiChatContent::Binary(_)
			| GeminiChatContent::Reasoning(_)
			| GeminiChatContent::ThoughtSignature(_) => None,
		})
		.expect("parsed tool call");
	assert_eq!(call.call_id, "native-123");

	let request = ChatRequest::new(vec![
		ChatMessage::from(vec![call]),
		ChatMessage::from(ToolResponse::new("native-123", "result")),
	]);
	let parts = GeminiAdapter::into_gemini_request_parts(&model, request).expect("serialize native response");
	let response = function_response(&parts);
	assert_eq!(response["name"], "read_files");
	assert_eq!(response["id"], "native-123");
	assert_eq!(parts.contents[0]["parts"][0]["functionCall"]["id"], "native-123");
}
