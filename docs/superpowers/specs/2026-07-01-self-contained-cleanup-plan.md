# Self-contained cleanup & template-preservation plan

> Remove everything **Warp/Oz-specific and out-of-scope** from the shipped UI, but
> **preserve reusable pane/sidebar scaffolding as templates** so the later
> claudeplex migration (which may well need an agent panel again) does not have to
> rebuild from scratch. Guiding rule from the user (2026-07-01): *"alles Spezifische
> zu WARP und OZ entfernen, aber bestimmte Pane- und Sidebar-Elemente als Vorlagen
> aufheben."*
>
> Grounded in [[self-contained-audit-findings]]. The audit's headline still holds:
> there is **no live Warp-cloud client left** — this is de-Warp-ification of
> localized features, not cutting active cloud links.

## Guiding principle: disable-don't-delete for anything with template value

Three buckets. Default to the *least destructive* that still yields a clean,
self-contained shipped UI.

1. **HIDE + PRESERVE (template):** keep the code compiled and revivable, remove it
   from the shipped UI, strip Warp/Oz branding. Mechanism, in order of preference:
   - It's already **runtime-flag-gated** and the flag is off in release → nothing to
     do but confirm + de-brand. (Most Oz/Agent-Mode UI is here: `FeatureFlag::AgentMode`
     is **not** in `RELEASE_FLAGS`, so it's off in release; it shows mainly in dev.)
   - It's pushed unconditionally into a list → **stop pushing it**, but keep the enum
     variant + its render match-arms (needed for exhaustive matches) and its module.
   - Add a short `// PRESERVED TEMPLATE (not shipped): …` doc comment at the module
     head so intent is obvious to the next person.
2. **HARD-DELETE:** only truly dead cloud stubs with **zero template value** — no UI,
   no revival intent. (pricing/Stripe stub, telemetry send layer, `request_usage_model`
   always-unlimited stub, unreachable session-sharing transport, orphaned Drive
   sharing/ACL type shells.) Separate, later commits; each must keep the build green.
3. **KEEP (already local, in scope):** ConversationListView (local BYOP history), CLI
   coding agents, voice, ambient agents, external_secrets, autoupdate/changelog.

## Element-by-element

| Element | Bucket | Action |
|---|---|---|
| **Zaplex Drive** left-panel tab | HIDE+PRESERVE | Stop pushing `ToolPanelView::ZaplexDrive` in `compute_left_panel_views` (view.rs:18839). Keep the enum variant, all `ZaplexDrive` render arms, `app/src/drive/`, `app/src/cloud_object/` as the **sidebar-panel template**. |
| **Drive** menu/keybindings (`ToggleWarpDrive`) | HIDE+PRESERVE | Remove/de-list the menu item + keybinding entry; keep the action code. |
| **Oz "Agent" `+`-menu entry / Agent Mode** | HIDE+PRESERVE | Already `FeatureFlag::AgentMode`-gated (off in release). Confirm off; this is the **agent-panel template** for the future BYOP panel. |
| **Oz branding** ("Fix with Oz", "Oz needs permission", `cli_command_name="oz"`, agent name) | DE-BRAND | Replace Oz-specific user-visible strings with neutral wording; the "Fix with …" flow is repurposed later to the user's own agent (separate increment). |
| **Left-panel bar order** | REORDER | Put the remote-dev core first: SSH → Server Files → Project → Global Search → Conversations → Skills. Fix `unwrap_or(ZaplexDrive)` fallbacks → `SshManager`. |
| **pricing/Stripe, telemetry send, request_usage, session-sharing transport, Drive ACL shells** | HARD-DELETE | Later dedicated commits; keep build green each step. |
| **zap_sync (Gist), models.dev fetch, resource_center warp.dev links** | FLAG (own decision) | Not in this pass; surfaced in [[self-contained-audit-findings]] for a separate call. |

## Order of execution (step 2, low-risk first)

1. **Left-panel bar**: drop Drive tab + reorder (this commit). ✔ low risk, visible.
2. **A1 Save** dirty-tracking; **A2** add-mode vs list distinction.
3. **Oz de-branding** sweep (strings only; no infra removal).
4. Drive menu item / keybinding de-listing.
5. Hard-delete the zero-value dead stubs (bucket 2), one green commit each.
6. Oz **repurpose** → "Fix with <Claude/Codex>" — its own spec + implementation.

Each step keeps the build green and the preserved templates compiling.
