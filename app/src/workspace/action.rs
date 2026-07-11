use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use warp_util::path::LineAndColumnArg;

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::auth::LoginGatedFeature;
use crate::drive::items::WarpDriveItemId;
use crate::drive::ObjectTypeAndId;
use crate::palette::PaletteMode;
use crate::prompt::editor_modal::OpenSource as PromptEditorOpenSource;
use crate::search;
use crate::server::ids::SyncId;
use crate::server::telemetry::{AddTabWithShellSource, AgentModeEntrypoint, PaletteSource};
use crate::settings_view::{SettingsAction as SettingsTabAction, SettingsSection};
use crate::tab::{NewSessionMenuItem, SelectedTabColor};
use crate::tab_configs::TabConfig;
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::CLIAgent;
use crate::terminal::view::inline_banner::ZeroStatePromptSuggestionType;
use crate::themes::theme::AnsiColorIdentifier;
use crate::themes::theme_chooser::ThemeChooserMode;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::PaneViewLocator;

use ui_components::lightbox;
use warpui::accessibility::AccessibilityVerbosity;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::platform::Cursor;
use warpui::{EntityId, WindowId};

use super::global_actions::{ForkFromExchange, ForkedConversationDestination};
use super::tab_settings::{
    VerticalTabsCompactSubtitle, VerticalTabsDisplayGranularity, VerticalTabsPrimaryInfo,
    VerticalTabsTabItemMode, VerticalTabsViewMode,
};
use super::view::{OnboardingTutorial, WorkspaceBanner};

/// This enum determines how the search query is initialized when opening command search.
#[derive(Clone, Default, Debug)]
pub enum InitContent {
    /// Read the content of the active terminal input, and make that the initial search query.
    #[default]
    FromInputBuffer,
    /// Specify an exact string to initialize the query to.
    Custom(String),
}

/// To initialize command search, we may want to specify a search filter, or the content of the
/// query itself.
#[derive(Clone, Default, Debug)]
pub struct CommandSearchOptions {
    pub filter: Option<search::QueryFilter>,
    pub init_content: InitContent,
}

/// Specifies how to restore a conversation when it's not already open in a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum RestoreConversationLayout {
    /// Restore the conversation into the currently active pane.
    ActivePane,
    /// Restore the conversation in a new split pane.
    SplitPane,
    /// Restore the conversation in a new tab.
    #[default]
    NewTab,
}

#[derive(Debug, Clone, Copy)]
pub enum TabContextMenuAnchor {
    Pointer(Vector2F),
    VerticalTabsKebab,
}

#[derive(Debug, Clone, Copy)]
pub enum VerticalTabsPaneContextMenuTarget {
    ClickedPane(PaneViewLocator),
    ActivePane(PaneViewLocator),
}

impl VerticalTabsPaneContextMenuTarget {
    pub fn locator(self) -> PaneViewLocator {
        match self {
            Self::ClickedPane(locator) | Self::ActivePane(locator) => locator,
        }
    }
}

