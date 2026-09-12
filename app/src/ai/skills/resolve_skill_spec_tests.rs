use std::{fs, path::Path};

use anyhow::{Context as _, Result};
use warp_cli::skill::SkillSpec;

use super::*;

fn write_skill_file(path: &Path, name: &str, description: &str, body: &str) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Missing parent for {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create dir {}", parent.display()))?;

    let content = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");

    fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

#[test]
fn resolve_from_skill_dirs_by_directory_scan_resolves_home_skill_dir() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let skill_dir = temp_dir.path().join(".warp").join("skills");
    let skill_path = skill_dir.join("my-skill").join("SKILL.md");

    write_skill_file(
        &skill_path,
        "my-skill",
        "desc",
        "# Global Zaplex skill\n\nUse this one.",
    )?;

    let spec = SkillSpec::without_repo("my-skill".to_string());
    let resolved = resolve_from_skill_dirs_by_directory_scan(&spec, [skill_dir])?
        .context("Expected to resolve skill from explicit home skill dir")?;

    assert_eq!(resolved.skill_path, skill_path);
    assert!(resolved.instructions.contains("Global Zaplex skill"));

    Ok(())
}

#[test]
fn resolve_from_root_path_by_directory_scan_respects_directory_precedence() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let root = temp_dir.path();

    let spec = SkillSpec::without_repo("my-skill".to_string());
    let agents_skill = root.join(".agents/skills/my-skill/SKILL.md");
    let warp_skill = root.join(".warp/skills/my-skill/SKILL.md");

    let claude_skill = root.join(".claude/skills/my-skill/SKILL.md");
    let codex_skill = root.join(".codex/skills/my-skill/SKILL.md");

    write_skill_file(
        &agents_skill,
        "my-skill",
        "desc",
        "# Agents version\n\nUse this one.",
    )?;
    write_skill_file(
        &warp_skill,
        "my-skill",
        "desc",
        "# Zaplex version\n\nDo not pick this when .agents exists.",
    )?;
    write_skill_file(
        &claude_skill,
        "my-skill",
        "desc",
        "# Claude version\n\nDo not pick this when .warp exists.",
    )?;
    write_skill_file(
        &codex_skill,
        "my-skill",
        "desc",
        "# Codex version\n\nDo not pick this when .claude exists.",
    )?;

    let resolved = resolve_from_root_path_by_directory_scan(&spec, root)?
        .context("Expected to resolve skill via directory scan")?;

    assert_eq!(resolved.skill_path, agents_skill);
    assert!(resolved.instructions.contains("Agents version"));
    assert!(!resolved.instructions.contains("Zaplex version"));
    assert!(!resolved.instructions.contains("Claude version"));
    assert!(!resolved.instructions.contains("Codex version"));
    assert!(!resolved.instructions.contains("name:"));
    assert!(!resolved.instructions.contains("description:"));
    assert!(!resolved.instructions.contains("---"));

    Ok(())
}

#[test]
fn instructions_body_strips_front_matter_using_line_range() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let skill_path = temp_dir.path().join(".agents/skills/my-skill/SKILL.md");

    write_skill_file(
        &skill_path,
        "my-skill",
        "desc",
        "# Title\n\n## Instructions\nDo the thing.",
    )?;

    let parsed = ai::skills::parse_skill(&skill_path).context("Failed to parse skill")?;
    let body = instructions_body(&parsed);

    assert!(body.contains("# Title"));
    assert!(body.contains("## Instructions"));
    assert!(body.contains("Do the thing."));
    assert!(!body.contains("name:"));
    assert!(!body.contains("description:"));
    assert!(!body.contains("---"));

    Ok(())
}

#[test]
fn parse_org_from_git_url_supports_ssh_and_https() {
    assert_eq!(
        parse_org_from_git_url("git@github.com:warpdotdev/warp-internal.git"),
        Some("warpdotdev".to_string())
    );

    assert_eq!(
        parse_org_from_git_url("https://github.com/warpdotdev/warp-internal.git"),
        Some("warpdotdev".to_string())
    );
}

#[test]
fn repository_org_filter_keeps_only_matching_known_orgs() {
    let matching = PathBuf::from("/workspace/first/repo");
    let mismatching = PathBuf::from("/workspace/second/repo");
    let repository_orgs = HashMap::from([
        (matching.clone(), Some("expected".to_string())),
        (mismatching.clone(), Some("different".to_string())),
    ]);

    let filtered = filter_candidate_repo_roots_by_org(
        "repo",
        "expected",
        vec![matching.clone(), mismatching],
        &repository_orgs,
    )
    .expect("matching repository should remain");

    assert_eq!(filtered, vec![matching]);
}

#[test]
fn repository_org_filter_rejects_unknown_remotes() {
    let unknown = PathBuf::from("/workspace/repo");
    let repository_orgs = HashMap::from([(unknown.clone(), None)]);

    let error =
        filter_candidate_repo_roots_by_org("repo", "expected", vec![unknown], &repository_orgs)
            .expect_err("fully-qualified repository must fail closed when its org is unknown");

    assert!(matches!(error, ResolveSkillError::OrgMismatch { .. }));
}

#[test]
fn repository_org_filter_rejects_roots_missing_from_snapshot() {
    let error = filter_candidate_repo_roots_by_org(
        "repo",
        "expected",
        vec![PathBuf::from("/workspace/repo")],
        &HashMap::new(),
    )
    .expect_err("root not included in org lookup snapshot must fail closed");

    assert!(matches!(error, ResolveSkillError::OrgMismatch { .. }));
}

