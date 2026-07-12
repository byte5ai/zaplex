# RC UI/UX audit findings — codex + grok (2026-07-12, read-only)

> Raw findings from the two independent audits that produced the RC master plan
> (`2026-07-12-rc-master-plan.md`). Preserved for the implementing session — full
> file:line evidence per workstream. Both models converged on the same 5 workstreams.

---
## grok audit

# UI/UX audit — zaplex RC readiness (read-only)

**Yardstick:** `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md` (binding). **`docs/superpowers/specs/2026-07-11-cockpit-sidebar-redesign.md` is not in the tree** (no `cockpit-sidebar` spec under `docs/`); sidebar gaps are inferred from that spine doc (B1/B2, D4/D7) and from `app/src/cockpit/panel.rs`.

---

## Modals

| # | Title | Evidence | Violates / gap | Class |
|---|--------|----------|----------------|-------|
| M1 | **At least four modal stacks, not one** | Consolidation TODO: `app/src/modal.rs:110`. Patterns in parallel: `Modal<T>` (`modal.rs:18–37`, scrim `ColorU::new(0,0,0,179)` at `454`), `Dialog` + `Dismiss` (`ui_components/dialog.rs:12–28`), cockpit hand-roll (`spawn_card.rs:943–986`, `attention_inbox.rs:424–453`), SFTP (`sftp_manager/dialogs.rs` e.g. `158`, `722`). | Spine: “one premium visual language”; modals feel like different apps. | **A** |
| M2 | **Spawn card is a full custom overlay (not `Modal` / `Dialog`)** | No `Dialog::` / `Modal::` in `spawn_card.rs`; full-screen `Hoverable` veil + centered `Stack` (`943–986`), `MODAL_WIDTH` 480 (`44`), `modal_scrim()` (`976`), chip footer (`909–920`), close = `Icon::X` `Hoverable` (`653–658`). Workspace stacks raw child (`view.rs:24402–24403`), not `ModalViewState`. | Deviates from `Modal` close (`modal.rs:238–245`) and `Dialog` header/footer (`dialog.rs:117–135`). | **A** |
| M3 | **Attention inbox shares cockpit scrim but not `Dialog`/`Modal` chrome** | `attention_inbox.rs:424–453`: `PhenomenonStyle::modal_background()`, `MODAL_RADIUS`, `modal_scrim()`; close = `ActionButton` + `Icon::X` (`124–128`, `224–226`). **No** backdrop click-to-dismiss (contrast `spawn_card.rs:979–985`). | Inconsistent dismiss grammar between the two “cockpit modals.” | **A** |
| M4 | **Git dialog follows `Dialog`; cockpit modals do not** | `git_dialog/mod.rs:693–758`: `Dialog::new` + `with_close_button` + `dialog_styles`, overlay `blurred_background_overlay()` (`757–758`) — third scrim variant vs `modal_scrim()` (`cockpit/style.rs:47–50`). | Same product, three overlay treatments. | **A** |
| M5 | **`SessionConfigModal`: `Modal` wrapper + body-built chrome** | Wrapped: `view.rs:1966–1972` `Modal::new(None, body, …)`. Body: English title/subtitle literals (`session_config_modal.rs:168–185`), own 420px `Stack` + corner `ActionButton` close (`314–331`), not `Dialog`. | “One modal pattern” broken; double chrome risk when title is set. | **A** |
| M6 | **`NewWorktreeModal`: `Modal` shell, Figma-custom body** | `view.rs:2004–2024` documents intentional `title: None`; body hardcodes `"New worktree"` (`new_worktree_modal.rs:324–325`), `"Select repository"` / `"Select branch"` (`407–412`), English validation (`74–75`, `494–495`). | Same as M5 + i18n (below). | **A** |
| M7 | **`TabConfigParamsModal` is body-only inside `Modal`** | `params_modal.rs:118–119`, cancel/submit use `t!` (`158–173`) — closer to standard, still depends on outer `Modal` from workspace. | Acceptable structurally; still different width/padding/scrim from spawn/inbox. | **B** |
| M8 | **`EnumCreationDialog`: inline card, no shared overlay/dismiss** | `enum_creation_dialog.rs:919–953`: constrained `Flex` + `2.` border (`945`), custom header/footer (`622–636`, `855–899`); **no** `FixedBinding` for Escape in file. Uses `t!` for actions. | Foreign body vs `Dialog`/`Modal`; dismiss depends on parent. | **A** |
| M9 | **SFTP dialogs: separate title bar + English copy** | `sftp_manager/dialogs.rs:158`, `722`, `528`, etc. — not `ui_components/dialog.rs`. | Warp-legacy island inside zaplex chrome. | **B** (large surface, often power-user) |

