# RC-Sprint — Übergabe-Stand 2026-07-22

> **Zweck:** Arbeitsübergabe an einen anderen Agenten (Codex) mitten im
> RC-Sprint. Enthält den exakten Stand, die offenen Punkte und die Fallen, die
> in diesem Sprint schon Tage gekostet haben. **Vor dem Merge nach `main`
> löschen** — das ist ein Arbeitsdokument, kein dauerhafter Bestandteil.

## 1. Wo wir stehen

- **Branch:** `rc/master-plan`, HEAD `390739c5`, gepusht (`origin/rc/master-plan`).
  Arbeitsverzeichnis ist der Worktree `~/projects/zaplex/repos/zaplex.worktrees/rc-master-plan`.
- **Alle blockierenden Audit-Tiers (T1.x, T2.1–T2.4) sind erledigt.**
- **Ausgeliefert:** `~/projects/zaplex/exports/zaplex-rc4-arm64.dmg`
  (CI-Run 29914466860, beide Jobs grün). Der Daemon `zaplex-v0.rc4` wird von
  der App beim ersten Verbinden selbst installiert.
- **Offen: die Runtime-Abnahme durch den User.** Nichts davon ist bestätigt,
  bis er es auf dem Mac getestet hat.

### Die drei letzten Commits (alle in rc4 enthalten)

| Commit | Inhalt |
|---|---|
| `3a5a3b8e` | **Bootstrap-Doppel-Lauf-Fix** — siehe Abschnitt 3, das ist der wichtige |
| `c2d7d105` | `Starting ...` → `Starting shell...` (leerer Shell-Name) |
| `390739c5` | Dateimanager: `..`-Klick navigiert; NC/MC-Farb-Markierung statt Häkchen |

## 2. Was als Nächstes ansteht

1. **Abnahme rc4 abwarten.** Testpfad: frisch zu `devhost`, `agenthost` und
   `hal9000` verbinden → **es darf kein Skript-Text im Terminal erscheinen**.
   Dann im Dateimanager `..` klicken und Dateien markieren.
2. Bei Fehlern: **Screenshots verlangen.** Sie waren in diesem Sprint zweimal
   das entscheidende Beweismittel, Messungen waren es nicht (Abschnitt 3).
