# Zaplex 1.0 — verbindlicher Umsetzungsplan

Status: **freigegebener Plan, GitHub-Epic und -Issues angelegt**
Stand: 2026-07-31 · Gate-0-Branch: `rc/master-plan`

## Quellen und Ziel

Verbindliche Quellen:

- `docs/2026-07-23-v1-gate0-resume.md`
- `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md`
- `docs/superpowers/specs/2026-07-01-filemanager-pane-mode-design.md`
- `docs/superpowers/specs/2026-07-18-ui-polish-audit.md`
- `docs/superpowers/specs/2026-07-19-rc-final-audit-plan.md`
- RC4-Screenshots, User-Abnahme und Fork-Diff-Review vom 22./23. Juli 2026

Ziel ist ein echtes **1.0**, kein MVP, keine Beta und kein Preview. Die
ursprüngliche Begrenzung, bestehende Issues nur als Vorgeschichte zu behandeln,
ist durch die Owner-Freigabe vom 31. Juli 2026 aufgehoben: Vor dem Release
werden **alle offenen Zaplex-Issues** abgearbeitet, einschließlich `#116` und
aller bei Reviews entdeckten Findings.

## Deckung der bereits offenen Issues

Die folgenden Issues lagen vor dem 1.0-Epic bereits offen. Sie bleiben eigene
Abnahmeeinheiten und werden nicht allein durch die neuen Issues `#124–#143`
implizit als erledigt behandelt:

- **`#18` Theme-Editor:** Rollen- und ANSI-Farben, Hex/RGB, UiColors,
  Vorlagen, YAML-Import/-Export, Gradients, optionales Bild und
  Full-Window-Vorschau liegen in `app/src/themes/theme_editor_body*.rs` und
  `docs/ui/theme-editor.html`.
- **`#19` inerte Warp-Stubs:** Die erneute 1.0-Entscheidung mit begründetem
  Verbleib steht in `docs/release/1.0-readiness.md`.
- **`#21` Multiplexer:** typisierte tmux-/byobu-Erkennung, ausgehandelte
  Daemon-Fähigkeit und sichtbares Ein-Klick-Attach liegen in
  `app/src/remote_server/multiplexer*.rs`, `app/src/ssh_manager/panel*.rs` und
  `docs/ui/multiplexer-adoption.html`.
- **`#38` Paritäts-Umbrella:** wird erst nach den noch offenen Kindern `#47`
  und `#50` sowie ihrer Laufzeitabnahme abgeschlossen.
- **`#47` Remote-Fleet:** Tailscale-Erkennung, hostübergreifende Aggregation
  und Session-/Host-RAM liegen in `app/src/cockpit/tailscale*.rs`,
  `crates/zaplex_cockpit/src/fleet*.rs`, `app/src/ssh_manager/panel*.rs` und
  `docs/ui/remote-session-ram.html`.
- **`#50` allgemeine Parität:** deutsche Kerntexte und
  `instances.json`-Overrides sind umgesetzt. Die Owner-Entscheidung vom
  2. August 2026 verwirft sichtbare Temp-Pfade: Clipboard-Bilder erscheinen im
  CLI-Agent-Composer als entfernbare Thumbnails und werden beim Absenden über
  den nativen Bild-Paste des Agents übergeben. Verbindlich ist
  `specs/APP-3891/`.
- **`#110–#120` Agent-/Cockpit-Ausbau:** Hook-Bridge (`#111`),
  Control-Surface (`#112`), Zaplex-Skill (`#113`), strukturierte Tasks
  (`#114`), verzögerter Pane-/Sidebar-Peek (`#115`), identitätsgebundene
  Fortsetzung mit Standard „Nachfragen“ (`#116`), persistente Tab-Pins
  (`#117`), Grok-Hooks (`#118`), Antigravity statt neuer Gemini-Starts
  (`#119`) und der nur bei `Blocked` sichtbare, durch DND unterdrückte statische
  Pane-/Tab-Halo (`#120`) besitzen jeweils Implementierung und statische Tests.
  Das gemeinsame UI-Artefakt `docs/ui/agent-continuity-attention.html` ist seit
  der Owner-Freigabe vom 2. August 2026 bindend.

Diese Zuordnung belegt den Quellstand, nicht die Laufzeit. Abschluss und
Schließen erfolgen erst nach den gemeinsamen Gates und der Phase-B-Abnahme am
freigegebenen Artefakt.

## Gate 0 — laufenden WIP zuerst schließen

Im Worktree liegt ein zusammenhängender 28-Dateien-WIP. Keine Lane startet von
`b4b6ed4f` oder einem Working Tree. Gate 0 verlangt:

