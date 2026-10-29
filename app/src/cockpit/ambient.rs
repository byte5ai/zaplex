//! The attention **ambient bit** — one passive, fleet-wide "is there anything
//! for me?" signal, and nothing else. Deliberately the *opposite* of
//! per-event notification spam: there is no toast, no banner, no OS
//! notification per session. A single indicator reflects
//! [`CockpitModel::needs_me`] and stays visible even when the app is minimised.
//! It is the same number as the title-bar pulse, the sidebar header and the
//! inbox: openable sessions with an open question/permission prompt or a
//! finished turn the user has not seen yet.
//!
//! - **macOS Dock badge** (the core deliverable): the fleet-wide waiting count
//!   painted on the app's dock tile — `"3"` when three agents wait on you,
//!   cleared to blank when none do. Updated on every cockpit reconcile.
//! - **A single, optional chime** on the calm→needy edge (`needs_me` going
//!   `0 → >0`): the fleet just went from "nothing for me" to "something for
//!   me". Never per session, never per event. Suppressed by the DND setting.
//!
//! The menu-bar `NSStatusItem` variant is intentionally *not* shipped here: it
//! needs a retained, main-thread-owned status item whose lifetime spans the app
//! — a heavier surface than the dock badge, and not something to fake. The dock
//! badge is the honest, self-contained core; the menu-bar item is a follow-up.
//!
//! OS surfaces belong to the active platform delegate. Native macOS paints the
//! Dock and plays the sound; test and headless backends retain all attention
//! state without creating an AppKit application.

use warpui::{Entity, ModelContext, SingletonEntity};
use zaplex_cockpit::{AgentInventoryStatus, FleetTree, HostAvailability, ScanHealth};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::settings::CockpitSettings;

/// Pure edge-detector for the single attention chime: the fleet-wide waiting
/// count crossing from **zero to non-zero** — "nothing for me" → "something for
/// me". Any other transition (staying at zero, staying busy, a second agent
/// joining an already-waiting fleet, or the count dropping back to zero) is
/// silent: the badge carries those, never a sound.
///
/// `suppressed` folds together the DND setting and the sound toggle — when
/// either silences audio, no edge ever chimes (the badge still updates).
pub fn should_chime(prev: usize, now: usize, suppressed: bool) -> bool {
    !suppressed && prev == 0 && now > 0
}

/// Whole chime decision for one cockpit update, folding in the driver's
/// first-refresh rule: the *first* real update only seeds the baseline and
/// never chimes (`initialized == false`), even if the fleet is already waiting
/// at launch. Every later update falls back to the pure [`should_chime`] edge.
pub fn should_chime_on_update(
    initialized: bool,
    prev: usize,
    now: usize,
    suppressed: bool,
) -> bool {
    initialized && should_chime(prev, now, suppressed)
}

/// The singleton that turns cockpit reconciles into the ambient bit. It owns
/// the previous *needs-me* value so the calm→needy edge is detected exactly
/// once for the whole fleet — regardless of how many windows are open — and so
/// the chime never fires per session or per window.
pub struct AttentionDriver {
    /// The fleet-wide waiting count as of the last reconcile, for edge
    /// detection.
    prev_needs_me: usize,
    /// Whether a real refresh has established [`Self::prev_needs_me`] yet. The
    /// driver's *first* update only seeds the baseline — it never chimes — so a
    /// fleet that is already waiting when the app launches does not fire a false
    /// "just now" edge. Only genuine later `0 → >0` transitions chime. The badge
    /// (`set_attention`) still updates on that first refresh; only the chime is
    /// suppressed.
    initialized: bool,
}

/// A pending or incomplete scan cannot establish a calm baseline. In particular,
/// publishing local results before remote RPCs finish must not manufacture a
/// startup edge when the first remote waiting sessions arrive.
fn attention_baseline_ready(health: &ScanHealth, inventory: &FleetTree) -> bool {
    health.is_loaded()
        && inventory.hosts.iter().all(|host| match host.availability {
            HostAvailability::Removed => true,
            HostAvailability::Unverified => false,
            HostAvailability::Available => matches!(
                host.inventory_status,
                AgentInventoryStatus::Ready | AgentInventoryStatus::Unsupported
            ),
        })
}

impl AttentionDriver {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Every cockpit reconcile emits `Updated`; needs_me may have changed.
        ctx.subscribe_to_model(&CockpitModel::handle(ctx), |me, event, ctx| {
            if matches!(event, CockpitEvent::Updated) {
                me.on_cockpit_update(ctx);
            }
        });
        // Paint the initial state immediately (the model may already hold a
        // snapshot with waiting agents). This first update seeds the baseline
        // without chiming.
        let mut me = Self {
            prev_needs_me: 0,
            initialized: false,
        };
        me.on_cockpit_update(ctx);
        me
    }

    fn on_cockpit_update(&mut self, ctx: &mut ModelContext<Self>) {
        let model = CockpitModel::as_ref(ctx);
        let now = model.needs_me();
        let baseline_ready = attention_baseline_ready(&model.snapshot().health, model.inventory());
        let label = if now > 0 {
            now.to_string()
        } else {
            String::new()
        };
        ctx.set_application_badge_label(&label);
        // The chime is the only audible thing, and only on a genuine calm→needy
        // edge. The first refresh merely establishes the baseline: it has no
        // real "previous" value to form an edge against, so it never chimes (a
        // fleet already waiting at launch is shown, not sounded).
        let dnd = *CockpitSettings::as_ref(ctx).attention_dnd;
        let sound_on = *CockpitSettings::as_ref(ctx).attention_sound;
        let chime = self.record_update(now, baseline_ready, dnd || !sound_on);
        if chime {
            ctx.play_system_alert();
        }
    }

    fn record_update(&mut self, now: usize, baseline_ready: bool, suppressed: bool) -> bool {
        let chime = baseline_ready
            && should_chime_on_update(self.initialized, self.prev_needs_me, now, suppressed);
        self.prev_needs_me = now;
        // An incomplete refresh also breaks an existing baseline: recovery is
        // not evidence that the fleet genuinely crossed from calm to waiting.
        self.initialized = baseline_ready;
        chime
    }
}

impl Entity for AttentionDriver {
    type Event = ();
}

impl SingletonEntity for AttentionDriver {}
