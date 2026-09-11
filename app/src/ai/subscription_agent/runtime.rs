use super::{
    discover_capabilities, query_cli_version, route_target, AccountIdentity, AgentCapability,
    AgentLifecycle, HostIdentity, InstallationIdentity, ProcessLocation, ResponseEventAdapter,
    RoutePreferences, RouteResult, SubscriptionAgent, SubscriptionAuthenticationError,
    SubscriptionSession, SubscriptionSessionRegistry, SubscriptionTarget,
};
use crate::ai::agent::{api, AIAgentInput, AIIdentifiers};
use crate::ai::api_error::AIApiError;
use crate::ai::blocklist::{BlocklistAIHistoryModel, SessionContext};
use crate::cockpit::CockpitModel;
use crate::remote_server::manager::RemoteServerManager;
use crate::report_if_error;
use crate::terminal::ssh::util::InteractiveSshCommand;
use anyhow::{anyhow, bail, Context, Result};
use futures::channel::oneshot;
use futures_util::{select, FutureExt};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use warpui::{AppContext, SingletonEntity};
use zaplex_cockpit::{AccountUsage, Provider};
use zaplex_remote_session::types::{has_feature, FEATURE_AGENT_ACCOUNT_ROUTING_V1};

#[derive(Clone)]
struct RuntimeCandidate {
    installation: InstallationIdentity,
    location: ProcessLocation,
}

#[derive(Clone)]
enum RuntimeCandidates {
    Ready(Vec<RuntimeCandidate>),
    Remote {
        client: Arc<crate::remote_server::client::RemoteServerClient>,
        host_id: String,
        host_name: String,
        ssh_argv: Vec<String>,
        use_cached_inventory: bool,
    },
}

impl RuntimeCandidates {
    fn is_known_empty(&self) -> bool {
        matches!(self, Self::Ready(candidates) if candidates.is_empty())
    }

    async fn resolve(
        self,
        preferred_agent: Option<SubscriptionAgent>,
    ) -> Result<Vec<RuntimeCandidate>> {
        match self {
            Self::Ready(candidates) => Ok(candidates),
            Self::Remote {
                client,
                host_id,
                host_name,
                ssh_argv,
                use_cached_inventory,
            } => {
                let inventory =
                    if use_cached_inventory {
                        match client.cached_agent_accounts() {
                            Some(inventory) => inventory,
                            None => client.list_agent_accounts().await.context(
                                "failed to read the remote subscription-account inventory",
                            )?,
                        }
                    } else {
                        client.list_agent_accounts().await.context(
                            "failed to refresh the remote subscription-account inventory",
                        )?
                    };
                remote_candidates_for_ssh(
                    &host_id,
                    &host_name,
                    ssh_argv,
                    &inventory,
                    preferred_agent,
                )
            }
        }
    }
}

pub(crate) struct SubscriptionDispatch {
    candidates: RuntimeCandidates,
    preferences: RoutePreferences,
    registry: SubscriptionSessionRegistry,
    conversation_id: String,
    task_id: String,
    needs_create_task: bool,
    prompt: String,
    working_directory: PathBuf,
}

pub(crate) struct SubscriptionPreflight {
    candidates: RuntimeCandidates,
    preferences: RoutePreferences,
    registry: SubscriptionSessionRegistry,
    conversation_id: String,
    working_directory: PathBuf,
}