**Spawn card quantified:** ~100% custom path (scrim, layout, close, footer, width, backdrop dismiss). It **does** align with cockpit tokens (`cockpit/style.rs:44–50` referenced at `spawn_card.rs:37`). It **does not** use the shared `Dialog` / `Modal` components.

---

## i18n

| # | Title | Evidence | Violates / gap | Class |
|---|--------|----------|----------------|-------|
| I1 | **Cockpit sidebar mixes `t!` with English runtime strings** | `panel.rs:261–265` `"today {} · {}"`; `282–289` `"active"`, `"waiting"`, `"running"`; `727–730` `"{} account{}"`; `756` `"⤢  Dashboard"`; `501` `"… {} more"`. Conductor title uses `t!` (`378`). | German locale + English fleet copy. | **A** |
| I2 | **Tab-config / first-tab flows hardcode English** | `session_config_modal.rs:168–185`; `session_config_rendering.rs:234` `"Select directory"`; `366` English tooltip; `new_worktree_modal.rs:325–325`, `407–412`, `464–465`, `74–75`. | Bypasses `warp.ftl` on high-traffic onboarding modals. | **A** |
| I3 | **Spawn card: mostly `t!`, but English editor placeholder** | `spawn_card.rs:249–250` `"remote directory (absolute path) — blank = host home"`. DE `warp.ftl` has spawn strings (`de/warp.ftl:56–78`) but placeholder not in FTL. | Visible EN in DE spawn flow for remote dir. | **A** |
| I4 | **Missing DE string → EN fallback for SSH connecting** | `en/warp.ftl:2833` `workspace-left-panel-ssh-manager-connecting = connecting…`; **no** match in `de/warp.ftl` (grep). Used at `ssh_manager/panel.rs:2215–2216`. | Literal “connecting…” in DE UI. | **A** |
| I5 | **Cockpit settings descriptions all English** | `cockpit/settings.rs:18–68` — every `description:` string English (shown in settings UI). | Settings read as EN product with DE chrome elsewhere. | **A** |
| I6 | **DE FTL still embeds emoji / anglicisms** | `de/warp.ftl:65–66` `⚡ Freest`; `91` `📁`; subtitle `57` uses “Launch”. | Icon pass + locale purity incomplete. | **B** |
| I7 | **SFTP / file manager dialogs English-only** | `sftp_manager/dialogs.rs` (see M9). Pane title `"File Manager"` (`sftp_manager/browser.rs:418`). | Large EN island. | **B** |

Spawn card is **not** a major i18n offender for labels (heavy `t!` usage); **sidebar, tab modals, SSH connecting, settings, and placeholders** are.

---

## Icons

| # | Title | Evidence | Violates / gap | Class |
|---|--------|----------|----------------|-------|
| IC1 | **⚡ remains UI chrome (not icon font)** | `ssh_manager/panel.rs:2199–2200` literal `"⚡"`; terminal status uses ANSI ⚡ (`daemon_tty/event_loop.rs:543`); FTL chips `de/warp.ftl:65–66`. | Spine D7 “⚡ badge quiet/wertig”; still Unicode bolt in tree. | **A** |
| IC2 | **Conductor verbs use Unicode, not `icons::`** | `panel.rs:646` `verb_button(..., "◈", …)`; favorites use `Icon::Stars` (`705`) — **mixed** on one row. `pane.rs:1387` `"◈ review"`. | “#107 icon pass” incomplete; glyph vocabulary ≠ icon font. | **A** |
| IC3 | **Status / attention glyphs system-wide** | `cockpit/pane.rs:807`, `1564` `✋`; `panel.rs:282–289` `●` / `◌`; `style.rs:33` documents `●` vs `✋`. Titlebar pulse `view.rs:19212+`. | Intentional design language per spine §2.5, but conflicts with “emoji replaced by icon font” claim for **chrome**. | **B** (concept allows ✋; inconsistent with icon-font goal) |
| IC4 | **Emoji in localized SSH empty state** | `de/warp.ftl:91` `📁` in user string. | i18n + icon system leak. | **B** |
| IC5 | **Spawn close uses icon font; inbox uses same `Icon::X`** | `spawn_card.rs:655` `Icon::X`; `attention_inbox.rs:126` — good. Footer uses **chips**, not `ActionButton` (`spawn_card.rs:890–914`). | Close consistent; primary actions differ from `GitDialog` / `Modal`. | **B** |

