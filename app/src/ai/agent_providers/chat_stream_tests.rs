use super::*;

#[test]
fn routes_models_only_on_matching_official_hosts() {
    let cases = [
        (
            AgentProviderApiType::OpenAi,
            "gpt-5.4",
            "https://api.openai.com/v1/",
            AdapterKind::OpenAIResp,
        ),
        (
            AgentProviderApiType::OpenAi,
            "gpt-4o",
            "https://api.openai.com/v1/",
            AdapterKind::OpenAI,
        ),
        (
            AgentProviderApiType::OpenAiResp,
            "gpt-5.4",
            "https://api.openai.com/v1/",
            AdapterKind::OpenAIResp,
        ),
        (
            AgentProviderApiType::OpenAi,
            "anthropic/claude-sonnet-4-6",
            "https://api.anthropic.com/v1/",
            AdapterKind::Anthropic,
        ),
        (
            AgentProviderApiType::OpenAi,
            "anthropic/claude-sonnet-4-6",
            "https://openrouter.ai/api/v1/",
            AdapterKind::OpenAI,
        ),
        (
            AgentProviderApiType::OpenAi,
            "gpt-5.4",
            "https://relay.example.com/v1/",
            AdapterKind::OpenAI,
        ),
        (
            AgentProviderApiType::Anthropic,
            "claude-opus-4-7",
            "https://proxy.example.com/v1/",
            AdapterKind::Anthropic,
        ),
    ];

    for (api_type, model_id, base_url, expected) in cases {
        assert_eq!(
            effective_adapter_kind_for(api_type, model_id, base_url),
            expected,
            "unexpected adapter for {api_type:?}, {model_id}, {base_url}"
        );
    }
}

#[test]
fn request_api_type_follows_the_effective_adapter() {
    let cases = [
        (
            "gpt-5.4",
            "https://api.openai.com/v1/",
            AgentProviderApiType::OpenAiResp,
        ),
        (
            "claude-sonnet-4-6",
            "https://api.anthropic.com/v1/",
            AgentProviderApiType::Anthropic,
        ),
        (
            "gpt-5.4",
            "https://relay.example.com/v1/",
            AgentProviderApiType::OpenAi,
        ),
    ];

    for (model_id, base_url, expected) in cases {
        assert_eq!(
            effective_api_type_for(AgentProviderApiType::OpenAi, model_id, base_url),
            expected,
            "unexpected request API type for {model_id}, {base_url}"
        );
    }
}

#[test]
fn normalizes_anthropic_and_ollama_endpoints() {
    let cases = [
        (
            AgentProviderApiType::Anthropic,
            "https://api.anthropic.com",
            "https://api.anthropic.com/v1/",
        ),
        (
            AgentProviderApiType::Anthropic,
            "https://api.anthropic.com/v1/messages",
            "https://api.anthropic.com/v1/",
        ),
        (
            AgentProviderApiType::OpenAi,
            "https://api.anthropic.com/v1/messages",
            "https://api.anthropic.com/v1/",
        ),
        (
            AgentProviderApiType::Ollama,
            "http://localhost:11434/v1/",
            "http://localhost:11434/",
        ),
        (
            AgentProviderApiType::Ollama,
            "http://localhost:11434",
            "http://localhost:11434/",
        ),
        (
            AgentProviderApiType::Ollama,
            "http://box:11434/ollama",
            "http://box:11434/ollama/",
        ),
    ];

    for (api_type, base_url, expected) in cases {
        assert_eq!(
            normalize_endpoint_url(api_type, base_url),
            expected,
            "unexpected normalized endpoint for {api_type:?}, {base_url}"
        );
    }
}