fn runtime_candidates(
    session_context: &SessionContext,
    registry: &SubscriptionSessionRegistry,
    use_cached_remote_inventory: bool,
    ctx: &AppContext,
) -> Result<(RuntimeCandidates, RoutePreferences, PathBuf)> {
    let working_directory = if session_context.is_legacy_ssh() {
        // A legacy SSH session has no remote shell hook, so its reported cwd
        // belongs to the local client. Start the remote CLI in its login cwd.
        PathBuf::from(".")
    } else {
        session_context
            .current_working_directory()
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let mut preferences = registry.preferences();
    let candidates = match session_context.host_id() {
        Some(host_id) => remote_candidates(host_id.as_str(), use_cached_remote_inventory, ctx)?,
        None if session_context.is_legacy_ssh() => RuntimeCandidates::Ready(legacy_ssh_candidates(
            session_context
                .ssh_connection_info()
                .context("the active SSH session has no reusable connection details")?,
        )?),
        None if session_context.is_remote() => {
            bail!("the active remote host is not connected; reconnect it and try again")
        }
        None => RuntimeCandidates::Ready(local_candidates(&mut preferences, ctx)),
    };
    Ok((candidates, preferences, working_directory))
}

pub(crate) fn subscription_preflight_info(
    conversation_id: String,
    session_context: &SessionContext,
    ctx: &AppContext,
) -> Result<SubscriptionPreflight> {
    let registry = SubscriptionSessionRegistry::as_ref(ctx).clone();
    registry.set_lifecycle(conversation_id.clone(), AgentLifecycle::Starting);
    let (candidates, preferences, working_directory) =
        match runtime_candidates(session_context, &registry, false, ctx) {
            Ok(runtime) => runtime,
            Err(error) => {
                registry.set_lifecycle(
                    conversation_id,
                    AgentLifecycle::RecoverableError {
                        message: error.to_string(),
                        session: None,
                    },
                );
                return Err(error);
            }
        };
    if candidates.is_known_empty() {
        registry.set_lifecycle(conversation_id, AgentLifecycle::NoAgentInstalled);
        bail!("Install Claude Code or Codex and sign in with a subscription account");
    }
    Ok(SubscriptionPreflight {
        candidates,
        preferences,
        registry,
        conversation_id,
        working_directory,
    })
}

pub(crate) fn subscription_dispatch_info(
    params: &api::RequestParams,
    identifiers: &AIIdentifiers,
    ctx: &AppContext,
) -> Result<SubscriptionDispatch> {
    let conversation_id = identifiers
        .client_conversation_id
        .context("subscription agent request has no conversation identity")?;
    let conversation_id_string = conversation_id.to_string();
    let conversation = BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&conversation_id)
        .context("subscription agent conversation is not in local history")?;
    let task_id = conversation.get_root_task_id().to_string();
    let needs_create_task = conversation.compute_active_tasks().is_empty();
    let prompt = prompt_from_inputs(&params.input)?;
    let registry = SubscriptionSessionRegistry::as_ref(ctx).clone();
    let (candidates, preferences, working_directory) =
        runtime_candidates(&params.session_context, &registry, true, ctx)?;
    if candidates.is_known_empty() {
        registry.set_lifecycle(
            conversation_id_string.clone(),
            AgentLifecycle::NoAgentInstalled,
        );
        bail!("Install Claude Code or Codex and sign in with a subscription account");
    }
    Ok(SubscriptionDispatch {
        candidates,
        preferences,
        registry,
        conversation_id: conversation_id_string,
        task_id,
        needs_create_task,
        prompt,
        working_directory,
    })
}

fn is_authentication_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not signed in")
        || message.contains("not logged in")
        || message.contains("please run /login")
        || message.contains("not using a chatgpt subscription account")
        || message.contains("authenticated account does not match selected account")
        || message.contains("did not report an account id for selected account")
}

fn discovery_failure_lifecycle(
    attempted_agents: &[SubscriptionAgent],
    message: String,
    registry: &SubscriptionSessionRegistry,
    conversation_id: &str,
) -> AgentLifecycle {
    if attempted_agents.len() == 1 && is_authentication_failure(&message) {
        registry.clear_session_identity(conversation_id);
        AgentLifecycle::NotSignedIn {
            agent: attempted_agents[0],
        }
    } else {
        AgentLifecycle::RecoverableError {
            message,
            session: registry.get(conversation_id).map(|stored| stored.session),
        }
    }
}

fn runtime_error_lifecycle(
    registry: &SubscriptionSessionRegistry,
    conversation_id: &str,
    agent: SubscriptionAgent,
    message: String,
) -> AgentLifecycle {
    if is_authentication_failure(&message) {
        registry.clear_session_identity(conversation_id);
        AgentLifecycle::NotSignedIn { agent }
    } else {
        recoverable_lifecycle(registry, conversation_id, message)
    }
}

fn classify_subscription_error(error: anyhow::Error) -> anyhow::Error {
    let message = error.to_string();
    if is_authentication_failure(&message) {
        SubscriptionAuthenticationError { message }.into()
    } else {
        error
    }
}

fn subscription_api_error(error: anyhow::Error) -> AIApiError {
    AIApiError::Other(classify_subscription_error(error))
}

