use std::path::{Path, PathBuf};
use std::process::Command;

pub fn watch_paths(manifest_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(head) = git_path(manifest_dir, "HEAD") {
        paths.push(head);
    }
    if let Some(reference) = git_output(manifest_dir, &["symbolic-ref", "-q", "HEAD"])
        .and_then(|reference| git_path(manifest_dir, &reference))
    {
        if !paths.contains(&reference) {
            paths.push(reference);
        }
    }
    paths
}

fn git_path(manifest_dir: &Path, name: &str) -> Option<PathBuf> {
    let path = git_output(manifest_dir, &["rev-parse", "--git-path", name])?;
    existing_path(manifest_dir, &path)
}

fn git_output(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.trim();
    (!output.is_empty()).then(|| output.to_string())
}

pub fn existing_path(manifest_dir: &Path, path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path.trim());
    let path = if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    };
    path.canonicalize().ok().filter(|path| path.is_file())
}