---

## Visual language

| # | Title | Evidence | Violates / gap | Class |
|---|--------|----------|----------------|-------|
| V1 | **Modal geometry tokens disagree** | Cockpit `MODAL_RADIUS` 10 (`style.rs:45`); generic `MODAL_CORNER_RADIUS` 8 (`modal.rs:18`); spawn width 480 (`spawn_card.rs:44`) vs `MODAL_WIDTH` 440 (`modal.rs:19`); inbox width 460 (`attention_inbox.rs:45`). | “One premium visual language” (spine §0, §2.5). | **A** |
| V2 | **Three scrim/background systems** | `modal_scrim()` RGBA (`style.rs:49–50`); `Modal` opacity 179 (`modal.rs:159`, `453`); git `blurred_background_overlay()` (`git_dialog/mod.rs:757–758`); inbox `PhenomenonStyle::modal_background()` (`attention_inbox.rs:443`). | Calm, unified overlays — not there yet. | **A** |
| V3 | **Sidebar redesign (two zone-cards, provider≠plan) not shipped** | Current layout: account header + conductor block + per-account cards (`panel.rs:799–843`); plan as pill on card (`248–258`). No zone-card split; spec file cited by you is absent from repo. | Gap vs stated 2026-07-11 direction; spine B2/B1 still open (`integrated-ux-spine-design.md:68–72`). | **A** (if that spec is still target) / **B** (if only spine B2 polish) |
| V4 | **Dashboard entry still hybrid glyph + English** | `panel.rs:753–756` `⤢  Dashboard` (improved from bare glyph per comment, still not `t!`/icon-only). | Discoverability + locale (ties I1). | **B** |
| V5 | **Account/session metrics typography ad hoc** | Heat bars centralized (`style.rs`); copy like `5h ↻` / `wk ↻` built inline (`panel.rs:296–300`) without FTL. | Polish debt per spine D7. | **B** |

`cockpit/style.rs:1–20` shows deliberate unification for cockpit modals; **the rest of the app did not join that system.**

---

## Completeness

| # | Title | Evidence | Violates / gap | Class |
|---|--------|----------|----------------|-------|
| C1 | **Conductor is not the primary spine in navigation** | Full tree in sidebar (`panel.rs:815–824`) but toolbelt is one icon among many (`left_panel.rs:96`, `611–617`, `1230–1231`). Spine doc D4: “buried among 8 sidebar icons.” | “One object model, one tree” as **primary** nav — not RC-complete. | **A** |
| C2 | **Spawn card discoverability incomplete vs spine A4** | Menu: `t!("cockpit-spawn-card-new-agent")` (`view.rs:8185–8186`). `AddAgentTab` → spawn when cockpit on (`21690–21691`). **No** `OpenSpawnCard` in `app/src/search` (grep empty). No dedicated editable binding for scoped spawn in `workspace/mod.rs` (only attention inbox `145–152`, jump `675`). | “≤2 obvious steps” + palette for every built capability. | **A** |
| C3 | **File manager still terminal-centric** | `OpenFileManagerHere` in terminal pane header (`terminal/view/pane_impl.rs:456`, `545+`); spine D5. No workspace-level palette grep hit for global open. | Hidden capability. | **A** |
| C4 | **Blind agent launch when cockpit disabled** | `view.rs:8218–8222` `AddSpecificAgentTab` in “+” menu if `!CockpitSettings.enabled`. | Violates launch grammar when flag off (edge case, but incoherent). | **B** |
| C5 | **Directory on spawn card — largely addressed** | Rows for local/remote dir, browse, remote editor (`spawn_card.rs:832–872`, `609+`). Updates spine D3 from “no picker” — **partial fix** vs ledger. | Reduces RC gap for launch grammar. | *(positive)* |
| C6 | **Titlebar launch regression fixed; attention path wired** | Retired titlebar agents (`view.rs:19265–19269`); pulse + inbox binding (`mod.rs:145–152`). | Spine A2 largely met. | *(positive)* |
| C7 | **Attention inbox: remote rows non-clickable by design** | Documented `attention_inbox.rs:12–13`, `230–231`. | Missing affordance/state copy for “awareness only” rows — verify DE strings exist. | **B** |
| C8 | **Enum / workflow UI reachable but modal-dismiss inconsistent** | Workflow enum uses custom dialog (M8). | Power-user completeness OK; UX consistency not. | **B** |

