//! Skill resolution for agent runs.
//!
//! This module exists primarily for `warp agent run --skill ...` (and related flows) where we need to
//! resolve a CLI-provided `--skill` specifier (`SkillSpec`) into a concrete `SKILL.md` file and its
//! parsed instruction body.
//!
//! While `SkillManager` maintains a cached view of known skills and can list skills "in scope",
//! agent runs need a single-shot resolver that:
//! - Works when invoked from directories *above* any detected repos (ambient/root-scope case).
//! - Supports qualified specs (`repo:skill` and `org/repo:skill`) and returns good errors on ambiguity.
//! - Applies consistent skill-directory precedence (e.g. `.claude/` vs `.codex/`, etc.).
//! - Falls back to scanning disk when the manager cache has not warmed yet.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use ai::skills::{
    home_skills_path, parse_skill, ParsedSkill, SkillProvider, SKILL_PROVIDER_DEFINITIONS,
};
use command::r#async::Command as AsyncCommand;
use warp_cli::skill::SkillSpec;
use warpui::AppContext;
use warpui::SingletonEntity as _;

use super::SkillManager;
use crate::warp_managed_paths_watcher::warp_managed_skill_dirs;

const SKILL_FILE_NAME: &str = "SKILL.md";
const GIT_REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    pub skill_path: PathBuf,
    pub name: String,
    pub instructions: String,
    /// The full parsed skill, used for proto conversion when sending to server.
    pub parsed_skill: ParsedSkill,
}

/// Foreground-only cache state copied into an owned value before filesystem
/// resolution moves to the blocking pool.
#[derive(Clone)]
pub struct SkillResolutionSnapshot {
    repository_roots: Vec<PathBuf>,
    all_matching_paths: Vec<PathBuf>,
    home_skill_paths: Vec<PathBuf>,
    current_repo_root: Option<PathBuf>,
    working_dir_skill_paths: Vec<PathBuf>,
    repo_skill_paths: HashMap<PathBuf, Vec<PathBuf>>,
    cached_skills: HashMap<PathBuf, ParsedSkill>,
}

impl SkillResolutionSnapshot {
    pub fn repository_roots(&self) -> &[PathBuf] {
        &self.repository_roots
    }
}

