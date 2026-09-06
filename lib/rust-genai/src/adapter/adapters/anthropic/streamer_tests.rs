use super::*;
use crate::adapter::AdapterKind;
use crate::chat::ChatOptions;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use tokio::net::TcpListener;

fn test_streamer(options: &ChatOptions) -> AnthropicStreamer {
	let request = reqwest::Client::new().get("http://127.0.0.1:1");
	AnthropicStreamer::new(
		EventSourceStream::new(request),
		ModelIden::new(AdapterKind::Anthropic, "test-model"),
		ChatOptionsSet::default().with_chat_options(Some(options)),
	)
}

async fn local_sse_stream(body: String, options: &ChatOptions) -> AnthropicStreamer {
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind local SSE server");
	let address = listener.local_addr().expect("read local SSE address");
	tokio::spawn(async move {
		let (socket, _) = listener.accept().await.expect("accept local SSE connection");
		let service = service_fn(move |_request: Request<Incoming>| {
			let body = body.clone();
			async move {
				Ok::<_, Infallible>(
					Response::builder()
						.header("content-type", "text/event-stream")
						.body(Full::new(Bytes::from(body)))
						.expect("build local SSE response"),
				)
			}
		});
		hyper::server::conn::http1::Builder::new()
			.serve_connection(TokioIo::new(socket), service)
			.await
			.expect("serve local SSE response");
	});

	let request = reqwest::Client::new().get(format!("http://{address}"));
	AnthropicStreamer::new(
		EventSourceStream::new(request),
		ModelIden::new(AdapterKind::Anthropic, "test-model"),
		ChatOptionsSet::default().with_chat_options(Some(options)),
	)
}

#[test]
fn cumulative_message_delta_replaces_start_usage() {
	let options = ChatOptions::default().with_capture_usage(true);
	let mut streamer = test_streamer(&options);
	streamer
		.capture_usage(
			"message_start",
			r#"{"message":{"usage":{"input_tokens":25,"output_tokens":1}}}"#,
		)
		.expect("capture initial usage");
	streamer
		.capture_usage("message_delta", r#"{"usage":{"input_tokens":25,"output_tokens":15}}"#)
		.expect("replace usage snapshot");

	let usage = streamer.captured_data.usage.expect("captured usage");
	assert_eq!(usage.prompt_tokens, Some(25));
	assert_eq!(usage.completion_tokens, Some(15));
}

#[test]
fn output_only_delta_preserves_prompt_and_cache_components() {
	let options = ChatOptions::default().with_capture_usage(true);
	let mut streamer = test_streamer(&options);
	streamer
		.capture_usage(
			"message_start",
			r#"{"message":{"usage":{"input_tokens":20,"output_tokens":1,"cache_creation_input_tokens":3,"cache_read_input_tokens":2}}}"#,
		)
		.expect("capture initial usage");
	streamer
		.capture_usage("message_delta", r#"{"usage":{"output_tokens":9}}"#)
		.expect("replace output-only snapshot");

	let usage = streamer.captured_data.usage.expect("captured usage");
	assert_eq!(usage.prompt_tokens, Some(25));
	assert_eq!(usage.completion_tokens, Some(9));
	let details = usage.prompt_tokens_details.expect("cache details");
	assert_eq!(details.cache_creation_tokens, Some(3));
	assert_eq!(details.cached_tokens, Some(2));
}

#[tokio::test]
async fn server_tool_and_citation_deltas_do_not_corrupt_text_stream() {
	let body = concat!(
		"event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n",
		"event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
		"event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"before\"}}\n\n",
		"event: content_block_stop\ndata: {}\n\n",
		"event: content_block_start\ndata: {\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srv_1\",\"name\":\"web_search\"}}\n\n",
		"event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
		"event: content_block_stop\ndata: {}\n\n",
		"event: content_block_start\ndata: {\"content_block\":{\"type\":\"web_search_tool_result\",\"tool_use_id\":\"srv_1\"}}\n\n",
		"event: content_block_stop\ndata: {}\n\n",
		"event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
		"event: content_block_delta\ndata: {\"delta\":{\"type\":\"citations_delta\",\"citation\":{\"type\":\"web_search_result_location\"}}}\n\n",
		"event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"after\"}}\n\n",
		"event: content_block_stop\ndata: {}\n\n",
		"event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":25,\"output_tokens\":15}}\n\n",
		"event: message_stop\ndata: {}\n\n",
	)
	.to_string();
	let options = ChatOptions::default().with_capture_content(true).with_capture_usage(true);
	let events = local_sse_stream(body, &options)
		.await
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect::<Result<Vec<_>>>()
		.expect("stream should complete");
	let chunks = events
		.iter()
		.filter_map(|event| match event {
			InterStreamEvent::Chunk(content) => Some(content.as_str()),
			InterStreamEvent::Start
			| InterStreamEvent::ReasoningChunk(_)
			| InterStreamEvent::ThoughtSignatureChunk(_)
			| InterStreamEvent::ToolCallChunk(_)
			| InterStreamEvent::End(_) => None,
		})
		.collect::<Vec<_>>();
	assert_eq!(chunks, vec!["before", "after"]);
	let end = events
		.iter()
		.find_map(|event| match event {
			InterStreamEvent::End(end) => Some(end),
			InterStreamEvent::Start
			| InterStreamEvent::Chunk(_)
			| InterStreamEvent::ReasoningChunk(_)
			| InterStreamEvent::ThoughtSignatureChunk(_)
			| InterStreamEvent::ToolCallChunk(_) => None,
		})
		.expect("stream end event");
	assert_eq!(end.captured_text_content.as_deref(), Some("beforeafter"));
	let usage = end.captured_usage.as_ref().expect("captured usage");
	assert_eq!(usage.prompt_tokens, Some(25));
	assert_eq!(usage.completion_tokens, Some(15));
}
