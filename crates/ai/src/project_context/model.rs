use anyhow::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use warpui::{Entity, ModelContext, SingletonEntity};

/// Default rule file list. Order = priority (earlier takes precedence); when multiple files
/// exist in the same directory, `RuleAtPath::respected_rule()` returns only the highest-priority one.
///
/// - WARP.md  — project-native convention.
/// - AGENTS.md — community standard (recognized by opencode / Cursor / Cline, etc.).
/// - CLAUDE.md — Claude Code native convention; enables seamless one-click migration for projects from Claude Code.
///
/// To extend with new names, simply adjust this array (insertion position = priority); `RuleAtPath`
/// is implemented as a priority-indexed slot array, no if-else logic needed.
///
/// Defined outside `cfg_if` so paths without the `local_fs` feature compiled (WASM / tests) can reference it.
pub(crate) const RULES_FILE_PATTERN: &[&str] = &["WARP.md", "AGENTS.md", "CLAUDE.md"];

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use repo_metadata::entry::{Entry, FileMetadata};
        use repo_metadata::repository::RepositorySubscriber;
        use repo_metadata::{Repository, DirectoryWatcher, RepositoryUpdate};
        use ignore::gitignore::Gitignore;
        use async_channel::Sender;
        // `instant::Instant` is this repo's global cross-platform (including WASM) convention, replacing
        // `std::time::Instant`. Enforced via disallowed_types in `clippy.toml`.
        use instant::Instant;
        use std::time::{Duration, SystemTime};

        const MAX_SCAN_DEPTH: usize = 3;
        const MAX_FILES_TO_SCAN: usize = 5000;

        // —— Fast-path (aligned with opencode `findUp` pattern) ——
        //
        // Main purpose: After cd into a new git repo, within the time window before async
        // `index_and_store_rules` completes, `pending_context` calls this fast-path synchronously
        // to directly stat + read rule files in cwd and ancestor directories, ensuring
        // AGENTS.md / WARP.md / CLAUDE.md **are never missed due to async races**.
        // Once the normal path (`find_applicable_rules`) is ready, fast-path yields and clears cache.
        //
        // UI responsiveness guarantee:
        //   - Worst case per call: `MAX_WALK_DEPTH * RULES_FILE_PATTERN.len()` metadata ops
        //     + `read_to_string` for hit files (rule files typically a few KB, Windows NTFS < 1ms/file).
        //   - `FAST_PATH_BUDGET` hard time cutoff; timeout returns collected results immediately, never blocks.
        //   - Steady state (no directory changes) does stat only, no re-reading files; any change to mtime / size / parent-dir-mtime triggers rescan.
        const MAX_WALK_DEPTH: usize = 6;
        const FAST_PATH_BUDGET: Duration = Duration::from_millis(20);
    }
}

/// Fast-path cache entry. `stamps` records (path, mtime, size) for hit files;
/// `walked_dir_stamps` records (path, mtime) for traversed directories, used to detect
/// invalidation from "new/deleted/modified rule files in directory". Negative cache indicates
/// the last scan found no rules; subsequent same stamps reuse directly without I/O.
#[cfg(feature = "local_fs")]
#[derive(Clone, Debug)]
struct FastPathEntry {
    rules: Vec<ProjectRule>,
    /// fast-path's "root" — the directory of the **first hit**; cwd itself if all miss.
    /// Used to construct `ProjectRulesResult.root_path`; semantics align with `find_applicable_rules`.
    root_path: PathBuf,
    stamps: Vec<(PathBuf, SystemTime, u64)>,
    walked_dir_stamps: Vec<(PathBuf, SystemTime)>,
}

#[derive(Debug, Default, Clone)]
pub struct ProjectRule {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Default)]
struct RuleAtPath {
    parent_path: PathBuf,
    warp_md: Option<ProjectRule>,
    agents_md: Option<ProjectRule>,
}

impl RuleAtPath {
    fn respected_rule(&self) -> Option<&ProjectRule> {
        self.warp_md.as_ref().or(self.agents_md.as_ref())
    }
}

