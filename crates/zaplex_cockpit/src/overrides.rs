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

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod tests;