---

## Verdict (RC gate)

The product has a **clear cockpit design system** (`cockpit/style.rs`, spawn card + inbox i18n for core launch), and **launch grammar / titlebar attention** are largely aligned with the 2026-07-08 spine. RC is still blocked by **fragmented modal architecture** (spawn card, inbox, `Modal`, `Dialog`, SFTP, enum card all different), **systematic DE/EN mixing** on surfaces users see daily (sidebar metrics, tab/worktree modals, SSH connecting, settings descriptions, remote-dir placeholder), and **incomplete discoverability** (spawn scoped paths and file manager not on par with attention inbox). Icon work is **half-done**: icon font for close/star vs Unicode **⚡ ◈ ● ✋ ⤢** and emoji in FTL.

**Five workstreams that unlock a usable RC:**

1. **Unify modal chrome** — Pick one of `Modal` or `Dialog` (or a thin cockpit wrapper around them), migrate spawn card + attention inbox + enum shell; one scrim, radius, width band, close + footer conventions; optional shared backdrop-dismiss policy.

2. **i18n sweep (Class A surfaces)** — `panel.rs` runtime strings, `session_config_*` / `new_worktree_modal.rs`, `spawn_card` placeholder, `cockpit/settings.rs`, add `de` for `workspace-left-panel-ssh-manager-connecting`; remove emoji from `warp.ftl` where icons exist.

3. **Icon/glyph policy** — Replace ⚡/◈/⤢ chrome with `icons::` (keep ✋/● only if spec explicitly retains them); align with inbox/spawn close pattern.

4. **Discoverability pass (spine A4)** — Command palette + keybindings for `OpenSpawnCard` (incl. scoped), file manager, cockpit dashboard; document in DE `keybinding-desc-*`.

5. **Sidebar / spine B1** — Either implement the missing sidebar redesign spec (zone-cards, provider≠plan) or update the spec; until then, treat conductor-as-secondary-icon (C1) as a known RC limitation and prioritize “open dashboard” + palette paths.

After (1)–(4), remaining items are mostly **Class B** polish (SFTP English, worktree Figma modals, OAuth/cockpit B3 correctness per spine). That is a credible **RC**: one visual/modal language, one locale on primary paths, and launch/attention surfaces reachable without hunting menus.

---
## codex audit (findings + verdict)


   - sichtbares `⚡` am SSH-Host: [ssh_manager/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ssh_manager/panel.rs:2200)
   - sichtbares `⚡` in Terminal-Statuszeilen: [event_loop.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/terminal/daemon_tty/event_loop.rs:543)
   - `⚡ Freest` liegt zusätzlich als Text in der deutschen Fluent-Datei: [warp.ftl](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/i18n/de/warp.ftl:65)
   - Cockpit-Review bleibt ein Textglyph `◈`: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:646)

   Gleichzeitig existiert bereits ein Icon-Font-Helfer und wird für Favoriten benutzt: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:128), [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:688). Literalzeichen und Icon-Font laufen somit sichtbar parallel.

10. **Statusglyphen sind konzeptuell absichtlich, aber technisch Textglyphen — Class B**

   `●`, `✋` und `◦` sind Teil des bindenden Statusvokabulars: [integrated-ux-spine-design.md](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md:50). Die gemeinsame Breitenzelle reduziert metrische Unterschiede: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:32). Das ist konsistent genug für RC; `⚡`, `◇`, `⑂` und `◈` sind dagegen Aktions-/Chrome-Symbole und sollten in den Icon-Font.