#[derive(Debug, Default, Clone)]
pub struct ProjectRulesResult {
    pub root_path: PathBuf,
    pub active_rules: Vec<ProjectRule>,
    pub additional_rule_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRulePath {
    pub path: PathBuf,
    pub project_root: PathBuf,
}

struct FindRulesResult {
    /// Rules that are active and should be eagerly applied.
    active_rules: Vec<ProjectRule>,
    /// Rule paths that are currently not active but available to be applied if
    /// a file under its directory is edited.
    available_rule_paths: Vec<String>,
}

#[cfg(feature = "local_fs")]
fn matches_rules_pattern(file_name_str: &str) -> bool {
    for pattern in RULES_FILE_PATTERN {
        if file_name_str.to_lowercase() == pattern.to_lowercase() {
            return true;
        }
    }
    false
}

#[derive(Debug, Default)]
struct ProjectRules {
    rules: Vec<RuleAtPath>,
}

impl ProjectRules {
    /// Finds the set of rules that are active in the given path and the set that are available to be applied.
    fn find_active_or_applicable_rules(&self, path: &Path) -> FindRulesResult {
        let mut active_rules = Vec::new();
        let mut available_rule_paths = Vec::new();

        // Collect all applicable rules (rules in directories that are ancestors of the target path)
        for rule in &self.rules {
            if let Some(respected_rule) = rule.respected_rule() {
                // Check if the rule's directory is an ancestor of or equal to the target path
                if path.starts_with(&rule.parent_path) {
                    active_rules.push(respected_rule.clone());
                } else {
                    available_rule_paths.push(respected_rule.path.to_string_lossy().to_string());
                }
            }
        }

        FindRulesResult {
            active_rules,
            available_rule_paths,
        }
    }

    /// Remove a rule from the set of project rules. This returns the removed rule.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn remove_rule(&mut self, path: &Path) -> Option<ProjectRule> {
        let parent = path.parent()?;
        let file_name = path.file_name().and_then(|name| name.to_str())?;

        let rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent)?;

        if file_name.to_lowercase() == "warp.md" {
            rule.warp_md.take()
        } else if file_name.to_lowercase() == "agents.md" {
            rule.agents_md.take()
        } else {
            None
        }
    }

    /// Upsert a rule to the set of project rules. This will create a new RuleAtPath entry if none exists and update the existing one
    /// otherwise.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn upsert_rule(&mut self, path: &Path, content: String) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };

        let existing_rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent);

        let rule_file = Some(ProjectRule {
            path: path.to_path_buf(),
            content,
        });

        match existing_rule {
            Some(rule) => {
                if file_name.to_lowercase() == "warp.md" {
                    rule.warp_md = rule_file;
                } else if file_name.to_lowercase() == "agents.md" {
                    rule.agents_md = rule_file;
                }
            }
            None => {
                let mut rule = RuleAtPath {
                    parent_path: parent.to_path_buf(),
                    ..Default::default()
                };
                if file_name.to_lowercase() == "warp.md" {
                    rule.warp_md = rule_file;
                } else if file_name.to_lowercase() == "agents.md" {
                    rule.agents_md = rule_file;
                }
                self.rules.push(rule);
            }
        };
    }
}

/// Singleton model that keeps track of mapping between paths and rule files
/// Currently supports WARP.md files, but designed to be extensible
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Debug, Default)]
pub struct ProjectContextModel {
    /// Mapping from directory path to list of rule files found in that directory
    path_to_rules: HashMap<PathBuf, ProjectRules>,
    /// Fast-path synchronous rule cache (aligned with opencode `findUp` pattern).
    ///
    /// Falls back only when `find_applicable_rules` returns None (async index not ready / not under indexed root),
    /// preventing missed AGENTS.md / WARP.md injection when AI request is sent immediately after cd.
    /// Single-threaded access (WarpUI Singleton on main thread), uses `RefCell` instead of locks,
    /// satisfying `pending_context(&self, app: &AppContext)` call pattern with `&self`.
    #[cfg(feature = "local_fs")]
    fast_path_cache: RefCell<HashMap<PathBuf, FastPathEntry>>,
}