async fn discover_routed_target(
    candidates: RuntimeCandidates,
    preferences: &RoutePreferences,
    registry: &SubscriptionSessionRegistry,
    conversation_id: &str,
    working_directory: &std::path::Path,
) -> Result<SubscriptionTarget> {
    let candidates = match candidates.resolve(preferences.agent).await {
        Ok(candidates) => candidates,
        Err(error) => {
            registry.set_lifecycle(
                conversation_id.to_string(),
                recoverable_lifecycle(registry, conversation_id, error.to_string()),
            );
            return Err(error);
        }
    };
    if candidates.is_empty() {
        registry.set_lifecycle(
            conversation_id.to_string(),
            AgentLifecycle::NoAgentInstalled,
        );
        bail!("Install Claude Code or Codex and sign in with a subscription account");
    }
    let mut attempted_agents: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.installation.agent)
        .collect();
    attempted_agents.sort_by_key(|agent| match agent {
        SubscriptionAgent::ClaudeCode => 0,
        SubscriptionAgent::Codex => 1,
    });
    attempted_agents.dedup();
    let mut capabilities = Vec::new();
    let mut discovery_errors = Vec::new();
    for candidate in candidates {
        let agent = candidate.installation.agent;
        let account_id = candidate.installation.account.id.clone();
        match discover_candidate(candidate, working_directory).await {
            Ok(capability) => capabilities.push(capability),
            Err(error) => discovery_errors.push((agent, account_id, error.to_string())),
        }
    }
    if let Some(preferred_agent) = preferences.agent {
        let selected_authentication_error =
            discovery_errors.iter().find(|(agent, account, error)| {
                *agent == preferred_agent
                    && preferences
                        .account_id
                        .as_ref()
                        .map_or(true, |selected| selected == account)
                    && is_authentication_failure(error)
            });
        if let Some((_, _, error)) = selected_authentication_error {
            let lifecycle = discovery_failure_lifecycle(
                &[preferred_agent],
                error.clone(),
                registry,
                conversation_id,
            );
            registry.set_lifecycle(conversation_id.to_string(), lifecycle);
            return Err(SubscriptionAuthenticationError {
                message: format!(
                    "No compatible signed-in subscription agent is available: {error}"
                ),
            }
            .into());
        }
    }
    if capabilities.is_empty() {
        let message = discovery_errors
            .into_iter()
            .map(|(_, _, error)| error)
            .collect::<Vec<_>>()
            .join("; ");
        let authentication_failure =
            attempted_agents.len() == 1 && is_authentication_failure(&message);
        let lifecycle = discovery_failure_lifecycle(
            &attempted_agents,
            message.clone(),
            registry,
            conversation_id,
        );
        registry.set_lifecycle(conversation_id.to_string(), lifecycle);
        let error = anyhow!("No compatible signed-in subscription agent is available: {message}");
        return if authentication_failure {
            Err(classify_subscription_error(error))
        } else {
            Err(error)
        };
    }
    match route_target(
        capabilities.clone(),
        preferences,
        working_directory.to_path_buf(),
    ) {
        RouteResult::Ready(target) => Ok(target),
        RouteResult::NoReachableAgent => {
            let message = "No compatible signed-in subscription agent is reachable".to_string();
            registry.set_lifecycle(
                conversation_id.to_string(),
                recoverable_lifecycle(registry, conversation_id, message.clone()),
            );
            bail!(message)
        }
        RouteResult::NeedsAgentChoice(agents) => {
            registry.set_agent_choices(conversation_id.to_string(), agents.clone());
            registry.set_lifecycle(conversation_id.to_string(), AgentLifecycle::Ready);
            bail!(
                "Choose the in-app agent first: {}",
                agents
                    .into_iter()
                    .map(SubscriptionAgent::display_name)
                    .collect::<Vec<_>>()
                    .join(" or ")
            )
        }
        RouteResult::NeedsAccountChoice { agent, .. } => {
            registry.set_lifecycle(conversation_id.to_string(), AgentLifecycle::Ready);
            bail!(
                "Choose a {} subscription account first",
                agent.display_name()
            )
        }
        RouteResult::NeedsModelChoice { agent, account_id } => {
            let models = capabilities
                .iter()
                .find(|capability| {
                    capability.installation.agent == agent
                        && capability.installation.account.id == account_id
                })
                .map(|capability| capability.models.clone())
                .unwrap_or_default();
            registry.set_model_choices(conversation_id.to_string(), models);
            registry.set_lifecycle(conversation_id.to_string(), AgentLifecycle::Ready);
            bail!(
                "{} did not report one unambiguous default model; choose one of its reported models",
                agent.display_name()
            )
        }
    }
}

