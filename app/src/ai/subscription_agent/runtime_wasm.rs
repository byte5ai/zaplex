use crate::ai::agent::{api, AIIdentifiers};
use anyhow::{bail, Result};
use futures::channel::oneshot;
use warpui::AppContext;

pub(crate) struct SubscriptionDispatch;

pub(crate) fn subscription_dispatch_info(
    params: &api::RequestParams,
    identifiers: &AIIdentifiers,
    ctx: &AppContext,
) -> Result<SubscriptionDispatch> {
    let _ = (params, identifiers, ctx);
    bail!("Subscription agents are available only in the native Zaplex app")
}

pub(crate) async fn generate_subscription_output(
    dispatch: SubscriptionDispatch,
    cancellation_rx: oneshot::Receiver<()>,
) -> Result<api::ResponseStream, api::ConvertToAPITypeError> {
    let _ = (dispatch, cancellation_rx);
    Err(api::ConvertToAPITypeError::Other(anyhow::anyhow!(
        "Subscription agents are available only in the native Zaplex app"
    )))
}
