//! In BYOP mode, infers which multimodal attachment types a model supports based on `api_type` × `model_id`.
//!
//! genai 0.6's `ContentPart::Binary` is automatically adapted at the wire protocol layer
//! (see comment table in `chat_stream.rs`):
//! - OpenAI: image→`image_url{data:URL}`, pdf/file→`type:"file" file_data:data:URL`, audio→`input_audio`
//! - Anthropic: image→`image base64`, others→`document base64` (actually only PDF is effective)
//! - Gemini: all via `inline_data`
//!
//! However, **wire protocol support** ≠ **model support**. This module only contains logic for what the model
//! actually consumes, avoiding sending images to text-only models like GPT-3.5 or Claude Sonnet 1.0 which
//! would cause upstream errors.
//!
//! Matching uses model_id substring matching, aligning with `prompt_renderer::resolve_template` style.
//! Substring rules are intentionally loose (substring presence counts as a match), aimed at "covering future
//! minor upgrades in the same family" rather than "exact version enumeration", with error probability vs.
//! maintenance cost trade-off biased toward the latter.

use super::models_dev;
use crate::settings::{AgentProviderApiType, AgentProviderModel};

/// A model's supported attachment type capabilities table.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttachmentCaps {
    /// Whether the model supports images (image/* MIME).
    pub images: bool,
    /// Whether the model supports PDF (application/pdf MIME).
    pub pdf: bool,
    /// Whether the model supports audio (audio/* MIME).
    pub audio: bool,
}

impl AttachmentCaps {
    /// No multimodal capabilities at all → upstream must degrade to text-only path.
    pub fn is_text_only(&self) -> bool {
        !self.images && !self.pdf && !self.audio
    }

    /// Given a MIME type, determine if the model can consume this binary attachment.
    pub fn supports_mime(&self, mime: &str) -> bool {
        let lower = mime.trim().to_ascii_lowercase();
        if lower.starts_with("image/") {
            return self.images;
        }
        if lower == "application/pdf" {
            return self.pdf;
        }
        if lower.starts_with("audio/") {
            return self.audio;
        }
        false
    }
}

/// Check models.dev catalog first; fall back to (api_type, model_id substring) matching on catalog miss.
///
/// The catalog is the authoritative source for actual model capabilities (users can click
/// "Sync from models.dev" in settings or it auto-refreshes every 24h); fallback rules ensure
/// mainstream models work even offline or before the catalog is loaded.
pub fn caps_for(api_type: AgentProviderApiType, model_id: &str) -> AttachmentCaps {
    if let Some(c) = models_dev::lookup_caps("", model_id) {
        return AttachmentCaps {
            images: c.vision,
            pdf: c.pdf,
            audio: c.audio,
        };
    }
    caps_for_by_substring(api_type, model_id)
}

/// Parse the final capability for a single model, **with user three-state override**.
/// Three priority levels:
/// 1. User explicitly sets to `Some(_)` in settings → use directly, skip inference
/// 2. `None` → infer from models.dev catalog
/// 3. Catalog miss → substring fallback
///
/// `provider_id` is used for exact provider matching in the catalog (handles aggregator
/// providers like OpenRouter); fallback doesn't need provider_id on catalog miss.
pub fn resolve_for_model(
    provider_id: &str,
    api_type: AgentProviderApiType,
    model: &AgentProviderModel,
) -> AttachmentCaps {
    let inferred = if let Some(c) = models_dev::lookup_caps(provider_id, &model.id) {
        AttachmentCaps {
            images: c.vision,
            pdf: c.pdf,
            audio: c.audio,
        }
    } else {
        caps_for_by_substring(api_type, &model.id)
    };
    AttachmentCaps {
        images: model.image.unwrap_or(inferred.images),
        pdf: model.pdf.unwrap_or(inferred.pdf),
        audio: model.audio.unwrap_or(inferred.audio),
    }
}

/// Inference result snapshot for UI (ignoring user overrides, checking only catalog/fallback).
/// Used to display "Auto: catalog says supported" semantics in chip tooltips.
pub fn inferred_for_model(
    provider_id: &str,
    api_type: AgentProviderApiType,
    model_id: &str,
) -> AttachmentCaps {
    if let Some(c) = models_dev::lookup_caps(provider_id, model_id) {
        AttachmentCaps {
            images: c.vision,
            pdf: c.pdf,
            audio: c.audio,
        }
    } else {
        caps_for_by_substring(api_type, model_id)
    }
}