pub(crate) async fn preflight_subscription_target(preflight: SubscriptionPreflight) -> Result<()> {
    let SubscriptionPreflight {
        candidates,
        preferences,
        registry,
        conversation_id,
        working_directory,
    } = preflight;
    let target = discover_routed_target(
        candidates,
        &preferences,
        &registry,
        &conversation_id,
        &working_directory,
    )
    .await?;
    registry.remember_target(&target);
    registry.set_target(conversation_id.clone(), target);
    registry.set_lifecycle(conversation_id, AgentLifecycle::Ready);
    Ok(())
}

pub(crate) async fn generate_subscription_output(
    dispatch: SubscriptionDispatch,
    cancellation_rx: oneshot::Receiver<()>,
) -> Result<api::ResponseStream, api::ConvertToAPITypeError> {
    let SubscriptionDispatch {
        candidates,
        preferences,
        registry,
        conversation_id,
        task_id,
        needs_create_task,
        prompt,
        working_directory,
    } = dispatch;
    let candidate_locations = candidates.clone();
    let preflight_target = registry
        .lifecycle(&conversation_id)
        .filter(AgentLifecycle::accepts_prompt)
        .and_then(|_| registry.target(&conversation_id));
    registry.set_lifecycle(conversation_id.clone(), AgentLifecycle::Starting);
    let target = match preflight_target {
        Some(target) => target,
        None => discover_routed_target(
            candidates,
            &preferences,
            &registry,
            &conversation_id,
            &working_directory,
        )
        .await
        .map_err(api::ConvertToAPITypeError::Other)?,
    };
    registry.remember_target(&target);
    registry.set_target(conversation_id.clone(), target.clone());
    let resume = registry
        .get(&conversation_id)
        .and_then(|stored| same_resume_target(&stored.target, &target).then_some(stored.session));
    let location = location_for_target(&target, &candidate_locations)
        .context("selected subscription target lost its process location")
        .map_err(api::ConvertToAPITypeError::Other)?;
    let mut session = match with_timeout(
        "subscription agent initialization",
        SubscriptionSession::open(target.clone(), resume, location),
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            registry.set_lifecycle(
                conversation_id.clone(),
                runtime_error_lifecycle(
                    &registry,
                    &conversation_id,
                    target.installation.agent,
                    error.to_string(),
                ),
            );
            return Err(api::ConvertToAPITypeError::Other(
                classify_subscription_error(error),
            ));
        }
    };
    if let Some(identity) = session.identity().cloned() {
        registry.store(conversation_id.clone(), target.clone(), identity);
    }
    if let Err(error) = with_timeout(
        "subscription agent prompt delivery",
        session.send_prompt(&prompt),
    )
    .await
    {
        end_session(&mut session, &registry, &conversation_id).await;
        registry.set_lifecycle(
            conversation_id.clone(),
            runtime_error_lifecycle(
                &registry,
                &conversation_id,
                target.installation.agent,
                error.to_string(),
            ),
        );
        return Err(api::ConvertToAPITypeError::Other(
            classify_subscription_error(error),
        ));
    }
    registry.set_lifecycle(conversation_id.clone(), AgentLifecycle::Responding);

    let context_window = target.model.context_window;
    let stream = async_stream::stream! {
        let mut adapter = ResponseEventAdapter::new(task_id, context_window);
        yield Ok(adapter.stream_init());
        if needs_create_task {
            yield Ok(adapter.create_task());
        }
        yield Ok(adapter.persist_user_query(prompt));
        yield Ok(adapter.target(&target));
        let cancellation = cancellation_rx.fuse();
        futures_util::pin_mut!(cancellation);
        loop {
            let event = {
                let next_event = session.next_event().fuse();
                futures_util::pin_mut!(next_event);
                select! {
                    _ = cancellation => None,
                    event = next_event => Some(event),
                }
            };
            let event = match event {
                None => {
                    report_if_error!(with_timeout(
                        "subscription agent cancellation",
                        session.cancel()
                    )
                    .await);
                    mark_cancelled(&registry, &conversation_id);
                    end_session(&mut session, &registry, &conversation_id).await;
                    break;
                }
                Some(event) => match event {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        registry.set_lifecycle(
                            conversation_id.clone(),
                            recoverable_lifecycle(
                                &registry,
                                &conversation_id,
                                "Subscription agent ended without a terminal event".to_string(),
                            ),
                        );
                        end_session(&mut session, &registry, &conversation_id).await;
                        break;
                    }
                    Err(error) => {
                        registry.set_lifecycle(
                            conversation_id.clone(),
                            runtime_error_lifecycle(
                                &registry,
                                &conversation_id,
                                target.installation.agent,
                                error.to_string(),
                            ),
                        );
                        end_session(&mut session, &registry, &conversation_id).await;
                        yield Err(Arc::new(subscription_api_error(error)));
                        break;
                    }
                },
            };
            match &event {
                super::SubscriptionEvent::SessionStarted(identity) => {
                    registry.store(conversation_id.clone(), target.clone(), identity.clone());
                }
                super::SubscriptionEvent::TextDelta(_)
                | super::SubscriptionEvent::ReasoningDelta(_) => {
                    registry.set_lifecycle(conversation_id.clone(), AgentLifecycle::Responding);
                }
                super::SubscriptionEvent::ToolStarted { name, .. } => {
                    registry.set_lifecycle(
                        conversation_id.clone(),
                        AgentLifecycle::RunningTool { name: name.clone() },
                    );
                }
                super::SubscriptionEvent::ApprovalRequested { request_id, .. } => {
                    registry.set_lifecycle(
                        conversation_id.clone(),
                        AgentLifecycle::WaitingForApproval {
                            request_id: request_id.clone(),
                        },
                    );
                }
                super::SubscriptionEvent::TurnCompleted { session: identity } => {
                    registry.store(conversation_id.clone(), target.clone(), identity.clone());
                    registry.set_lifecycle(
                        conversation_id.clone(),
                        AgentLifecycle::TurnCompleted {
                            session: identity.clone(),
                        },
                    );
                }
                super::SubscriptionEvent::Error {
                    message,
                    recoverable,
                    session: identity,
                } => {
                    let lifecycle = if is_authentication_failure(message) {
                        registry.clear_session_identity(&conversation_id);
                        AgentLifecycle::NotSignedIn {
                            agent: target.installation.agent,
                        }
                    } else if *recoverable {
                            AgentLifecycle::RecoverableError {
                                message: message.clone(),
                                session: identity.clone(),
                            }
                        } else {
                            AgentLifecycle::SessionEnded
                        };
                    registry.set_lifecycle(conversation_id.clone(), lifecycle);
                }
                super::SubscriptionEvent::ToolOutput { .. }
                | super::SubscriptionEvent::Diff(_)
                | super::SubscriptionEvent::Usage(_) => {}
            }
            let pending_approval = match &event {
                super::SubscriptionEvent::ApprovalRequested { request_id, .. } => Some((
                    request_id.clone(),
                    registry.register_approval(conversation_id.clone(), request_id.clone()),
                )),
                super::SubscriptionEvent::SessionStarted(_)
                | super::SubscriptionEvent::TextDelta(_)
                | super::SubscriptionEvent::ReasoningDelta(_)
                | super::SubscriptionEvent::ToolStarted { .. }
                | super::SubscriptionEvent::ToolOutput { .. }
                | super::SubscriptionEvent::Diff(_)
                | super::SubscriptionEvent::Usage(_)
                | super::SubscriptionEvent::TurnCompleted { .. }
                | super::SubscriptionEvent::Error { .. } => None,
            };
            let turn_completed = matches!(&event, super::SubscriptionEvent::TurnCompleted { .. });
            if turn_completed {
                end_session(&mut session, &registry, &conversation_id).await;
            }
            for response in adapter.adapt(event.clone()) {
                yield Ok(response);
            }
            if let Some((request_id, approval)) = pending_approval {
                let approval = approval.fuse();
                futures_util::pin_mut!(approval);
                let decision = select! {
                    _ = cancellation => {
                        report_if_error!(with_timeout(
                            "subscription agent cancellation",
                            session.cancel()
                        )
                        .await);
                        mark_cancelled(&registry, &conversation_id);
                        end_session(&mut session, &registry, &conversation_id).await;
                        break;
                    }
                    decision = approval => decision.unwrap_or(super::ApprovalDecision::Deny),
                };
                if let Err(error) = with_timeout(
                    "subscription agent approval response",
                    session.respond_to_approval(&request_id, decision),
                )
                .await
                {
                    registry.set_lifecycle(
                        conversation_id.clone(),
                        runtime_error_lifecycle(
                            &registry,
                            &conversation_id,
                            target.installation.agent,
                            error.to_string(),
                        ),
                    );
                    end_session(&mut session, &registry, &conversation_id).await;
                    yield Err(Arc::new(subscription_api_error(error)));
                    break;
                }
                if decision == super::ApprovalDecision::Cancel {
                    report_if_error!(with_timeout(
                        "subscription agent cancellation",
                        session.cancel()
                    )
                    .await);
                    mark_cancelled(&registry, &conversation_id);
                    end_session(&mut session, &registry, &conversation_id).await;
                    break;
                }
                registry.set_lifecycle(conversation_id.clone(), AgentLifecycle::Responding);
            } else if turn_completed {
                break;
            } else if let super::SubscriptionEvent::Error { message, .. } = event {
                end_session(&mut session, &registry, &conversation_id).await;
                let error = if is_authentication_failure(&message) {
                    AIApiError::Other(SubscriptionAuthenticationError { message }.into())
                } else {
                    AIApiError::Other(anyhow!(message))
                };
                yield Err(Arc::new(error));
                break;
            }
        }
    };
    Ok(Box::pin(stream))
}