fn resolve_from_skill_dirs_by_directory_scan(
    spec: &SkillSpec,
    skill_dirs: impl IntoIterator<Item = PathBuf>,
) -> Result<Option<ResolvedSkill>, ResolveSkillError> {
    if is_direct_skill_path(spec) {
        return Ok(None);
    }

    for skill_dir in skill_dirs {
        let path = skill_dir.join(&spec.skill_identifier).join(SKILL_FILE_NAME);

        if path.exists() {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
    }

    Ok(None)
}

fn home_skill_dirs_for_resolution() -> Vec<PathBuf> {
    let mut skill_dirs = Vec::new();
    for provider in SKILL_PROVIDER_DEFINITIONS.iter() {
        if provider.provider == SkillProvider::Zaplex {
            for dir in warp_managed_skill_dirs() {
                push_unique_path(&mut skill_dirs, dir);
            }
        } else if let Some(dir) = home_skills_path(provider.provider) {
            push_unique_path(&mut skill_dirs, dir);
        }
    }
    skill_dirs
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveSkillError {
    #[error("Skill '{skill}' not found")]
    NotFound { skill: String },
    #[error("Repository '{repo}' not found")]
    RepoNotFound { repo: String },
    #[error("Skill '{skill}' is ambiguous; specify as repo:skill_name")]
    Ambiguous {
        skill: String,
        candidates: Vec<PathBuf>,
    },
    #[error("Repository '{repo}' found but belongs to org '{found}', expected '{expected}'")]
    OrgMismatch {
        repo: String,
        expected: String,
        found: String,
    },
    #[error("Failed to parse skill file {path}: {message}")]
    ParseFailed { path: PathBuf, message: String },
    #[error("Skill path '{skill}' is not confined to root {root}: {message}")]
    ConfinementFailed {
        skill: String,
        root: PathBuf,
        message: String,
    },
    #[error("Failed to clone repository '{org}/{repo}': {message}")]
    CloneFailed {
        org: String,
        repo: String,
        message: String,
    },
}

/// Resolve a `SkillSpec` (from the `--skill` CLI arg) into a concrete SKILL.md file.
///
/// Resolution flow:
/// - If the spec is repo-qualified (`repo:skill` or `org/repo:skill`):
///   - Find candidate repo roots from `SkillManager` (fallback: a direct child of `working_dir`).
///   - Optionally filter by `org` using the repo's `origin` remote when available.
///   - For each candidate repo root, try to resolve the skill (cache-first, then disk scan).
/// - If the spec is unqualified (`skill`):
///   1. Check cached home-directory skills.
///   2. Scan home/global skill directories directly for cold-start resolution.
///   3. Try the current repo root (if `working_dir` is inside a detected repo).
///   4. Otherwise, search across repos *under* `working_dir` (ambient/root-scope support).
///      - If multiple matches exist, return an ambiguity error.
///   5. Finally, fall back to scanning the filesystem relative to `working_dir`.
///
/// Once a SKILL.md is selected, we return the parsed instruction body (front matter stripped).
pub fn resolve_skill_spec(
    spec: &SkillSpec,
    working_dir: &Path,
    snapshot: &SkillResolutionSnapshot,
    repository_orgs: &HashMap<PathBuf, Option<String>>,
) -> Result<ResolvedSkill, ResolveSkillError> {
    match &spec.repo {
        Some(repo) => resolve_repo_qualified(spec, repo, snapshot, repository_orgs),
        None => resolve_unqualified(spec, working_dir, snapshot),
    }
}

/// Copies all app-owned cache state needed for resolution. The returned value
/// is independent of `AppContext`, so disk scanning and parsing can run on the
/// blocking pool without retaining the foreground executor.
pub fn snapshot_skill_resolution(
    spec: &SkillSpec,
    working_dir: &Path,
    ctx: &AppContext,
) -> SkillResolutionSnapshot {
    let skill_manager = SkillManager::as_ref(ctx);
    let repository_roots = spec
        .repo
        .as_deref()
        .map(|repo| candidate_repo_roots(repo, working_dir, skill_manager))
        .unwrap_or_default();
    let all_matching_paths = skill_manager.skill_paths_by_name(&spec.skill_identifier);
    let home_skill_paths = skill_manager.home_skill_paths();
    let current_repo_root = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
        .get_root_for_path(working_dir);
    let working_dir_skill_paths = skill_manager.skill_paths_in_scope(working_dir);

    let mut roots_for_cache = repository_roots.clone();
    if let Some(current_repo_root) = &current_repo_root {
        push_unique_path(&mut roots_for_cache, current_repo_root.clone());
    }
    let repo_skill_paths = roots_for_cache
        .into_iter()
        .map(|root| {
            let paths = skill_manager.skill_paths_in_scope(&root);
            (root, paths)
        })
        .collect();

    let mut cached_paths = all_matching_paths.clone();
    for path in &home_skill_paths {
        push_unique_path(&mut cached_paths, path.clone());
    }
    for path in &working_dir_skill_paths {
        push_unique_path(&mut cached_paths, path.clone());
    }
    let cached_skills = cached_paths
        .into_iter()
        .filter_map(|path| {
            skill_manager
                .skill_by_path(&path)
                .cloned()
                .map(|skill| (path, skill))
        })
        .collect();

    SkillResolutionSnapshot {
        repository_roots,
        all_matching_paths,
        home_skill_paths,
        current_repo_root,
        working_dir_skill_paths,
        repo_skill_paths,
        cached_skills,
    }
}

/// Reads repository organizations without blocking the app foreground executor.
pub async fn repository_orgs_for_skill_roots(
    repository_roots: Vec<PathBuf>,
) -> HashMap<PathBuf, Option<String>> {
    let lookups = repository_roots
        .into_iter()
        .map(|repository_root| async move {
            let org = get_git_remote_org(&repository_root).await;
            (repository_root, org)
        });
    futures::future::join_all(lookups)
        .await
        .into_iter()
        .collect()
}

/// Clone a repository from GitHub into the working directory for skill resolution.
///
/// Uses HTTPS format: `https://github.com/org/repo.git`
///
/// This is used in sandboxed environments to auto-clone repos when a fully-qualified
/// skill spec references a repo that doesn't exist locally.
pub async fn clone_repo_for_skill(
    org: &str,
    repo: &str,
    working_dir: &Path,
) -> Result<(), ResolveSkillError> {
    let repo_url = format!("https://github.com/{org}/{repo}.git");
    let target_dir = working_dir.join(repo);

    // Check if target already exists.
    if target_dir.exists() {
        if target_dir.join(".git").is_dir() && git_repository_has_head(&target_dir).await {
            log::info!(
                "Target directory {} already contains a complete git repo, skipping clone",
                target_dir.display()
            );
            return Ok(());
        }

        return Err(ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!(
                "Target directory {} already exists but is not a complete git repository",
                target_dir.display()
            ),
        });
    }

    let staging_dir = tempfile::Builder::new()
        .prefix(".zaplex-skill-clone-")
        .tempdir_in(working_dir)
        .map_err(|error| ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!("Failed to create temporary clone directory: {error}"),
        })?;
    let staging_path = staging_dir.path().to_path_buf();

    log::info!("Cloning {} into {}", repo_url, target_dir.display());
    log::debug!(
        "[GIT OPERATION] resolve_skill_spec.rs clone_repo_for_skill git clone {} {}",
        repo_url,
        target_dir.display()
    );

    let mut command = AsyncCommand::new("git");
    command
        .arg("clone")
        .arg(&repo_url)
        .arg(&staging_path)
        .current_dir(working_dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_CLONE_TIMEOUT, command.output())
        .await
        .map_err(|_| ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!("git clone exceeded the {GIT_CLONE_TIMEOUT:?} timeout"),
        })?
        .map_err(|e| ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!("Failed to execute git clone: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: stderr.trim().to_string(),
        });
    }

    let staging_path = staging_dir.keep();
    if let Err(error) = std::fs::rename(&staging_path, &target_dir) {
        let _ = std::fs::remove_dir_all(&staging_path);
        return Err(ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!("Failed to install completed clone: {error}"),
        });
    }

    log::info!("Successfully cloned {org}/{repo}");
    Ok(())
}

