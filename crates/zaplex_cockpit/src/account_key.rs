use std::path::Path;

use sha2::{Digest as _, Sha256};

use crate::types::Provider;

const ROOT_HASH_HEX_LEN: usize = 12;

pub(crate) fn stable_account_key(
    provider: Provider,
    canonical_root: &Path,
    home: &Path,
    is_default: bool,
) -> String {
    let provider_name = provider.as_str();
    if is_default {
        return format!("{provider_name}:default");
    }

    let canonical_home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if canonical_root.parent() == Some(canonical_home.as_path()) {
        if let Some(suffix) = direct_home_sibling_suffix(provider_name, canonical_root) {
            if !suffix.eq_ignore_ascii_case("default") {
                return format!("{provider_name}:{suffix}");
            }
        }
    }

    let slug = root_slug(provider_name, canonical_root);
    let mut digest = Sha256::new();
    digest.update(b"zaplex-account-root-v1\0");
    digest.update(provider_name.as_bytes());
    digest.update(b"\0");
    digest.update(canonical_root.as_os_str().as_encoded_bytes());
    let root_hash = format!("{:x}", digest.finalize());
    format!("{provider_name}:{slug}:{}", &root_hash[..ROOT_HASH_HEX_LEN])
}

pub(crate) fn legacy_account_key(
    provider: Provider,
    config_dir: &Path,
    is_default: bool,
) -> String {
    let provider_name = provider.as_str();
    if is_default {
        return format!("{provider_name}:default");
    }
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("account");
    let dashed_prefix = format!(".{provider_name}-");
    let plain_prefix = format!(".{provider_name}");
    let suffix = name
        .strip_prefix(&dashed_prefix)
        .or_else(|| name.strip_prefix(&plain_prefix))
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or(name);
    format!("{provider_name}:{suffix}")
}

fn direct_home_sibling_suffix<'a>(provider_name: &str, root: &'a Path) -> Option<&'a str> {
    let name = root.file_name()?.to_str()?;
    let prefix = format!(".{provider_name}-");
    name.strip_prefix(&prefix)
        .filter(|suffix| !suffix.is_empty())
}

fn root_slug(provider_name: &str, root: &Path) -> String {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("account");
    let dashed_prefix = format!(".{provider_name}-");
    let plain_prefix = format!(".{provider_name}");
    let readable = name
        .strip_prefix(&dashed_prefix)
        .or_else(|| name.strip_prefix(&plain_prefix))
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or(name);
    let mut slug = String::new();
    for character in readable.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "account".to_string()
    } else {
        slug.to_string()
    }
}