fn recoverable_lifecycle(
    registry: &SubscriptionSessionRegistry,
    conversation_id: &str,
    message: String,
) -> AgentLifecycle {
    AgentLifecycle::RecoverableError {
        message,
        session: registry.get(conversation_id).map(|stored| stored.session),
    }
}

fn mark_cancelled(registry: &SubscriptionSessionRegistry, conversation_id: &str) {
    let lifecycle = registry
        .get(conversation_id)
        .map(|stored| AgentLifecycle::TurnCompleted {
            session: stored.session,
        })
        .unwrap_or(AgentLifecycle::Ready);
    registry.set_lifecycle(conversation_id.to_string(), lifecycle);
}

async fn end_session(
    session: &mut SubscriptionSession,
    registry: &SubscriptionSessionRegistry,
    conversation_id: &str,
) {
    registry.clear_approvals(conversation_id);
    if let Err(error) = session.end().await {
        log::warn!("Failed to end subscription-agent process: {error:#}");
    }
}

async fn discover_candidate(
    mut candidate: RuntimeCandidate,
    working_directory: &std::path::Path,
) -> Result<AgentCapability> {
    with_timeout("subscription agent capability discovery", async move {
        candidate.installation.version = query_cli_version(
            &candidate.installation,
            working_directory.to_path_buf(),
            candidate.location.clone(),
        )
        .await?;
        discover_capabilities(
            candidate.installation,
            working_directory.to_path_buf(),
            candidate.location,
        )
        .await
    })
    .await
}

