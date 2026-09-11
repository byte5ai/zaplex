mod catalog;
mod claude;
mod codex;
#[cfg(not(target_family = "wasm"))]
mod discovery;
mod presentation;
#[cfg(not(target_family = "wasm"))]
mod process;
mod registry;
mod response_adapter;
mod router;
#[cfg(not(target_family = "wasm"))]
mod runtime;
#[cfg(target_family = "wasm")]
#[path = "runtime_wasm.rs"]
mod runtime;
#[cfg(not(target_family = "wasm"))]
mod session;
mod types;

pub(crate) const CLAUDE_PROVIDER_MANAGED_BY_HOST: (&str, &str) =
    ("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "zaplex");
pub(crate) const CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES: [&str; 34] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "ANTHROPIC_WORKSPACE_ID",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_AWS_API_KEY",
    "ANTHROPIC_AWS_BASE_URL",
    "ANTHROPIC_AWS_WORKSPACE_ID",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "ANTHROPIC_BEDROCK_MANTLE_BASE_URL",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_SKIP_MANTLE_AUTH",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLOUD_ML_REGION",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
];

#[cfg(not(target_family = "wasm"))]
pub(crate) use claude::ClaudeProtocol;
#[cfg(not(target_family = "wasm"))]
pub(crate) use codex::CodexProtocol;
#[cfg(not(target_family = "wasm"))]
pub(crate) use discovery::discover_capabilities;
pub(crate) use presentation::{
    conversation_identity_fields, ComposerPolicy, ConversationAction, ConversationPresentation,
};
#[cfg(not(target_family = "wasm"))]
pub(crate) use process::{query_cli_version, JsonLineProcess, ProcessLaunch, ProcessLocation};
pub(crate) use registry::SubscriptionSessionRegistry;
#[cfg(not(target_family = "wasm"))]
pub(crate) use response_adapter::ResponseEventAdapter;
pub(crate) use router::RoutePreferences;
#[cfg(not(target_family = "wasm"))]
pub(crate) use router::{route_target, RouteResult};
pub(crate) use runtime::{generate_subscription_output, subscription_dispatch_info};
#[cfg(not(target_family = "wasm"))]
pub(crate) use runtime::{preflight_subscription_target, subscription_preflight_info};
#[cfg(not(target_family = "wasm"))]
pub(crate) use session::SubscriptionSession;
pub(crate) use types::{
    AccountIdentity, AgentCapability, AgentLifecycle, ApprovalDecision, HostIdentity,
    InstallationIdentity, ModelCapability, ModelEffort, SessionIdentity, SubscriptionAgent,
    SubscriptionAuthenticationError, SubscriptionEvent, SubscriptionTarget, Usage,
};