async fn git_repository_has_head(repository: &Path) -> bool {
    let mut command = AsyncCommand::new("git");
    command
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--verify", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(GIT_REMOTE_TIMEOUT, command.output()).await,
        Ok(Ok(output)) if output.status.success()
    )
}

fn resolve_repo_qualified(
    spec: &SkillSpec,
    repo: &str,
    snapshot: &SkillResolutionSnapshot,
    repository_orgs: &HashMap<PathBuf, Option<String>>,
) -> Result<ResolvedSkill, ResolveSkillError> {
    let mut candidate_repo_roots = snapshot.repository_roots.clone();
    candidate_repo_roots.retain(|root| root.is_dir());

    if candidate_repo_roots.is_empty() {
        return Err(ResolveSkillError::RepoNotFound {
            repo: repo.to_string(),
        });
    }

    if let Some(expected_org) = &spec.org {
        candidate_repo_roots = filter_candidate_repo_roots_by_org(
            repo,
            expected_org,
            candidate_repo_roots,
            repository_orgs,
        )?;
    }

    for repo_root in candidate_repo_roots {
        match resolve_in_single_repo_root(spec, &repo_root, snapshot) {
            Ok(resolved) => return Ok(resolved),
            Err(ResolveSkillError::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn candidate_repo_roots(
    repo: &str,
    working_dir: &Path,
    skill_manager: &SkillManager,
) -> Vec<PathBuf> {
    let mut candidate_repo_roots: Vec<PathBuf> = skill_manager
        .directories_with_skills()
        .into_iter()
        .filter(|dir| dir.file_name().is_some_and(|n| n == repo))
        .collect();

    // Fallback: if we don't know about the repo yet, check a direct child directory.
    if candidate_repo_roots.is_empty() {
        let direct_child = working_dir.join(repo);
        candidate_repo_roots.push(direct_child);
    }

    candidate_repo_roots.sort();
    candidate_repo_roots.dedup();
    candidate_repo_roots
}

fn filter_candidate_repo_roots_by_org(
    repo: &str,
    expected_org: &str,
    candidate_repo_roots: Vec<PathBuf>,
    repository_orgs: &HashMap<PathBuf, Option<String>>,
) -> Result<Vec<PathBuf>, ResolveSkillError> {
    let mut filtered = Vec::new();
    let mut first_mismatch = None;

    for repo_root in candidate_repo_roots {
        match repository_orgs.get(&repo_root) {
            Some(Some(found_org)) if found_org != expected_org => {
                if first_mismatch.is_none() {
                    first_mismatch = Some(found_org.to_string());
                }
            }
            Some(Some(_)) => filtered.push(repo_root),
            Some(None) | None => {}
        }
    }

    if filtered.is_empty() {
        return Err(ResolveSkillError::OrgMismatch {
            repo: repo.to_string(),
            expected: expected_org.to_string(),
            found: first_mismatch.unwrap_or_else(|| "unknown".to_string()),
        });
    }

    Ok(filtered)
}

fn resolve_unqualified(
    spec: &SkillSpec,
    working_dir: &Path,
    snapshot: &SkillResolutionSnapshot,
) -> Result<ResolvedSkill, ResolveSkillError> {
    // Direct paths skip cache lookup and go straight to confined disk resolution.
    // Malformed single-component paths also take this route so they cannot reach name lookup.
    if is_direct_skill_path(spec) {
        if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, working_dir)? {
            return Ok(resolved);
        }
        return Err(ResolveSkillError::NotFound {
            skill: spec.skill_identifier.clone(),
        });
    }

    // Get all skill paths matching the requested name from the cache.
    let all_matching_paths = snapshot.all_matching_paths.clone();
    let home_dir = dirs::home_dir();

    // Per the skills spec, home directory skills take precedence over project skills.
    // Check home directory skills first.
    let home_skill_paths = &snapshot.home_skill_paths;
    let home_matches: Vec<PathBuf> = all_matching_paths
        .iter()
        .filter(|p| home_skill_paths.contains(p))
        .cloned()
        .collect();

    if let Some(skill_path) = best_match_by_directory_precedence(home_matches, home_dir.as_deref())
    {
        return parsed_skill_from_cache_or_disk(snapshot, &skill_path)
            .map(|parsed| to_resolved_skill(skill_path, parsed));
    }

    if let Some(resolved) =
        resolve_from_skill_dirs_by_directory_scan(spec, home_skill_dirs_for_resolution())?
    {
        return Ok(resolved);
    }

    // Next, try to scope to the current repo root (if known).
    if let Some(repo_root) = &snapshot.current_repo_root {
        match resolve_in_single_repo_root(spec, repo_root, snapshot) {
            Ok(resolved) => return Ok(resolved),
            Err(ResolveSkillError::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
    }

    // If we're not in a known repo, try searching across repos under the working directory.
    let in_scope_matches: Vec<PathBuf> = all_matching_paths
        .into_iter()
        .filter(|p| {
            // Only include project skills (not home skills) that are under working_dir
            snapshot.working_dir_skill_paths.contains(p)
        })
        .collect();

    if in_scope_matches.len() == 1 {
        let skill_path = in_scope_matches[0].clone();
        return parsed_skill_from_cache_or_disk(snapshot, &skill_path)
            .map(|parsed| to_resolved_skill(skill_path, parsed));
    }

    if in_scope_matches.len() > 1 {
        return Err(ResolveSkillError::Ambiguous {
            skill: spec.skill_identifier.clone(),
            candidates: in_scope_matches,
        });
    }

    // Fallback: if SkillManager hasn't cached anything yet, try resolving relative to the working dir.
    if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, working_dir)? {
        return Ok(resolved);
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn resolve_in_single_repo_root(
    spec: &SkillSpec,
    repo_root: &Path,
    snapshot: &SkillResolutionSnapshot,
) -> Result<ResolvedSkill, ResolveSkillError> {
    // Direct paths skip cache lookup and go straight to confined disk resolution.
    // Malformed single-component paths also take this route so they cannot reach name lookup.
    if is_direct_skill_path(spec) {
        if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, repo_root)? {
            return Ok(resolved);
        }
        return Err(ResolveSkillError::NotFound {
            skill: spec.skill_identifier.clone(),
        });
    }

    // Prefer cached skills (fast path).
    // Use the name-based cache and filter to paths within this repo root.
    let repo_skill_paths = snapshot
        .repo_skill_paths
        .get(repo_root)
        .cloned()
        .unwrap_or_default();
    let cached_paths: Vec<PathBuf> = snapshot
        .all_matching_paths
        .iter()
        .filter(|p| repo_skill_paths.contains(p))
        .cloned()
        .collect();

    if let Some(best_path) = best_match_by_directory_precedence(cached_paths, Some(repo_root)) {
        let parsed = parsed_skill_from_cache_or_disk(snapshot, &best_path)?;
        return Ok(to_resolved_skill(best_path, parsed));
    }

    // Cold start fallback: check disk in precedence order.
    if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, repo_root)? {
        return Ok(resolved);
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn resolve_from_root_path_by_directory_scan(
    spec: &SkillSpec,
    root: &Path,
) -> Result<Option<ResolvedSkill>, ResolveSkillError> {
    // Resolve direct paths through one confinement helper without consulting provider precedence.
    if is_direct_skill_path(spec) {
        if let Some(path) = confined_direct_skill_path(root, &spec.skill_identifier)? {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
        // If the direct path doesn't exist, return None without falling through to name lookup.
        return Ok(None);
    }

    // For simple skill names, iterate through SKILL_PROVIDER_DEFINITIONS in precedence order.
    for provider in SKILL_PROVIDER_DEFINITIONS.iter() {
        let path = root
            .join(&provider.skills_path)
            .join(&spec.skill_identifier)
            .join(SKILL_FILE_NAME);

        if path.exists() {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
    }

    Ok(None)
}

fn confined_direct_skill_path(
    root: &Path,
    skill_identifier: &str,
) -> Result<Option<PathBuf>, ResolveSkillError> {
    let skill_path = Path::new(skill_identifier);
    if has_forbidden_path_component(skill_path) {
        return Err(ResolveSkillError::ConfinementFailed {
            skill: skill_identifier.to_string(),
            root: root.to_path_buf(),
            message: "path contains a parent, root, or prefix component".to_string(),
        });
    }

    let candidate = root.join(skill_path);
    if !candidate.exists() {
        return Ok(None);
    }

    let canonical_root =
        dunce::canonicalize(root).map_err(|err| ResolveSkillError::ConfinementFailed {
            skill: skill_identifier.to_string(),
            root: root.to_path_buf(),
            message: format!("failed to canonicalize root: {err}"),
        })?;
    let canonical_candidate =
        dunce::canonicalize(&candidate).map_err(|err| ResolveSkillError::ConfinementFailed {
            skill: skill_identifier.to_string(),
            root: canonical_root.clone(),
            message: format!("failed to canonicalize candidate: {err}"),
        })?;

    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(ResolveSkillError::ConfinementFailed {
            skill: skill_identifier.to_string(),
            root: canonical_root,
            message: "canonical candidate is outside the root".to_string(),
        });
    }

    Ok(Some(canonical_candidate))
}

fn is_direct_skill_path(spec: &SkillSpec) -> bool {
    spec.is_full_path() || has_forbidden_path_component(Path::new(&spec.skill_identifier))
}

fn has_forbidden_path_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn parsed_skill_from_cache_or_disk(
    snapshot: &SkillResolutionSnapshot,
    skill_path: &Path,
) -> Result<ParsedSkill, ResolveSkillError> {
    if let Some(parsed) = snapshot.cached_skills.get(skill_path).cloned() {
        return Ok(parsed);
    }

    parse_skill(skill_path).map_err(|err| ResolveSkillError::ParseFailed {
        path: skill_path.to_path_buf(),
        message: err.to_string(),
    })
}

fn to_resolved_skill(skill_path: PathBuf, parsed: ParsedSkill) -> ResolvedSkill {
    let instructions = instructions_body(&parsed);
    ResolvedSkill {
        name: parsed.name.clone(),
        instructions,
        skill_path,
        parsed_skill: parsed,
    }
}

fn instructions_body(skill: &ParsedSkill) -> String {
    let Some(line_range) = &skill.line_range else {
        return skill.content.clone();
    };

    // line_range is 1-indexed, end-exclusive.
    let start = line_range.start.saturating_sub(1);
    let end = line_range.end.saturating_sub(1);

    let lines: Vec<&str> = skill.content.lines().collect();
    if start >= lines.len() {
        return String::new();
    }

    let end = end.min(lines.len());
    lines[start..end].join("\n").trim().to_string()
}

fn best_match_by_directory_precedence(
    mut matches: Vec<PathBuf>,
    root: Option<&Path>,
) -> Option<PathBuf> {
    if matches.is_empty() {
        return None;
    }

    // If we can't determine the root, fall back to a stable path sort.
    let Some(root) = root else {
        matches.sort();
        return matches.into_iter().next();
    };

    matches.sort_by(|a, b| {
        let a_rank = directory_precedence_rank(root, a);
        let b_rank = directory_precedence_rank(root, b);

        a_rank.cmp(&b_rank).then_with(|| a.cmp(b))
    });

    matches.into_iter().next()
}

fn directory_precedence_rank(root: &Path, skill_path: &Path) -> usize {
    for (idx, provider) in SKILL_PROVIDER_DEFINITIONS.iter().enumerate() {
        if skill_path.starts_with(root.join(&provider.skills_path)) {
            return idx;
        }
    }

    SKILL_PROVIDER_DEFINITIONS.len()
}

async fn get_git_remote_org(repo_path: &Path) -> Option<String> {
    log::debug!(
        "[GIT OPERATION] resolve_skill_spec.rs get_git_remote_org git remote get-url origin"
    );
    let mut command = AsyncCommand::new("git");
    command
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .kill_on_drop(true);
    let output = match tokio::time::timeout(GIT_REMOTE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            log::warn!(
                "Failed to inspect git origin for {}: {error}",
                repo_path.display()
            );
            return None;
        }
        Err(_) => {
            log::warn!(
                "Timed out while inspecting git origin for {}",
                repo_path.display()
            );
            return None;
        }
    };

    if !output.status.success() {
        log::debug!("Repository {} has no readable origin", repo_path.display());
        return None;
    }

    let url = String::from_utf8(output.stdout).ok()?.trim().to_string();
    parse_org_from_git_url(&url)
}

fn parse_org_from_git_url(url: &str) -> Option<String> {
    // SSH format: git@github.com:org/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        if let Some(path_start) = rest.find(':') {
            let path = &rest[path_start + 1..];
            if let Some(slash_pos) = path.find('/') {
                return Some(path[..slash_pos].to_string());
            }
        }
    }

    // HTTPS format: https://github.com/org/repo.git
    if url.starts_with("https://") || url.starts_with("http://") {
        let parts: Vec<&str> = url.split('/').collect();
        // https://github.com/org/repo.git
        if parts.len() >= 4 {
            return Some(parts[3].to_string());
        }
    }

    None
}

#[cfg(test)]
#[path = "resolve_skill_spec_tests.rs"]
mod tests;