async fn with_timeout<T>(action: &str, future: impl Future<Output = Result<T>>) -> Result<T> {
    match futures_util::future::select(
        Box::pin(future),
        Box::pin(warpui::r#async::Timer::after(Duration::from_secs(15))),
    )
    .await
    {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right((_, _)) => {
            Err(anyhow!("{action} timed out after 15 seconds"))
        }
    }
}

fn prompt_from_inputs(inputs: &[AIAgentInput]) -> Result<String> {
    let prompt = inputs
        .iter()
        .filter_map(AIAgentInput::user_query)
        .collect::<Vec<_>>()
        .join("\n\n");
    if prompt.is_empty() {
        bail!("this in-app action has no prompt supported by the subscription-agent protocol");
    }
    Ok(prompt)
}

fn local_candidates(preferences: &mut RoutePreferences, ctx: &AppContext) -> Vec<RuntimeCandidate> {
    let snapshot = CockpitModel::as_ref(ctx).snapshot();
    if let Some(selected) = CockpitModel::as_ref(ctx).selected_account() {
        if let Some(account) = snapshot
            .accounts
            .iter()
            .find(|usage| usage.account.key == selected)
        {
            preferences.agent = provider_agent(account.account.provider);
            preferences.account_id = Some(account.account.key.clone());
        }
    }
    let mut candidates = Vec::new();
    for (agent, provider, command, default_dir) in local_agent_specs() {
        let Some(executable) = crate::terminal::cli_agent::resolve_cli_executable(command) else {
            continue;
        };
        let accounts: Vec<&AccountUsage> = snapshot
            .accounts
            .iter()
            .filter(|usage| usage.account.provider == provider)
            .collect();
        if preferences.agent == Some(agent) && preferences.account_id.is_none() {
            preferences.account_id = zaplex_cockpit::pick_freest(provider, &snapshot.accounts)
                .map(|usage| usage.account.key.clone());
        }
        if accounts.is_empty() {
            candidates.push(RuntimeCandidate {
                installation: installation(
                    agent,
                    "local",
                    "Local",
                    format!("{}:default", provider.as_str()),
                    "Default subscription".to_string(),
                    None,
                    dirs::home_dir().map(|home| home.join(default_dir)),
                    executable.clone(),
                ),
                location: ProcessLocation::Local,
            });
        } else {
            candidates.extend(accounts.into_iter().map(|usage| RuntimeCandidate {
                installation: installation(
                    agent,
                    "local",
                    "Local",
                    usage.account.key.clone(),
                    usage.account.label.clone(),
                    usage.account.provider_account_id.clone(),
                    Some(usage.account.config_dir.clone()),
                    executable.clone(),
                ),
                location: ProcessLocation::Local,
            }));
        }
    }
    let mut available_agents: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.installation.agent)
        .collect();
    available_agents.sort_by_key(|agent| match agent {
        SubscriptionAgent::ClaudeCode => 0,
        SubscriptionAgent::Codex => 1,
    });
    available_agents.dedup();
    if preferences
        .agent
        .is_some_and(|agent| !available_agents.contains(&agent))
    {
        preferences.agent = None;
        preferences.account_id = None;
        preferences.model_id = None;
        preferences.effort = None;
    }
    if preferences.agent.is_none() && available_agents.len() == 1 {
        preferences.agent = Some(available_agents[0]);
    }
    if let Some(agent) = preferences.agent {
        let account_is_valid = preferences.account_id.as_deref().is_some_and(|account_id| {
            candidates.iter().any(|candidate| {
                candidate.installation.agent == agent
                    && candidate.installation.account.id == account_id
            })
        });
        if !account_is_valid {
            preferences.account_id =
                zaplex_cockpit::pick_freest(agent_provider(agent), &snapshot.accounts)
                    .map(|usage| usage.account.key.clone())
                    .or_else(|| {
                        candidates
                            .iter()
                            .find(|candidate| candidate.installation.agent == agent)
                            .map(|candidate| candidate.installation.account.id.clone())
                    });
        }
    }
    candidates
}

