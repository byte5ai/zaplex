use crate::adapter::adapters::support::{StreamerCapturedData, StreamerOptions};
use crate::adapter::anthropic::parse_cache_creation_details;
use crate::adapter::inter_stream::{InterStreamEnd, InterStreamEvent};
use crate::chat::{ChatOptionsSet, PromptTokensDetails, StopReason, ToolCall, Usage};
use crate::webc::{Event, EventSourceStream};
use crate::{Error, ModelIden, Result};
use serde_json::{Map, Value};
use std::pin::Pin;
use std::task::{Context, Poll};
use value_ext::JsonValueExt;

pub struct AnthropicStreamer {
	inner: EventSourceStream,
	options: StreamerOptions,

	// -- Set by the poll_next
	/// Flag to prevent polling the EventSource after a MessageStop event
	done: bool,

	captured_data: StreamerCapturedData,
	in_progress_block: InProgressBlock,
}

enum InProgressBlock {
	Text,
	ToolUse { id: String, name: String, input: String },
	Thinking,
	Ignored,
}

impl AnthropicStreamer {
	pub fn new(inner: EventSourceStream, model_iden: ModelIden, options_set: ChatOptionsSet<'_, '_>) -> Self {
		Self {
			inner,
			done: false,
			options: StreamerOptions::new(model_iden, options_set),
			captured_data: Default::default(),
			in_progress_block: InProgressBlock::Ignored,
		}
	}
}

#[cfg(test)]
#[path = "streamer_tests.rs"]
mod tests;

impl futures::Stream for AnthropicStreamer {
	type Item = Result<InterStreamEvent>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		if self.done {
			return Poll::Ready(None);
		}