/// Fallback table lookup by (api_type, model_id substring).
///
/// By default, conservatively returns "all false" for unknown models, which avoids
/// accidentally sending binary to unsupported models (causing 400 errors); the trade-off
/// is that new models need manual addition (acceptable, since each new model needs updates
/// for reasoning_effort, context_window, etc. anyway).
fn caps_for_by_substring(api_type: AgentProviderApiType, model_id: &str) -> AttachmentCaps {
    let lower = model_id.to_ascii_lowercase();
    match api_type {
        AgentProviderApiType::OpenAi | AgentProviderApiType::OpenAiResp => {
            // GPT-4o / 4.1 / 5 series: image + pdf. 3.5 series text-only.
            if lower.contains("gpt-4o")
                || lower.contains("gpt-4.1")
                || lower.contains("gpt-5")
                || lower.contains("o1")
                || lower.contains("o3")
                || lower.contains("o4")
            {
                AttachmentCaps {
                    images: true,
                    pdf: true,
                    audio: false,
                }
            } else if lower.contains("gpt-4o-audio") || lower.contains("gpt-realtime") {
                AttachmentCaps {
                    images: true,
                    pdf: true,
                    audio: true,
                }
            } else {
                AttachmentCaps::default()
            }
        }
        AgentProviderApiType::Anthropic => {
            // Claude 3 / 3.5 / 4 / 4.5 / 4.7 series all support vision + document (PDF).
            if lower.contains("claude-3")
                || lower.contains("claude-4")
                || lower.contains("claude-opus")
                || lower.contains("claude-sonnet")
                || lower.contains("claude-haiku")
            {
                AttachmentCaps {
                    images: true,
                    pdf: true,
                    audio: false,
                }
            } else {
                AttachmentCaps::default()
            }
        }
        AgentProviderApiType::Gemini => {
            // Gemini 1.5+ / 2 / 2.5 series fully multimodal, inline_data supports image/pdf/audio/video.
            if lower.contains("gemini-1.5")
                || lower.contains("gemini-2")
                || lower.contains("gemini-pro-vision")
            {
                AttachmentCaps {
                    images: true,
                    pdf: true,
                    audio: true,
                }
            } else {
                AttachmentCaps::default()
            }
        }
        AgentProviderApiType::Ollama => {
            // Ollama: most models text-only. Vision models (LLaVA / bakllava / llama3.2-vision /
            // qwen2-vl / minicpm-v / moondream) enable image capability via model_id substring matching.
            // PDF/audio are essentially unsupported under Ollama protocol; conservatively return false.
            let vision = lower.contains("llava")
                || lower.contains("bakllava")
                || lower.contains("vision")
                || lower.contains("-vl")
                || lower.contains("minicpm-v")
                || lower.contains("moondream");
            AttachmentCaps {
                images: vision,
                pdf: false,
                audio: false,
            }
        }
        AgentProviderApiType::DeepSeek => {
            // DeepSeek: current public models (v3/r1/coder/chat) are all text-only.
            // Enable when deepseek-vl series launches in the future.
            if lower.contains("vl") {
                AttachmentCaps {
                    images: true,
                    pdf: false,
                    audio: false,
                }
            } else {
                AttachmentCaps::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_4o_supports_image_and_pdf() {
        // Using fallback rules: in test environment, models.dev catalog is not loaded, lookup_caps returns None.
        let caps = caps_for_by_substring(AgentProviderApiType::OpenAi, "gpt-4o-2024-08-06");
        assert!(caps.images);
        assert!(caps.pdf);
        assert!(!caps.audio);
    }

    #[test]
    fn openai_3_5_text_only() {
        let caps = caps_for_by_substring(AgentProviderApiType::OpenAi, "gpt-3.5-turbo");
        assert!(caps.is_text_only());
    }

    #[test]
    fn claude_sonnet_supports_image_and_pdf() {
        let caps = caps_for_by_substring(AgentProviderApiType::Anthropic, "claude-sonnet-4-5");
        assert!(caps.images);
        assert!(caps.pdf);
    }

    #[test]
    fn gemini_2_5_full_multimodal() {
        let caps = caps_for_by_substring(AgentProviderApiType::Gemini, "gemini-2.5-pro");
        assert!(caps.images);
        assert!(caps.pdf);
        assert!(caps.audio);
    }

    #[test]
    fn ollama_default_text_only() {
        let caps = caps_for_by_substring(AgentProviderApiType::Ollama, "qwen2.5:7b");
        assert!(caps.is_text_only());
    }

    #[test]
    fn ollama_vision_models_get_images() {
        let caps = caps_for_by_substring(AgentProviderApiType::Ollama, "llava:13b");
        assert!(caps.images);
        assert!(!caps.pdf);
    }

    #[test]
    fn deepseek_chat_text_only() {
        let caps = caps_for_by_substring(AgentProviderApiType::DeepSeek, "deepseek-chat");
        assert!(caps.is_text_only());
    }

    #[test]
    fn supports_mime_routing() {
        let full = AttachmentCaps {
            images: true,
            pdf: true,
            audio: true,
        };
        assert!(full.supports_mime("image/png"));
        assert!(full.supports_mime("application/pdf"));
        assert!(full.supports_mime("audio/mp3"));
        assert!(!full.supports_mime("application/zip"));
        assert!(!full.supports_mime("text/plain"));
    }
}