## Visual Language

11. **Die konkrete Sidebar-Redesign-Spec fehlt im Checkout — Class A**

   `docs/superpowers/specs/2026-07-11-cockpit-sidebar-redesign.md` ist nicht vorhanden. Damit sind die genannten B2/B3/B4/D1+C2-Entscheidungen weder prüfbar noch als Implementierungsvertrag verfügbar. Das ist selbst eine Spec-/RC-Governance-Lücke.

12. **Der Sidebar-Spine ist teilweise gebaut, aber nicht vollständig auf die gewünschte Zeilenstruktur normiert — Class A**

   Der vorhandene Nordstern verlangt Host-Wurzeln, Projekte/Sessions, feste rechte Metrikspalte und Status nur als Farbpunkt: [integrated-ux-spine-design.md](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md:254).

   Positiv gebaut:

   - „Add host“ direkt am Spine: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:526)
   - lokale und entfernte Sessions sind anklickbar: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:623)
   - Favoriten-Stern ist vorhanden: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:656)

   Abweichung: Modell/Effort und Prozent werden als normale nachlaufende Flex-Kinder angefügt, nicht als fest vermessene rechte Spalte: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:596). Damit können Werte und Labels horizontal springen; genau das verbietet die Spezifikation.

13. **Cockpit besitzt eine eigene, vom Rest isolierte Tokenwelt — Class A**

   Der Cockpit-Stil zentralisiert zwar intern Scrim, Radien, Farben und Verben: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:1). Diese Tokens sind aber nicht der appweite Dialog-/Layout-Standard. Das Cockpit ist intern kohärenter als vorher, bleibt gegenüber Git-, Workflow-, Tab-Config- und Settings-Flächen ein eigenes Designsystem.

14. **Header-, Footer- und Spacing-Werte variieren ohne gemeinsamen Maßstab — Class B**

   Beispiele: Standarddialog 20px Padding/8px Radius; `Modal<T>` 28px/8px; Cockpit 24px/10px; Session Config 32px horizontal/40px vertikal. Siehe [dialog.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ui_components/dialog.rs:15), [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:18), [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:924), [session_config_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/session_config_modal.rs:309).

## Completeness

15. **Cockpit-Pane enthält einen erreichbaren `unimplemented!()`-Pfad — Class A**

   Der Pane-Overflow-Handler endet in `unimplemented!()`: [cockpit/pane.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/pane.rs:1725). Falls das Framework den Header-Overflow auslöst, ist das ein Panic statt eines fehlenden/disabled Menüs. Vor RC muss der Pfad implementiert oder die Affordance sicher entfernt werden.

16. **Attention Inbox zeigt Remote-Zeilen, lässt sie aber bewusst nicht öffnen — Class A**

   Remote-/nicht-resumierbare Einträge werden als statische Zeilen gerendert: [attention_inbox.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/attention_inbox.rs:255), [attention_inbox.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/attention_inbox.rs:303). Im Sidebar-Spine sind Remote-Sessions dagegen anklickbar. Dieselbe Session wirkt in zwei Cockpit-Linsen unterschiedlich interaktiv – ein direkter Bruch des „one object model / one operating grammar“.

17. **MCP-Settings bieten Aktionen an, deren Handler nicht implementiert sind — Class A**

   Delete, View Logs und Run sind für mehrere sichtbare Itemtypen nur Logmeldungen:

   - Delete: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:492)
   - Logs: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:595)
   - Run: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:625)

   Das sind echte Built-but-nonfunctional-Flächen. Für RC müssen die Aktionen funktionieren oder capability-basiert gar nicht erscheinen.

18. **Einzelne Altflächen wurden bereits korrekt entfernt, aber nicht systematisch — Class B**

   Settings dokumentiert die Entfernung zweier Stub-/Empty-List-Seiten: [settings_view/mod.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mod.rs:82). Das ist die richtige RC-Richtung; derselbe „funktioniert oder wird nicht angeboten“-Standard fehlt noch bei MCP und Pane-Overflow.

## Verdict

