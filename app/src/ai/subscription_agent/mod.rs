mod catalog;
mod claude;
mod codex;
#[cfg(not(target_family = "wasm"))]
mod discovery;
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

#[cfg(not(target_family = "wasm"))]
pub(crate) use claude::ClaudeProtocol;
#[cfg(not(target_family = "wasm"))]
pub(crate) use codex::CodexProtocol;
#[cfg(not(target_family = "wasm"))]
pub(crate) use discovery::discover_capabilities;
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
pub(crate) use session::SubscriptionSession;
pub(crate) use types::{
    AccountIdentity, AgentCapability, AgentLifecycle, ApprovalDecision, HostIdentity,
    InstallationIdentity, ModelCapability, ModelEffort, SessionIdentity, SubscriptionAgent,
    SubscriptionEvent, SubscriptionTarget, Usage,
};