#[derive(Default, Debug)]
pub struct RulesDelta {
    pub discovered_rules: Vec<ProjectRulePath>,
    pub deleted_rules: Vec<PathBuf>,
}

/// Events emitted by the ProjectContextModel
pub enum ProjectContextModelEvent {
    /// Emitted when a path has been indexed
    PathIndexed,
    /// Emitted when the known set of rule files changed
    KnownRulesChanged(RulesDelta),
}

impl ProjectContextModel {
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn new_from_persisted(
        persisted_rules: Vec<ProjectRulePath>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg(feature = "local_fs")]
        ctx.spawn(
            async move { Self::read_persisted_rules(persisted_rules).await },
            |me, mut res, ctx| {
                // Zap: Originally, this would call `try_initialize_and_register_watcher` for each persisted root,
                // which internally invokes `DetectedRepositories::detect_possible_git_repo(ProjectRulesIndexing)`
                // to trigger events, having RepoMetadataModel perform full indexing of 6 persisted repos
                // (biggest cold-startup background CPU cost for Zap BYOP).
                //
                // Now only populates in-memory path_to_rules cache, no proactive detect events.
                // When user later cd into a repo via terminal, RepoDetectionSource::TerminalNavigation
                // naturally triggers an independent detect, at which point register_watcher_for_path runs.
                //
                // Practical effect: persisted rules are not watched in real-time until user enters the repo.
                // Cache itself remains usable; AI rule lookups are unaffected.
                res.extend(me.path_to_rules.drain());
                me.path_to_rules = res;
                ctx.emit(ProjectContextModelEvent::PathIndexed);
            },
        );

