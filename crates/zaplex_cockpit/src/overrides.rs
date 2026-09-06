//! User overrides for discovered accounts (instances.json-style), audit (a)/(h/i).
//!
//! Discovery yields accounts exactly as the providers report them. A user may
//! want to **rename**, **recolor**, **reorder**, or **hide** them in the cockpit
//! without editing provider config. This module is the pure model + apply logic
//! (no file IO): given parsed overrides + the discovered accounts, it yields the
//! display list. Loading the JSON from disk and threading `color_for` into the
//! card renderer build on top.
//!
//! Keyed by the account's stable `key` (e.g. `claude:work`, `codex:default`),
//! which is derived from provider + config dir and survives restarts.

use crate::types::AccountUsage;
use serde::Deserialize;
use std::collections::HashMap;

/// One account's user overrides. Every field is optional — an entry may tweak
/// just the color, or just hide, etc.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AccountOverride {
    /// Replacement display label.
    #[serde(default)]
    pub label: Option<String>,
    /// Display color as a hex string like `#22C55E`. Stored verbatim; the UI
    /// validates/parses it (an invalid string simply yields no tint).
    #[serde(default)]
    pub color: Option<String>,
    /// Explicit sort position; lower sorts first. Accounts without an order keep
    /// discovery order, after all explicitly-ordered ones.
    #[serde(default)]
    pub order: Option<i64>,
    /// Hide this account from the cockpit entirely.
    #[serde(default)]
    pub hidden: bool,
}

/// The full override set, keyed by account key. Deserializes directly from an
/// `instances.json` object: `{ "claude:work": { "label": "...", "hidden": true } }`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct AccountOverrides {
    entries: HashMap<String, AccountOverride>,
}

impl AccountOverrides {
    /// Parse instances.json-style overrides. **Lenient**: returns an empty set
    /// (no overrides) on any parse failure — a broken overrides file must never
    /// hide the user's accounts or blank the cockpit.
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The overridden display color for an account key, if any.
    pub fn color_for(&self, key: &str) -> Option<&str> {
        self.entries.get(key).and_then(|o| o.color.as_deref())
    }

    /// Move legacy basename-only overrides to a root-bound key only when the
    /// legacy key identifies exactly one currently discovered account.
    pub fn migrate_legacy_keys(&mut self, accounts: &[AccountUsage]) {
        let mut targets: HashMap<String, Vec<String>> = HashMap::new();
        for account in accounts {
            let legacy_key = crate::account_key::legacy_account_key(
                account.account.provider,
                &account.account.config_dir,
                account.account.is_default,
            );
            targets
                .entry(legacy_key)
                .or_default()
                .push(account.account.key.clone());
        }

        for (legacy_key, new_keys) in targets {
            let [new_key] = new_keys.as_slice() else {
                continue;
            };
            if &legacy_key == new_key || self.entries.contains_key(new_key) {
                continue;
            }
            if let Some(account_override) = self.entries.remove(&legacy_key) {
                self.entries.insert(new_key.clone(), account_override);
            }
        }
    }

    /// Apply the overrides to a discovered account list:
    /// 1. **drop** hidden accounts,
    /// 2. **relabel** those with a `label` override,
    /// 3. **sort** by explicit `order` ascending; un-ordered accounts keep their
    ///    discovery order, after all ordered ones.
    ///
    /// The sort is stable, so accounts sharing an `order` (or both un-ordered)
    /// keep their input relative order. Color is *not* applied here (it isn't a
    /// field on `Account`); the renderer calls [`Self::color_for`] per card.
    pub fn apply(&self, accounts: Vec<AccountUsage>) -> Vec<AccountUsage> {
        let mut kept: Vec<AccountUsage> = accounts
            .into_iter()
            .filter(|a| {
                !self
                    .entries
                    .get(&a.account.key)
                    .map(|o| o.hidden)
                    .unwrap_or(false)
            })
            .map(|mut a| {
                if let Some(label) = self
                    .entries
                    .get(&a.account.key)
                    .and_then(|o| o.label.clone())
                {
                    a.account.label = label;
                }
                a
            })
            .collect();
        // Stable sort; un-ordered accounts sink to the end via the MAX sentinel
        // while preserving their relative discovery order.
        kept.sort_by_key(|a| {
            self.entries
                .get(&a.account.key)
                .and_then(|o| o.order)
                .unwrap_or(i64::MAX)
        });
        kept
    }
}