Zaplex ist noch kein „nur noch Polish“-RC. Die fünf fundamentalen Workstreams sind: erstens ein appweit verbindliches Modal-/Dialog-System und Migration der Fremdkörper; zweitens vollständige deutsche Fluent-Abdeckung plus Verbot harter UI-Literale; drittens Abschluss des Icon-Passes für alle Chrome-/Action-Glyphen; viertens Fertigstellung des Sidebar-Spine inklusive fester Metrikspalte und Wiederherstellung der fehlenden Redesign-Spec; fünftens Entfernung aller angebotenen, aber nicht funktionierenden Interaktionen – besonders Remote-Inbox, MCP-Aktionen und der Cockpit-Panicpfad. Danach bleibt überwiegend echte Class-B-Politur.
tokens used
97,442
## Modals

1. **Drei konkurrierende Modal-Systeme statt eines Standards — Class A**

   Es existieren parallel:

   - generischer `Modal<T>` mit Scrim, Escape, Close-Button und optionalem Click-outside-Dismiss: [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:24), [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:109), [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:169)
   - separater `ui_components::Dialog` für Header, Footer und `ActionButton`s: [dialog.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ui_components/dialog.rs:31), [dialog.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ui_components/dialog.rs:67)
   - handgebaute Cockpit-Modals mit eigenem Scrim/Radii: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:44)

   Der Code bestätigt sogar selbst den offenen Konsolidierungsbedarf: [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:109). Das verletzt „one premium visual language“ aus [integrated-ux-spine-design.md](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md:18).

2. **Spawn Card ist vollständig handgebaut — Class A**

   Die Spawn Card verwendet weder `Modal<T>` noch `Dialog` oder `ActionButton`. Sie baut selbst:

   - Titel und X-Button: [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:643)
   - Chip-basierte Confirm/Cancel-Aktionen: [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:886)
   - Hintergrund, Border, Radius und Shadow: [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:924)
   - kompletten Overlay-Stack und Click-outside-Verhalten: [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:943)

   Quantifiziert: mindestens vier Modal-Verantwortungen werden lokal dupliziert. Positiv: Die frühere Behauptung „kein Close-Affordance“ ist im aktuellen Code nicht mehr wahr; X, Cancel, Escape und Click-outside sind vorhanden. Das Problem ist Konsistenz, nicht fehlende Schließbarkeit.

3. **Dismiss-Verhalten ist zwischen Cockpit-Modals inkonsistent — Class A**

   Die Spawn Card schließt beim Klick auf den Scrim: [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:979). Die Attention Inbox baut denselben Scrim, aber ohne Click-Handler: [attention_inbox.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/attention_inbox.rs:424). Sie kann nur über X/Escape geschlossen werden.

4. **Auch etablierte Dialoge weichen untereinander stark ab — Class A**

   - Git-Dialog folgt dem `Dialog`-Muster sauber: Header-Icon, X, Separator, Cancel/Confirm: [git_dialog/mod.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/mod.rs:693)
   - New Worktree benutzt zwar außen `Modal<T>`, baut innen aber erneut eigenen Header, X+ESC-Badge und Footer: [new_worktree_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/new_worktree_modal.rs:322), [new_worktree_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/new_worktree_modal.rs:517)
   - Params Modal dupliziert denselben speziellen X+ESC-Header: [params_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/params_modal.rs:528)
   - Session Config baut ein drittes Layout mit 24px-Titel, absolut positioniertem X und nur einem primären Button: [session_config_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/session_config_modal.rs:165), [session_config_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/session_config_modal.rs:299)
   - Enum Creation ist eine eigenständige 2px-Rahmenkarte ohne Standard-`Dialog`: [enum_creation_dialog.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/drive/workflows/enum_creation_dialog.rs:919)

   Für einen RC braucht es einen verbindlichen Modal-Vertrag: Scrim, Radius, Titeltypografie, X, Escape, Click-outside, Footer-Reihenfolge und Button-Komponenten.

## i18n

5. **Die deutsche Lokalisierung ist strukturell unvollständig — Class A**

   Deutsch ist offiziell auswählbar: [language.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings/language.rs:33). Fehlende Schlüssel fallen jedoch automatisch auf Englisch zurück: [i18n.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/i18n.rs:7).

   Statische Inventur:

   - Englisch: 3.233 Fluent-Schlüssel
   - Deutsch: 333 Schlüssel
   - 2.900 englische Schlüssel fehlen in Deutsch

   Damit ist der EN/DE-Mix kein Einzelfehler, sondern der normale Fallbackpfad.