fn remote_candidates(
    host_id: &str,
    use_cached_inventory: bool,
    ctx: &AppContext,
) -> Result<RuntimeCandidates> {
    let daemon = RemoteServerManager::as_ref(ctx)
        .connected_daemons()
        .into_iter()
        .find(|daemon| daemon.host_id == host_id)
        .with_context(|| format!("remote host {host_id} is not connected"))?;
    if !has_feature(&daemon.features, FEATURE_AGENT_ACCOUNT_ROUTING_V1) {
        bail!("remote host cannot verify subscription account identity; update its Zaplex daemon");
    }
    let node_id = daemon
        .registry_node_id
        .clone()
        .context("remote host has no SSH registry identity")?;
    let connection = warp_ssh_manager::with_conn(|database| {
        let connection =
            warp_ssh_manager::SshRepository::get_server_with_resolved_auth(database, &node_id)?
                .ok_or_else(|| warp_ssh_manager::SshRepositoryError::NotFound(node_id.clone()))?;
        Ok(connection)
    })?;
    Ok(RuntimeCandidates::Remote {
        client: daemon.client,
        host_id: host_id.to_string(),
        host_name: daemon.host_label,
        ssh_argv: warp_ssh_manager::ssh_command::build_ssh_args(&connection.server),
        use_cached_inventory,
    })
}

#[cfg(test)]
fn remote_candidates_for_resolved_ssh(
    host_id: &str,
    host_label: &str,
    connection: &warp_ssh_manager::ResolvedSshConnection,
    inventory: &crate::remote_server::proto::AgentAccountInventory,
) -> Result<Vec<RuntimeCandidate>> {
    remote_candidates_for_ssh(
        host_id,
        host_label,
        warp_ssh_manager::ssh_command::build_ssh_args(&connection.server),
        inventory,
        None,
    )
}

fn legacy_ssh_candidates(connection: &InteractiveSshCommand) -> Result<Vec<RuntimeCandidate>> {
    connection
        .host
        .as_deref()
        .filter(|host| !host.is_empty())
        .context("the active SSH session has no reusable host")?;
    bail!(
        "subscription agents on legacy SSH cannot verify the selected remote account; reconnect this host with the Zaplex remote daemon"
    )
}