#[test]
fn resolve_with_full_path_skips_directory_precedence() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let root = temp_dir.path();

    // Create a skill in .claude/skills directory
    let claude_skill = root.join(".claude/skills/my-skill/SKILL.md");
    write_skill_file(
        &claude_skill,
        "my-skill",
        "desc",
        "# Claude version\n\nThis is in .claude directory.",
    )?;

    // Create a skill in .agents/skills directory (higher precedence)
    let agents_skill = root.join(".agents/skills/my-skill/SKILL.md");
    write_skill_file(
        &agents_skill,
        "my-skill",
        "desc",
        "# Agents version\n\nThis is in .agents directory.",
    )?;

    // Resolve using full path to .claude skill - should get .claude version, not .agents
    // (even though .agents has higher precedence when using simple names)
    let spec = SkillSpec::without_repo(".claude/skills/my-skill/SKILL.md".to_string());
    let resolved = resolve_from_root_path_by_directory_scan(&spec, root)?
        .context("Expected to resolve skill via full path")?;

    assert_eq!(resolved.skill_path, claude_skill);
    assert!(resolved.instructions.contains("Claude version"));
    assert!(!resolved.instructions.contains("Agents version"));

    Ok(())
}

#[test]
fn resolve_with_full_path_returns_none_if_not_found() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let root = temp_dir.path();

    // Try to resolve a full path that doesn't exist
    let spec = SkillSpec::without_repo(".claude/skills/nonexistent/SKILL.md".to_string());
    let resolved = resolve_from_root_path_by_directory_scan(&spec, root)?;

    assert!(resolved.is_none());

    Ok(())
}

#[test]
fn direct_skill_paths_reject_forbidden_components_for_qualified_and_unqualified_specs() {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let root = temp_dir.path().join("trusted");
    fs::create_dir_all(&root).expect("Failed to create repository root");

    let specs = [
        SkillSpec::without_repo("..".to_string()),
        SkillSpec::without_repo(std::path::MAIN_SEPARATOR.to_string()),
        SkillSpec::without_repo("../outside/.agents/skills/x/SKILL.md".to_string()),
        "trusted:../outside/.agents/skills/x/SKILL.md"
            .parse::<SkillSpec>()
            .expect("Qualified test spec must parse"),
        SkillSpec::without_repo(root.join("SKILL.md").display().to_string()),
    ];

    for spec in specs {
        let error = resolve_from_root_path_by_directory_scan(&spec, &root)
            .expect_err("Path with forbidden components must fail confinement");
        assert!(matches!(error, ResolveSkillError::ConfinementFailed { .. }));
    }
}

#[cfg(unix)]
#[test]
fn direct_skill_path_rejects_symlink_outside_root_before_parsing() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let root = temp_dir.path().join("trusted");
    let external_skill = temp_dir.path().join("outside/SKILL.md");
    let linked_skill = root.join(".agents/skills/x/SKILL.md");

    fs::create_dir_all(
        external_skill
            .parent()
            .context("External skill must have a parent")?,
    )?;
    fs::write(&external_skill, "not valid skill front matter")?;
    fs::create_dir_all(
        linked_skill
            .parent()
            .context("Linked skill must have a parent")?,
    )?;
    symlink(&external_skill, &linked_skill)?;

    let spec = SkillSpec::with_repo(
        "trusted".to_string(),
        ".agents/skills/x/SKILL.md".to_string(),
    );
    let error = resolve_from_root_path_by_directory_scan(&spec, &root)
        .expect_err("Symlink outside the repository must fail confinement");

    assert!(matches!(error, ResolveSkillError::ConfinementFailed { .. }));
    Ok(())
}

#[test]
fn resolve_simple_name_uses_directory_precedence() -> Result<()> {
    let temp_dir = tempfile::TempDir::new().context("Failed to create temp dir")?;
    let root = temp_dir.path();

    // Create skills with same name in different directories
    // Note: .agents/skills has highest precedence, followed by .warp, then .claude.
    let agents_skill = root.join(".agents/skills/my-skill/SKILL.md");
    write_skill_file(
        &agents_skill,
        "my-skill",
        "desc",
        "# Agents version\n\nThis should be picked by precedence.",
    )?;

    let warp_skill = root.join(".warp/skills/my-skill/SKILL.md");
    write_skill_file(
        &warp_skill,
        "my-skill",
        "desc",
        "# Zaplex version\n\nThis should lose to .agents but beat .claude.",
    )?;

    let claude_skill = root.join(".claude/skills/my-skill/SKILL.md");
    write_skill_file(
        &claude_skill,
        "my-skill",
        "desc",
        "# Claude version\n\nThis should not be picked.",
    )?;

    // Resolve using simple name - should get .agents version due to precedence
    let spec = SkillSpec::without_repo("my-skill".to_string());
    let resolved = resolve_from_root_path_by_directory_scan(&spec, root)?
        .context("Expected to resolve skill by name")?;
    assert_eq!(resolved.skill_path, agents_skill);
    assert!(resolved.instructions.contains("Agents version"));
    assert!(!resolved.instructions.contains("Zaplex version"));
    assert!(!resolved.instructions.contains("Claude version"));

    Ok(())
}
