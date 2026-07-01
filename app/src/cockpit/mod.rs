//! Cockpit app integration: the `CockpitModel` singleton + file-watch/reconcile
//! wiring over the pure `zaplex_cockpit` data spine, plus scalar settings.
//!
//! Increment 1: data only (no UI). The account cards / heat bars / cost UI that
//! subscribe to `CockpitEvent::Updated` land in Increment 2 (`app/src/cockpit/…`).

pub mod model;
pub mod settings;

pub use model::CockpitModel;
pub use settings::CockpitSettings;