		while let Poll::Ready(event) = Pin::new(&mut self.inner).poll_next(cx) {
			// NOTE: At this point, we capture more events than needed for genai::StreamItem, but it serves as documentation.
			match event {
				Some(Ok(Event::Open)) => return Poll::Ready(Some(Ok(InterStreamEvent::Start))),
				Some(Ok(Event::Message(message))) => {
					let message_type = message.event.as_str();

					match message_type {
						"message_start" => {
							self.capture_usage(message_type, &message.data)?;
							continue;
						}
						"message_delta" => {
							self.capture_usage(message_type, &message.data)?;
							// Capture stop_reason from delta (e.g., "end_turn", "max_tokens", "tool_use")
							if let Ok(data) = self.parse_message_data(&message.data)
								&& let Ok(reason) = data.x_get::<String>("/delta/stop_reason")
							{
								self.captured_data.stop_reason = Some(reason);
							}
							continue;
						}
						"content_block_start" => {
							let mut data: Value =
								serde_json::from_str(&message.data).map_err(|serde_error| Error::StreamParse {
									model_iden: self.options.model_iden.clone(),
									serde_error,
								})?;
							self.in_progress_block = InProgressBlock::Ignored;

							match data.x_get_str("/content_block/type") {
								Ok("text") => self.in_progress_block = InProgressBlock::Text,
								Ok("thinking") => self.in_progress_block = InProgressBlock::Thinking,
								Ok("tool_use") => {
									let id = data.x_take::<String>("/content_block/id");
									let name = data.x_take::<String>("/content_block/name");
									let (id, name) = match (id, name) {
										(Ok(id), Ok(name)) => (id, name),
										(Err(error), Ok(_)) | (Ok(_), Err(error)) | (Err(error), Err(_)) => {
											tracing::warn!("ignoring malformed tool_use block: {error}");
											continue;
										}
									};

									// Emit an initial ToolCallChunk with name and empty args,
									// matching OpenAI's incremental streaming behaviour.
									let tc = ToolCall {
										call_id: id.clone(),
										fn_name: name.clone(),
										fn_arguments: Value::String(String::new()),
										thought_signatures: None,
									};

									self.in_progress_block = InProgressBlock::ToolUse {
										id,
										name,
										input: String::new(),
									};

									return Poll::Ready(Some(Ok(InterStreamEvent::ToolCallChunk(tc))));
								}
								Ok(txt) => {
									tracing::warn!("unhandled content type: {txt}");
								}
								Err(e) => {
									tracing::error!("{e:?}");
								}
							}

							continue;
						}
						"content_block_delta" => {
							let mut data: Value =
								serde_json::from_str(&message.data).map_err(|serde_error| Error::StreamParse {
									model_iden: self.options.model_iden.clone(),
									serde_error,
								})?;
							let delta_type = match data.x_get::<String>("/delta/type") {
								Ok(delta_type) => delta_type,
								Err(error) => {
									tracing::warn!("ignoring content block delta without a valid type: {error}");
									continue;
								}
							};

							match delta_type.as_str() {
								"text_delta" if matches!(self.in_progress_block, InProgressBlock::Text) => {
									let content: String = data.x_take("/delta/text")?;

									// Add to the captured_content if chat options say so
									if self.options.capture_content {
										match self.captured_data.content {
											Some(ref mut c) => c.push_str(&content),
											None => self.captured_data.content = Some(content.clone()),
										}
									}

									return Poll::Ready(Some(Ok(InterStreamEvent::Chunk(content))));
								}
								"input_json_delta" => {
									let InProgressBlock::ToolUse { id, name, input } = &mut self.in_progress_block
									else {
										continue;
									};
									let partial = data.x_get_str("/delta/partial_json")?;
									input.push_str(partial);

									// Emit incremental ToolCallChunk with accumulated args
									// (as Value::String, same convention as OpenAI adapter).
									let tc = ToolCall {
										call_id: id.clone(),
										fn_name: name.clone(),
										fn_arguments: Value::String(input.clone()),
										thought_signatures: None,
									};

									return Poll::Ready(Some(Ok(InterStreamEvent::ToolCallChunk(tc))));
								}
								"thinking_delta" if matches!(self.in_progress_block, InProgressBlock::Thinking) => {
									let thinking: String = data.x_take("/delta/thinking")?;
									if self.options.capture_reasoning_content {
										match self.captured_data.reasoning_content {
											Some(ref mut reasoning) => reasoning.push_str(&thinking),
											None => self.captured_data.reasoning_content = Some(thinking.clone()),
										}
									}
									return Poll::Ready(Some(Ok(InterStreamEvent::ReasoningChunk(thinking))));
								}
								"signature_delta" if matches!(self.in_progress_block, InProgressBlock::Thinking) => {
									let signature: String = data.x_take("/delta/signature")?;
									return Poll::Ready(Some(Ok(InterStreamEvent::ThoughtSignatureChunk(signature))));
								}
								"citations_delta" | "text_delta" | "thinking_delta" | "signature_delta" => continue,
								unknown => {
									tracing::warn!("ignoring unsupported content block delta type: {unknown}");
									continue;
								}
							}
						}
						"content_block_stop" => {
							match std::mem::replace(&mut self.in_progress_block, InProgressBlock::Ignored) {
								InProgressBlock::ToolUse { id, name, input } if self.options.capture_tool_calls => {
									// ToolCallChunks were already emitted incrementally
									// during content_block_start and content_block_delta.
									// Here we only finalize capture with parsed arguments.
									let fn_arguments = if input.is_empty() {
										Value::Object(Map::new())
									} else {
										serde_json::from_str(&input)?
									};

									let tc = ToolCall {
										call_id: id,
										fn_name: name,
										fn_arguments,
										thought_signatures: None,
									};

									match self.captured_data.tool_calls {
										Some(ref mut t) => t.push(tc),
										None => self.captured_data.tool_calls = Some(vec![tc]),
									}
								}
								InProgressBlock::ToolUse { .. }
								| InProgressBlock::Text
								| InProgressBlock::Thinking
								| InProgressBlock::Ignored => {}
							}

							continue;
						}
						// -- END MESSAGE
						"message_stop" => {
							// Ensure we do not poll the EventSource anymore on the next poll.
							// NOTE: This way, the last MessageStop event is still sent,
							//       but then, on the next poll, it will be stopped.
							self.done = true;

							// Capture the usage
							let captured_usage = if self.options.capture_usage {
								self.captured_data.usage.take().map(|mut usage| {
									// Compute the total if any of input/output are not null
									if usage.prompt_tokens.is_some() || usage.completion_tokens.is_some() {
										usage.total_tokens = Some(
											usage.prompt_tokens.unwrap_or(0) + usage.completion_tokens.unwrap_or(0),
										);
									}
									usage
								})
							} else {
								None
							};

							let inter_stream_end = InterStreamEnd {
								captured_usage,
								captured_stop_reason: self.captured_data.stop_reason.take().map(StopReason::from),
								captured_text_content: self.captured_data.content.take(),
								captured_reasoning_content: self.captured_data.reasoning_content.take(),
								captured_tool_calls: self.captured_data.tool_calls.take(),
								captured_thought_signatures: None,
								captured_response_id: None,
							};

							// TODO: Need to capture the data as needed
							return Poll::Ready(Some(Ok(InterStreamEvent::End(inter_stream_end))));
						}

						"ping" => continue, // Loop to the next event
						other => tracing::warn!("UNKNOWN MESSAGE TYPE: {other}"),
					}
				}
				Some(Err(err)) => {
					tracing::error!("Error: {}", err);
					return Poll::Ready(Some(Err(Error::WebStream {
						model_iden: self.options.model_iden.clone(),
						cause: err.to_string(),
						error: err,
					})));
				}
				None => return Poll::Ready(None),
			}
		}
		Poll::Pending
	}
}

