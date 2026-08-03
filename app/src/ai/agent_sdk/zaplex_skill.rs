use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const SKILL_MARKDOWN: &str = include_str!("../../../../zaplex-skills/skills/zaplex/SKILL.md");
const OPENAI_YAML: &str =
    include_str!("../../../../zaplex-skills/skills/zaplex/agents/openai.yaml");

#[derive(Clone, Debug)]
struct SkillInstallPaths {
    canonical_dir: PathBuf,
    link: PathBuf,
}

impl SkillInstallPaths {
    fn for_current_user() -> Result<Self> {
        let home = dirs::home_dir().context("could not determine the home directory")?;
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("zaplex");
        Ok(Self {
            canonical_dir: data_dir.join("skills").join("zaplex"),
            link: home.join(".agents").join("skills").join("zaplex"),
        })
    }
}

pub(crate) fn install() -> Result<()> {
    let paths = SkillInstallPaths::for_current_user()?;
    install_at(&paths)?;
    println!("Installed Zaplex skill at {}", paths.link.display());
    Ok(())
}

pub(crate) fn remove() -> Result<()> {
    let paths = SkillInstallPaths::for_current_user()?;
    remove_at(&paths)?;
    println!("Removed Zaplex skill from {}", paths.link.display());
    Ok(())
}

pub(crate) fn refresh_if_installed() -> Result<()> {
    let paths = SkillInstallPaths::for_current_user()?;
    refresh_at(&paths)
}

fn refresh_at(paths: &SkillInstallPaths) -> Result<()> {
    match managed_link_state(paths)? {
        ManagedLinkState::Missing => Ok(()),
        ManagedLinkState::Managed => write_canonical_skill(&paths.canonical_dir),
        ManagedLinkState::Foreign => {
            bail!(
                "{} exists but is not managed by Zaplex",
                paths.link.display()
            )
        }
    }
}

fn install_at(paths: &SkillInstallPaths) -> Result<()> {
    match managed_link_state(paths)? {
        ManagedLinkState::Managed => return write_canonical_skill(&paths.canonical_dir),
        ManagedLinkState::Foreign => {
            bail!(
                "refusing to replace unrelated skill at {}",
                paths.link.display()
            )
        }
        ManagedLinkState::Missing => {}
    }

    write_canonical_skill(&paths.canonical_dir)?;
    let parent = paths
        .link
        .parent()
        .context("skill link has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    create_directory_symlink(&paths.canonical_dir, &paths.link)
        .with_context(|| format!("failed to link {}", paths.link.display()))
}

fn remove_at(paths: &SkillInstallPaths) -> Result<()> {
    match managed_link_state(paths)? {
        ManagedLinkState::Missing => Ok(()),
        ManagedLinkState::Foreign => {
            bail!(
                "refusing to remove unrelated skill at {}",
                paths.link.display()
            )
        }
        ManagedLinkState::Managed => remove_directory_symlink(&paths.link)
            .with_context(|| format!("failed to remove {}", paths.link.display())),
    }
}

fn write_canonical_skill(canonical_dir: &Path) -> Result<()> {
    let agents_dir = canonical_dir.join("agents");
    fs::create_dir_all(&agents_dir)
        .with_context(|| format!("failed to create {}", agents_dir.display()))?;
    write_if_changed(&canonical_dir.join("SKILL.md"), SKILL_MARKDOWN.as_bytes())?;
    write_if_changed(&agents_dir.join("openai.yaml"), OPENAI_YAML.as_bytes())
}

fn write_if_changed(path: &Path, expected: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == expected => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    }

    let parent = path
        .parent()
        .context("skill file has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    use std::io::Write as _;
    temporary
        .write_all(expected)
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedLinkState {
    Missing,
    Managed,
    Foreign,
}

fn managed_link_state(paths: &SkillInstallPaths) -> Result<ManagedLinkState> {
    let metadata = match fs::symlink_metadata(&paths.link) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ManagedLinkState::Missing);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", paths.link.display()));
        }
    };
    if !metadata.file_type().is_symlink() {
        return Ok(ManagedLinkState::Foreign);
    }

    let target = fs::read_link(&paths.link)
        .with_context(|| format!("failed to read {}", paths.link.display()))?;
    let resolved = if target.is_absolute() {
        target
    } else {
        paths
            .link
            .parent()
            .context("skill link has no parent directory")?
            .join(target)
    };
    if resolved == paths.canonical_dir {
        Ok(ManagedLinkState::Managed)
    } else {
        Ok(ManagedLinkState::Foreign)
    }
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
}

#[cfg(not(any(unix, windows)))]
fn create_directory_symlink(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "skill links are unsupported on this platform",
    ))
}

#[cfg(unix)]
fn remove_directory_symlink(target: &Path) -> io::Result<()> {
    fs::remove_file(target)
}

#[cfg(windows)]
fn remove_directory_symlink(target: &Path) -> io::Result<()> {
    fs::remove_dir(target)
}

#[cfg(not(any(unix, windows)))]
fn remove_directory_symlink(_target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "skill links are unsupported on this platform",
    ))
}

#[cfg(test)]
#[path = "zaplex_skill_tests.rs"]
mod tests;
