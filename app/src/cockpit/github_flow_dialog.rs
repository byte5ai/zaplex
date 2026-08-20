//! Native GitHub issue/PR analysis, review, confirmation, and mutation dialog.
//!
//! The routed subscription agent is read-only and returns a typed JSON result.
//! GitHub mutations are built and executed by Zaplex only after this view shows
//! and confirms the exact frozen operation.

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CrossAxisAlignment, Element, Fill as ElementFill, Flex, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, ScrollbarWidth, Stack,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::{
    button::ButtonVariant,
    components::{Coords, UiComponent, UiComponentStyles},
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::cockpit::github_flows::{
    analysis_accounts, automatic_analysis_account, issue_triage_analysis_prompt,
    pull_request_analysis_prompt, quick_issue_analysis_prompt, ConfirmedGitHubOperation,
    GitHubAnalysisAccount, GitHubFlowError, GitHubIssue, GitHubOperation, GitHubPullRequest,
    GitHubTarget, IssueDraft, PrReviewDecision, PrReviewVerdict, RepositoryContext, TriageVerdict,
    FLOW_PR_REVIEW, FLOW_QUICK_ISSUE, FLOW_TRIAGE,
};
use crate::cockpit::model::CockpitModel;
use crate::terminal::cli_agent::CLIAgentInstallModel;
use crate::ui_components::dialog::{dialog_styles, Dialog};

const DIALOG_WIDTH: f32 = 720.;
const DIALOG_MAX_CONTENT_HEIGHT: f32 = 520.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlowKind {
    QuickIssue,
    PullRequestReview,
    IssueTriage,
}

impl FlowKind {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            FLOW_QUICK_ISSUE => Some(Self::QuickIssue),
            FLOW_PR_REVIEW => Some(Self::PullRequestReview),
            FLOW_TRIAGE => Some(Self::IssueTriage),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::QuickIssue => "Draft GitHub issue",
            Self::PullRequestReview => "Review GitHub pull request",
            Self::IssueTriage => "Triage GitHub issue",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AnalysisResult {
    QuickIssue {
        repository: RepositoryContext,
        draft: IssueDraft,
    },
    IssueTriage {
        target: GitHubTarget,
        verdict: TriageVerdict,
    },
    PullRequestReview {
        target: GitHubTarget,
        verdict: PrReviewVerdict,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoticeKind {
    Success,
    Error,
    Info,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MutationKind {
    CreateIssue,
    CommentIssue,
    CloseIssue,
    SubmitReview,
    MergePullRequest,
}

#[derive(Clone, Debug)]
enum DialogState {
    Inert,
    ReadyQuickIssue,
    LoadingTargets,
    ChooseIssue(Vec<GitHubIssue>),
    ChoosePullRequest(Vec<GitHubPullRequest>),
    Analyzing {
        target: String,
        account: String,
    },
    Review {
        result: AnalysisResult,
        notice: Option<(NoticeKind, String)>,
        completed: Vec<MutationKind>,
    },
    Confirming {
        result: AnalysisResult,
        operation: GitHubOperation,
        confirmation: String,
        completed: Vec<MutationKind>,
    },
    Mutating {
        kind: MutationKind,
    },
    Empty(String),
    Error(String),
}

#[derive(Clone, Debug)]
pub(crate) enum GitHubFlowDialogAction {
    Close,
    Retry,
    SelectAccount(Option<String>),
    AnalyzeQuickIssue,
    AnalyzeIssue(u64),
    AnalyzePullRequest(u64),
    RequestMutation(MutationKind),
    ConfirmMutation,
    CancelMutation,
}

pub(crate) enum GitHubFlowDialogEvent {
    Close,
}

pub(crate) struct GitHubFlowDialog {
    repository: Option<RepositoryContext>,
    flow: Option<FlowKind>,
    selected_account_key: Option<String>,
    state: DialogState,
    generation: u64,
    scroll_state: ClippedScrollStateHandle,
}

impl Default for GitHubFlowDialog {
    fn default() -> Self {
        Self {
            repository: None,
            flow: None,
            selected_account_key: None,
            state: DialogState::Inert,
            generation: 0,
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }
}

impl GitHubFlowDialog {
    fn account_label(account: &GitHubAnalysisAccount) -> String {
        let status = if account.over_budget {
            format!(" · {}% usage", account.binding_percent)
        } else if account.working {
            " · busy".to_string()
        } else {
            String::new()
        };
        format!("{} · {}{status}", account.provider.as_str(), account.label)
    }

    pub(crate) fn begin(
        &mut self,
        flow_key: &str,
        repository: RepositoryContext,
        ctx: &mut ViewContext<Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        let Some(flow) = FlowKind::from_key(flow_key) else {
            self.repository = Some(repository);
            self.flow = None;
            self.state = DialogState::Error("This GitHub workflow is unavailable.".to_string());
            ctx.notify();
            return;
        };
        let selected_account_key = CockpitModel::as_ref(ctx)
            .selected_account()
            .map(str::to_string);
        self.selected_account_key = selected_account_key.filter(|selected| {
            self.available_accounts(&*ctx)
                .iter()
                .any(|account| account.key == *selected)
        });
        self.repository = Some(repository);
        self.flow = Some(flow);
        self.start(ctx);
    }

    fn start(&mut self, ctx: &mut ViewContext<Self>) {
        let (Some(flow), Some(repository)) = (self.flow, self.repository.clone()) else {
            self.state = DialogState::Error("The GitHub target is unavailable.".to_string());
            ctx.notify();
            return;
        };
        if let Err(error) = repository.revalidate() {
            self.state = DialogState::Error(error.to_string());
            ctx.notify();
            return;
        }
        match flow {
            FlowKind::QuickIssue => {
                self.state = DialogState::ReadyQuickIssue;
                ctx.notify();
            }
            FlowKind::PullRequestReview | FlowKind::IssueTriage => {
                let generation = self.generation;
                self.state = DialogState::LoadingTargets;
                ctx.notify();
                #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
                ctx.spawn(
                    async move {
                        match flow {
                            FlowKind::PullRequestReview => {
                                crate::cockpit::github_flows::list_pull_requests(&repository)
                                    .await
                                    .map(TargetList::PullRequests)
                            }
                            FlowKind::IssueTriage => {
                                crate::cockpit::github_flows::list_issues(&repository)
                                    .await
                                    .map(TargetList::Issues)
                            }
                            FlowKind::QuickIssue => Err(GitHubFlowError::CommandUnavailable(
                                "The quick-issue workflow does not load GitHub targets."
                                    .to_string(),
                            )),
                        }
                    },
                    move |view, result, ctx| {
                        if view.generation == generation {
                            view.finish_loading_targets(result, ctx);
                        }
                    },
                );
                #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
                {
                    self.state = DialogState::Error(
                        "GitHub workflows require the native local-files build.".to_string(),
                    );
                    ctx.notify();
                }
            }
        }
    }

    fn finish_loading_targets(
        &mut self,
        result: Result<TargetList, GitHubFlowError>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.state = target_list_state(result);
        ctx.notify();
    }

    fn available_accounts(&self, app: &AppContext) -> Vec<GitHubAnalysisAccount> {
        let installed = CLIAgentInstallModel::as_ref(app);
        analysis_accounts(CockpitModel::as_ref(app).snapshot())
            .into_iter()
            .filter(|account| {
                installed.is_cli_agent_installed(crate::cockpit::agent_of(account.provider))
            })
            .collect()
    }

    fn selected_account(&self, app: &AppContext) -> Result<GitHubAnalysisAccount, GitHubFlowError> {
        let accounts = self.available_accounts(app);
        if let Some(selected) = self.selected_account_key.as_deref() {
            return accounts
                .into_iter()
                .find(|account| account.key == selected)
                .ok_or_else(|| {
                    GitHubFlowError::CommandUnavailable(
                        "The selected Claude/Codex account is no longer available.".to_string(),
                    )
                });
        }
        automatic_analysis_account(CockpitModel::as_ref(app).snapshot(), &accounts).ok_or_else(
            || {
                GitHubFlowError::CommandUnavailable(
                    "No healthy installed Claude/Codex subscription account is available."
                        .to_string(),
                )
            },
        )
    }

    fn start_analysis(&mut self, number: Option<u64>, ctx: &mut ViewContext<Self>) {
        let (Some(flow), Some(repository)) = (self.flow, self.repository.clone()) else {
            self.state = DialogState::Error("The GitHub target is unavailable.".to_string());
            ctx.notify();
            return;
        };
        let generation = self.generation;
        let account = match self.selected_account(&*ctx) {
            Ok(account) => account,
            Err(error) => {
                self.state = DialogState::Error(error.to_string());
                ctx.notify();
                return;
            }
        };
        let target_label = number
            .map(|number| format!("{}#{number}", repository.slug))
            .unwrap_or_else(|| repository.slug.clone());
        self.state = DialogState::Analyzing {
            target: target_label,
            account: Self::account_label(&account),
        };
        ctx.notify();
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        ctx.spawn(
            async move {
                let target = number.map(|number| GitHubTarget {
                    repository: repository.clone(),
                    number,
                });
                let raw = match (flow, target.as_ref()) {
                    (FlowKind::QuickIssue, None) => {
                        let prompt = quick_issue_analysis_prompt(&repository);
                        crate::cockpit::github_flows::run_structured_analysis(
                            &repository,
                            &account,
                            &prompt,
                        )
                        .await?
                    }
                    (FlowKind::IssueTriage, Some(target)) => {
                        let detail =
                            crate::cockpit::github_flows::load_issue_detail(target).await?;
                        let prompt = issue_triage_analysis_prompt(target, &detail)?;
                        crate::cockpit::github_flows::run_structured_analysis(
                            &repository,
                            &account,
                            &prompt,
                        )
                        .await?
                    }
                    (FlowKind::PullRequestReview, Some(target)) => {
                        let (detail, diff) =
                            crate::cockpit::github_flows::load_pull_request_analysis_input(target)
                                .await?;
                        let prompt = pull_request_analysis_prompt(target, &detail, &diff)?;
                        crate::cockpit::github_flows::run_structured_analysis(
                            &repository,
                            &account,
                            &prompt,
                        )
                        .await?
                    }
                    (FlowKind::QuickIssue, Some(_))
                    | (FlowKind::IssueTriage, None)
                    | (FlowKind::PullRequestReview, None) => {
                        return Err(GitHubFlowError::TargetChanged {
                            expected: flow.title().to_string(),
                            actual: "missing or unexpected GitHub number".to_string(),
                        });
                    }
                };
                parse_analysis_result(flow, target, repository, &raw)
            },
            move |view, result, ctx| {
                if view.generation != generation {
                    return;
                }
                view.state = match result {
                    Ok(result) => DialogState::Review {
                        result,
                        notice: None,
                        completed: Vec::new(),
                    },
                    Err(error) => DialogState::Error(error.to_string()),
                };
                ctx.notify();
            },
        );
        #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
        {
            let _ = (number, account);
            self.state = DialogState::Error(
                "GitHub workflows require the native local-files build.".to_string(),
            );
            ctx.notify();
        }
    }

    fn operation_for(result: &AnalysisResult, kind: MutationKind) -> Option<GitHubOperation> {
        match (result, kind) {
            (AnalysisResult::QuickIssue { repository, draft }, MutationKind::CreateIssue) => {
                Some(GitHubOperation::CreateIssue {
                    repository: repository.clone(),
                    draft: draft.clone(),
                })
            }
            (AnalysisResult::IssueTriage { target, verdict }, MutationKind::CommentIssue) => {
                verdict
                    .comment
                    .as_ref()
                    .map(|body| GitHubOperation::CommentIssue {
                        target: target.clone(),
                        body: body.clone(),
                    })
            }
            (AnalysisResult::IssueTriage { target, .. }, MutationKind::CloseIssue) => {
                Some(GitHubOperation::CloseIssue {
                    target: target.clone(),
                    comment: None,
                })
            }
            (AnalysisResult::PullRequestReview { target, verdict }, MutationKind::SubmitReview) => {
                Some(GitHubOperation::ReviewPullRequest {
                    target: target.clone(),
                    decision: verdict.decision,
                    body: Some(review_body(verdict)),
                })
            }
            (AnalysisResult::PullRequestReview { target, .. }, MutationKind::MergePullRequest) => {
                Some(GitHubOperation::MergePullRequest {
                    target: target.clone(),
                })
            }
            _ => None,
        }
    }

    fn begin_mutation_confirmation(&mut self, kind: MutationKind, ctx: &mut ViewContext<Self>) {
        let DialogState::Review {
            result, completed, ..
        } = &self.state
        else {
            return;
        };
        if completed.contains(&kind) {
            return;
        }
        let Some(operation) = Self::operation_for(result, kind) else {
            return;
        };
        self.state = DialogState::Confirming {
            result: result.clone(),
            confirmation: operation.confirmation_text(),
            operation,
            completed: completed.clone(),
        };
        ctx.notify();
    }

    fn confirm_mutation(&mut self, ctx: &mut ViewContext<Self>) {
        let DialogState::Confirming {
            result,
            operation,
            confirmation,
            completed,
        } = &self.state
        else {
            return;
        };
        let Some(confirmed) =
            ConfirmedGitHubOperation::confirm(operation.clone(), true, confirmation.as_str())
        else {
            self.state = DialogState::Error(
                "The confirmation no longer matches the GitHub operation.".to_string(),
            );
            ctx.notify();
            return;
        };
        let kind = mutation_kind(operation);
        let generation = self.generation;
        let result = result.clone();
        let completed = completed.clone();
        self.state = DialogState::Mutating { kind };
        ctx.notify();
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        ctx.spawn(
            async move { crate::cockpit::github_flows::execute_confirmed(confirmed).await },
            move |view, execution, ctx| {
                if view.generation != generation {
                    return;
                }
                let mut completed = completed;
                let notice = match execution {
                    Ok(outputs) => {
                        completed.push(kind);
                        let output = outputs
                            .into_iter()
                            .map(|output| output.trim().to_string())
                            .filter(|output| !output.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        (
                            NoticeKind::Success,
                            if output.is_empty() {
                                "GitHub operation completed.".to_string()
                            } else {
                                output
                            },
                        )
                    }
                    Err(error) => (NoticeKind::Error, error.to_string()),
                };
                view.state = DialogState::Review {
                    result,
                    notice: Some(notice),
                    completed,
                };
                ctx.notify();
            },
        );
        #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
        {
            let _ = confirmed;
            self.state = DialogState::Error(
                "GitHub workflows require the native local-files build.".to_string(),
            );
            ctx.notify();
        }
    }

    fn cancel_confirmation(&mut self, ctx: &mut ViewContext<Self>) {
        let DialogState::Confirming {
            result, completed, ..
        } = &self.state
        else {
            return;
        };
        self.state = DialogState::Review {
            result: result.clone(),
            notice: Some((
                NoticeKind::Info,
                "Cancelled — no GitHub change was made.".to_string(),
            )),
            completed: completed.clone(),
        };
        ctx.notify();
    }

    fn text(
        value: impl Into<String>,
        size: f32,
        muted: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        appearance
            .ui_builder()
            .wrappable_text(value.into(), true)
            .with_style(UiComponentStyles {
                font_size: Some(size),
                font_color: Some(if muted {
                    theme.disabled_text_color(theme.surface_1()).into()
                } else {
                    theme.main_text_color(theme.surface_1()).into()
                }),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn button(
        label: impl Into<String>,
        variant: ButtonVariant,
        action: GitHubFlowDialogAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(variant, MouseStateHandle::default())
            .with_centered_text_label(label.into())
            .with_style(UiComponentStyles {
                height: Some(34.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    fn render_accounts(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let accounts = self.available_accounts(app);
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(Self::text("Analyze with", 12., true, appearance));
        row.add_child(Self::button(
            "Automatic",
            if self.selected_account_key.is_none() {
                ButtonVariant::Accent
            } else {
                ButtonVariant::Basic
            },
            GitHubFlowDialogAction::SelectAccount(None),
            appearance,
        ));
        for account in accounts {
            row.add_child(Self::button(
                Self::account_label(&account),
                if self.selected_account_key.as_deref() == Some(account.key.as_str()) {
                    ButtonVariant::Accent
                } else {
                    ButtonVariant::Basic
                },
                GitHubFlowDialogAction::SelectAccount(Some(account.key)),
                appearance,
            ));
        }
        row.finish()
    }

    fn render_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(8.);
        match &self.state {
            DialogState::Inert => {
                column.add_child(Self::text(
                    "No GitHub workflow selected.",
                    13.,
                    true,
                    appearance,
                ));
            }
            DialogState::ReadyQuickIssue => {
                column.add_child(Self::text(
                    "Analyze this repository and draft one issue. No GitHub change is made during analysis.",
                    13.,
                    true,
                    appearance,
                ));
                column.add_child(Self::button(
                    "Analyze repository",
                    ButtonVariant::Accent,
                    GitHubFlowDialogAction::AnalyzeQuickIssue,
                    appearance,
                ));
            }
            DialogState::LoadingTargets => {
                column.add_child(Self::text(
                    "Loading GitHub targets…",
                    13.,
                    false,
                    appearance,
                ));
            }
            DialogState::ChooseIssue(rows) => {
                column.add_child(Self::text("Choose an open issue", 13., true, appearance));
                for row in rows {
                    column.add_child(Self::button(
                        format!("#{} · {}", row.number, row.title),
                        ButtonVariant::Basic,
                        GitHubFlowDialogAction::AnalyzeIssue(row.number),
                        appearance,
                    ));
                }
            }
            DialogState::ChoosePullRequest(rows) => {
                column.add_child(Self::text(
                    "Choose an open pull request",
                    13.,
                    true,
                    appearance,
                ));
                for row in rows {
                    column.add_child(Self::button(
                        format!("#{} · {}", row.number, row.title),
                        ButtonVariant::Basic,
                        GitHubFlowDialogAction::AnalyzePullRequest(row.number),
                        appearance,
                    ));
                }
            }
            DialogState::Analyzing { target, account } => {
                column.add_child(Self::text(
                    format!("Analyzing {target} with {account}…"),
                    13.,
                    false,
                    appearance,
                ));
                column.add_child(Self::text(
                    "This analysis is read-only; command and file-change approvals are denied.",
                    12.,
                    true,
                    appearance,
                ));
            }
            DialogState::Review {
                result,
                notice,
                completed,
            } => {
                if let Some((kind, message)) = notice {
                    column.add_child(Self::text(
                        match kind {
                            NoticeKind::Success => format!("Completed: {message}"),
                            NoticeKind::Error => format!("GitHub error: {message}"),
                            NoticeKind::Info => message.clone(),
                        },
                        13.,
                        false,
                        appearance,
                    ));
                }
                column.add_child(Self::text(result_summary(result), 13., false, appearance));
                column.add_child(self.render_mutation_buttons(result, completed, appearance));
            }
            DialogState::Confirming { confirmation, .. } => {
                column.add_child(Self::text(
                    "Confirm this exact GitHub change",
                    13.,
                    true,
                    appearance,
                ));
                column.add_child(Self::text(confirmation, 13., false, appearance));
            }
            DialogState::Mutating { kind } => {
                column.add_child(Self::text(
                    format!("Running {}…", mutation_label(*kind)),
                    13.,
                    false,
                    appearance,
                ));
            }
            DialogState::Empty(message) => {
                column.add_child(Self::text(message, 13., true, appearance));
            }
            DialogState::Error(message) => {
                column.add_child(Self::text(message, 13., false, appearance));
            }
        }
        column.finish()
    }

    fn render_mutation_buttons(
        &self,
        result: &AnalysisResult,
        completed: &[MutationKind],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.);
        let mut add = |kind: MutationKind, label: &str, variant: ButtonVariant| {
            if !completed.contains(&kind) {
                row.add_child(Self::button(
                    label,
                    variant,
                    GitHubFlowDialogAction::RequestMutation(kind),
                    appearance,
                ));
            }
        };
        match result {
            AnalysisResult::QuickIssue { .. } => add(
                MutationKind::CreateIssue,
                "Create issue",
                ButtonVariant::Accent,
            ),
            AnalysisResult::IssueTriage { verdict, .. } => {
                if verdict
                    .comment
                    .as_ref()
                    .is_some_and(|body| !body.trim().is_empty())
                {
                    add(
                        MutationKind::CommentIssue,
                        "Post comment",
                        ButtonVariant::Accent,
                    );
                }
                if verdict.close {
                    add(MutationKind::CloseIssue, "Close issue", ButtonVariant::Warn);
                }
            }
            AnalysisResult::PullRequestReview { verdict, .. } => {
                add(
                    MutationKind::SubmitReview,
                    match verdict.decision {
                        PrReviewDecision::Approve => "Submit approval",
                        PrReviewDecision::Comment => "Submit comment",
                        PrReviewDecision::RequestChanges => "Request changes",
                    },
                    ButtonVariant::Accent,
                );
                add(
                    MutationKind::MergePullRequest,
                    "Squash merge",
                    ButtonVariant::Warn,
                );
            }
        }
        row.finish()
    }
}

#[derive(Debug)]
enum TargetList {
    Issues(Vec<GitHubIssue>),
    PullRequests(Vec<GitHubPullRequest>),
}

fn target_list_state(result: Result<TargetList, GitHubFlowError>) -> DialogState {
    match result {
        Ok(TargetList::Issues(rows)) if rows.is_empty() => {
            DialogState::Empty("No open issues in this repository.".to_string())
        }
        Ok(TargetList::Issues(rows)) => DialogState::ChooseIssue(rows),
        Ok(TargetList::PullRequests(rows)) if rows.is_empty() => {
            DialogState::Empty("No open pull requests in this repository.".to_string())
        }
        Ok(TargetList::PullRequests(rows)) => DialogState::ChoosePullRequest(rows),
        Err(error) => DialogState::Error(error.to_string()),
    }
}

fn parse_analysis_result(
    flow: FlowKind,
    target: Option<GitHubTarget>,
    repository: RepositoryContext,
    raw: &str,
) -> Result<AnalysisResult, GitHubFlowError> {
    match (flow, target) {
        (FlowKind::QuickIssue, None) => crate::cockpit::github_flows::parse_issue_draft(raw)
            .map(|draft| AnalysisResult::QuickIssue { repository, draft })
            .ok_or_else(|| {
                GitHubFlowError::InvalidOutput(
                    "The analysis did not return a valid issue draft.".to_string(),
                )
            }),
        (FlowKind::IssueTriage, Some(target)) => {
            crate::cockpit::github_flows::parse_triage_verdict(raw)
                .map(|verdict| AnalysisResult::IssueTriage { target, verdict })
                .ok_or_else(|| {
                    GitHubFlowError::InvalidOutput(
                        "The analysis did not return a valid triage verdict.".to_string(),
                    )
                })
        }
        (FlowKind::PullRequestReview, Some(target)) => {
            crate::cockpit::github_flows::parse_pr_review_verdict(raw)
                .map(|verdict| AnalysisResult::PullRequestReview { target, verdict })
                .ok_or_else(|| {
                    GitHubFlowError::InvalidOutput(
                        "The analysis did not return a valid pull-request verdict.".to_string(),
                    )
                })
        }
        (FlowKind::QuickIssue, Some(_))
        | (FlowKind::IssueTriage, None)
        | (FlowKind::PullRequestReview, None) => Err(GitHubFlowError::InvalidOutput(
            "The analysis target did not match the workflow.".to_string(),
        )),
    }
}

fn review_body(verdict: &PrReviewVerdict) -> String {
    let mut body = verdict.summary.clone();
    for comment in &verdict.comments {
        body.push_str(&format!(
            "\n\n- {}:{} — {}",
            comment.path, comment.line, comment.body
        ));
    }
    body
}

fn result_summary(result: &AnalysisResult) -> String {
    match result {
        AnalysisResult::QuickIssue { draft, .. } => format!(
            "Title: {}\nLabels: {}\n\n{}",
            draft.title,
            if draft.labels.is_empty() {
                "none".to_string()
            } else {
                draft.labels.join(", ")
            },
            draft.body,
        ),
        AnalysisResult::IssueTriage { target, verdict } => format!(
            "{}#{}\nType: {}\nPriority: {}\nActionable: {}\nClose recommended: {}{}",
            target.repository.slug,
            target.number,
            verdict.issue_type,
            verdict.priority,
            verdict.actionable,
            verdict.close,
            verdict
                .comment
                .as_deref()
                .map(|comment| format!("\n\nProposed comment:\n{comment}"))
                .unwrap_or_default(),
        ),
        AnalysisResult::PullRequestReview { target, verdict } => format!(
            "{}#{}\nDecision: {:?}\n\n{}",
            target.repository.slug,
            target.number,
            verdict.decision,
            review_body(verdict),
        ),
    }
}

fn mutation_kind(operation: &GitHubOperation) -> MutationKind {
    match operation {
        GitHubOperation::CreateIssue { .. } => MutationKind::CreateIssue,
        GitHubOperation::CommentIssue { .. } => MutationKind::CommentIssue,
        GitHubOperation::CloseIssue { .. } => MutationKind::CloseIssue,
        GitHubOperation::ReviewPullRequest { .. } => MutationKind::SubmitReview,
        GitHubOperation::MergePullRequest { .. } => MutationKind::MergePullRequest,
    }
}

fn mutation_label(kind: MutationKind) -> &'static str {
    match kind {
        MutationKind::CreateIssue => "issue creation",
        MutationKind::CommentIssue => "issue comment",
        MutationKind::CloseIssue => "issue close",
        MutationKind::SubmitReview => "pull-request review",
        MutationKind::MergePullRequest => "pull-request merge",
    }
}

impl Entity for GitHubFlowDialog {
    type Event = GitHubFlowDialogEvent;
}

impl View for GitHubFlowDialog {
    fn ui_name() -> &'static str {
        "GitHubFlowDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let title = self.flow.map(FlowKind::title).unwrap_or("GitHub workflow");
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(12.);
        if matches!(
            &self.state,
            DialogState::ReadyQuickIssue
                | DialogState::LoadingTargets
                | DialogState::ChooseIssue(_)
                | DialogState::ChoosePullRequest(_)
                | DialogState::Empty(_)
                | DialogState::Error(_)
        ) {
            content.add_child(self.render_accounts(appearance, app));
        }
        content.add_child(
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.scroll_state.clone(),
                    self.render_state(appearance),
                    ScrollbarWidth::Auto,
                    theme.disabled_text_color(theme.surface_1()).into(),
                    theme.main_text_color(theme.surface_1()).into(),
                    ElementFill::None,
                )
                .with_overlayed_scrollbar()
                .finish(),
            )
            .with_max_height(DIALOG_MAX_CONTENT_HEIGHT)
            .finish(),
        );
        let content = content.finish();

        let close = Self::button(
            "Close",
            ButtonVariant::Basic,
            GitHubFlowDialogAction::Close,
            appearance,
        );
        let mut dialog = Dialog::new(
            title.to_string(),
            self.repository
                .as_ref()
                .map(|repository| repository.slug.clone()),
            UiComponentStyles {
                width: Some(DIALOG_WIDTH),
                padding: Some(Coords::uniform(24.)),
                ..dialog_styles(appearance)
            },
        )
        .with_child(content)
        .with_bottom_row_child(close);
        match &self.state {
            DialogState::Empty(_) | DialogState::Error(_) => {
                dialog = dialog.with_bottom_row_child(Self::button(
                    "Retry",
                    ButtonVariant::Accent,
                    GitHubFlowDialogAction::Retry,
                    appearance,
                ));
            }
            DialogState::Confirming { .. } => {
                dialog = dialog
                    .with_bottom_row_child(Self::button(
                        "Cancel",
                        ButtonVariant::Basic,
                        GitHubFlowDialogAction::CancelMutation,
                        appearance,
                    ))
                    .with_bottom_row_child(Self::button(
                        "Confirm",
                        ButtonVariant::Warn,
                        GitHubFlowDialogAction::ConfirmMutation,
                        appearance,
                    ));
            }
            DialogState::Inert
            | DialogState::ReadyQuickIssue
            | DialogState::LoadingTargets
            | DialogState::ChooseIssue(_)
            | DialogState::ChoosePullRequest(_)
            | DialogState::Analyzing { .. }
            | DialogState::Review { .. }
            | DialogState::Mutating { .. } => {}
        }
        let dialog = Container::new(dialog.build().finish())
            .with_margin_top(35.)
            .finish();
        let mut stack = Stack::new();
        stack.add_positioned_child(
            dialog,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );
        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl TypedActionView for GitHubFlowDialog {
    type Action = GitHubFlowDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GitHubFlowDialogAction::Close => ctx.emit(GitHubFlowDialogEvent::Close),
            GitHubFlowDialogAction::Retry => self.start(ctx),
            GitHubFlowDialogAction::SelectAccount(account_key) => {
                if matches!(
                    &self.state,
                    DialogState::ReadyQuickIssue
                        | DialogState::LoadingTargets
                        | DialogState::ChooseIssue(_)
                        | DialogState::ChoosePullRequest(_)
                        | DialogState::Empty(_)
                        | DialogState::Error(_)
                ) {
                    self.selected_account_key = account_key.clone();
                    ctx.notify();
                }
            }
            GitHubFlowDialogAction::AnalyzeQuickIssue => {
                if matches!(&self.state, DialogState::ReadyQuickIssue) {
                    self.start_analysis(None, ctx);
                }
            }
            GitHubFlowDialogAction::AnalyzeIssue(number) => {
                if matches!(
                    &self.state,
                    DialogState::ChooseIssue(rows)
                        if rows.iter().any(|row| row.number == *number)
                ) {
                    self.start_analysis(Some(*number), ctx);
                }
            }
            GitHubFlowDialogAction::AnalyzePullRequest(number) => {
                if matches!(
                    &self.state,
                    DialogState::ChoosePullRequest(rows)
                        if rows.iter().any(|row| row.number == *number)
                ) {
                    self.start_analysis(Some(*number), ctx);
                }
            }
            GitHubFlowDialogAction::RequestMutation(kind) => {
                self.begin_mutation_confirmation(*kind, ctx)
            }
            GitHubFlowDialogAction::ConfirmMutation => self.confirm_mutation(ctx),
            GitHubFlowDialogAction::CancelMutation => self.cancel_confirmation(ctx),
        }
    }
}

#[cfg(test)]
#[path = "github_flow_dialog_tests.rs"]
mod tests;