#[derive(Debug, Clone)]
pub enum WorkspaceAction {
    ActivateTab(usize),
    ActivatePrevTab,
    ActivateNextTab,
    ActivateLastTab,
    CyclePrevSession,
    CycleNextSession,
    MoveActiveTabLeft,
    MoveActiveTabRight,
    MoveTabLeft(usize),
    MoveTabRight(usize),
    RenameTab(usize),
    ResetTabName(usize),
    RenamePane(PaneViewLocator),
    ResetPaneName(PaneViewLocator),
    RenameActiveTab,
    SetActiveTabName(String),
    /// Sets the manual color override for the active tab.
    ///
    /// - `Color(_)` — apply that color.
    /// - `Cleared` — explicitly clear (suppresses any directory default).
    /// - `Unset` — remove the manual override (lets the directory default apply, if any).
    SetActiveTabColor(SelectedTabColor),
    ToggleTabRightClickMenu {
        tab_index: usize,
        anchor: TabContextMenuAnchor,
    },
    ToggleVerticalTabsPaneContextMenu {
        tab_index: usize,
        target: VerticalTabsPaneContextMenuTarget,
        position: Vector2F,
    },
    TabHoverWidthStart {
        width: f32,
    },
    TabHoverWidthEnd,
    ToggleTabBarOverflowMenu,
    ToggleWelcomeTips,
    CloseTab(usize),
    CloseActiveTab,
    CloseOtherTabs(usize),
    CloseNonActiveTabs,
    CloseTabsRight(usize),
    CloseTabsRightActiveTab,
    AddDefaultTab,
    AddTerminalTab {
        hide_homepage: bool,
    },
    /// Open a new terminal pane in the center of the current tab and execute `ssh user@host` (openWarp-specific).
    /// Triggered by the Connect button in SshServerView / right-click "Connect" in SshManagerPanel.
    OpenSshTerminal {
        node_id: String,
        server: warp_ssh_manager::SshServerInfo,
    },
    /// Open a terminal on a registered SSH host given only its `node_id` — the
    /// handler resolves the `SshServerInfo` from the registry. Dispatched by the
    /// Conductor spine when a registered host row (with no live agent) is clicked,
    /// so the caller need not carry the server info.
    OpenSshTerminalByNode {
        node_id: String,
    },
    /// Toggle a curated favorite (design §10): the ★ affordance on a Conductor
    /// tree node, and the add-favorite picker. Adds the typed tree-object pointer
    /// if absent, removes it if present. `label` is presentation-only.
    ToggleFavorite {
        kind: zaplex_cockpit::FavoriteKind,
        target: String,
        label: String,
    },
    /// Remove a curated favorite by `(kind, target)` — the one-click remove on a
    /// stale dropdown entry whose target has vanished from the tree.
    RemoveFavorite {
        kind: zaplex_cockpit::FavoriteKind,
        target: String,
    },
    /// Open the SSH-manager editor for a registered host, given its `node_id` —
    /// the Conductor spine's per-host "⋯ manage" affordance (design §10 folds the
    /// SSH-manager add/edit function onto the host nodes).
    ManageSshHost {
        node_id: String,
    },
    /// Create a blank registered SSH host and open its editor — the Conductor
    /// spine's "＋ Add host" root (same path as the SSH-manager's "New server").
    AddSshHost,
    /// A remote directory was picked in the SFTP browser (opened in pick mode from
    /// the spawn card's "Browse…"): fill the spawn card's remote-dir field with
    /// `path`, re-show the card, and close the picker (#105).
    RemoteSpawnDirPicked {
        path: std::path::PathBuf,
    },
    /// The SFTP directory picker was closed without a pick (cancel / pane close):
    /// re-show the hidden spawn card unchanged so its selections aren't stranded
    /// (#105).
    RemoteSpawnDirPickCanceled,
    /// Open the local file-manager pane (FM pane-mode P1) rooted at `start_path`.
    OpenLocalFileManager {
        start_path: std::path::PathBuf,
    },
    /// Open/close the SSH Manager view on the left panel (openWarp-specific).
    ToggleSshManager,
    /// Open/close the Skill Manager view on the left panel (openWarp-specific).
    ToggleSkillManager,
    AddTabWithShell {
        shell: AvailableShell,
        source: AddTabWithShellSource,
    },
    AddGetStartedTab,
    /// Add a new tab that immediately enters agent view with a new conversation.
    AddAgentTab,
    /// Add a new terminal tab and launch a specific CLI agent.
    AddSpecificAgentTab(CLIAgent),
    /// Add a new tab running a local Docker sandbox via `sbx`.
    AddDockerSandboxTab,
    OpenNewSessionMenu {
        position: Vector2F,
    },
    ToggleTabConfigsMenu,
    ToggleNewSessionMenu {
        position: Vector2F,
        is_vertical_tabs: bool,
    },
    SelectNewSessionMenuItem(NewSessionMenuItem),
    AutoupdateFailureLink,
    ApplyUpdate,
    // Decentralized branch: `LogOut` has been removed.
    CopyVersion(&'static str),
    DownloadNewVersion,
    ConfigureKeybindingSettings {
        keybinding_name: Option<String>,
    },
    ShowSettings,
    ShowSettingsPage(SettingsSection),
    ShowSettingsPageWithSearch {
        search_query: String,
        section: Option<SettingsSection>,
    },
    ShowThemeChooser(ThemeChooserMode),
    ShowThemeChooserForActiveTheme,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    IncreaseZoom,
    DecreaseZoom,
    ResetZoom,
    ActivateTabByNumber(usize),
    OpenPalette {
        mode: PaletteMode,
        source: PaletteSource,
        query: Option<String>,
    },
    TogglePalette {
        mode: PaletteMode,
        source: PaletteSource,
    },
    // Decentralized branch: `ShowUpgrade` / `ShowReferralSettingsPage` have been removed.
    JoinSlack,
    ViewUserDocs,
    ViewLatestChangelog,
    ViewPrivacyPolicy,
    SendFeedback,
    /// Open the log directory in the system file explorer with the current log file selected.
    #[cfg(not(target_family = "wasm"))]
    ViewLogs,
    /// Prompt the user with a native save-file dialog and write the log
    /// bundle (recent logs + MCP / update logs + diagnostic manifest) to
    /// the chosen path. Used by the "Export logs" link on the About page.
    #[cfg(not(target_family = "wasm"))]
    ExportLogsToPath,
    ChangeCursor(Cursor),
    ToggleBlockSnackbar,
    ToggleErrorUnderlining,
    ToggleSyntaxHighlighting,
    CheckForUpdate,
    ExportAllWarpDriveObjects,
    SetA11yVerbosityLevel(AccessibilityVerbosity),
    ToggleNotifications,
    ToggleTabColor {
        color: AnsiColorIdentifier,
        tab_index: usize,
    },
    OpenLaunchConfigSaveModal,
    SelectTabConfig(TabConfig),
    DispatchToSettingsTab(SettingsTabAction),
    ToggleResourceCenter,
    ToggleUserMenu,
    ToggleAIAssistant,
    ClickedAIAssistantIcon,
    ToggleKeybindingsPage,
    ShowCommandSearch(CommandSearchOptions),
    CreatePersonalNotebook,
    ImportToPersonalDrive,
    CreatePersonalWorkflow,
    CreatePersonalFolder,
    CreatePersonalEnvVarCollection,
    CreatePersonalAIPrompt,
    ToggleMouseReporting,
    ToggleScrollReporting,
    ToggleFocusReporting,
    StartTabDrag,
    DragTab {
        tab_index: usize,
        tab_position: RectF,
    },
    DropTab,
    /// Toggles the left panel. In Code Mode V1 this toggles Zaplex Drive.
    /// In Code Mode V2 this toggles the left panel which contains both the project explorer and
    /// Zaplex Drive. This happens as explicit action from the user.
    ToggleLeftPanel,
    /// Toggles directly to the Zaplex Drive tab of the left panel in Code Mode V2
    ToggleWarpDrive,
    /// Unconditionally opens Zaplex Drive. This is used in the case of user lifecycle
    /// events like new user onboarding or when the user joins a team.
    ZaplexDrive,
    /// Toggles the right panel. This happens as an explicit action from the user.
    ToggleRightPanel,
    /// Opens the code review panel (right panel) without toggling. If already open,
    /// switches to the target pane's repo. Used by vertical tabs diff stats chip.
    OpenCodeReviewPanel(PaneViewLocator),
    /// Toggles the vertical tabs panel. This happens as an explicit action from the user.
    ToggleVerticalTabsPanel,
    ToggleVerticalTabsSettingsPopup,
    SetVerticalTabsDisplayGranularity(VerticalTabsDisplayGranularity),
    SetVerticalTabsTabItemMode(VerticalTabsTabItemMode),
    SetVerticalTabsViewMode(VerticalTabsViewMode),
    SetVerticalTabsPrimaryInfo(VerticalTabsPrimaryInfo),
    SetVerticalTabsCompactSubtitle(VerticalTabsCompactSubtitle),
    ToggleVerticalTabsShowPrLink,
    ToggleVerticalTabsShowDiffStats,
    ToggleVerticalTabsShowDetailsOnHover,
    /// Closes the focused panel. This happens as an explicit action from the user.
    ClosePanel,
    CopyTextToClipboard(String),
    /// An action only registered in dev and local builds, which writes the user's current access
    /// token to the system clipboard to aid debugging and development.
    CopyAccessTokenToClipboard,
    DismissWorkspaceBanner(WorkspaceBanner),
    /// An action only registered in dev and local builds, which crashes the app immediately when called.
    Crash,
    /// An action only registered in dev and local builds, which triggers a
    /// panic immediately when called.
    Panic,
    /// Stops the heap profiler (if one is running) and writes the profiling
    /// data to disk.
    DumpHeapProfile,
    ShowAIAssistantWarmWelcome,
    ClickedAIAssistantWarmWelcome,
    /// An action to open a new window with a view hierarchy debugger.
    OpenViewTreeDebugWindow,
    DismissAIAssistantWarmWelcome,
    /// An action to either upgrade syncing status from none or just in one tab
    /// to syncing all tabs, or downgrade from syncing all tabs to no syncing
    ToggleSyncAllTerminalInputsInAllTabs,
    /// An action to either cancel syncing
    /// or switch from no syncing/syncing all tabs to syncing within one tab
    ToggleSyncTerminalInputsInTab,
    /// An action to force terminal input syncing off
    DisableTerminalInputSync,
    HandleConflictingWorkflow(SyncId),
    HandleConflictingEnvVarCollection(SyncId),
    OpenPromptEditor {
        open_source: PromptEditorOpenSource,
    },
    OpenAgentToolbarEditor,
    OpenCLIAgentToolbarEditor,
    OpenHeaderToolbarEditor,
    ShowHeaderToolbarContextMenu {
        position: Vector2F,
    },
    // Decentralized branch: `Reauth` / `SignupAnonymousUser` / `SignInAnonymousWebUser` have been removed.
    OpenLink(String),
    /// On WASM, opens a given URL in the desktop Zaplex app (if installed) or redirects to download page.
    #[cfg(target_family = "wasm")]
    OpenLinkOnDesktop(url::Url),
    ReopenClosedSession,
    AddWindow,
    AddWindowWithShell {
        shell: AvailableShell,
    },
    /// Moves focus to the panel on the left
    FocusLeftPanel,
    /// Moves focus to the panel on the right
    FocusRightPanel,
    /// An action to view a newly created/edited workflow in WD from the toast
    ViewObjectInWarpDrive(WarpDriveItemId),
    UndoTrash(ObjectTypeAndId),
    /// Open a local path in the file explorer.
    OpenInExplorer {
        path: PathBuf,
    },
    /// Open a local file with the system's default application.
    OpenFilePath {
        path: PathBuf,
    },
    TerminateApp,
    CloseWindow,
    /// Help the user call the Zaplex executable with the [`crate::args::DEBUG_DUMP_FLAG`].
    DumpDebugInfo,
    /// Log review comment send eligibility for panes in the active tab.
    LogReviewCommentSendStatusForActiveTab,
    ToggleRecordingMode,
    ToggleInBandGenerators,
    ToggleDebugNetworkStatus,
    ToggleShowMemoryStats,
    RunAISuggestedCommand(String),
    RunCommand(String),
    InsertInInput {
        content: String,
        replace_buffer: bool,
        /// Whether to ensure agent mode is enabled when inserting content
        ensure_agent_mode: bool,
    },
    /// Open a new tab with its input in AI mode.
    NewTabInAgentMode {
        /// The entrypoint that triggered this action.
        entrypoint: AgentModeEntrypoint,
        /// The type of zero state prompt suggestion to start with (optional).
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
    },
    /// Open a new pane with its input in AI mode.
    NewPaneInAgentMode {
        /// The entrypoint that triggered this action.
        entrypoint: AgentModeEntrypoint,
        /// The type of zero state prompt suggestion to start with (optional).
        zero_state_prompt_suggestion_type: Option<ZeroStatePromptSuggestionType>,
    },
    // Decentralized branch: `AttemptLoginGatedAIUpgrade` has been removed.
    /// Dismisses the Wayland crash recovery banner and opens a link to our docs page with more
    /// information.
    #[cfg(target_os = "linux")]
    DismissWaylandCrashRecoveryBannerAndOpenLink,
    /// Open a new pane with its input in AI mode
    /// with query "Fix this" with error name and details from AI summary.
    FixInAgentMode {
        query: String,
    },
    OpenAIFactCollection,
    OpenMCPServerCollection,
    // Zaplex Wave 7-3: `OpenEnvironmentManagementPane` WorkspaceAction physically removed along with ambient-agent UI subsystem.
    ToggleAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    /// Closes all visible AI document panes in the active pane group.
    HideAIDocumentPanes,
    /// Closes any other ai document panes in the active pane group, and opens the specified document_id.
    OpenAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    FocusTerminalViewInWorkspace {
        terminal_view_id: EntityId,
    },
    /// Focus a specific pane by its locator (pane_group_id and pane_id).
    FocusPane(PaneViewLocator),
    /// Start a new AI conversation in a terminal view. This sets the pending query state
    /// to default and focuses the terminal view.
    StartNewConversation {
        terminal_view_id: EntityId,
    },
    /// Jump to the terminal pane of the most recent agent toast
    JumpToLatestToast,
    /// Open a file in a new tab with a code pane
    OpenFileInNewTab {
        full_path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
    },
    OpenNotebook {
        id: SyncId,
    },
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        workflow_source: WorkflowSource,
        workflow_selection_source: WorkflowSelectionSource,
        argument_override: Option<HashMap<String, String>>,
    },
    ScrollToSettingsWidget {
        page: SettingsSection,
        widget_id: &'static str,
    },
    /// Navigate to an existing AI conversation, focusing on its terminal view.
    ///
    /// If the conversation is not in an open pane, restore it based on the layout setting or override.
    RestoreOrNavigateToConversation {
        pane_view_locator: Option<PaneViewLocator>,
        window_id: Option<WindowId>,
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
        /// If provided, use this layout to restore the conversation.
        /// Otherwise, fall back to the user's setting.
        restore_layout: Option<RestoreConversationLayout>,
    },
    /// Fork an existing AI conversation.
    /// Optionally summarizes the conversation after forking and/or sends an initial prompt.
    ForkAIConversation {
        conversation_id: AIConversationId,
        /// When Some, fork from the given response (or exchange if `fork_from_exact_exchange`
        /// is true). When None, fork from the last exchange.
        fork_from_exchange: Option<ForkFromExchange>,
        /// Whether to summarize the conversation after forking.
        summarize_after_fork: bool,
        /// Prompt to use for summarization when `summarize_after_fork` is true.
        summarization_prompt: Option<String>,
        /// Initial prompt to send in the forked conversation (sent after summarization if enabled).
        initial_prompt: Option<String>,
        /// Where to open the forked conversation.
        destination: ForkedConversationDestination,
    },
    /// Fork an existing AI conversation into a new pane and prefill the input with a local
    /// continuation command (selecting all text).
    #[cfg(not(target_family = "wasm"))]
    ContinueConversationLocally {
        conversation_id: AIConversationId,
    },
    /// Insert the /fork slash command into the active terminal's input.
    InsertForkSlashCommand,
    /// Summarize the active AI conversation in the focused pane.
    SummarizeAIConversation {
        prompt: Option<String>,
        /// Optional prompt to send after summarization completes successfully.
        initial_prompt: Option<String>,
    },
    /// Queue a prompt to be sent after the current conversation finishes.
    QueuePromptForConversation {
        prompt: String,
    },
    /// Install the Zaplex CLI command to /usr/local/bin
    #[cfg(target_os = "macos")]
    InstallCLI,
    /// Uninstall the Zaplex CLI command from /usr/local/bin
    #[cfg(target_os = "macos")]
    UninstallCLI,
    UndoRevertInCodeReviewPane {
        window_id: WindowId,
        view_id: EntityId,
    },
    /// Handle a file being renamed in the file tree
    #[cfg(feature = "local_fs")]
    FileRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    /// Handle a file being deleted in the file tree
    #[cfg(feature = "local_fs")]
    FileDeleted {
        path: PathBuf,
    },
    /// Open a repository directory via file picker. The `path` is an `Option` because some
    /// dispatchers don't know the path to open yet (so the Workspace must open the file picker)
    /// and some do, e.g. the GetStartedView. The GetStartedView needs to handle the file picker
    /// because it needs to determine whether or not to close itself based on whether the user
    /// actually selects a file in the file picker or cancels it.
    OpenRepository {
        path: Option<String>,
    },
    /// Open the native folder picker for a repo param in the tab-config modal after the
    /// current interaction cycle finishes.
    OpenTabConfigRepoPicker {
        param_index: usize,
    },
    /// Open a new blank code file in the current tab
    NewCodeFile,
    NavigatePrevPaneOrPanel,
    NavigateNextPaneOrPanel,
    ToggleProjectExplorer,
    ToggleGlobalSearch,
    OpenGlobalSearch,
    ToggleConversationListView,
    /// Reset the AWS Bedrock login banner dismissed state (for debugging).
    #[cfg(debug_assertions)]
    DebugResetAwsBedrockLoginBannerDismissed,
    /// Open the Zaplex Launch Modal (for debugging)
    #[cfg(debug_assertions)]
    OpenZapLaunchModal,
    /// Reset the Zaplex launch modal dismissed state (for debugging)
    #[cfg(debug_assertions)]
    ResetZapLaunchModalState,
    /// Install the opencode-warp plugin from GitHub into the global opencode config.
    #[cfg(debug_assertions)]
    InstallOpenCodeWarpPlugin,
    /// Use a local checkout of the opencode-warp plugin (for testing/development).
    #[cfg(debug_assertions)]
    UseLocalOpenCodeWarpPlugin,
    /// Take a process sample of the app (equivalent to Activity Monitor > Sample Process).
    #[cfg(target_os = "macos")]
    SampleProcess,
    ToggleNotificationMailbox {
        select_first: bool,
    },
    /// Show the rewind confirmation dialog before rewinding an AI conversation
    ShowRewindConfirmationDialog {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Execute the actual rewind after confirmation
    ExecuteRewindAIConversation {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Execute the actual deletion of a conversation after confirmation
    ExecuteDeleteConversation {
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
    },
    /// Open an ambient agent session by joining its shared session.
    /// Used when the sandbox is running or when we need to view a live session.
    OpenAmbientAgentSession {
        task_id: AmbientAgentTaskId,
    },
    /// Load conversation data into a transcript viewer.
    /// Used for persisted view-only conversations.
    OpenConversationTranscriptViewer {
        conversation_id: ServerConversationToken,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
    },
    /// Toggle the conversation transcript details panel (WASM-only).
    #[cfg(target_family = "wasm")]
    ToggleConversationTranscriptDetailsPanel,
    /// Open a full-window lightbox displaying the given images.
    OpenLightbox {
        images: Vec<lightbox::LightboxImage>,
        /// The index of the image to display initially.
        initial_index: usize,
    },
    /// Update a single image in the currently open lightbox.
    UpdateLightboxImage {
        index: usize,
        image: lightbox::LightboxImage,
    },
    StartAgentOnboardingTutorial(OnboardingTutorial),
    ShowSessionConfigModal,
    DismissSessionConfigTabConfigChip,
    /// Start the HOA onboarding flow (for debugging)
    #[cfg(debug_assertions)]
    ShowHoaOnboardingFlow,
    /// Open the "New worktree" modal for creating a reusable worktree tab config.
    OpenNewWorktreeModal,
    /// Open the native folder picker for the repo field in the new-worktree modal.
    OpenNewWorktreeRepoPicker,
    /// Create a new worktree in the given repo using the default worktree tab config.
    /// The branch name is auto-generated.
    OpenWorktreeInRepo {
        repo_path: String,
    },
    SaveCurrentTabAsNewConfig(usize),
    SyncTrafficLights,
    /// Opens a tab config file in the editor and dismisses the associated error toast.
    OpenTabConfigErrorFile {
        path: PathBuf,
        toast_object_id: String,
    },
    /// Sidecar action: set the hovered item as the Cmd+T default.
    TabConfigSidecarMakeDefault {
        mode: crate::settings::ai::DefaultSessionMode,
        tab_config_path: Option<PathBuf>,
        shell: Option<AvailableShell>,
    },
    /// Sidecar action: open the tab config TOML in the user's editor.
    TabConfigSidecarEditConfig {
        path: PathBuf,
    },
    /// Sidecar action: show the remove confirmation dialog for a tab config.
    TabConfigSidecarRemoveConfig {
        name: String,
        path: PathBuf,
    },
    /// Opens the settings.toml file in a code editor pane.
    OpenSettingsFile,
    /// Opens a new agent session to fix settings.toml errors using the modify-settings skill.
    /// Send a fully-formed prompt to the user's own CLI coding agent (opens a
    /// terminal tab running the agent, prompt prefilled in the input box for
    /// review — the "ask my agent about this context" primitive; Oz-repurpose
    /// design §4). `agent: None` resolves via the installed-set rule.
    AskAgent {
        prompt: String,
        agent: Option<crate::terminal::cli_agent::CLIAgent>,
    },
    /// Like [`WorkspaceAction::AskAgent`] but the agent is launched **routed to a
    /// subscription** (C4 plexing): open a Claude tab pinned to `config_dir`'s
    /// account (API-key env scrubbed) with `prompt` prefilled and ready to send.
    /// Backs the C5 GitHub instance-flows (Quick-Issue / PR-Review / Triage) so a
    /// background task runs on the freest instance. `config_dir: None` = default
    /// login.
    AskAgentRouted {
        prompt: String,
        config_dir: Option<PathBuf>,
    },
    /// Fork an agent conversation into a NEW session (fork/worktree design §2):
    /// opens a terminal tab in the source session's cwd — or, with
    /// `into_worktree`, in a fresh sibling git worktree of its repo — and runs
    /// the provider's fork command. The original session stays untouched.
    ForkAgentSession {
        agent: CLIAgent,
        session_id: String,
        /// The source session's working directory.
        cwd: PathBuf,
        /// Non-default account config dir for subscription pinning
        /// (`None` = the provider's default login).
        config_dir: Option<PathBuf>,
        /// Isolate the fork's file effects in a fresh sibling worktree.
        into_worktree: bool,
    },
    /// Adopt an idle CLI session in place (cockpit "open = focus" verb): opens a
    /// terminal tab in the session's cwd and *resumes the same session* (no
    /// fork, no new session) via the provider's resume command, pinned to
    /// `config_dir`'s account. See [`CLIAgent::resume_command_pinned`].
    AdoptAgentSession {
        agent: CLIAgent,
        session_id: String,
        /// The session's working directory.
        cwd: PathBuf,
        /// Non-default account config dir for subscription pinning
        /// (`None` = the provider's default login).
        config_dir: Option<PathBuf>,
    },
    /// Run a Claude Code slash command (`/compact`, `/clear`) against a
    /// discovered agent-session (cockpit model-lever, step 8).
    ///
    /// Cockpit sessions are *external* processes — zaplex does not own their
    /// stdin — so the honest mechanism is: **resume the same conversation into a
    /// zaplex-owned tab** (`agent.resume_command_pinned`, exactly like adopt)
    /// and prefill the slash command in that live PTY's input, ready to send.
    /// The command operates on the same session, and the human presses Enter
    /// (in-the-loop, mirroring the review-loop verbs). Local + Claude only:
    /// resume runs a local PTY at `cwd`, and `/compact`/`/clear` are Claude Code
    /// commands.
    SlashCommandSession {
        agent: CLIAgent,
        session_id: String,
        /// The session's working directory.
        cwd: PathBuf,
        /// Non-default account config dir for subscription pinning
        /// (`None` = the provider's default login).
        config_dir: Option<PathBuf>,
        /// The literal slash command to prefill (e.g. `/compact`, `/clear`).
        command: String,
    },
    /// Open a session's conversation transcript (cockpit "◇ log" verb): reads the
    /// session's `.jsonl`, renders it to Markdown, and opens it read-only in a
    /// code/text pane. No regression vs claudeplex/-desktop's transcript view.
    ViewTranscript {
        session_id: String,
        /// The account's config dir under which the transcript lives
        /// (`projects/<mangled-cwd>/<session_id>.jsonl`).
        config_dir: PathBuf,
        /// The session's working directory (for the tab label).
        cwd: PathBuf,
        /// Keep the view following the live transcript: register it so each
        /// cockpit reconcile re-renders + reloads it. `false` = one-shot open.
        watch: bool,
    },
    /// Open a session's **review** view (cockpit "◈ review" verb, step 6): read
    /// the working changes of the session's repo (`git diff HEAD` + untracked),
    /// render them to Markdown, and open them read-only in a code/text pane —
    /// the same pane-opening mechanism [`ViewTranscript`] uses. An empty change
    /// set opens a calm "no changes" state rather than a blank/erroring pane.
    ReviewSession {
        /// Git root of the reviewed session (`SessionSnapshot::project_root`).
        project_root: PathBuf,
        /// Human repo label, for the pane title.
        project_name: String,
    },
    /// Commit an agent's reviewed working changes (cockpit review "commit" verb):
    /// open the reused commit dialog for `project_root` (message editor + the
    /// `git add -A && git commit` / optional chained push+PR the code-review flow
    /// already runs). Never stages/commits without the user confirming in the
    /// dialog.
    CommitReviewChanges {
        project_root: PathBuf,
    },
    /// Open a PR for an agent's reviewed changes (cockpit review "PR" verb):
    /// open the reused create-PR dialog for `project_root` (push the current
    /// branch + `gh pr create`). Requires an `origin` remote; the dialog reports
    /// missing-remote / auth failures as a toast rather than failing silently.
    CreateReviewPr {
        project_root: PathBuf,
    },
    /// Open the calm "Offene Punkte" attention inbox: the in-app, fleet-wide
    /// list of agents in [`SessionState::Waiting`], grouped waiting-first.
    /// Selecting a row jumps to that agent (reuses [`AdoptAgentSession`]). The
    /// deliberate, on-demand counterpart to the passive Dock badge — never a
    /// per-event notification.
    OpenAttentionInbox,
    /// Launch a *fresh* CLI agent routed to a subscription (the C4 "plexing"
    /// launch): open a terminal tab in `cwd` (or the default dir) and run the
    /// agent pinned to `config_dir`'s account (`None` = default login) with the
    /// inherited API-key env scrubbed, so it authenticates via the chosen
    /// subscription. See [`CLIAgent::launch_command_routed`].
    LaunchAgent {
        agent: CLIAgent,
        /// Non-default account config dir for subscription pinning
        /// (`None` = the provider's default login). Local launches only —
        /// a remote host uses its own default account (config dirs are local).
        config_dir: Option<PathBuf>,
        /// Working directory for the new agent (`None` = the default dir).
        cwd: Option<PathBuf>,
        /// SSH host node to launch on (`None` = local). When set, the routed
        /// command runs on that host via the daemon session's startup command.
        node_id: Option<String>,
        /// Model to start the agent with (Spawn-Karte). `None` = provider
        /// default. Claude: `--model <model>`; Codex: `--model <model>`.
        model: Option<String>,
        /// Thinking-effort to start with (Spawn-Karte). `None` = provider
        /// default. Codex: `-c model_reasoning_effort=<effort>`; Claude has no
        /// effort CLI flag, so it is only recorded (launch registry), not passed.
        effort: Option<String>,
    },
    /// Open the Spawn-Karte: the launch card that makes model + effort a real,
    /// visible launch attribute (which model / thinking-effort / account / host /
    /// project you are starting). `host`/`project` pre-scope the card when it is
    /// opened from a Conductor host/project header `+`; both `None` opens it
    /// unscoped (the global "New Agent" entry).
    OpenSpawnCard {
        /// Stable id of the pre-selected host (`None` = local / unscoped). This
        /// is the authoritative scoping key: two connected hosts can share a
        /// display label, so `host` (the label) alone can resolve to the wrong
        /// node. When present, the card resolves the scoped host by this id and
        /// only falls back to `host` (name) when it is `None`.
        host_id: Option<String>,
        /// Pre-selected host label (`None` = local / unscoped). Used for display
        /// and as the resolution fallback when `host_id` is absent.
        host: Option<String>,
        /// Pre-selected project directory (`None` = the default dir).
        project: Option<PathBuf>,
    },
    /// Open a file from the file manager in a code editor pane. `node_id` empty
    /// = a local file (opened directly); non-empty = a remote host — native
    /// remote editing (buffer-sync via the SSH daemon) is a follow-up, so a
    /// remote path is reported rather than silently downloaded.
    OpenFileInEditor {
        node_id: String,
        path: PathBuf,
    },
    FixSettingsWithOz {
        error_description: String,
    },
    /// Attach to a specific agent from the unified Conductor inventory, keyed by
    /// `(host, session_id)` (session ids are unique only within a host). A local
    /// host adopts the session in place (resume, no fork); a remote host's agent
    /// lives on that host, so it's handled honestly (see the workspace handler).
    /// The workspace resolves provider / cwd / config_dir from the live
    /// inventory, so the action stays tiny and is shared by the Conductor
    /// row-click and the `w`-jump.
    AttachFleetSession {
        host: String,
        /// Stable per-daemon `host_id` from the inventory node
        /// ([`zaplex_cockpit::HostNode::host_id`]): `None` for the local host,
        /// `Some(id)` for a remote. Used to resolve the exact inventory node /
        /// daemon by id rather than by the collidable `host` label.
        host_id: Option<String>,
        session_id: String,
        /// Whether `host` is *this* machine, taken from the inventory's explicit
        /// [`zaplex_cockpit::HostNode::is_local`] marker — never re-derived from
        /// `host` label equality. Drives local adopt-in-place vs. the remote
        /// path, so a remote host whose label collides with the local hostname
        /// can't be routed to a local adopt.
        is_local: bool,
    },
    /// The `w`-jump: cycle to the next Waiting agent across the whole fleet (the
    /// Conductor's waiting-first order) and attach it. Cursor + resolution live
    /// on the workspace.
    JumpToNextWaiting,
    /// Guardrails (step 7) "⏸ pause" verb: interrupt (SIGINT) a single Conductor
    /// session in place — graceful, the same signal as Ctrl-C, no confirmation.
    /// A local host signals the pid directly (`libc::kill`); a remote host
    /// attempts the daemon's `RunCommandRequest` (`kill -INT <pid>`) via the
    /// matching connected daemon, and reports honestly (toast) when it can't be
    /// routed. `pid == 0` (unknown) always surfaces a toast, never a silent
    /// no-op.
    StopAgent {
        host: String,
        /// Stable per-daemon `host_id` from the inventory node
        /// ([`zaplex_cockpit::HostNode::host_id`]): `None` for the local host,
        /// `Some(id)` for a remote. The remote path resolves the target daemon
        /// by this id — never by the `host` label — so two daemons sharing a
        /// label can't send a host-local `pid` signal to the wrong machine.
        host_id: Option<String>,
        session_id: String,
        pid: u32,
        /// Whether `host` is *this* machine, from the inventory's explicit
        /// [`zaplex_cockpit::HostNode::is_local`] marker. Drives local
        /// `libc::kill` vs. the daemon path — `pid` is host-local, so this
        /// must never be re-derived from a label comparison (a colliding
        /// remote label would send the signal to the wrong machine).
        is_local: bool,
        /// Session row label (name — dir), for the toast.
        agent_label: String,
    },
    /// Guardrails "⨯ kill" verb: open the kill-confirmation dialog for one
    /// Conductor session. Destructive (SIGKILL) — always confirms first via
    /// [`crate::workspace::agent_guardrail_dialog::AgentGuardrailDialog`]; the
    /// dialog itself dispatches the SIGKILL on confirm (no separate action —
    /// see `Workspace::handle_agent_guardrail_dialog_event`).
    KillAgentRequest {
        host: String,
        /// Stable per-daemon `host_id` from the inventory node
        /// ([`zaplex_cockpit::HostNode::host_id`]); `None` local, `Some(id)`
        /// remote. Threaded through the confirm dialog so the eventual SIGKILL
        /// resolves the target daemon by id, not by the collidable label.
        host_id: Option<String>,
        session_id: String,
        pid: u32,
        /// Whether `host` is *this* machine, from the inventory's explicit
        /// [`zaplex_cockpit::HostNode::is_local`] marker (see [`Self::StopAgent`]).
        /// Threaded through the confirm dialog so the eventual SIGKILL routes
        /// by locality, not by label.
        is_local: bool,
        /// Session row label (name — dir), for the dialog's message.
        agent_label: String,
        project_name: String,
    },
    /// Guardrails fleet-wide control: open the "Stop all N agents?" confirmation
    /// dialog (Conductor pane header). Destructive/broad — always confirms; the
    /// dialog itself interrupts every live agent on confirm.
    StopAllRequest,
}

impl From<&WorkspaceAction> for LoginGatedFeature {
    fn from(val: &WorkspaceAction) -> LoginGatedFeature {
        let _ = val;
        "Unknown reason"
    }
}

impl WorkspaceAction {
    pub fn blocked_for_anonymous_user(&self) -> bool {
        false
    }

    /// Matches what actions require the app state to be saved, and which don't. We match all
    /// actions directly, rather than using _, so we're forced to make a conscious decision for each
    /// of them, rather than following some default.
    pub fn should_save_app_state_on_action(&self) -> bool {
        use WorkspaceAction::*;
        match self {
            #[cfg(not(target_family = "wasm"))]
            ContinueConversationLocally { .. } => true,
            ActivateTab(_)
            | ActivateTabByNumber(_)
            | ActivatePrevTab
            | ActivateNextTab
            | ActivateLastTab
            | CyclePrevSession
            | CycleNextSession
            | MoveActiveTabLeft
            | MoveActiveTabRight
            | MoveTabLeft(_)
            | MoveTabRight(_)
            | DropTab
            | RenameTab(_)
            | ResetTabName(_)
            | RenamePane(_)
            | ResetPaneName(_)
            | RenameActiveTab
            | SetActiveTabName(_)
            | SetActiveTabColor(_)
            | CloseTab(_)
            | CloseActiveTab
            | CloseOtherTabs(_)
            | CloseNonActiveTabs
            | CloseTabsRight(_)
            | CloseTabsRightActiveTab
            | ToggleTabColor { .. }
            | AddDefaultTab
            | AddTerminalTab { .. }
            | OpenSshTerminal { .. }
            | OpenSshTerminalByNode { .. }
            | OpenLocalFileManager { .. }
            | ToggleSshManager
            | ToggleSkillManager
            | AddTabWithShell { .. }
            | AddGetStartedTab
            | AddAgentTab
            | AddSpecificAgentTab(_)
            | AddDockerSandboxTab
            | AddWindow
            | AddWindowWithShell { .. }
            | CloseWindow
            | ScrollToSettingsWidget { .. }
            | NewTabInAgentMode { .. }
            | NewPaneInAgentMode { .. }
            | FixInAgentMode { .. }
            | OpenNotebook { .. }
            | RunWorkflow { .. }
            | OpenFileInNewTab { .. }
            | RestoreOrNavigateToConversation { .. }
            | NewCodeFile
            | ForkAIConversation { .. }
            | SummarizeAIConversation { .. }
            | OpenRepository { .. }
            | SelectTabConfig(_)
            | ToggleVerticalTabsPanel => true, // actions that actually change a state of the state of user's
            // workspace would most likely require a save, so that if the app gets
            // restarted, the user can continue working
            AutoupdateFailureLink
            | ApplyUpdate
            | CopyVersion(_)
            | DownloadNewVersion
            | ConfigureKeybindingSettings { .. }
            | ExportAllWarpDriveObjects
            | ShowSettings
            | ShowSettingsPage(_)
            | ShowSettingsPageWithSearch { .. }
            | ShowThemeChooser(_)
            | ShowThemeChooserForActiveTheme
            | IncreaseFontSize
            | DecreaseFontSize
            | ResetFontSize
            | IncreaseZoom
            | DecreaseZoom
            | ResetZoom
            | OpenPalette { .. }
            | TogglePalette { mode: _, source: _ }
            | JoinSlack
            | ViewUserDocs
            | ViewLatestChangelog
            | ViewPrivacyPolicy
            | SendFeedback
            | ChangeCursor(_)
            | ToggleBlockSnackbar
            | ToggleErrorUnderlining
            | ToggleSyntaxHighlighting
            | OpenLaunchConfigSaveModal
            | ToggleTabRightClickMenu { .. }
            | ToggleVerticalTabsPaneContextMenu { .. }
            | OpenNewSessionMenu { .. }
            | ToggleTabConfigsMenu
            | ToggleNewSessionMenu { .. }
            | SelectNewSessionMenuItem(_)
            | ToggleTabBarOverflowMenu
            | CheckForUpdate
            | SetA11yVerbosityLevel(_)
            | ToggleNotifications
            | DispatchToSettingsTab { .. }
            | ToggleResourceCenter
            | ToggleUserMenu
            | ClickedAIAssistantIcon
            | ToggleAIAssistant
            | ToggleKeybindingsPage
            | ShowCommandSearch(_)
            | ToggleMouseReporting
            | ToggleScrollReporting
            | ToggleFocusReporting
            | ImportToPersonalDrive
            | CreatePersonalNotebook
            | CreatePersonalWorkflow
            | CreatePersonalFolder
            | CreatePersonalEnvVarCollection
            | CreatePersonalAIPrompt
            | OpenInExplorer { .. }
            | DragTab { .. }
            | StartTabDrag
            | ToggleLeftPanel
            | ToggleWarpDrive
            | ZaplexDrive
            | ClosePanel
            | ToggleRightPanel
            | OpenCodeReviewPanel(..)
            | ToggleVerticalTabsSettingsPopup
            | SetVerticalTabsDisplayGranularity(_)
            | SetVerticalTabsTabItemMode(_)
            | SetVerticalTabsViewMode(_)
            | SetVerticalTabsPrimaryInfo(_)
            | SetVerticalTabsCompactSubtitle(_)
            | ToggleVerticalTabsShowPrLink
            | ToggleVerticalTabsShowDiffStats
            | ToggleVerticalTabsShowDetailsOnHover
            | ToggleWelcomeTips
            | CopyTextToClipboard(_)
            | CopyAccessTokenToClipboard
            | OpenTabConfigRepoPicker { .. }
            | OpenNewWorktreeModal
            | OpenNewWorktreeRepoPicker
            | OpenWorktreeInRepo { .. }
            | Crash
            | Panic
            | DumpHeapProfile
            | OpenViewTreeDebugWindow
            | ShowAIAssistantWarmWelcome
            | ClickedAIAssistantWarmWelcome
            | DismissAIAssistantWarmWelcome
            | DismissWorkspaceBanner(..)
            | ToggleSyncAllTerminalInputsInAllTabs
            | ToggleSyncTerminalInputsInTab
            | DisableTerminalInputSync
            | HandleConflictingWorkflow(_)
            | HandleConflictingEnvVarCollection(_)
            | OpenPromptEditor { .. }
            | OpenAgentToolbarEditor
            | OpenCLIAgentToolbarEditor
            | OpenHeaderToolbarEditor
            | ShowHeaderToolbarContextMenu { .. }
            | OpenLink(_)
            | ReopenClosedSession
            | FocusLeftPanel
            | FocusRightPanel
            | DumpDebugInfo
            | LogReviewCommentSendStatusForActiveTab
            | ToggleRecordingMode
            | ToggleInBandGenerators
            | ToggleDebugNetworkStatus
            | ToggleShowMemoryStats
            | RunAISuggestedCommand { .. }
            | RunCommand { .. }
            | InsertInInput { .. }
            | InsertForkSlashCommand
            | QueuePromptForConversation { .. }
            | UndoTrash(_)
            | OpenFilePath { .. }
            | ViewObjectInWarpDrive(_)
            | TerminateApp
            | TabHoverWidthStart { .. }
            | TabHoverWidthEnd
            | OpenAIFactCollection
            | OpenMCPServerCollection
            | FocusTerminalViewInWorkspace { .. }
            | FocusPane(..)
            | StartNewConversation { .. }
            | UndoRevertInCodeReviewPane { .. }
            | JumpToLatestToast
            | NavigatePrevPaneOrPanel
            | NavigateNextPaneOrPanel
            | ToggleProjectExplorer
            | ToggleGlobalSearch
            | OpenGlobalSearch
            | ToggleConversationListView
            | ToggleNotificationMailbox { .. }
            | ToggleAIDocumentPane { .. }
            | HideAIDocumentPanes
            | OpenAIDocumentPane { .. }
            | ShowRewindConfirmationDialog { .. }
            | ExecuteRewindAIConversation { .. }
            | ExecuteDeleteConversation { .. }
            | OpenAmbientAgentSession { .. }
            | OpenConversationTranscriptViewer { .. }
            | OpenLightbox { .. }
            | UpdateLightboxImage { .. }
            | StartAgentOnboardingTutorial(_)
            | ShowSessionConfigModal
            | DismissSessionConfigTabConfigChip
            | SaveCurrentTabAsNewConfig(_)
            | SyncTrafficLights
            | OpenTabConfigErrorFile { .. }
            | TabConfigSidecarMakeDefault { .. }
            | TabConfigSidecarEditConfig { .. }
            | TabConfigSidecarRemoveConfig { .. }
            | OpenSettingsFile
            | AskAgent { .. }
            | AskAgentRouted { .. }
            | ForkAgentSession { .. }
            | AdoptAgentSession { .. }
            | SlashCommandSession { .. }
            | OpenAttentionInbox
            | ViewTranscript { .. }
            | ReviewSession { .. }
            | CommitReviewChanges { .. }
            | CreateReviewPr { .. }
            | LaunchAgent { .. }
            | OpenSpawnCard { .. }
            | OpenFileInEditor { .. }
            | AttachFleetSession { .. }
            | JumpToNextWaiting
            | StopAgent { .. }
            | KillAgentRequest { .. }
            | StopAllRequest
            | FixSettingsWithOz { .. } => false,
            #[cfg(debug_assertions)]
            ShowHoaOnboardingFlow => false,
            #[cfg(target_family = "wasm")]
            ToggleConversationTranscriptDetailsPanel => false,
            #[cfg(debug_assertions)]
            DebugResetAwsBedrockLoginBannerDismissed
            | OpenZapLaunchModal
            | ResetZapLaunchModalState
            | InstallOpenCodeWarpPlugin
            | UseLocalOpenCodeWarpPlugin => false,
            #[cfg(not(target_family = "wasm"))]
            ViewLogs => false,
            #[cfg(not(target_family = "wasm"))]
            ExportLogsToPath => false,
            #[cfg(target_os = "macos")]
            SampleProcess => false,
            #[cfg(target_os = "macos")]
            InstallCLI | UninstallCLI => false,
            #[cfg(feature = "local_fs")]
            FileRenamed { .. } => false, // File rename doesn't change workspace state
            #[cfg(feature = "local_fs")]
            FileDeleted { .. } => false, // File deletion doesn't change workspace state
            // Zaplex Wave 7-3: `OpenEnvironmentManagementPane` WorkspaceAction physically removed along with ambient-agent UI subsystem.
            #[cfg(target_os = "linux")]
            DismissWaylandCrashRecoveryBannerAndOpenLink => false,
            #[cfg(target_family = "wasm")]
            OpenLinkOnDesktop(_) => false,
            // Favorites live in their own persisted store, not in app state.
            ToggleFavorite { .. } | RemoveFavorite { .. } => false,
            // Managing/adding a host opens an editor pane; the registry itself is
            // persisted separately (SQLite), so this isn't app-state either.
            ManageSshHost { .. } | AddSshHost => false,
            // Fills a transient modal field / re-shows a modal; not app state.
            RemoteSpawnDirPicked { .. } | RemoteSpawnDirPickCanceled => false,
            // actions that are related to updating user settings or
            // managing some ui elements (like closing/opening modals)
            // that don't reflect on actual workspace and don't need to
            // be preserved between restarts.
        }
    }
}

#[cfg(test)]
#[path = "action_tests.rs"]
mod tests;