6. **User-facing Text umgeht Fluent direkt — Class A**

   Konkrete Beispiele:

   - komplette Session-Config-Einführung: [session_config_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/session_config_modal.rs:168)
   - „New worktree“ und „Select repository“: [new_worktree_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/new_worktree_modal.rs:324), [new_worktree_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/new_worktree_modal.rs:407)
   - Git-Dialog-Titel: [git_dialog/mod.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/mod.rs:656)
   - Commit-Varianten und Validierungstooltip: [git_dialog/commit.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/commit.rs:97), [git_dialog/commit.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/commit.rs:225)
   - „Changes“, „Include unstaged“, „Included commits“: [git_dialog/commit.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/commit.rs:429), [git_dialog/pr.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/pr.rs:172), [git_dialog/push.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/code_review/git_dialog/push.rs:190)
   - Settings-Aktionen und „Context window“: [ai_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/ai_page.rs:235), [ai_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/ai_page.rs:4377)
   - native Modal „Don't show again“: [native_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/native_modal.rs:140)

   Ein enger Render-API-Grep findet mindestens 68 direkte verdächtige String-Callsites; mehrzeilige und indirekt über Settings-Metadaten eingespeiste Texte kommen hinzu.

7. **Cockpit-Settings erscheinen mit englischen Beschreibungen — Class A**

   Sämtliche Cockpit-Beschreibungen sind feste englische Strings, teilweise sogar intern gemischt („Offene-Punkte inbox“): [cockpit/settings.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/settings.rs:18), [cockpit/settings.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/settings.rs:48), [cockpit/settings.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/settings.rs:58).

8. **Sprachumschaltung ist nicht live — Class B**

   Die eigene Dokumentation sagt, dass sie erst nach Neustart vollständig greift: [language.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings/language.rs:8). Akzeptabel für RC, sofern die UI das klar kommuniziert.

## Icons

9. **Der Icon-Pass ist nicht abgeschlossen — Class A**

   `rg '⚡|✧|📌|✋|◇|⑂' app/src` zeigt weiterhin Literal-Glyphen im UI:

   - sichtbares `⚡` am SSH-Host: [ssh_manager/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ssh_manager/panel.rs:2200)
   - sichtbares `⚡` in Terminal-Statuszeilen: [event_loop.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/terminal/daemon_tty/event_loop.rs:543)
   - `⚡ Freest` liegt zusätzlich als Text in der deutschen Fluent-Datei: [warp.ftl](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/i18n/de/warp.ftl:65)
   - Cockpit-Review bleibt ein Textglyph `◈`: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:646)

   Gleichzeitig existiert bereits ein Icon-Font-Helfer und wird für Favoriten benutzt: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:128), [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:688). Literalzeichen und Icon-Font laufen somit sichtbar parallel.

10. **Statusglyphen sind konzeptuell absichtlich, aber technisch Textglyphen — Class B**

   `●`, `✋` und `◦` sind Teil des bindenden Statusvokabulars: [integrated-ux-spine-design.md](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md:50). Die gemeinsame Breitenzelle reduziert metrische Unterschiede: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:32). Das ist konsistent genug für RC; `⚡`, `◇`, `⑂` und `◈` sind dagegen Aktions-/Chrome-Symbole und sollten in den Icon-Font.

## Visual Language

11. **Die konkrete Sidebar-Redesign-Spec fehlt im Checkout — Class A**

   `docs/superpowers/specs/2026-07-11-cockpit-sidebar-redesign.md` ist nicht vorhanden. Damit sind die genannten B2/B3/B4/D1+C2-Entscheidungen weder prüfbar noch als Implementierungsvertrag verfügbar. Das ist selbst eine Spec-/RC-Governance-Lücke.