// Support
impl AnthropicStreamer {
	fn capture_usage(&mut self, message_type: &str, message_data: &str) -> Result<()> {
		if self.options.capture_usage {
			let data = self.parse_message_data(message_data)?;

			let usage_path = if message_type == "message_start" {
				"/message/usage"
			} else if message_type == "message_delta" {
				"/usage"
			} else {
				tracing::debug!("Anthropic message type has no usage snapshot: {message_type}");
				return Ok(());
			};

			let input_tokens = data.x_get::<i32>(&format!("{usage_path}/input_tokens")).ok();
			let output_tokens = data.x_get::<i32>(&format!("{usage_path}/output_tokens")).ok();
			let cache_creation = data.x_get::<i32>(&format!("{usage_path}/cache_creation_input_tokens")).ok();
			let cache_read = data.x_get::<i32>(&format!("{usage_path}/cache_read_input_tokens")).ok();
			let cache_creation_details = data
				.x_get::<Value>(&format!("{usage_path}/cache_creation"))
				.ok()
				.as_ref()
				.and_then(parse_cache_creation_details);

			let usage = self.captured_data.usage.get_or_insert_with(Usage::default);
			let previous_cache_creation = usage
				.prompt_tokens_details
				.as_ref()
				.and_then(|details| details.cache_creation_tokens);
			let previous_cache_read = usage.prompt_tokens_details.as_ref().and_then(|details| details.cached_tokens);
			let previous_input = usage
				.prompt_tokens
				.map(|tokens| tokens - previous_cache_creation.unwrap_or(0) - previous_cache_read.unwrap_or(0));
			let input_tokens = input_tokens.or(previous_input);
			let cache_creation = cache_creation.or(previous_cache_creation);
			let cache_read = cache_read.or(previous_cache_read);

			if input_tokens.is_some() || cache_creation.is_some() || cache_read.is_some() {
				usage.prompt_tokens =
					Some(input_tokens.unwrap_or(0) + cache_creation.unwrap_or(0) + cache_read.unwrap_or(0));
			}
			if let Some(output_tokens) = output_tokens {
				usage.completion_tokens = Some(output_tokens);
			}

			let previous_details = usage.prompt_tokens_details.take();
			if cache_creation_details.is_some()
				|| previous_details.is_some()
				|| cache_creation.is_some()
				|| cache_read.is_some()
			{
				usage.prompt_tokens_details = Some(PromptTokensDetails {
					cache_creation_tokens: cache_creation,
					cache_creation_details: cache_creation_details.or_else(|| {
						previous_details
							.as_ref()
							.and_then(|details| details.cache_creation_details.clone())
					}),
					cached_tokens: cache_read,
					audio_tokens: previous_details.and_then(|details| details.audio_tokens),
				});
			}
		}

		Ok(())
	}

	/// Simple wrapper for now, with the corresponding map_err.
	/// Might have more logic later.
	fn parse_message_data(&self, payload: &str) -> Result<Value> {
		serde_json::from_str(payload).map_err(|serde_error| Error::StreamParse {
			model_iden: self.options.model_iden.clone(),
			serde_error,
		})
	}
}
