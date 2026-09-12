use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Output;

use anyhow::{anyhow, Context, Result};
use command::blocking::Command;
use warp_core::channel::ChannelState;

const CREATE_SYMLINK_ADMIN_SCRIPT: &str = concat!(
    "on run argv\n",
    "do shell script (\"/bin/ln -sf \" & quoted form of (item 1 of argv) & \" \" & quoted form of (item 2 of argv)) ",
    "with prompt \"Zaplex needs administrator privileges to install the command in /usr/local/bin.\" ",
    "with administrator privileges\n",
    "end run",
);

const REMOVE_FILE_ADMIN_SCRIPT: &str = concat!(
    "on run argv\n",
    "do shell script (\"/bin/rm \" & quoted form of (item 1 of argv)) ",
    "with prompt \"Zaplex needs administrator privileges to uninstall the command from /usr/local/bin.\" ",
    "with administrator privileges\n",
    "end run",
);

/// Compute the target path where the symlink should be installed, based on channel
fn cli_install_target_path() -> PathBuf {
    PathBuf::from("/usr/local/bin").join(ChannelState::channel().cli_command_name())
}

fn admin_script_arguments(script: &str, paths: &[&Path]) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("-e"),
        OsString::from(script),
        OsString::from("--"),
    ];
    arguments.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
    arguments
}

fn run_admin_script(script: &str, paths: &[&Path]) -> Result<Output> {
    Command::new("osascript")
        .args(admin_script_arguments(script, paths))
        .output()
        .context("Failed to execute osascript for admin privileges")
}

/// Create a symlink with elevated privileges using osascript
///
/// This function uses macOS's osascript to prompt for administrator privileges
/// and create a symlink
fn create_symlink_with_admin(source: &Path, target: &Path) -> Result<()> {
    log::debug!("Creating symlink with admin privileges");

    let output = run_admin_script(CREATE_SYMLINK_ADMIN_SCRIPT, &[source, target])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("cancelled") {
            return Err(anyhow!("Installation cancelled by user."));
        }
        return Err(anyhow!(
            "Failed to create symlink with admin privileges: {stderr}"
        ));
    }

    Ok(())
}

/// Remove a file with elevated privileges using osascript
///
/// This function uses macOS's osascript to prompt for administrator privileges
/// and remove a file, used for CLI uninstallation.
fn remove_file_with_admin(target: &Path) -> Result<()> {
    log::debug!("Removing file with admin privileges");

    let output = run_admin_script(REMOVE_FILE_ADMIN_SCRIPT, &[target])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("cancelled") {
            return Err(anyhow!("Uninstallation cancelled by user."));
        }
        return Err(anyhow!(
            "Failed to remove file with admin privileges: {stderr}"
        ));
    }

    Ok(())
}

/// Install the CLI by creating a symlink (channel-specific target)
///
/// This function:
/// 1. Detects the current Zaplex channel and finds the appropriate binary
/// 2. Attempts to create a symlink without admin privileges first
/// 3. Falls back to prompting for admin privileges if needed
/// 4. Handles existing installations and edge cases
pub fn install_cli() -> Result<()> {
    let cli_path = cli_install_target_path();
    let current_binary =
        std::env::current_exe().context("Failed to get current executable path")?;

    // Check if target file exists and handle conflicts
    if cli_path.exists() && !cli_path.is_symlink() {
        return Err(anyhow!(
            "Cannot install: {:?} exists but is not a symlink. Please remove it manually first.",
            cli_path
        ));
    }

    // Try to create symlink without admin privileges first
    let symlink_result = symlink(&current_binary, &cli_path);

    match symlink_result {
        Ok(_) => {
            log::debug!(
                "CLI installed successfully without admin privileges: {:?} -> {}",
                cli_path,
                current_binary.display()
            );
        }
        Err(_) => {
            log::debug!("Symlink creation failed, trying with admin privileges");

            create_symlink_with_admin(&current_binary, &cli_path)
                .context("Failed to create symlink even with admin privileges")?;

            log::debug!("CLI installed successfully with admin privileges");
        }
    }

    Ok(())
}

/// Uninstall the CLI by removing the symlink (channel-specific target)
///
/// This function:
/// 1. Verifies that the target is actually a symlink (safety check)
/// 2. Attempts to remove without admin privileges first
/// 3. Falls back to prompting for admin privileges if needed
pub fn uninstall_cli() -> Result<()> {
    let cli_path = cli_install_target_path();

    if !cli_path.exists() {
        return Err(anyhow!("Oz command is not currently installed."));
    }

    // Safety check: verify it's actually a symlink before removing
    if !cli_path.is_symlink() {
        return Err(anyhow!(
            "Cannot uninstall: {:?} exists but is not a symlink. Please remove it manually.",
            cli_path
        ));
    }

    // Try to remove without admin privileges first
    let remove_result = fs::remove_file(&cli_path);

    match remove_result {
        Ok(_) => {
            log::debug!("CLI uninstalled successfully without admin privileges");
        }
        Err(_) => {
            log::debug!("File removal failed, trying with admin privileges");

            remove_file_with_admin(&cli_path)
                .context("Failed to remove symlink even with admin privileges")?;

            log::debug!("CLI uninstalled successfully with admin privileges");
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "cli_install_tests.rs"]
mod tests;
