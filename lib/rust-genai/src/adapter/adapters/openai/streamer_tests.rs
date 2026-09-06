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

async fn local_sse_stream(body: String, options: &ChatOptions) -> OpenAIStreamer {
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
	OpenAIStreamer::new(
		EventSourceStream::new(request),
		ModelIden::new(AdapterKind::OpenAI, "test-model"),
		ChatOptionsSet::default().with_chat_options(Some(options)),
	)
}

#[tokio::test]
async fn batched_non_finish_delta_preserves_all_tool_calls() {
	let body = concat!(
		"data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
		"{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"alpha\",\"arguments\":\"{\\\"a\\\":1}\"}},",
		"{\"index\":1,\"id\":\"call_b\",\"function\":{\"name\":\"beta\",\"arguments\":\"{\\\"b\\\":2}\"}}",
		"]},\"finish_reason\":null}]}\n\n",
		"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
		"data: [DONE]\n\n",
	)
	.to_string();
	let options = ChatOptions::default().with_capture_tool_calls(true);
	let events = local_sse_stream(body, &options)
		.await
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect::<Result<Vec<_>>>()
		.expect("stream should complete");

	let chunk_ids = events
		.iter()
		.filter_map(|event| match event {
			InterStreamEvent::ToolCallChunk(call) => Some(call.call_id.as_str()),
			InterStreamEvent::Start
			| InterStreamEvent::Chunk(_)
			| InterStreamEvent::ReasoningChunk(_)
			| InterStreamEvent::ThoughtSignatureChunk(_)
			| InterStreamEvent::End(_) => None,
		})
		.collect::<Vec<_>>();
	assert_eq!(chunk_ids, vec!["call_a", "call_b"]);

	let captured = events
		.iter()
		.find_map(|event| match event {
			InterStreamEvent::End(end) => end.captured_tool_calls.as_ref(),
			InterStreamEvent::Start
			| InterStreamEvent::Chunk(_)
			| InterStreamEvent::ReasoningChunk(_)
			| InterStreamEvent::ThoughtSignatureChunk(_)
			| InterStreamEvent::ToolCallChunk(_) => None,
		})
		.expect("stream end should contain tool calls");
	assert_eq!(captured.len(), 2);
	assert_eq!(captured[0].call_id, "call_a");
	assert_eq!(captured[0].fn_arguments, serde_json::json!({"a": 1}));
	assert_eq!(captured[1].call_id, "call_b");
	assert_eq!(captured[1].fn_arguments, serde_json::json!({"b": 2}));
}

#[tokio::test]
async fn oversized_tool_call_index_returns_protocol_error() {
	let body = concat!(
		"data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
		"{\"index\":257,\"id\":\"call_bad\",\"function\":{\"name\":\"bad\",\"arguments\":\"{}\"}}",
		"]},\"finish_reason\":null}]}\n\n",
		"data: [DONE]\n\n",
	)
	.to_string();
	let options = ChatOptions::default().with_capture_tool_calls(true);
	let mut stream = local_sse_stream(body, &options).await;

	assert!(matches!(stream.next().await, Some(Ok(InterStreamEvent::Start))));
	let error = stream.next().await.expect("protocol error item").expect_err("index must fail");
	assert!(matches!(error, Error::StreamProtocol { .. }));
	assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn gapped_tool_call_index_returns_protocol_error_without_placeholder() {
	let body = concat!(
		"data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
		"{\"index\":1,\"id\":\"call_gap\",\"function\":{\"name\":\"gap\",\"arguments\":\"{}\"}}",
		"]},\"finish_reason\":null}]}\n\n",
		"data: [DONE]\n\n",
	)
	.to_string();
	let options = ChatOptions::default().with_capture_tool_calls(true);
	let mut stream = local_sse_stream(body, &options).await;

	assert!(matches!(stream.next().await, Some(Ok(InterStreamEvent::Start))));
	let error = stream.next().await.expect("protocol error item").expect_err("gap must fail");
	assert!(matches!(error, Error::StreamProtocol { .. }));
	assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn fragmented_tool_calls_merge_without_placeholder_entries() {
	let body = concat!(
		"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"alpha\",\"arguments\":\"{\\\"value\\\"\"}}]},\"finish_reason\":null}]}\n\n",
		"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":1}\"}}]},\"finish_reason\":null}]}\n\n",
		"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
		"data: [DONE]\n\n",
	)
	.to_string();
	let options = ChatOptions::default().with_capture_tool_calls(true);
	let events = local_sse_stream(body, &options)
		.await
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect::<Result<Vec<_>>>()
		.expect("stream should complete");
	let captured = events
		.iter()
		.find_map(|event| match event {
			InterStreamEvent::End(end) => end.captured_tool_calls.as_ref(),
			InterStreamEvent::Start
			| InterStreamEvent::Chunk(_)
			| InterStreamEvent::ReasoningChunk(_)
			| InterStreamEvent::ThoughtSignatureChunk(_)
			| InterStreamEvent::ToolCallChunk(_) => None,
		})
		.expect("stream end should contain tool calls");
	assert_eq!(captured.len(), 1);
	assert_eq!(captured[0].call_id, "call_a");
	assert_eq!(captured[0].fn_name, "alpha");
	assert_eq!(captured[0].fn_arguments, serde_json::json!({"value": 1}));
}
