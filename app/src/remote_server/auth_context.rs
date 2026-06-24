use std::sync::Arc;

use remote_server::auth::RemoteServerAuthContext;
use warpui::r#async::BoxFuture;

use crate::auth::AuthState;

/// Construct auth context for use by remote-server module.
///
/// Zap Wave 3-1: `AuthClient` trait already physically deleted. Bearer token source changed to directly read
/// `AuthState::get_access_token_ignoring_validity()`(on Zap path only returns `Some` when user has BYOP API key mounted,
/// otherwise always `None`).
pub fn server_api_auth_context(auth_state: Arc<AuthState>) -> RemoteServerAuthContext {
    let token_auth_state = auth_state.clone();
    let identity_auth_state = auth_state;

    RemoteServerAuthContext::new(
        move || -> BoxFuture<'static, Option<String>> {
            let token = token_auth_state.get_access_token_ignoring_validity();
            Box::pin(async move { token })
        },
        move || remote_server_identity_key(&identity_auth_state),
    )
}

fn remote_server_identity_key(auth_state: &AuthState) -> String {
    // Zap no longer distinguishes anonymous / authenticated identity, uniformly uses `user_id()` (local test UID).
    auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .unwrap_or_else(|| auth_state.anonymous_id())
}
