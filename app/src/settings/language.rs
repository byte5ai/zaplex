//! User interface language settings (persisted via settings.toml, applied to i18n loader on startup).
//!
//! Currently supports English (plus "System default"). To add a new language:
//!   1. Add a variant to `Language`
//!   2. Create translation file in `app/i18n/<locale>/warp.ftl`
//!   3. Add a case for `Display` + `to_locale_str`
//!
//! Language switching takes full effect after restart (already-rendered UI text won't auto-reflow; views need rebuilding).
//! Settings page dropdown should include a hint: "takes full effect after restarting Zaplex".

use enum_iterator::Sequence;
use serde::{Deserialize, Serialize};
use warp_core::settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};

#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "The language used in Zaplex's user interface.",
    rename_all = "snake_case"
)]
pub enum Language {
    /// Follow system language; if system locale is not a supported language, fall back to English.
    #[default]
    #[schemars(description = "System default")]
    System,
    #[schemars(description = "English")]
    English,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Language::System => "System default",
            Language::English => "English",
        };
        write!(f, "{value}")
    }
}

impl Language {
    /// Convert to BCP-47 locale string; `System` returns `None` to use system detection.
    pub fn to_locale_str(self) -> Option<&'static str> {
        match self {
            Language::System => None,
            Language::English => Some("en"),
        }
    }
}

define_settings_group!(LanguageSettings, settings: [
    language: LanguageState {
        type: Language,
        default: Language::System,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "Language",
        toml_path: "appearance.language",
        description: "The language used in Zaplex's user interface. Falls back to English when the chosen language is not fully translated.",
    },
]);
