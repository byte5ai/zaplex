//! The attention **ambient bit** — one passive, fleet-wide "is there anything
//! for me?" signal, and nothing else. Deliberately the *opposite* of
//! per-event notification spam: there is no toast, no banner, no OS
//! notification per session. A single indicator reflects
//! [`CockpitModel::needs_me`] and stays visible even when the app is minimised.
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
//! Everything platform-specific is `#[cfg(target_os = "macos")]`; every other
//! platform gets a typechecking no-op so the driver logic below is
//! platform-agnostic.

use warpui::{Entity, ModelContext, SingletonEntity};

use crate::cockpit::model::{CockpitEvent, CockpitModel};
use crate::cockpit::settings::CockpitSettings;

/// Paint the fleet-wide *needs-me* count on the ambient surface (the macOS Dock
/// badge). `count == 0` clears the badge; `> 0` shows the number. A no-op on
/// non-macOS platforms.
#[cfg(target_os = "macos")]
pub fn set_attention(count: usize) {
    #[allow(deprecated)]
    use cocoa::{
        appkit::NSApp,
        base::{id, nil},
    };
    use objc::{msg_send, sel, sel_impl};
    use warpui::platform::mac::{make_nsstring, AutoreleasePoolGuard};

    // `AutoreleasePoolGuard` drains on `Drop`, covering every exit path — same
    // idiom as `appearance::set_app_icon`.
    unsafe {
        let _pool = AutoreleasePoolGuard::new();
        let app: id = NSApp();
        if app == nil {
            return;
        }
        let tile: id = msg_send![app, dockTile];
        if tile == nil {
            return;
        }
        // An empty label clears the badge; the count otherwise. The dock tile
        // copies the string, so the autoreleased `NSString` is safe to pass.
        let label: id = if count > 0 {
            make_nsstring(count.to_string())
        } else {
            make_nsstring("")
        };
        let _: () = msg_send![tile, setBadgeLabel: label];
    }
}

/// No-op ambient surface off macOS (the dock badge has no cross-platform
/// equivalent yet; the in-app "Offene Punkte" inbox carries attention there).
#[cfg(not(target_os = "macos"))]
pub fn set_attention(_count: usize) {}

/// Play the single, subtle attention chime (the system alert sound). Called at
/// most once per calm→needy edge; never per session. A no-op off macOS.
#[cfg(target_os = "macos")]
pub fn play_chime() {
    // `NSBeep` is the plain system alert sound — no asset, no retained sound
    // object, no volume knob. AppKit is already linked (cocoa crate).
    extern "C" {
        fn NSBeep();
    }
    unsafe {
        NSBeep();
    }
}

/// No-op chime off macOS.
#[cfg(not(target_os = "macos"))]
pub fn play_chime() {}

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
        let now = CockpitModel::as_ref(ctx).needs_me();
        // The passive surface always tracks the truth, DND or not — including on
        // the very first refresh.
        set_attention(now);
        // The chime is the only audible thing, and only on a genuine calm→needy
        // edge. The first refresh merely establishes the baseline: it has no
        // real "previous" value to form an edge against, so it never chimes (a
        // fleet already waiting at launch is shown, not sounded).
        let dnd = *CockpitSettings::as_ref(ctx).attention_dnd;
        let sound_on = *CockpitSettings::as_ref(ctx).attention_sound;
        if should_chime_on_update(self.initialized, self.prev_needs_me, now, dnd || !sound_on) {
            play_chime();
        }
        self.prev_needs_me = now;
        self.initialized = true;
    }
}

impl Entity for AttentionDriver {
    type Event = ();
}

impl SingletonEntity for AttentionDriver {}

#[cfg(test)]
mod tests {
    use super::{should_chime, should_chime_on_update};

    #[test]
    fn first_update_never_chimes_even_when_already_waiting() {
        // The driver's first real refresh only seeds the baseline: a fleet that
        // is already waiting at launch (0 → N against the seed) must NOT chime.
        assert!(!should_chime_on_update(false, 0, 5, false));
        assert!(!should_chime_on_update(false, 0, 1, false));
        // ...but every later genuine calm→needy edge does.
        assert!(should_chime_on_update(true, 0, 5, false));
        assert!(should_chime_on_update(true, 0, 1, false));
        // Later updates still respect the pure edge + suppression rules.
        assert!(!should_chime_on_update(true, 2, 3, false)); // already needy
        assert!(!should_chime_on_update(true, 0, 1, true)); // suppressed
    }

    #[test]
    fn chimes_only_on_zero_to_nonzero_edge() {
        // The one case that chimes: calm → needy.
        assert!(should_chime(0, 1, false));
        assert!(should_chime(0, 5, false));
    }

    #[test]
    fn no_chime_without_an_edge() {
        // Staying calm.
        assert!(!should_chime(0, 0, false));
        // Already needy, another agent joins — the badge grows, no new chime.
        assert!(!should_chime(2, 3, false));
        assert!(!should_chime(1, 1, false));
        // Dropping back toward calm never chimes.
        assert!(!should_chime(3, 1, false));
        assert!(!should_chime(1, 0, false));
    }

    #[test]
    fn suppressed_never_chimes_even_on_the_edge() {
        // DND / sound-off silences the one edge that would otherwise chime.
        assert!(!should_chime(0, 1, true));
        assert!(!should_chime(0, 9, true));
    }
}