1. alle angefangenen SFTP-, Session-, Bootstrap-, PID- und i18n-Änderungen fertig;
2. jeden Fix nachweislich ohne Fix rot und mit Fix grün;
3. unabhängiges kritisches Review aller Grenzen;
4. behobene Review-Funde und erneute Prüfung;
5. logisch getrennte Commits, `git diff --check` grün, sauberer Worktree.

Danach wird der saubere HEAD als `V1_BASE_SHA` festgehalten. Erst dieser Commit
darf Basis von `release/v1.0` und Lane-Branches sein. Branch-/Worktree-Refs
brauchen je Instanz ausdrückliche Freigabe. Gate 0 autorisiert keinen Build,
Push, PR, Tag, Publish oder Release.

## Gemeinsame Gates

- Verhaltenstest gegen den Fehler rot beweisen; grün mit Fix. Ohne Rot-Beweis
  ist das Issue nicht fertig.
- Vor jedem Commit unabhängiges, kritisches Review; Daten/Security/Protokoll
  mindestens `gpt-5.6-sol high`.
- Visuelle Fehler zusätzlich per Screenshot in gleicher Größe/Originalauflösung.
- Mouse-State im View-State, nie pro Render; keine neuen unbewiesenen
  `TerminalModel::lock()`; Prozesse nur über `crates/command`.
- Code/Kommentare Englisch; berührter sichtbarer Text in EN/DE-Fluent.
- Kein CI-/DMG-Build ohne frische ausdrückliche Freigabe.
- `max` ist kein Startwert; Eskalation nur nach dokumentiertem Scheitern.

## Umbrella-Epic

### GitHub [#124](https://github.com/byte5ai/zaplex/issues/124) — `[1.0] Zaplex als sichere integrierte Premium-Version veröffentlichen`

- **Problem/Scope:** RC4 enthält belegte Daten-, Zugangs-, Session- und
  UI-Fehler. Das Epic steuert Gate 0, 19 Children, Integration und Release-Gate.
- **Deps/Abschlussreihenfolge:** Gate 0; Issues 01–18 geschlossen; Issue 19
  Phase A grün; dann STOP und frische CI-/DMG-Freigabe; erst danach Issue 19
  Phase B gegen das ausgelieferte Artefakt.
- **Abnahme:** Issue 19 und das Epic schließen erst nach grüner Phase B; keine
  P0/P1-Funde; sauberer Release-Branch; Erstnutzer-Risikomatrix und
  macOS-Abnahme vollständig.
- **Rot-Testfamilie:** erbt alle Child-Tests; Release-Konsistenzcheck scheitert
  zusätzlich an absichtlich inkonsistenter Fixture.
- **Dateidomäne:** Koordination/Release-Gates.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

## Paket 1 — Daten und Zugänge schützen

### Issue 01 · [#125](https://github.com/byte5ai/zaplex/issues/125) — SFTP-Navigation, Links und Spezialdateien typkorrekt behandeln

- **Problem:** Links werden als falscher Typ aktiviert; Navigationsfehler
  mischen Pfad/Historie/Liste; FIFO/Socket/Gerät erscheint als Datei.
- **Scope:** `stat`/`lstat`; Datei-/Ordner-/defekte Links; atomarer
  Navigation-Commit; `Other` blockieren; verständliche Fehler.
- **Deps/Abnahme:** Gate 0; Fehler ändert keinen sichtbaren Ort, Spezialdateien
  werden nie geöffnet/übertragen.
- **Rot-Tests:** `resolved_symlink_target_selects_file_open_or_navigation_explicitly`,
  `failed_navigation_keeps_the_entire_visible_location_unchanged`,
  `file_symlink_opens_as_a_file_without_navigating`,
  `local_fifo_is_classified_as_other_and_never_opened`.
