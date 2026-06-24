//! Global SSH tree change broadcast — any view that modifies tree structure
//! (add/delete/rename/change server field) calls `notify()` once, and subscribers
//! like SshManagerPanel refresh accordingly.
//!
//! Same pattern as `KeybindingChangedNotifier` (`app/src/settings_view/keybindings.rs:72`):
//! Empty struct + SingletonEntity + single Event variant.

use warpui::{Entity, SingletonEntity};

#[derive(Default)]
pub struct SshTreeChangedNotifier {}

impl SshTreeChangedNotifier {
    pub fn new() -> Self {
        Default::default()
    }
}

#[derive(Clone, Debug)]
pub enum SshTreeChangedEvent {
    /// Node list / server details changed; need to re-list_nodes.
    TreeChanged,
}

impl Entity for SshTreeChangedNotifier {
    type Event = SshTreeChangedEvent;
}

impl SingletonEntity for SshTreeChangedNotifier {}
