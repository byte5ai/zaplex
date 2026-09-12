//! Shared JSON read/merge/write helpers for third-party harness config prep.
//!
//! Third-party CLIs like Claude Code and Gemini CLI persist onboarding, trust,
//! and auth state in JSON files. The harness preparation step
//! needs to set a few keys on those files without clobbering
//! user-owned state. These helpers allow us to read and merge with existing
//! JSON file state easily.

use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Read a JSON file as `T`, or return `T::default()` if the file does not exist.
///
/// Returns an error if the file exists but cannot be read or parsed.
pub(super) fn read_json_file_or_default<T>(path: &Path) -> Result<T>
where
    T: Default + for<'de> Deserialize<'de>,
{
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(T::default());
        }
        Err(e) => {
            return Err(
                anyhow::Error::from(e).context(format!("Failed to read {}", path.display()))
            );
        }
    };
    serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Serialize `value` as pretty JSON and write it to `path`, creating parent
/// directories as needed. `serialize_error` is used as the context for the
/// serialization step so the caller-facing error is specific to the config
/// file being written.
pub(super) fn write_json_file<T>(
    path: &Path,
    value: &T,
    serialize_error: &'static str,
) -> Result<()>
where
    T: Serialize,
{
    let serialized = serde_json::to_vec_pretty(value).context(serialize_error)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".zaplex-provider-config-")
            .tempfile_in(parent)
            .with_context(|| {
                format!("Failed to create a temporary file in {}", parent.display())
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .with_context(|| {
                    format!("Failed to secure temporary file for {}", path.display())
                })?;
        }

        temporary
            .write_all(&serialized)
            .and_then(|()| temporary.as_file().sync_all())
            .with_context(|| format!("Failed to write temporary file for {}", path.display()))?;
        temporary
            .persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("Failed to replace {}", path.display()))?;
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        return Ok(());
    }
    anyhow::bail!("Config path has no parent: {}", path.display())
}

#[cfg(test)]
#[path = "json_utils_tests.rs"]
mod tests;