        Self::default()
    }

    /// Index a path and find all rule files from that path up to the root directory
    /// Returns a list of all rule files found
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn index_and_store_rules(
        &mut self,
        root_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        if self.path_to_rules.contains_key(&root_path) {
            return Ok(());
        }
        #[cfg(feature = "local_fs")]
        {
            let root_clone = root_path.clone();

            ctx.spawn(
                async move { Self::scan_directory_for_rules(&root_path).await },
                move |me, res: Result<ProjectRules>, ctx| match res {
                    Ok(rule_files) => {
                        me.register_watcher_for_path(&root_clone, ctx);

                        // Persist the discovered rules.
                        let delta = RulesDelta {
                            discovered_rules: rule_files
                                .rules
                                .iter()
                                .filter_map(|rule| {
                                    rule.warp_md.as_ref().map(|rule| ProjectRulePath {
                                        project_root: root_clone.clone(),
                                        path: rule.path.clone(),
                                    })
                                })
                                .chain(rule_files.rules.iter().filter_map(|rule| {
                                    rule.agents_md.as_ref().map(|rule| ProjectRulePath {
                                        project_root: root_clone.clone(),
                                        path: rule.path.clone(),
                                    })
                                }))
                                .collect(),
                            deleted_rules: Default::default(),
                        };
                        ctx.emit(ProjectContextModelEvent::KnownRulesChanged(delta));

                        me.path_to_rules.insert(root_clone, rule_files);
                        ctx.emit(ProjectContextModelEvent::PathIndexed);
                    }
                    Err(e) => log::warn!(
                        "Couldn't index rules for path {}: {}",
                        root_clone.display(),
                        e
                    ),
                },
            );
        }

        Ok(())
    }

    // Zap: `try_initialize_and_register_watcher` was originally the entry point to force repo detection
    // on startup from persisted rule paths, leading to RepoMetadataModel full indexing.
    // Removed together with detect call in `new_from_persisted`; now only passive `register_watcher_for_path`
    // path triggered by terminal cd via `RepoDetectionSource::TerminalNavigation`.

    #[cfg(feature = "local_fs")]
    fn register_watcher_for_path(&self, path: &Path, ctx: &mut ModelContext<Self>) {
        let Some(repository_model) =
            DirectoryWatcher::as_ref(ctx).get_watched_directory_for_path(path)
        else {
            return;
        };

        let (repository_update_tx, repository_update_rx) = async_channel::unbounded();
        let start = repository_model.update(ctx, |repo, ctx| {
            repo.start_watching(
                Box::new(ProjectContextRepositorySubscriber {
                    repository_update_tx,
                }),
                ctx,
            )
        });

        let subscriber_id = start.subscriber_id;
        let repository_model_for_cleanup = repository_model.downgrade();
        let path_clone = path.to_path_buf();
        let path_for_log = path_clone.clone();
        ctx.spawn(start.registration_future, move |_, res, ctx| {
            if let Err(err) = res {
                log::warn!(
                    "Failed to start watching repository for rule updates at {}: {err}",
                    path_for_log.display()
                );

                if let Some(repository_model) = repository_model_for_cleanup.upgrade(ctx) {
                    repository_model.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });

        ctx.spawn_stream_local(
            repository_update_rx.clone(),
            move |me, update, ctx| {
                if update.is_empty() {
                    return;
                }

                let existing_rules = me.path_to_rules.remove(&path_clone);
                let repo_path = path_clone.clone();
                if let Some(rules) = existing_rules {
                    let repo_path_for_closure = repo_path.clone();
                    ctx.spawn(
                        async move {
                            Self::process_repository_updates(update, rules, repo_path).await
                        },
                        move |me, (rules, rule_delta), ctx| {
                            ctx.emit(ProjectContextModelEvent::KnownRulesChanged(rule_delta));

                            me.path_to_rules.insert(repo_path_for_closure, rules);
                            ctx.emit(ProjectContextModelEvent::PathIndexed);
                        },
                    );
                }
            },
            |_, _| {},
        );
    }

    pub fn find_applicable_rules(&self, path: &Path) -> Option<ProjectRulesResult> {
        let mut current_path = path.to_owned();
        let mut active_rules = Vec::new();
        let mut available_rule_paths = Vec::new();

        // Find the root path with indexed rules and collect active rules
        let mut found_rules = false;
        loop {
            if let Some(rules) = self.path_to_rules.get(&current_path) {
                let result = rules.find_active_or_applicable_rules(path);

                active_rules = result.active_rules;
                available_rule_paths = result.available_rule_paths;

                found_rules = true;
                break;
            }

            if !current_path.pop() {
                break;
            }
        }

        if !found_rules {
            return None;
        }

        if active_rules.is_empty() && available_rule_paths.is_empty() {
            return None;
        }

        Some(ProjectRulesResult {
            root_path: current_path,
            active_rules,
            additional_rule_paths: available_rule_paths,
        })
    }

    /// Unified entry point for rule lookups: normal path takes priority, fast-path fallback when async index not ready.
    ///
    /// Aligns with opencode's `Instruction.systemPaths()` `findUp` behavior
    /// (`opencode/packages/opencode/src/session/instruction.ts`): stat rule files level by level upward from cwd,
    /// stop on first hit. Normal and fast-path are **mutually exclusive**: when normal path returns Some,
    /// immediately clear the corresponding fast-path cache entry, ensuring that after indexing completes,
    /// all subsequent requests use the normal path 100% (get subdirectory rules + live watcher updates).
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn find_rules_with_fast_path(&self, cwd: &Path) -> Option<ProjectRulesResult> {
        if let Some(found) = self.find_applicable_rules(cwd) {
            #[cfg(feature = "local_fs")]
            {
                // Normal path ready; discard fast-path cache (avoid stale data in subsequent calls).
                self.fast_path_cache.borrow_mut().remove(cwd);
            }
            return Some(found);
        }

        #[cfg(feature = "local_fs")]
        {
            return self.fast_path_lookup(cwd);
        }

        #[allow(unreachable_code)]
        None
    }

    /// Fast-path synchronous lookup + read rule files from cwd and ancestor directories. Called only when normal path returns None.
    ///
    /// Return semantics match `find_applicable_rules`:
    ///   - Some(ProjectRulesResult) with at least 1 active rule
    ///   - None means no rules found (negative cache written; subsequent same stamps skip I/O)
    #[cfg(feature = "local_fs")]
    fn fast_path_lookup(&self, cwd: &Path) -> Option<ProjectRulesResult> {
        // 1) Cache hit path: stat stamps once; if all align, reuse cache (no re-reading files).
        if let Some(entry) = self.fast_path_cache.borrow().get(cwd).cloned() {
            if Self::fast_path_entry_still_valid(&entry) {
                return Self::result_from_fast_path_entry(&entry);
            }
        }

        // 2) Cache miss / invalid: sync scan. Hard cutoff via `FAST_PATH_BUDGET`, UI never blocks.
        let entry = Self::scan_fast_path(cwd);
        let result = Self::result_from_fast_path_entry(&entry);
        self.fast_path_cache
            .borrow_mut()
            .insert(cwd.to_path_buf(), entry);
        result
    }

    /// Synchronously stat + read rule files level by level upward from `start`. Aligns with opencode `findUp`,
    /// but adds dual safeguards `MAX_WALK_DEPTH` + `FAST_PATH_BUDGET` to ensure UI never blocks.
    ///
    /// At each level, take first hit by `RULES_FILE_PATTERN` (WARP.md > AGENTS.md),
    /// aligning with `RuleAtPath::respected_rule()` semantics.
    #[cfg(feature = "local_fs")]
    fn scan_fast_path(start: &Path) -> FastPathEntry {
        let deadline = Instant::now() + FAST_PATH_BUDGET;
        let mut rules = Vec::new();
        let mut stamps = Vec::new();
        let mut walked_dir_stamps = Vec::new();
        let mut first_hit_dir: Option<PathBuf> = None;
        let mut current: PathBuf = start.to_path_buf();

        for _ in 0..MAX_WALK_DEPTH {
            if Instant::now() >= deadline {
                break;
            }

            // Record directory mtime; later can detect "rule files added/deleted in directory" changes.
            if let Ok(meta) = std::fs::metadata(&current) {
                if let Ok(mtime) = meta.modified() {
                    walked_dir_stamps.push((current.clone(), mtime));
                }
            }

            // At this level, look for first rule file by priority. Aligns with RuleAtPath::respected_rule() semantics.
            for filename in RULES_FILE_PATTERN {
                if Instant::now() >= deadline {
                    break;
                }
                let candidate = current.join(filename);
                let Ok(meta) = std::fs::metadata(&candidate) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let Ok(mtime) = meta.modified() else { continue };
                let size = meta.len();
                let Ok(content) = std::fs::read_to_string(&candidate) else {
                    continue;
                };
                if first_hit_dir.is_none() {
                    first_hit_dir = Some(current.clone());
                }
                rules.push(ProjectRule {
                    path: candidate.clone(),
                    content,
                });
                stamps.push((candidate, mtime, size));
                break; // Take only 1 per level
            }

            if !current.pop() {
                break;
            }
        }

        FastPathEntry {
            root_path: first_hit_dir.unwrap_or_else(|| start.to_path_buf()),
            rules,
            stamps,
            walked_dir_stamps,
        }
    }

    /// Cache validity check. Only stat, no file content read.
    /// - Hit file mtime/size unchanged → content reusable
    /// - Traversed directory mtime unchanged → no new/deleted rule files
    ///
    /// With `FAST_PATH_BUDGET` budget; timeout during stat marks as invalid, rescan.
    #[cfg(feature = "local_fs")]
    fn fast_path_entry_still_valid(entry: &FastPathEntry) -> bool {
        let deadline = Instant::now() + FAST_PATH_BUDGET;
        for (path, mtime, size) in &entry.stamps {
            if Instant::now() >= deadline {
                return false;
            }
            let Ok(meta) = std::fs::metadata(path) else {
                return false;
            };
            if meta.len() != *size {
                return false;
            }
            if meta.modified().ok().as_ref() != Some(mtime) {
                return false;
            }
        }
        for (dir, mtime) in &entry.walked_dir_stamps {
            if Instant::now() >= deadline {
                return false;
            }
            let Ok(meta) = std::fs::metadata(dir) else {
                return false;
            };
            if meta.modified().ok().as_ref() != Some(mtime) {
                return false;
            }
        }
        true
    }

    /// Convert FastPathEntry to uniform external `ProjectRulesResult`.
    /// Empty rules returns None; semantics align with `find_applicable_rules`.
    #[cfg(feature = "local_fs")]
    fn result_from_fast_path_entry(entry: &FastPathEntry) -> Option<ProjectRulesResult> {
        if entry.rules.is_empty() {
            return None;
        }
        Some(ProjectRulesResult {
            root_path: entry.root_path.clone(),
            active_rules: entry.rules.clone(),
            additional_rule_paths: Vec::new(),
        })
    }

    #[cfg(feature = "local_fs")]
    async fn process_repository_updates(
        repository_update: RepositoryUpdate,
        mut existing_rules: ProjectRules,
        project_root: PathBuf,
    ) -> (ProjectRules, RulesDelta) {
        let mut rules_delta = RulesDelta::default();
        // Handle deleted files - remove rules for deleted rule files
        for target_file in &repository_update.deleted {
            // Skip gitignored files
            if target_file.is_ignored {
                continue;
            }
            if let Some(file_name_str) = target_file.path.file_name().and_then(|name| name.to_str())
            {
                if matches_rules_pattern(file_name_str) {
                    // Remove the rule from existing rules
                    existing_rules.remove_rule(&target_file.path);
                    rules_delta.deleted_rules.push(target_file.path.clone());

                    log::debug!("Removed rule file: {}", target_file.path.display());
                }
            }
        }

        // Handle moved files - update paths for moved rule files
        for (to_target, from_target) in &repository_update.moved {
            // Skip gitignored files
            if to_target.is_ignored || from_target.is_ignored {
                continue;
            }
            if let Some(file_name_str) = to_target.path.file_name().and_then(|name| name.to_str()) {
                if matches_rules_pattern(file_name_str) {
                    // Find and update the rule with the old path
                    if let Some(rule) = existing_rules.remove_rule(&from_target.path) {
                        // Emit deletion event for old path
                        rules_delta.deleted_rules.push(from_target.path.clone());

                        existing_rules.upsert_rule(&to_target.path, rule.content);

                        // Emit upsert event for new path
                        rules_delta.discovered_rules.push(ProjectRulePath {
                            path: to_target.path.clone(),
                            project_root: project_root.clone(),
                        });

                        log::debug!(
                            "Updated rule file path: {} -> {}",
                            from_target.path.display(),
                            to_target.path.display()
                        );
                    }
                }
            }
        }

        // Handle added/updated files - upsert rules for rule files
        for target_file in repository_update.added_or_modified() {
            // Skip gitignored files
            if target_file.is_ignored {
                continue;
            }
            if let Some(file_name_str) = target_file.path.file_name().and_then(|name| name.to_str())
            {
                if matches_rules_pattern(file_name_str) {
                    // Read the content of the rule file
                    match async_fs::read_to_string(&target_file.path).await {
                        Ok(content) => {
                            existing_rules.upsert_rule(&target_file.path, content);
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to read updated rule file {}: {}",
                                target_file.path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        (existing_rules, rules_delta)
    }

    /// Scan a directory for rule files (currently WARP.md, extensible for future file types)
    /// Uses repo_metadata::entry::build_tree for efficient directory traversal
    #[cfg(feature = "local_fs")]
    async fn scan_directory_for_rules(dir_path: &Path) -> Result<ProjectRules> {
        use repo_metadata::entry::IgnoredPathStrategy;

        let mut rule_files = ProjectRules::default();

        if !async_fs::metadata(dir_path).await?.is_dir() {
            return Ok(rule_files);
        }

        // Use build_tree to collect all files, then filter for rule files
        let mut files = Vec::<FileMetadata>::new();
        let mut gitignores = Vec::<Gitignore>::new();

        // Collect patterns that should not be ignored
        let override_ignore_patterns: Vec<String> =
            RULES_FILE_PATTERN.iter().map(|s| s.to_string()).collect();
        let mut file_limit = MAX_FILES_TO_SCAN;

        // Build the file tree using repo_metadata's build_tree function
        let ignore_behavior = IgnoredPathStrategy::IncludeOnly(override_ignore_patterns.clone());

        let _ = Entry::build_tree(
            dir_path,
            &mut files,
            &mut gitignores,
            Some(&mut file_limit),
            MAX_SCAN_DEPTH,
            0,
            &ignore_behavior,
        )?;

        // Filter files to only include those matching RULES_FILE_PATTERN
        for file_metadata in files {
            let path = &file_metadata.path;
            let file_name = path.file_name();

            if let Some(file_name_str) = file_name {
                if matches_rules_pattern(file_name_str) {
                    // Read the content of the rule file
                    let local_path = file_metadata.path.to_local_path_lossy();
                    let content = match async_fs::read_to_string(&local_path).await {
                        Ok(content) => content,
                        Err(e) => {
                            log::warn!("Failed to read rule file {}: {e}", file_metadata.path,);
                            break;
                        }
                    };

                    rule_files.upsert_rule(&local_path, content);
                }
            }
        }

        Ok(rule_files)
    }

    #[cfg(feature = "local_fs")]
    async fn read_persisted_rules(
        rule_paths: Vec<ProjectRulePath>,
    ) -> HashMap<PathBuf, ProjectRules> {
        let mut rules: HashMap<PathBuf, ProjectRules> = HashMap::new();

        for rule in rule_paths {
            match async_fs::read_to_string(&rule.path).await {
                Ok(content) => {
                    let existing_rules = rules.entry(rule.project_root).or_default();
                    existing_rules.upsert_rule(&rule.path, content);
                }
                Err(e) => {
                    log::debug!(
                        "Failed to read rule file from persistence {}: {}",
                        rule.path.display(),
                        e
                    );
                    // Continue processing other files even if one fails
                }
            }
        }

        rules
    }

    pub fn indexed_rules(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.path_to_rules.values().flat_map(|rules| {
            rules.rules.iter().filter_map(|rules| {
                rules
                    .respected_rule()
                    .map(|project_rule| project_rule.path.clone())
            })
        })
    }

    /// Returns the rule file paths associated with a specific workspace root path.
    pub fn rules_for_workspace(&self, workspace_path: &Path) -> Vec<PathBuf> {
        self.path_to_rules
            .get(workspace_path)
            .into_iter()
            .flat_map(|rules| {
                rules.rules.iter().filter_map(|rule| {
                    rule.respected_rule()
                        .map(|project_rule| project_rule.path.clone())
                })
            })
            .collect()
    }
}

impl Entity for ProjectContextModel {
    type Event = ProjectContextModelEvent;
}

impl SingletonEntity for ProjectContextModel {}

#[cfg(feature = "local_fs")]
struct ProjectContextRepositorySubscriber {
    repository_update_tx: Sender<RepositoryUpdate>,
}

#[cfg(feature = "local_fs")]
impl RepositorySubscriber for ProjectContextRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::prelude::rust_2024::Future<Output = ()> + Send + 'static>> {
        // The model can safely ignore the initial scan because the model only subscribes
        // after the repository is already scanned.
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &repo_metadata::RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> std::pin::Pin<Box<dyn std::prelude::rust_2024::Future<Output = ()> + Send + 'static>> {
        let tx = self.repository_update_tx.clone();
        let update = update.clone();
        Box::pin(async move {
            let _ = tx.send(update).await;
        })
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