/// Set (or clear, with `None`) an account's **label** override in an
/// `instances.json`, preserving every other *value* in the file.
///
/// This file is **not ours**. It is claudeplex's format, and a user's copy may
/// carry keys this crate has never heard of. So the edit goes through
/// `serde_json::Value` rather than [`AccountOverrides`]: that struct knows four
/// fields, drops the rest on parse, and writing it back would silently delete
/// whatever the other tool put there. Round-tripping the raw value touches one
/// key and leaves the other values standing.
///
/// **Not byte-for-byte.** The file is re-serialised, so formatting, key order
/// and comments-as-whitespace change, and duplicate keys (legal JSON, last one
/// wins) collapse to one. Values survive; the text does not.
///
/// Refuses rather than clobbers:
/// - A file that is not a JSON object is left alone (`Err`). Overwriting it
///   would destroy something we failed to understand.
/// - A missing file is fine — the object starts empty.
/// - Unreadable/unwritable paths surface the IO error; the caller decides
///   whether to toast. Silence would be the one unacceptable outcome.
///
/// Written via a temp file + rename, so a crash mid-write cannot leave a
/// half-written file where a valid one was: the reader is the cockpit's own
/// startup, and a truncated file would blank every alias at once. The temp name
/// carries this process's id — a fixed one would collide with another zaplex
/// writing at the same moment, and with whatever else happens to sit there.
/// A symlinked `instances.json` is resolved first, so the write lands on the
/// file the user pointed at instead of replacing their link with a copy, and the
/// original's permissions are carried onto the replacement.
///
/// **Last writer wins.** This is read-modify-write with no lock: if claudeplex
/// saves between our read and our rename, its change is lost. A lock would not
/// help — it only works when both writers honour it, and the other tool has
/// never heard of ours, so it would buy false confidence rather than safety. The
/// window is the few milliseconds of one edit, and the loss is one field of one
/// account, recoverable by setting it again.
pub fn set_label_override(
    path: &std::path::Path,
    account_key: &str,
    label: Option<&str>,
) -> std::io::Result<()> {
    use serde_json::Value;

    let mut root: Value = match std::fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("instances.json is not valid JSON ({e}) — refusing to overwrite it"),
            )
        })?,
        // Absent or empty: start from an empty object.
        Ok(_) => Value::Object(Default::default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(e),
    };

    let Some(obj) = root.as_object_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "instances.json is not a JSON object — refusing to overwrite it",
        ));
    };

    let entry = obj
        .entry(account_key.to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let Some(entry) = entry.as_object_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("instances.json entry for {account_key} is not an object"),
        ));
    };

    match label {
        // An empty alias means "no alias", not an account named "".
        Some(l) if !l.trim().is_empty() => {
            entry.insert("label".into(), Value::String(l.trim().to_string()));
        }
        _ => {
            entry.remove("label");
        }
    }
    // An entry we emptied carries nothing — leave no bookkeeping behind.
    if entry.is_empty() {
        obj.remove(account_key);
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Follow a symlink to its target: renaming onto the link would leave the
    // user with a regular file where they had deliberately pointed elsewhere.
    // Unresolvable (the file may not exist yet) → write where we were told.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // A per-process temp name: a fixed one races another zaplex mid-edit, and
    // would happily overwrite anything already sitting at that path.
    let tmp = target.with_extension(format!("tmp{}", std::process::id()));
    let body = serde_json::to_string_pretty(&root)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&tmp, body)?;
    // Carry the original's permissions across — the rename swaps the inode, so
    // without this a 0600 file would come back with whatever the umask says.
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    match std::fs::rename(&tmp, &target) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Don't leave our scratch file behind on a failed swap.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod tests;