12. **Der Sidebar-Spine ist teilweise gebaut, aber nicht vollständig auf die gewünschte Zeilenstruktur normiert — Class A**

   Der vorhandene Nordstern verlangt Host-Wurzeln, Projekte/Sessions, feste rechte Metrikspalte und Status nur als Farbpunkt: [integrated-ux-spine-design.md](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md:254).

   Positiv gebaut:

   - „Add host“ direkt am Spine: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:526)
   - lokale und entfernte Sessions sind anklickbar: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:623)
   - Favoriten-Stern ist vorhanden: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:656)

   Abweichung: Modell/Effort und Prozent werden als normale nachlaufende Flex-Kinder angefügt, nicht als fest vermessene rechte Spalte: [cockpit/panel.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/panel.rs:596). Damit können Werte und Labels horizontal springen; genau das verbietet die Spezifikation.

13. **Cockpit besitzt eine eigene, vom Rest isolierte Tokenwelt — Class A**

   Der Cockpit-Stil zentralisiert zwar intern Scrim, Radien, Farben und Verben: [style.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/style.rs:1). Diese Tokens sind aber nicht der appweite Dialog-/Layout-Standard. Das Cockpit ist intern kohärenter als vorher, bleibt gegenüber Git-, Workflow-, Tab-Config- und Settings-Flächen ein eigenes Designsystem.

14. **Header-, Footer- und Spacing-Werte variieren ohne gemeinsamen Maßstab — Class B**

   Beispiele: Standarddialog 20px Padding/8px Radius; `Modal<T>` 28px/8px; Cockpit 24px/10px; Session Config 32px horizontal/40px vertikal. Siehe [dialog.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/ui_components/dialog.rs:15), [modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/modal.rs:18), [spawn_card.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/spawn_card.rs:924), [session_config_modal.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/tab_configs/session_config_modal.rs:309).

## Completeness

15. **Cockpit-Pane enthält einen erreichbaren `unimplemented!()`-Pfad — Class A**

   Der Pane-Overflow-Handler endet in `unimplemented!()`: [cockpit/pane.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/cockpit/pane.rs:1725). Falls das Framework den Header-Overflow auslöst, ist das ein Panic statt eines fehlenden/disabled Menüs. Vor RC muss der Pfad implementiert oder die Affordance sicher entfernt werden.

16. **Attention Inbox zeigt Remote-Zeilen, lässt sie aber bewusst nicht öffnen — Class A**

   Remote-/nicht-resumierbare Einträge werden als statische Zeilen gerendert: [attention_inbox.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/attention_inbox.rs:255), [attention_inbox.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/workspace/view/attention_inbox.rs:303). Im Sidebar-Spine sind Remote-Sessions dagegen anklickbar. Dieselbe Session wirkt in zwei Cockpit-Linsen unterschiedlich interaktiv – ein direkter Bruch des „one object model / one operating grammar“.

17. **MCP-Settings bieten Aktionen an, deren Handler nicht implementiert sind — Class A**

   Delete, View Logs und Run sind für mehrere sichtbare Itemtypen nur Logmeldungen:

   - Delete: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:492)
   - Logs: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:595)
   - Run: [list_page.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mcp_servers/list_page.rs:625)

   Das sind echte Built-but-nonfunctional-Flächen. Für RC müssen die Aktionen funktionieren oder capability-basiert gar nicht erscheinen.

18. **Einzelne Altflächen wurden bereits korrekt entfernt, aber nicht systematisch — Class B**

   Settings dokumentiert die Entfernung zweier Stub-/Empty-List-Seiten: [settings_view/mod.rs](/home/cwendler/projects/zaplex/repos/zaplex.worktrees/fix-cli-detection/app/src/settings_view/mod.rs:82). Das ist die richtige RC-Richtung; derselbe „funktioniert oder wird nicht angeboten“-Standard fehlt noch bei MCP und Pane-Overflow.

## Verdict

Zaplex ist noch kein „nur noch Polish“-RC. Die fünf fundamentalen Workstreams sind: erstens ein appweit verbindliches Modal-/Dialog-System und Migration der Fremdkörper; zweitens vollständige deutsche Fluent-Abdeckung plus Verbot harter UI-Literale; drittens Abschluss des Icon-Passes für alle Chrome-/Action-Glyphen; viertens Fertigstellung des Sidebar-Spine inklusive fester Metrikspalte und Wiederherstellung der fehlenden Redesign-Spec; fünftens Entfernung aller angebotenen, aber nicht funktionierenden Interaktionen – besonders Remote-Inbox, MCP-Aktionen und der Cockpit-Panicpfad. Danach bleibt überwiegend echte Class-B-Politur.