3. Wenn die Abnahme sauber ist: eigentlicher Release-Kandidat — Doku,
   Versionsnummern überall konsistent, Risiko-Check („was geht bei den ersten
   10 Nutzern schief?").

### Bekannte, bewusst offene Punkte (keine Blocker)

- **fish/pwsh als Remote-Login-Shell**: der Daemon liefert dort nur den
  Startteil des Bootstraps, nicht den Hauptteil; der Client-Write ist deren
  einziger Bootstrap-Weg und bleibt sichtbar-ish. Saubere Lösung wäre
  daemon-seitige Body-Lieferung mit Idempotenz-Guard.
- **Dreifach-Klick auf `..`** steigt zwei Ebenen hoch (Doppelklick ist
  abgesichert). „Mash-to-ascend", vertretbar.
- **T2.3**: einzelner unlesbarer Konto-Ordner (Datei-Rechte-Edge) wird nicht
  als „degradiert" erkannt — bräuchte I/O-Durchreichung durch die ganze
  Scan-Kette. Vom User abgenommen.
- Pseudo-Plurale („Datei(en)") in der DE-i18n; sauberer via Fluent-Plurale.

## 3. ⚠️ Die Falle, die 4 Anläufe gekostet hat (NICHT neu aufmachen)

**Symptom:** Beim Verbinden wurde das Bootstrap-Shell-Skript sichtbar ins
Terminal geschrieben — hunderte Zeilen.

**Es gab drei Schichten, alle sind gefixt:**

1. `bash_init_shell.sh` startete mit `stty raw` — **`raw` löscht ECHO NICHT**
   (weder GNU/uutils noch BSD; nur C's `cfmakeraw`). → `stty raw -echo`.
2. `bash.sh` leerte **PS2 nie** → bash malt `> ` für jede Heredoc-Zeile.
   `PS2=""` muss die erste ausgeführte Zeile sein und einzeilig (ein
   mehrzeiliges `if` malt selbst Prompts). `zsh.sh` machte es immer richtig.
3. **Die eigentliche Ursache (`3a5a3b8e`): der Body lief ZWEIMAL.** Der
   Daemon-Durchlauf war nach 1+2 sauber und unsichtbar. Aber das
   *Live*-`InitShell` eines frischen Opens erreichte den Client ungestempelt
   (die T1.3-Suppression galt nur dem Adopt-Preamble-Re-Feed) →
   `PtyController::initialize_shell` tippte die ~90 KB **erneut** in die schon
   gebootstrappte Shell. Das lief *nach* deren `stty sane`, also mit Echo an →
   jede Zeile als sichtbarer Befehlsblock.

**Der Fix:** persistentes `bootstrap_delivered_server_side` am `TerminalModel`
(gesetzt in `daemon_tty::EventLoop::start` vor dem ersten geparsten Byte);
`TerminalModel::init_shell` stempelt `suppress_bootstrap_write`. **Exakt auf
den Daemon-Liefervertrag gescopet — nur bash/zsh-Root.** Ausgenommen (jede
Ausnahme hat einen Test):
- Subshells im Tab (der Daemon bootstrapt nie verschachtelte Shells),
- fish/pwsh-Roots (Daemon liefert init-only),
- Legacy-SSH-`InitShell`s (nested `ssh` bei abgeschaltetem
  `SshRemoteServer`-Flag wird nicht nach `SshInitShell` umgeleitet).

**Wenn du an dieser Stelle etwas änderst:** die Ausnahmen sind nicht optional.
Ohne sie bleiben fish/pwsh-Sitzungen dauerhaft ohne Shell-Integration.

### Lektionen, die hier teuer waren

- **„Fix ist im Build + Repro ist grün" beweist gar nichts** über den echten
  User-Pfad. Mein Repro modellierte nur den Server-Durchlauf, nie den Client —
  deshalb war alles grün, während der User weiter Müll sah.
- **Immer prüfen: wird der Test ohne den Fix rot?** Für beide Fixes dieses Tages
  habe ich das explizit gegengeprüft (Flag temporär deaktiviert → rot → zurück).
  Ein Test, der auch ohne Fix grün ist, ist wertlos — genau das ist hier passiert.
- **Screenshots lesen.** Der Beweis war: die „Dump"-Zeilen hatten
  Prompt-Kopfzeilen **und Laufzeiten** → bash-preexec war schon aktiv → die
  Shell war fertig gebootstrapt → es MUSSTE ein zweiter Durchlauf sein.
  Blöcke mit Laufzeiten = ausgeführte Befehle, kein Terminal-Echo.

### Zweite Falle: tote Klick-Ziele

Ein `MouseStateHandle::default()`, der **pro Render** neu erzeugt wird, macht
das Control tot: das Framework rendert bei Mouse-Down neu (`ctx.notify()`), der
Down-Zustand liegt danach in einem weggeworfenen Handle, das Mouse-Up findet
ihn nie → der Klick feuert nicht. So war die `..`-Zeile tot, vorher schon der
Rescan-Button (T2.3). **Handles gehören in den View-State**, nicht in die
Render-Funktion. Bei „Control reagiert nicht" ist das der erste Verdacht.

## 4. Arbeitsregeln in diesem Repo (verbindlich)

- **Commit-Identität:** `iret77 <63622643+iret77@users.noreply.github.com>`
  (ist im Worktree schon gesetzt — vor dem ersten Commit `git config user.email`
  prüfen).
- **Kein Force-Push auf `main` oder `rc/master-plan`.** Kein eigenmächtiger
  Release/Publish.
- **Kein DMG-/CI-Build ohne frische, ausdrückliche Freigabe des Users** im
  Moment der Aktion. Eine frühere Freigabe gilt nicht weiter. Nach Fixes:
  committen → STOP → fragen.
- **`exports/` enthält immer genau EIN DMG.** Beim Liefern eines neuen die
  alten löschen.
- **Builds liefert man über `exports/` oder einen Klick-Link** — der User sitzt
  am MacBook, nicht auf diesem Host. Keine `gh`-Befehle an den User geben.
- **Code-Review vor jedem Commit.** Wenn kein zweites Modell verfügbar ist:
  einen adversarialen Review-Subagenten laufen lassen. Der hat heute zwei echte
  Regressionen gefunden (fish/pwsh, nested SSH), die sonst durchgerutscht wären.
- **Keine Quick-Wins.** Lieber die tragfähige Lösung, auch wenn sie länger dauert.
- **Sprache:** Code und Kommentare Englisch. Geerbtes Chinesisch übersetzen,
  kein neues.
- **`Cargo.lock` ist committet**, Builds laufen mit `--locked`. Nach Änderungen
  an `Cargo.toml` das Lock mit-committen.
- **Antworten an den User in klarer Alltagssprache**, kein Fachjargon. Technische
  Tiefe gehört in Commit-Messages und Code-Kommentare, nicht in den Chat.

## 5. Nützliche Kommandos

```bash
# Kompilieren (schnell, nur das App-Binary)
cargo check --bin zaplex

# Gezielte Tests (der volle Lib-Lauf hat ~74 vorbestehende Harness/Env-Fehler,
# die KEINE Regressionen sind — immer gezielt testen)
cargo test --lib -p warp daemon_tty::event_loop
cargo test --lib -p warp sftp_manager
```

Die Schlüssel-Tests dieses Sprints:
- `terminal::daemon_tty::event_loop::tests::live_initshell_of_a_daemon_session_is_stamped_suppressed`
  (+ die Grenzfall-Tests für Subshell, fish-Root, Nicht-Daemon)
- `sftp_manager::browser_integration_tests::test_row_click_right_after_navigate_up_is_swallowed`

**Push:** Das Token liegt unter `~/.github_tokens/byte5ai` (Mode 0600). Es wird
transient als Auth-Header mitgegeben — **niemals in eine Remote-URL einbetten**
und niemals in eine Datei im Repo schreiben.

**GitHub-CLI:** immer `-R byte5ai/zaplex` mitgeben, sonst landet man im
Upstream-Repo. Issues über die REST-API anlegen (`gh api -X POST
repos/byte5ai/zaplex/issues`), nicht mit `gh issue create` — der Fine-grained-PAT
darf die GraphQL-Mutation nicht.