- **Dateidomäne:** `app/src/sftp_manager/{browser,file_list,sftp_backend}*.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 02 · [#126](https://github.com/byte5ai/zaplex/issues/126) — Dateiänderungen atomar, pfadbegrenzt und konfliktfest ausführen

- **Problem:** Rename/Copy, Selbstkopie, Traversal, Directory-Move und feste
  Temp-Namen können Ziel/Quelle zerstören oder parallele Jobs kollidieren lassen.
- **Scope:** exklusive Temp-Ziele; atomarer Commit; Konfliktregeln; strikte
  Pfadbegrenzung; Links/leere Ordner; Quelle erst nach vollständig verifiziertem
  Ziel löschen. Übersprungene Konflikte sowie nicht übertragene Links oder
  Spezialdateien machen einen Move unvollständig und blockieren jede
  Quelllöschung.
- **Deps/Abnahme:** Issue 01; Fehler oder Skip lässt altes Ziel und den gesamten
  Quellbaum intakt; kein Zielausbruch und kein Temp-Race.
- **Rot-Tests:** `local_rename_never_replaces_existing_destination`,
  `local_copy_failure_keeps_existing_destination_intact`,
  `recursive_download_rejects_path_outside_source_root`,
  `concurrent_copies_reserve_distinct_temporary_paths`,
  `failed_move_never_deletes_source`,
  `remote_to_remote_move_with_skip_conflict_preserves_source_tree`,
  `move_never_deletes_untransferred_symlink`,
  `move_never_deletes_source_tree_with_untransferred_special_file`.
- **Dateidomäne:** `app/src/sftp_manager/{sftp_backend,sftp_ops,browser}.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 03 · [#127](https://github.com/byte5ai/zaplex/issues/127) — Dateiaktionen an stabile Identitäten statt Indizes binden

- **Problem:** Refresh/Sort/alte Async-Antwort kann Save-As, Delete, Open oder
  Transfer auf eine andere Datei umlenken.
- **Scope:** Refresh-Generation; Entry-Identität; alte Antworten verwerfen;
  ausstehende Aktion neu auflösen oder abbrechen; Marks erhalten.
- **Deps/Abnahme:** Issue 01, blockiert 16; keine Aktion retargetiert, entfernte
  Auswahl bricht sichtbar ab.
- **Rot-Tests:** `refresh_generation_discards_older_completion`,
  `save_as_resolves_same_entry_after_refresh`,
  `queued_delete_never_retargets_after_sort`,
  `removed_entry_aborts_pending_action`.
- **Dateidomäne:** `app/src/sftp_manager/{browser,file_list,*tests}.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 04 · [#128](https://github.com/byte5ai/zaplex/issues/128) — Tote SFTP-Breadcrumb-, Cancel- und Row-Klickziele beseitigen

- **Problem:** Pro Render neue Mouse-Handles verlieren Down vor Up;
  SFTP-Breadcrumb, `..`, Rescan, Transfer-Cancel oder Datei-Rows bleiben
  sichtbar, aber tot.
- **Scope:** ausschließlich SFTP-Klickziele; Handles im View-State; stabile
  Control-IDs; Cancel beendet echte Transferarbeit; Tests mit Re-Render zwischen
  Down/Up. Klickzielprüfungen in Cockpit oder Workspace bleiben Gate 0
  beziehungsweise ihrer jeweiligen Lane zugeordnet.
- **Deps/Abnahme:** Gate 0; jedes Control feuert trotz Re-Render genau einmal.
- **Rot-Tests:** `breadcrumb_click_survives_rerender`,
  `parent_row_click_survives_rerender`,
  `cancel_cancels_inflight_operation_exactly_once`,
  `rescan_keeps_persistent_mouse_state`.
- **Dateidomäne:** `app/src/sftp_manager/`.
- **Primär/Review:** `terra high` / `gpt-5.6-sol medium`.

### Issue 05 · [#129](https://github.com/byte5ai/zaplex/issues/129) — Prozesssignale gegen ungültige und recycelte PIDs härten

- **Problem:** `u32` kann zu negativem `pid_t` werden; recycelte PID kann
  falschen Prozess treffen.
- **Scope:** darstellbare positive PID; Lebens-/Identitätsprüfung;
  Startzeit/Fingerprint auf unterstützten Plattformen zwingend. Kann die
  Prozessidentität nicht belastbar belegt werden, gilt fail-closed und es wird
  kein Signal gesendet.
- **Deps/Abnahme:** Gate 0; `0`, `u32::MAX`, tote oder recycelte PID erreicht
  das Signal-Backend nicht.
- **Rot-Tests:** `pid_must_fit_positive_signed_range`,
  `invalid_pid_never_reaches_signal_backend`,
  `recycled_pid_is_rejected_by_process_identity`,
  `signal_fails_closed_without_provable_process_identity`.
- **Dateidomäne:** `crates/zaplex_cockpit/src/{guardrails,sessions}*.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 06 · [#130](https://github.com/byte5ai/zaplex/issues/130) — SSH-Credential-Lifecycle konsistent und referenzsicher machen

- **Problem:** Folder-Delete lässt Child-Secrets; referenziertes OneKey kann
  verschwinden; Clone/Save/Sync ignorieren Teilfehler.
- **Scope:** Descendants/Secrets; Referenzschutz; wiederholbare
  Delete/Clone/Save/Sync-Schritte; sichtbare, explizite Kompensation zwischen
  DB, Keychain und Sync statt einer vorgetäuschten Transaktion; exakte
  Löschbestätigung.
- **Deps/Abnahme:** keine; keine verwaisten Secrets/Referenzen, kein scheinbarer
  Erfolg nach Teilfehler; Retry oder Kompensation konvergiert in einen
  konsistenten Zustand.
- **Rot-Tests:** `delete_folder_removes_all_descendant_secrets`,
  `delete_onekey_is_blocked_while_referenced`,
  `clone_compensates_secret_failure`,
  `credential_operation_retry_is_idempotent`,
  `sync_failure_preserves_recoverable_state`.
- **Dateidomäne:** `app/src/ssh_manager/`,
  `crates/warp_ssh_manager/src/{repository,secrets,sync_provider}.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 07 · [#131](https://github.com/byte5ai/zaplex/issues/131) — SSH-Ziel, Host-Key und Tests einheitlich validieren

- **Problem:** Leer/ungültig fällt still auf Port 22; Test deaktiviert
  Host-Key-Prüfung; alte Tests überschreiben neue; Tailscale umgeht Wrapper.
- **Scope:** eine Save/Test/Connect-Validierung; Port 1–65535; Unknown bestätigen,
  Changed blockieren; kein `StrictHostKeyChecking=no`; Test-Generation;
  injizierbare Workspace-Command-Factory über `crates/command`; statischer Guard
  gegen direkte `std::process::Command`-Aufrufe in der SSH-Workspace-Domäne.
- **Deps/Abnahme:** Issue 06 vor finaler UI; keine stille Korrektur/Identitäts-
  umgehung, altes Ergebnis bleibt wirkungslos.
- **Rot-Tests:** `empty_host_is_rejected_everywhere`,
  `invalid_port_never_falls_back_to_22`,
  `test_never_disables_host_key_verification`,
  `changed_key_is_hard_failure`,
  `stale_test_cannot_replace_current_state`,
  `workspace_ssh_uses_injected_command_factory`,
  `ssh_workspace_has_no_direct_std_process_command`.
- **Dateidomäne:** `app/src/ssh_manager/`,
  `crates/warp_ssh_manager/src/ssh_command*.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

## Paket 2 — Agenten zuverlässig öffnen

### Issue 08 · [#132](https://github.com/byte5ai/zaplex/issues/132) — Startup-Befehl erst nach Bootstrap und genau einmal senden

- **Problem:** `codex resume`/Claude bleibt sichtbar stehen oder wird mehrfach
  gesendet.
- **Scope:** Versand nur nach `Bootstrapped`; Replay/Live/Pending entprellen;
  bestehende Bootstrap-Ausnahmen bewahren; pro Startup eine stabile
  Command-ID über Retry und Reconnect hinweg. Lokales Enqueue entfernt den
  Befehl nicht aus `pending`: Erst ein Daemon-Ack bestätigt Annahme/Ausführung.
  Der Daemon dedupliziert Wiederholungen derselben ID, insbesondere bei
  verlorener Ack oder Mid-flight-Reconnect. Ein unbestätigter Startup-Befehl
  besitzt geschützten Pending-Speicher und darf nie durch Pufferdruck verdrängt
  werden.
- **Deps/Abnahme:** Gate 0, Vorgeschichte `#110`; Screenshot-Repro weg, genau
  eine Ausführung, kein Skript-/Befehlstext; geschlossener Writer, verlorene Ack
  und Reconnect verlieren oder duplizieren keinen Startup-Befehl.
- **Rot-Tests:** `startup_command_waits_for_bootstrap_and_runs_once`,
  `does_not_run_on_session_opened_or_init_shell`,
  `survives_replay_then_live_bootstrap`,
  `startup_command_is_retained_when_daemon_enqueue_fails` (Writer schließen,
  Übergabe scheitert, reconnecten, danach exakt eine Ausführung),
  `startup_command_remains_pending_until_daemon_ack`,
  `lost_ack_retry_is_deduplicated_by_command_id`,
  `midflight_reconnect_reuses_command_id_and_runs_once`,
  `pending_buffer_never_evicts_unacknowledged_startup`.
- **Dateidomäne:** `app/src/terminal/{view,daemon_tty/event_loop}*.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 09 · [#133](https://github.com/byte5ai/zaplex/issues/133) — Session nach Host, Provider, Konto und ID auflösen

- **Problem:** gleiche IDs kreuzen Konten/Provider/Hosts; Live-ohne-Pane darf
  nicht blind dupliziert werden.
- **Scope:** zentraler Session-Key/Open-Plan; Pane fokussieren; nur Idle resume;
  Live-ohne-Pane sichtbar ablehnen; alle Einstiegspunkte angleichen.
- **Deps/Abnahme:** Issue 08; kein Cross-Match, keine Live-Dublette.
- **Rot-Tests:** `identity_never_crosses_provider_or_account`,
  `host_matching_never_crosses_boundary`,
  `open_plan_refuses_unlocated_live_duplicate`,
  `only_dormant_session_can_resume`.
- **Dateidomäne:** `app/src/{cockpit,workspace,terminal/cli_agent_sessions}`,
  `crates/zaplex_cockpit/src/{conductor,sessions}*.rs`.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 10 · [#134](https://github.com/byte5ai/zaplex/issues/134) — Agent fest an PTY binden und sicher reattachen

- **Problem:** Inventar kennt keine verlässliche PTY-Bindung; cwd-Raten ist
  unsicher; fish/pwsh besitzen noch keinen vollständigen idempotenten Body-Weg.
- **Scope:** optionale `pty_session_id`; explizite daemon-seitige Bind/Unbind-
  Operationen; an Session-Lifecycle und Generation gebundene IDs; fremde,
  veraltete oder wiederverwendete IDs ablehnen; Reattach nur per validierter ID;
  Fortsetzungsmodus `off|prompt|auto`, wobei `prompt` („Nachfragen“) der
  Owner-freigegebene Standard ist und `auto` denselben validierten Resume-Pfad nutzt;
  Capability `agent-pty-binding` vor Verwendung des neuen Feldes aushandeln;
  rückwärtskompatibler Decode anhand echter serialisierter Legacy-Client- und
  -Daemon-Fixtures. Mehrere historische Agent-Sessions pro PTY sind erlaubt,
  aber höchstens ein attachbarer Live-Vordergrund-Agent. Eine zweite
  Live-Bindung wird abgelehnt oder nur über einen expliziten kontrollierten
  Handoff abgelöst.
- **Serielle Teilstufe:** Erst nach integriertem und getestetem
  Capability-/Lifecycle-Protokoll folgen fish und pwsh mit jeweils eigenem
  Rot-/Grün-Nachweis für einen genau einmal ausgeführten Body.
- **Deps/Abnahme:** Issues 08/09, Vorgeschichte `#116`; Neustart findet exakt
  Host/Konto/Provider/PTY, Unbind entfernt die Route, neue Generationen können
  keine alte Bindung übernehmen, ältere Peers bleiben per ausgehandelter
  Capability sicher, keine Dublette.
- **Rot-Tests:** `inventory_round_trips_pty_id`,
  `reattach_uses_id_without_cwd_guessing`,
  `bind_and_unbind_follow_session_lifecycle`,
  `new_generation_rejects_stale_pty_binding`,
  `foreign_pty_id_is_rejected`,
  `capability_negotiation_gates_pty_binding`,
  `real_legacy_daemon_fixture_decodes_without_binding`,
  `real_legacy_client_fixture_interoperates`,
  `multiple_historical_agents_can_share_pty`,
  `second_live_foreground_binding_requires_explicit_handoff`,
  `fish_body_is_idempotent`,
  `pwsh_body_is_idempotent`.
- **Dateidomäne:** Remote-Server-Proto und Client/Daemon, Workspace, Shell-Vertrag.
- **Primär/Review:** `gpt-5.6-sol xhigh` / `gpt-5.6-sol high`.

### Issue 11 · [#135](https://github.com/byte5ai/zaplex/issues/135) — Remote-Pfad und Effort nur als echte Fähigkeiten anbieten

- **Problem:** relativer Remote-Pfad fällt auf Home; Claude-Effort-Chips sind
  Placebo.
- **Scope:** leer=Home, absolut=gültig, relativ=Fehler; Codex-Effort bis CLI;
  Claude nur „CLI default“; keine Shell-String-Montage.
- **Deps/Abnahme:** Issues 08/09; Summary entspricht Start, invalid blockiert,
  kein wirkungsloses Control.
- **Rot-Tests:** `relative_remote_path_is_rejected`,
  `empty_path_maps_explicitly_home`,
  `claude_exposes_only_cli_default`,
  `codex_effort_reaches_cli_args`.
- **Dateidomäne:** `app/src/workspace/view/spawn_card.rs`, Workspace,
  `app/src/terminal/{cli_agent.rs,cli_agent_tests.rs}`.
- **Primär/Review:** `terra high` / `gpt-5.6-sol high`.

## Paket 3 — Eine klare Navigation

### Issue 12 · [#136](https://github.com/byte5ai/zaplex/issues/136) — Host–Projekt–Session als einzige primäre Sidebar

- **Problem:** Cockpit und SSH bleiben Parallelwelten; `#100/#108` sind sichtbar
  unvollständig. Eine reine Live-Inventaransicht lässt registrierte, momentan
  nicht erreichbare SSH-Hosts verschwinden.
- **Scope:** Hostwurzeln; Projekte/Sessions/Agenten; Hostaktionen; Konten kompakt;
  Files/Chats/Skills sekundär; stabile Metrikspalte/Statuspunkte; getesteter,
  deterministischer Join über stabile Host-IDs aus registrierten SSH-Hosts und
  Live-Cockpit-Inventar. Anzeigenamen sind kein Join-Key. Der Live-Status
  reichert den registrierten Host an, ersetzt ihn aber nicht; der lokale Host
  erscheint genau einmal. Entfernte registrierte Hosts werden trotz
  verspätetem Live-Eintrag nie weiter geroutet.
- **Deps/Abnahme:** Issues 09/11, vor 13; jeder Host einmal, keine parallele
  Hauptwurzel, Kernaktionen in höchstens zwei Schritten; registrierte
  Offline-Hosts bleiben sichtbar, Live-Hosts erzeugen keine Dubletten, gleiche
  Anzeigenamen vermischen keine Hosts und entfernte Ziele sind nicht
  ausführbar.
- **Rot-Tests:** `sidebar_has_one_host_project_session_tree`,
  `host_node_exposes_primary_actions`,
  `session_uses_shared_open_plan`,
  `parallel_roots_are_absent`,
  `registered_and_live_hosts_join_once`,
  `registered_offline_host_remains_visible`,
  `live_status_enriches_registered_host_without_duplicate`,
  `same_display_name_hosts_remain_distinct_by_stable_id`,
  `local_host_appears_exactly_once`,
  `removed_registered_host_is_never_routable`.
- **Dateidomäne:** Left panel, Cockpit-/SSH-Panel, Workspace.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

### Issue 13 · [#137](https://github.com/byte5ai/zaplex/issues/137) — Plus-Menü nur mit Favoriten-Hosts und rechtem Untermenü

- **Problem:** Auto-Host-/Agent-Zeilen blähen Menü auf; Persistenz kann letzten
  guten Stand überschreiben. `#101` bleibt Vorgeschichte.
- **Spec-Gate:** Vor Implementierung die authoritative Spine-Spec
  `docs/superpowers/specs/2026-07-08-integrated-ux-spine-design.md` auf die
  neueste Nutzerentscheidung aktualisieren: Der Hostbereich des Plus-Menüs
  enthält ausschließlich favorisierte Hosts; jeder öffnet rechts ein Untermenü
  mit „Terminal“ und „Neuer Agent“.
- **Scope:** nur Favoriten-Hosts; Submenu Terminal/Neuer Agent; Spawn-Card statt
  Blindstart; atomare Persistenz; stale/corrupt sichtbar; bestehende Projekt-,
  Session- und Launch-Favoritendaten erhalten oder explizit verlustfrei
  migrieren, auch wenn sie nicht mehr als flache Host-/Agentzeilen erscheinen.
- **Deps/Abnahme:** Issues 11/12 und abgeschlossenes Spec-Gate; eine Zeile je
  Favorit, Nicht-Favoriten fehlen, kein stiller Datenverlust; vorhandene
  Projekt-/Session-/Launch-Favoriten bleiben nach der Migration erhalten.
- **Rot-Tests:** `menu_lists_only_favorite_hosts`,
  `submenu_offers_terminal_and_new_agent`,
  `new_agent_opens_spawn_card`,
  `corrupt_store_is_never_overwritten`,
  `project_session_and_launch_favorites_survive_menu_migration`.
- **Dateidomäne:** authoritative Spine-Spec, beide Favorites-Module,
  Workspace-Menü/Left panel.
- **Primär/Review:** `terra high` / `gpt-5.6-sol medium`.

## Paket 4 — Filemanager auf MC-Niveau

### Issue 14 · [#138](https://github.com/byte5ai/zaplex/issues/138) — MC-Tastatur- und Funktionstasten vollständig umsetzen

- **Problem:** Tab wechselt keine FM-Panes; F2 fehlt; F3/F4 und weitere Tasten
  sind nicht verlässlich getrennt.
- **Scope:** Tab/Shift-Tab; F2/F3/F4/F5/F6/F7/F8/F10; Cursor/Marks priorisieren.
- **Deps/Abnahme:** Issues 03/04; lokal/remote funktional, Fokus stabil, Legende
  entspricht Verhalten.
- **Rot-Tests:** `tab_cycles_fm_panes_clockwise`,
  `shift_tab_cycles_counterclockwise`,
  `function_keys_dispatch_documented_actions`,
  `marks_win_before_cursor`.
- **Dateidomäne:** SFTP browser/list, pane-group, Workspace-Actions/Keymaps.
- **Primär/Review:** `terra high` / `gpt-5.6-sol medium`.

### Issue 15 · [#139](https://github.com/byte5ai/zaplex/issues/139) — FM-Layout, aktives Pane und F-Leiste responsiv machen

- **Problem:** Footer/Dateien/Nachbarpanes überlagern sich; Leerraum unter
  Footer; aktives Pane unklar.
- **Scope:** Body-Resthöhe; am unteren Rand jedes Pane eine pane-eigene,
  optionale und kompakte F-Legende; responsive Captions; Accent-Rahmen;
  Cursor/Mark/Hover; feste Spalten. Es gibt ausdrücklich keine globale, über
  beide Panes gespannte F-Leiste. Verständliche SFTP-Fehler gehören Issue 01;
  SSH-Fehler gehören Issue 18.
- **Deps/Abnahme:** Issues 01/04/14; kein Überlauf in schmalen Doppelpanes,
  Screenshot-Repros 4/6/7 behoben; jede sichtbare Legende gehört eindeutig zu
  ihrem Pane.
- **Rot-Tests:** `pane_function_legend_drops_captions_before_overlap`,
  `each_pane_owns_optional_compact_function_legend`,
  `dual_pane_never_renders_global_function_bar`,
  `body_consumes_remaining_height`,
  `active_style_differs_from_selection`,
  `columns_do_not_move_with_filename`.
- **Dateidomäne:** `app/src/sftp_manager/{browser,file_list}.rs`, Styles, FTL.
- **Primär/Review:** `terra high` / `gpt-5.6-sol medium`.

### Issue 16 · [#140](https://github.com/byte5ai/zaplex/issues/140) — Sichere Transfer-Queue mit Konflikten und Fortschritt

- **Problem:** MC-Basis fehlt für große/cross-pane/cross-host Transfers, Cancel,
  Konflikte und Quellschutz.
- **Scope:** FM-Registry; hidden-tab Ziele; Streaming; local↔remote/Remote-Relay;
  Progress/ETA/Pause/Cancel; Konflikte; globale Aktivität.
- **Deps/Abnahme:** Issues 02/03/14/15; überlebt Tabwechsel; kein Full-Buffer;
  Move löscht Quelle erst nach verifiziertem Ziel.
- **Rot-Tests:** `one_other_pane_is_default_target`,
  `hidden_panes_are_targets`,
  `large_copy_streams`,
  `cancel_preserves_source`,
  `failed_relay_never_deletes_source`.
- **Dateidomäne:** neue SFTP-Registry/Queue/Jobs, pane-group, Workspace-Aktivität.
- **Primär/Review:** `gpt-5.6-sol high` / `gpt-5.6-sol high`.

## Paket 5 — Hochwertige und ehrliche Oberfläche

### Issue 17 · [#141](https://github.com/byte5ai/zaplex/issues/141) — Cockpit-Verbrauch und Kosten ehrlich berechnen

- **Problem:** Cached Input kann doppelt zählen; unbekanntes Modell wirkt `$0`;
  Schätzung wirkt exakt. `#106` bleibt Vorgeschichte.
- **Scope:** cached/uncached/total/work; `~`-Provenienz; unknown=unpriced;
  dokumentierte Preisquelle; keine Remote-Fantasiewerte.
- **Deps/Abnahme:** keine; Fixture 100/90/10/5 ergibt 10/110/20; Reasoning
  bleibt als Output-Untermenge sichtbar, wird aber nie doppelt addiert; unbekannt nie `$0`.
- **Rot-Tests:** `usage_subtracts_cached_tokens`,
  `fixture_reports_total_110_work_20`,
  `unknown_model_is_not_zero_cost`,
  `estimate_has_provenance`.
- **Dateidomäne:** `crates/zaplex_cockpit/src/{codex*,types,pricing,format}*.rs`.
- **Primär/Review:** `terra high` / `gpt-5.6-sol high`.

### Issue 18 · [#142](https://github.com/byte5ai/zaplex/issues/142) — Verbleibende Cockpit-/SSH-Zustände premium integrieren

- **Problem:** In Cockpit und SSH bleiben Persistenzfehler, ungesicherte
  Dialogänderungen, inkonsistente Komponenten sowie Clipping- und
  Integrationslücken. Allgemeine Parität bleibt bei `#50`,
  Polish-Vorgeschichte bei `#108`.
- **Scope:** ausschließlich verbleibende Cockpit-/SSH-Zustands-,
  Persistenz- und Integrationsarbeit; geerbte Inputs/Search/Dialoge;
  Clip/Scroll/Typografie; Dirty Save/Discard/Cancel; sichtbare
  Persistenzfehler; berührte 1.0-Texte EN/DE; humanisierte SSH-Fehler.
  Filemanager-Arbeit bleibt vollständig in Issues 14–16. Bereits in Gate 0
  geschlossene Loading-/Generation-/Routingfälle und der Ellipsen-Katalogguard
  werden nicht erneut implementiert, sondern nur als Integrationsregressionen
  gegen den zusammengeführten Stand geprüft.
- **Deps/Abnahme:** Issues 06/07/12/13/17; kein Dirty-Verlust, kein scheinbarer
  Persistenzerfolg, keine Cockpit-/SSH-Überlagerung, gemeinsame Komponenten
  verhalten sich identisch.
- **Rot-Tests:** `dirty_dialog_requires_explicit_choice`,
  `cockpit_persistence_failure_preserves_last_good_state`,
  `ssh_persistence_failure_remains_visible`,
  `cockpit_table_clips_to_card`,
  `cockpit_and_ssh_dialogs_use_shared_state_contract`.
- **Dateidomäne:** Cockpit settings/pane/panel/style, SSH server-view,
  vorhandene UI-Komponenten, EN/DE-FTL.
- **Primär/Review:** `terra high` / `gpt-5.6-sol high`.

## Paket 6 — Release-Härtung

### Issue 19 · [#143](https://github.com/byte5ai/zaplex/issues/143) — Versionen, Doku und Erstnutzer-Risiken schließen

- **Problem:** Es fehlt der reproduzierbare Beleg, dass Installation, Version,
  Doku, Recovery und Kernflows ein echtes 1.0 bilden.
- **Phase A — Pre-build readiness:** Versionen; ausgelieferte RC/Beta-Texte;
  Handoff vor Merge entfernen; aktive Nutzer-/Recovery-Doku; Risiko-Register;
  vollständige Runtime-Matrix und Abnahmeschritte vorbereiten; sicherstellen,
  dass Test- und Release-DMG denselben versionsgebundenen Remote-Daemon offline
  bündeln, dessen Cache alle App-/Crate-/Resource-Eingaben verfolgt, der
  Download-Fallback aus dem Zaplex-Release-Repo kommt und die von den Updatern
  erwarteten macOS-/Linux-/Windows-Artefaktnamen exakt den Bundles entsprechen. Textscans
  erfassen ausschließlich ausgelieferte UI/Ressourcen und aktive Nutzerdoku,
  nicht historische Pläne, interne Handoffs oder Quellkommentare.
- **STOP/Gate:** Nach grüner Phase A stoppen und unmittelbar vor der Aktion
  eine frische ausdrückliche Buildfreigabe einholen. Eine frühere Freigabe
  zählt nicht.
- **Phase B — Build und Runtime:** Erst nach dieser Freigabe CI/DMG starten und
  das ausgelieferte Artefakt über Clean Install, Keychain, SSH, devhost,
  agenthost, Claude/Codex, Restart, Netzverlust, FM und beschädigte
  Settings prüfen.
- **Deps/Abnahme:** Issues 01–18; Phase A grün; jedes Risiko mit
  Auslöser/Wirkung/Erkennung/Recovery/Test; freigegebene Phase B vollständig
  grün. Issue 19 wird erst nach der Runtime-Matrix mit dem ausgelieferten
  Artefakt geschlossen.
- **Rot-Tests:** `release_metadata_is_consistent`,
  `shipped_user_facing_copy_has_no_rc_beta_markers`,
  `release_tree_has_no_rc_handoff`,
  `release_workflows_bundle_version_matched_remote_server`,
  `remote_server_cache_tracks_app_sources`,
  `release_artifact_names_match_autoupdaters`,
  `first_ten_user_risks_have_mitigation`,
  `runtime_matrix_requires_built_artifact_results`; Versionsfixture bewusst rot.
- **Dateidomäne:** Manifeste, macOS-/Updater-Ressourcen, `docs/`, Release-Checks.
- **Primär/Review:** `terra medium` / `gpt-5.6-sol high`.

## Abschlussreihenfolge

1. Gate 0 → `V1_BASE_SHA`
2. Pakete 1/2 parallel nach Hotspot-Ownership
3. Paket 3 nach stabiler Session-Semantik
4. Paket 4 nach sicheren Datei-Primitiven
5. Paket 5 nach stabilem Verhalten
6. Issue 19 Phase A auf sauber integriertem `release/v1.0` abschließen
7. STOPP und frische User-Freigabe für CI-/DMG-Build einholen
8. Issue 19 Phase B mit dem freigegebenen Artefakt und der Runtime-Matrix
   durchführen
9. Erst nach grüner Phase B Issue 19 und das Umbrella-Epic schließen

Ein grüner Plan, Commit oder Release-Branch ist keine Build-, Push-, PR-, Tag-,
Publish- oder Release-Freigabe.
