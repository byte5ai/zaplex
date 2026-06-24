//! BYOP (Bring Your Own Provider) `LLMId` prefix encoding/decoding.
//!
//! Custom Agent provider models are distinguished in the `LLMId` string by the `byop:` prefix,
//! allowing the controller to determine at request egress whether to use the Zap backend
//! or the user's own OpenAI-compatible endpoint.
//!
//! Encoding format: `byop:<provider_id>:<model_id>`
//! - `provider_id` is `AgentProvider.id` (UUID)
//! - `model_id` is `AgentProviderModel.id` (the `model` field value sent to the upstream API)
//!
//! Example: `byop:6f3b...:deepseek-chat`
//!
//! `provider_id` is a UUID without colons; `model_id` may contain colons
//! (some upstream providers use `vendor:model` naming styles), so split only on the first colon.

use ai::LLMId;

pub const BYOP_PREFIX: &str = "byop:";

/// Encode `(provider_id, model_id)` into a single `LLMId`.
pub fn encode(provider_id: &str, model_id: &str) -> LLMId {
    LLMId::from(format!("{BYOP_PREFIX}{provider_id}:{model_id}"))
}

/// If `LLMId` is BYOP-encoded, return `(provider_id, model_id)`; otherwise return `None`.
pub fn decode(id: &LLMId) -> Option<(String, String)> {
    let s = id.as_str().strip_prefix(BYOP_PREFIX)?;
    let (pid, mid) = s.split_once(':')?;
    if pid.is_empty() || mid.is_empty() {
        return None;
    }
    Some((pid.to_owned(), mid.to_owned()))
}

/// Check if an `LLMId` is BYOP-encoded (for quick checks when field extraction is unnecessary).
pub fn is_byop(id: &LLMId) -> bool {
    id.as_str().starts_with(BYOP_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let id = encode("uuid-123", "deepseek-chat");
        assert_eq!(id.as_str(), "byop:uuid-123:deepseek-chat");
        assert_eq!(
            decode(&id),
            Some(("uuid-123".to_owned(), "deepseek-chat".to_owned()))
        );
    }

    #[test]
    fn model_id_with_colon_is_preserved() {
        // For example, OpenRouter's "anthropic/claude-3-haiku" has no colon, but some gateways
        // may use "vendor:model:variant" naming. We split only on the first colon, with the
        // remainder treated as model_id in its entirety.
        let id = encode("uuid-1", "vendor:model:v2");
        assert_eq!(
            decode(&id),
            Some(("uuid-1".to_owned(), "vendor:model:v2".to_owned()))
        );
    }

    #[test]
    fn non_byop_returns_none() {
        let id = LLMId::from("gpt-5.2");
        assert_eq!(decode(&id), None);
        assert!(!is_byop(&id));
    }

    #[test]
    fn missing_parts_returns_none() {
        assert_eq!(decode(&LLMId::from("byop:")), None);
        assert_eq!(decode(&LLMId::from("byop:uuid")), None); // No colon
        assert_eq!(decode(&LLMId::from("byop::model")), None); // Empty provider_id
        assert_eq!(decode(&LLMId::from("byop:uuid:")), None); // Empty model_id
    }
}
