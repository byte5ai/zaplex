---
name: warp-ui-guidelines
description: Catalog of guidelines for writing UI code in the Warp client. Read whenever doing any UI work in this repo, up front before writing the change, so the relevant guidelines shape the implementation.
---

# warp-ui-guidelines

This skill is a growing catalog of guidelines for working on Warp's UI code. Each guideline captures a lesson that would otherwise be re-learned through review — typically because an agent or contributor reinvented a component, drifted from the design system, or bypassed a shared abstraction.

**How to use this skill:**

- Read through the guidelines below once at the start of any UI task, then keep them in mind while implementing. The list is short enough to scan.
- Each guideline is self-contained. Not every one will apply to every task — use judgment. But if a guideline *does* apply, follow it.
- When in doubt, prefer reusing an existing abstraction over introducing a new one. The Warp UI has accumulated a well-factored set of shared components and themes; new one-offs almost always drift.

New guidelines get added here over time. If you discover a recurring UI mistake that would have been caught by a written rule, add it.

---

## Guideline: Reuse button themes

Button colors come from a shared set of `ActionButtonTheme` impls in `app/src/view_components/action_button.rs` (and the parallel `Theme` impls in `crates/ui_components/src/button/themes.rs`) — `PrimaryTheme`, `SecondaryTheme`, `NakedTheme`, `DangerPrimaryTheme`, etc. These encode the design system and keep button colors consistent across the app.

When styling a button, **use one of the existing themes unchanged**. The shared themes are well-established and vetted; if one looks "wrong" for your use case, the most likely explanation is that you're reaching for the wrong theme, not that the theme is buggy.

Do **not** modify a shared theme on your own initiative. Changing `PrimaryTheme`, `SecondaryTheme`, etc. affects every button in the app, and a tweak that fixes your screen can silently regress others. Only edit a shared theme when the user has explicitly confirmed that the design-system component itself needs to change.

Red flags that you're about to make buttons inconsistent:

- Writing a new `impl ActionButtonTheme for FooPrimaryTheme` that delegates to `PrimaryTheme` and only tweaks one method (usually `text_color`). Almost always the right move is to use `PrimaryTheme` directly and accept the result.
- Hard-coding `ColorU::new(...)` instead of using `appearance.theme()` accessors (`accent`, `font_color(bg)`, `foreground`, etc.).
- Setting `should_opt_out_of_contrast_adjustment` to `true` to force a specific label color.
- Naming a theme after a feature or view (`FooPrimaryTheme`, `BarSubmitTheme`) rather than a design-system role.

If an existing theme genuinely doesn't fit and you think the shared theme should change, surface that to the user before editing it, rather than either editing it unilaterally or papering over it with a one-off.

---

## Guideline: Protect identity in compact repeated rows

In a narrow sidebar or another compact repeated list, the row's unique object identity is the primary content. Give that identity the flexible slot (`Shrinkable` with weight `1.0`) and keep status, metrics, and secondary actions in fixed-width slots. A hostname, session name, branch, or file name must never be truncated merely because every row repeats the same action words.

Repeated secondary actions in these rows are **icon-only**. Build them with `CompactRowAction` from `app/src/ui_components/compact_row_action.rs`; its API deliberately accepts an icon and localized tooltip, but no visible label, and reuses `PaneHeaderTheme` unchanged. Use a stable, domain-matching icon everywhere, and do not reveal or replace actions on hover because that changes row geometry.

Visible text buttons remain appropriate for forms, dialogs, empty states, roomy toolbars, and a single primary action whose meaning would otherwise be unclear. They are not appropriate for the same secondary verb repeated on every row of a width-constrained list.

Before delivery, inspect the minimum supported panel width with the longest realistic identity and the longest supported translation. The identity should consume all remaining width, action geometry must stay constant across locales and hover states, and each icon must retain its localized tooltip.