fn remote_candidates_for_ssh(
    host_id: &str,
    host_name: &str,
    ssh_argv: Vec<String>,
    inventory: &crate::remote_server::proto::AgentAccountInventory,
    preferred_agent: Option<SubscriptionAgent>,
) -> Result<Vec<RuntimeCandidate>> {
    if inventory.schema_version != 1 {
        bail!("remote host returned an unsupported subscription-account inventory");
    }
    if inventory.health != "loaded" {
        bail!("remote subscription-account discovery is incomplete; refresh the host and retry");
    }

    let mut candidates = Vec::new();
    let mut has_unverifiable_default = false;
    for (agent, provider, command, _) in local_agent_specs() {
        let mut matching = inventory.accounts.iter().filter(|account| {
            account.provider == provider.as_str()
                && account.is_default
                && account.health == "loaded"
        });
        let Some(account) = matching.next() else {
            if preferred_agent == Some(agent) {
                bail!(
                    "the selected {} subscription account is unavailable on remote host {host_name}",
                    agent.display_name()
                );
            }
            continue;
        };
        if matching.next().is_some() || account.account_id.trim().is_empty() {
            bail!(
                "remote host {host_name} returned an ambiguous {} subscription account",
                agent.display_name()
            );
        }
        let Some(provider_account_id) = account
            .provider_account_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        else {
            if preferred_agent == Some(agent) {
                bail!(
                    "remote host cannot verify the selected {} subscription account; update its Zaplex daemon or refresh that CLI login",
                    agent.display_name()
                );
            }
            has_unverifiable_default = true;
            continue;
        };
        let account_name = if account.display_label.trim().is_empty() {
            account.email.trim()
        } else {
            account.display_label.trim()
        };
        candidates.push(RuntimeCandidate {
            installation: installation(
                agent,
                host_id,
                host_name,
                account.account_id.clone(),
                if account_name.is_empty() {
                    "Remote default subscription".to_string()
                } else {
                    account_name.to_string()
                },
                Some(provider_account_id.to_string()),
                None,
                PathBuf::from(command),
            ),
            location: ProcessLocation::Remote {
                ssh_argv: ssh_argv.clone(),
            },
        });
    }
    if candidates.is_empty() && has_unverifiable_default {
        bail!(
            "remote host cannot verify its subscription account identity; update its Zaplex daemon or refresh the CLI login"
        );
    }
    Ok(candidates)
}

fn local_agent_specs() -> [(SubscriptionAgent, Provider, &'static str, &'static str); 2] {
    [
        (
            SubscriptionAgent::ClaudeCode,
            Provider::Claude,
            "claude",
            ".claude",
        ),
        (SubscriptionAgent::Codex, Provider::Codex, "codex", ".codex"),
    ]
}

fn provider_agent(provider: Provider) -> Option<SubscriptionAgent> {
    match provider {
        Provider::Claude => Some(SubscriptionAgent::ClaudeCode),
        Provider::Codex => Some(SubscriptionAgent::Codex),
        Provider::Antigravity => None,
    }
}

fn agent_provider(agent: SubscriptionAgent) -> Provider {
    match agent {
        SubscriptionAgent::ClaudeCode => Provider::Claude,
        SubscriptionAgent::Codex => Provider::Codex,
    }
}

fn installation(
    agent: SubscriptionAgent,
    host_id: &str,
    host_name: &str,
    account_id: String,
    account_name: String,
    provider_account_id: Option<String>,
    config_dir: Option<PathBuf>,
    executable: PathBuf,
) -> InstallationIdentity {
    InstallationIdentity {
        agent,
        host: HostIdentity {
            id: host_id.to_string(),
            display_name: host_name.to_string(),
        },
        account: AccountIdentity {
            id: account_id,
            display_name: account_name,
            provider_account_id,
            config_dir,
        },
        executable,
        version: String::new(),
    }
}

fn same_resume_target(previous: &SubscriptionTarget, current: &SubscriptionTarget) -> bool {
    previous.installation.agent == current.installation.agent
        && previous.installation.host.id == current.installation.host.id
        && previous.installation.account.id == current.installation.account.id
}

fn location_for_target(
    target: &SubscriptionTarget,
    candidates: &[RuntimeCandidate],
) -> Option<ProcessLocation> {
    candidates
        .iter()
        .find(|candidate| {
            candidate.installation.agent == target.installation.agent
                && candidate.installation.host.id == target.installation.host.id
                && candidate.installation.account.id == target.installation.account.id
        })
        .map(|candidate| candidate.location.clone())
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
