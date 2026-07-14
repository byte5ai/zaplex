# Deutsche Übersetzung (de) für Zaplex.
#
# Fluent fällt pro Schlüssel auf `en` zurück, wenn hier keine Übersetzung steht
# (siehe app/src/settings/language.rs) — daher ist diese Datei bewusst
# inkrementell: sie deckt zuerst die zaplex-eigenen Oberflächen ab (Cockpit,
# Sessions, Launcher, SSH-Hosts). Weitere Abschnitte von warp.ftl folgen.
#
# Ton: informell (du/dich), wie in Entwickler-Tools üblich. Symbole (⑂ ◇ ⚡ ▸)
# und Fluent-Platzhalter ({ $name }) bleiben unverändert.

# ── Einstellungen: Sprache ───────────────────────────────────────────────────
settings-language-system-default = Systemstandard
settings-language-english = Englisch
settings-language-german = Deutsch

# ── Neue Session / Launcher ──────────────────────────────────────────────────
workspace-new-session-agent = Agent
workspace-new-session-terminal = Terminal

# ── Cockpit ──────────────────────────────────────────────────────────────────
workspace-left-panel-cockpit = Cockpit
workspace-left-panel-cockpit-empty = Keine Claude- oder Codex-Konten gefunden.
cockpit-open-dashboard-tooltip = Cockpit-Dashboard öffnen
cockpit-pane-title = Cockpit
cockpit-pane-col-today = Heute
cockpit-pane-col-5h = 5-Std-Block
cockpit-pane-col-week = Woche
cockpit-pane-provenance-legend = ~ = aus Transkripten geschätzt · ohne Markierung = Live-Abo-Verbrauch
cockpit-sessions-waiting-toast = Wartet auf dich: { $sessions }
cockpit-attention-inbox-title = Offene Punkte
cockpit-attention-inbox-count = { $count ->
    [0] Nichts wartet auf dich
    [one] { $count } Agent wartet auf dich
   *[other] { $count } Agents warten auf dich
}
cockpit-attention-inbox-empty = Alles erledigt — nichts wartet auf dich.
cockpit-conductor-title = Hosts

# Session-Zeilen-Aktionen im Cockpit
cockpit-session-adopt = ▸ übernehmen
cockpit-session-transcript = Verlauf
cockpit-session-fork = Fork
cockpit-session-fork-worktree = +Worktree
# Cockpit-Sidebar Runtime-Strings (WS2)
cockpit-card-cost-line = heute { $cost } · { $tokens }
cockpit-card-provider-plan = { $provider } · { $plan }
cockpit-card-plan-none = —
cockpit-card-sessions-active = ● { $count } aktiv
cockpit-card-sessions-waiting = ● { $count } wartend
cockpit-card-sessions-running = ◌ { $count } laufend
cockpit-card-reset-5h = 5h ↻ { $reset }
cockpit-card-reset-week = Wo ↻ { $reset }
cockpit-card-reset-both = 5h ↻ { $reset5h } · Wo ↻ { $resetwk }
cockpit-conductor-needs-me-badge = ● { $count }
cockpit-header-account-count = { $count } { $count ->
        [one] Konto
       *[other] Konten
    }
cockpit-header-cost-summary = heute { $today } · { $week } Wo
cockpit-header-today-total = heute { $today }
cockpit-header-dashboard-button = Dashboard
cockpit-session-compact = ⚙ /compact
cockpit-session-clear = ⌫ /clear

# GitHub-Flows auf der freiesten Instanz
cockpit-flow-quick-issue = GitHub-Issue entwerfen (freieste)
cockpit-flow-pr-review = PR reviewen (freieste)
cockpit-flow-triage = Issue triagieren (freieste)
cockpit-spawn-card-new-agent = Neuer Agent…
cockpit-conductor-add-host = Host hinzufügen
cockpit-conductor-empty = Noch keine Agents oder Hosts
cockpit-tt-favorite-add = Zu Favoriten
cockpit-tt-favorite-remove = Aus Favoriten entfernen
cockpit-tt-manage-host = Host verwalten
cockpit-tt-review = Änderungen prüfen
cockpit-host-no-agents = keine Agents
cockpit-conductor-rest-show-both = { $running } laufend · { $idle } ruhend — zeigen
cockpit-conductor-rest-show-running = { $running } laufend — zeigen
cockpit-conductor-rest-show-idle = { $idle } ruhend — zeigen
cockpit-conductor-rest-hide-both = { $running } laufend · { $idle } ruhend — verbergen
cockpit-conductor-rest-hide-running = { $running } laufend — verbergen
cockpit-conductor-rest-hide-idle = { $idle } ruhend — verbergen

# Spawn-Card („Neuer Agent")
cockpit-spawn-card-title = Neuer Agent
cockpit-spawn-card-subtitle = Wähle exakt, was startet — Modell und Denk-Aufwand sind der Launch.
cockpit-spawn-card-agent = Agent
cockpit-spawn-card-no-cli = Kein Agent-CLI installiert — Claude Code oder Codex installieren
cockpit-spawn-card-model = Modell
cockpit-spawn-card-effort = Aufwand
cockpit-spawn-card-effort-tracked = Aufwand (getrackt)
cockpit-spawn-card-account = Account
cockpit-spawn-card-remote-login = Läuft unter dem eigenen CLI-Login von { $host } (kein lokales Account-Routing)
cockpit-spawn-card-freest = Freest
cockpit-spawn-card-freest-named = Freest — { $label }
cockpit-spawn-card-host = Host
cockpit-spawn-card-host-local = Lokal
cockpit-spawn-card-directory = Verzeichnis
cockpit-spawn-card-directory-on = Verzeichnis (auf { $host })
cockpit-spawn-card-dir-host-home = Home von { $host }
cockpit-spawn-card-browse = Durchsuchen…
cockpit-spawn-card-choose-folder = Ordner wählen…
cockpit-spawn-card-dir-default = Standard (Home)
cockpit-spawn-card-launching = Startet: { $summary }
cockpit-spawn-card-launch = { $agent } starten
cockpit-spawn-card-launch-plain = Starten
cockpit-spawn-card-cancel = Abbrechen
cockpit-spawn-card-sum-host-account = Host-Account
cockpit-spawn-card-sum-freest = freest
cockpit-spawn-card-sum-local = lokal
cockpit-spawn-card-sum-remote = remote
cockpit-spawn-card-sum-host-home = Home von { $host }
cockpit-spawn-card-sum-default = Standard (Home)
workspace-favorites-header = Favoriten
workspace-favorites-empty = Noch keine Favoriten — ★ einen Host oder ein Projekt in der Seitenleiste
workspace-favorites-add-header = Zu Favoriten hinzufügen
workspace-favorite-unavailable = nicht verfügbar — zum Entfernen klicken

# ── SSH-Hosts (linkes Panel) ─────────────────────────────────────────────────
workspace-left-panel-ssh-manager-tree-empty = Noch keine SSH-Server. Nutze den Ordner-Button für einen Ordner, + für einen Server.
workspace-left-panel-ssh-manager-add-blank = Leeren Server anlegen
workspace-left-panel-ssh-manager-discover-tailscale = Tailscale-Hosts entdecken

# ── Allgemein (überall sichtbare Buttons/Labels) ─────────────────────────────
common-ok = OK
common-cancel = Abbrechen
common-apply = Übernehmen
common-save = Speichern
common-delete = Löschen
common-confirm = Bestätigen
common-close = Schließen
common-reset = Zurücksetzen
common-back = Zurück
common-next = Weiter
common-yes = Ja
common-no = Nein
common-continue = Fortfahren
common-approve = Genehmigen
common-deny = Ablehnen
common-import = Importieren
common-upgrade = Upgraden
common-default = Standard
common-editing = Bearbeiten
common-viewing = Ansehen
common-tooltip-enter-edit-mode = Zum Bearbeiten klicken
common-tooltip-exit-edit-mode = Zum Beenden des Bearbeitens klicken
common-restored = Wiederhergestellt
common-continued = Fortgesetzt
common-deleted = Gelöscht
common-send-feedback = Feedback senden
common-something-went-wrong = Etwas ist schiefgelaufen
common-no-results-found = Keine Ergebnisse gefunden.
common-edit = Bearbeiten
common-add = Hinzufügen
common-remove = Entfernen
common-rename = Umbenennen
common-copy = Kopieren
common-paste = Einfügen
common-search = Suchen
common-view = Ansehen
common-loading = Wird geladen…
common-error = Fehler
common-warning = Warnung
common-info = Info
common-success = Erfolg
common-all = Alle
common-none = Keine
common-unknown = Unbekannt
common-open = Öffnen
common-restore = Wiederherstellen
common-duplicate = Duplizieren
common-export = Exportieren
common-trash = Papierkorb
common-copy-link = Link kopieren
common-untitled = Ohne Titel
common-retry = Erneut versuchen
common-maximize = Maximieren
common-discard = Verwerfen
common-undo = Rückgängig
common-commit = Committen
common-push = Pushen
common-publish = Veröffentlichen
common-create = Erstellen
common-configure = Konfigurieren
common-dismiss = Verwerfen
common-manage = Verwalten
common-failed = Fehlgeschlagen
common-done = Fertig
common-working = Arbeitet
common-cut = Ausschneiden
common-previous = Zurück
common-suggested = Vorgeschlagen
common-copied-to-clipboard = In die Zwischenablage kopiert
common-new = Neu
common-no-results = Keine Ergebnisse
common-learn-more = Mehr erfahren
common-skip = Überspringen
common-get-warping = Loslegen
common-try-again = Erneut versuchen
common-settings = Einstellungen
common-recommended = Empfohlen
common-enabled = Aktiviert
common-disabled = Deaktiviert
common-free = Kostenlos
common-current-directory = das aktuelle Verzeichnis
common-current = Aktuell
common-hide = Ausblenden
common-update = Aktualisieren
common-reject = Ablehnen
common-open-link = Link öffnen
common-open-file = Datei öffnen
common-open-folder = Ordner öffnen
common-name = Name
common-rule = Regel
common-skip-for-now = Vorerst überspringen
common-never = Nie
common-save-changes = Änderungen speichern
common-do-not-show-again = Nicht mehr anzeigen
common-dont-show-again-with-period = Nicht mehr anzeigen.
common-refresh = Aktualisieren
common-resource-not-found-or-access-denied = Ressource nicht gefunden oder Zugriff verweigert

# ── App-Menü ─────────────────────────────────────────────────────────────────
app-menu-new-window = Neues Fenster
app-menu-save-new = Neu speichern…
app-menu-launch-configurations = Start-Konfigurationen
app-menu-preferences = Einstellungen
app-menu-privacy-policy = Datenschutzrichtlinie…
app-menu-debug = Debug
app-menu-set-default-terminal = Zaplex als Standard-Terminal festlegen
app-menu-file = Datei
app-menu-edit = Bearbeiten
app-menu-use-warp-prompt = Zaplex-Prompt verwenden
app-menu-copy-on-select-terminal = Beim Auswählen im Terminal kopieren
app-menu-synchronize-inputs = Eingaben synchronisieren
app-menu-view = Ansicht
app-menu-toggle-mouse-reporting = Maus-Reporting umschalten
app-menu-toggle-scroll-reporting = Scroll-Reporting umschalten
app-menu-toggle-focus-reporting = Fokus-Reporting umschalten
app-menu-compact-mode = Kompaktmodus
app-menu-tab = Tab
app-menu-ai = KI
app-menu-blocks = Blöcke
app-menu-drive = Drive
app-menu-show-in-band-command-blocks = In-band-Befehlsblöcke anzeigen
app-menu-hide-in-band-command-blocks = In-band-Befehlsblöcke ausblenden
app-menu-show-zaplexified-ssh-blocks = Zaplexifizierte SSH-Blöcke anzeigen
app-menu-hide-zaplexified-ssh-blocks = Zaplexifizierte SSH-Blöcke ausblenden
app-menu-show-initialization-block = Initialisierungsblock anzeigen
app-menu-hide-initialization-block = Initialisierungsblock ausblenden
app-menu-window = Fenster
app-menu-enable-shell-debug-mode = Shell-Debug-Modus (-x) für neue Sessions aktivieren
app-menu-disable-shell-debug-mode = Shell-Debug-Modus (-x) für neue Sessions deaktivieren
app-menu-enable-pty-recording = PTY-Aufzeichnungsmodus aktivieren (warp.pty.recording)
app-menu-disable-pty-recording = PTY-Aufzeichnungsmodus deaktivieren (warp.pty.recording)
app-menu-enable-in-band-generators = In-band-Generatoren für neue Sessions aktivieren
app-menu-disable-in-band-generators = In-band-Generatoren für neue Sessions deaktivieren
app-menu-manually-toggle-network-status = Netzwerkstatus manuell umschalten
app-menu-export-default-settings-csv = Standardeinstellungen als CSV ins Home-Verzeichnis exportieren
app-menu-create-anonymous-user = Anonymen Nutzer anlegen
app-menu-send-feedback = Feedback senden…
app-menu-help = Hilfe
app-menu-warp-documentation = Zaplex-Dokumentation…
app-menu-github-issues = GitHub-Issues…
app-menu-warp-slack-community = Zaplex-Slack-Community…

# ── Block-Kontextmenü ────────────────────────────────────────────────────────
menu-block-copy = Kopieren
menu-block-copy-url = URL kopieren
menu-block-copy-path = Pfad kopieren
menu-block-show-in-finder = Im Finder anzeigen
menu-block-show-containing-folder = Enthaltenden Ordner anzeigen
menu-block-open-in-warp = In Zaplex öffnen
menu-block-open-in-editor = Im Editor öffnen
menu-block-insert-into-input = In Eingabe einfügen
menu-block-copy-command = Befehl kopieren
menu-block-copy-commands = Befehle kopieren
menu-block-find-within-block = Im Block suchen
menu-block-find-within-blocks = In Blöcken suchen
menu-block-scroll-to-top-of-block = Zum Blockanfang scrollen
menu-block-scroll-to-top-of-blocks = Zum Anfang der Blöcke scrollen
menu-block-scroll-to-bottom-of-block = Zum Blockende scrollen
menu-block-scroll-to-bottom-of-blocks = Zum Ende der Blöcke scrollen
menu-block-save-as-workflow = Als Workflow speichern
menu-block-ask-warp-ai = Zaplex-KI fragen
menu-block-copy-output = Ausgabe kopieren
menu-block-copy-filtered-output = Gefilterte Ausgabe kopieren
menu-block-toggle-block-filter = Blockfilter umschalten
menu-block-toggle-bookmark = Lesezeichen umschalten
menu-block-copy-prompt = Prompt kopieren
menu-block-copy-right-prompt = Rechten Prompt kopieren
menu-block-copy-working-directory = Arbeitsverzeichnis kopieren
menu-block-copy-git-branch = Git-Branch kopieren
menu-block-edit-prompt = Prompt bearbeiten
menu-block-edit-cli-agent-toolbelt = CLI-Agent-Toolbelt bearbeiten
menu-block-edit-agent-toolbelt = Agent-Toolbelt bearbeiten
menu-block-split-pane-right = Pane rechts teilen
menu-block-split-pane-left = Pane links teilen
menu-block-split-pane-down = Pane nach unten teilen
menu-block-split-pane-up = Pane nach oben teilen
menu-block-close-pane = Pane schließen

# ── Terminal-Suche (Platzhalter) ─────────────────────────────────────────────
terminal-input-search-queries = Anfragen suchen
terminal-input-search-queries-rewind = Anfragen zum Zurückspulen suchen
terminal-input-search-conversations = Unterhaltungen suchen
terminal-input-search-skills = Skills suchen
terminal-input-search-models = Modelle suchen
terminal-input-search-profiles = Profile suchen
terminal-input-search-commands = Befehle suchen
terminal-input-search-prompts = Prompts suchen
terminal-input-search-indexed-repos = Indizierte Repos suchen
terminal-input-search-plans = Pläne suchen

# ── Einstellungen: Bereiche ──────────────────────────────────────────────────
settings-section-about = Über
settings-section-mcp-servers = MCP-Server
settings-section-billing-and-usage = Abrechnung und Nutzung
settings-section-appearance = Erscheinungsbild
settings-section-features = Funktionen
settings-section-keybindings = Tastenkürzel
settings-section-referrals = Empfehlungen
settings-section-shared-blocks = Geteilte Blöcke
settings-section-warp-drive = Zaplex Drive
settings-section-zaplexify = Zaplexify
settings-section-network = Netzwerk
settings-section-cloud-sync = Cloud-Sync
settings-section-ai = KI
settings-section-warp-agent = Zaplex-Agent
settings-section-agent-profiles = Profile
settings-section-agent-mcp-servers = MCP-Server
settings-section-agent-providers = Anbieter
settings-section-knowledge = Wissen
settings-section-third-party-cli-agents = Externe CLI-Agenten
settings-section-code = Code
settings-section-editor-and-code-review = Editor und Code-Review
settings-section-cloud-environments = Umgebungen
settings-section-oz-cloud-api-keys = Agent-API-Schlüssel

# ── Einstellungen: Erscheinungsbild-Kategorien ───────────────────────────────
settings-appearance-category-themes = Themes
settings-appearance-category-language = Sprache
settings-appearance-category-icon = Symbol
settings-appearance-category-window = Fenster
settings-appearance-category-input = Eingabe
settings-appearance-category-panes = Panes
settings-appearance-category-blocks = Blöcke
settings-appearance-category-text = Text
settings-appearance-category-cursor = Cursor
settings-appearance-category-tabs = Tabs
settings-appearance-category-fullscreen-apps = Vollbild-Apps

# ── Suchfilter (Anzeige-Labels) ──────────────────────────────────────────────
search-filter-display-history = Verlauf
search-filter-display-workflows = Workflows
search-filter-display-agent-mode-workflows = Prompts
search-filter-display-notebooks = Notizbücher
search-filter-display-plans = Pläne
search-filter-display-natural-language = KI-Befehlsvorschläge
search-filter-display-actions = Aktionen
search-filter-display-sessions = Sessions
search-filter-display-conversations = Unterhaltungen
search-filter-display-launch-configurations = Start-Konfigurationen
search-filter-display-drive = Zaplex Drive
search-filter-display-environment-variables = Umgebungsvariablen
search-filter-display-prompt-history = Prompt-Verlauf
search-filter-display-files = Dateien
search-filter-display-commands = Befehle
search-filter-display-blocks = Blöcke

# ── Neue Session (weitere) ───────────────────────────────────────────────────
workspace-new-session-cloud-oz = Agent-Tab
workspace-new-session-local-docker-sandbox = Lokale Docker-Sandbox
workspace-tab-configs-tooltip = Tab-Konfigurationen
workspace-close-session = Session schließen
workspace-restart-app-and-update-now = App neu starten und jetzt aktualisieren

# ── Vertikale Tabs: Pane-Arten ───────────────────────────────────────────────
vertical-tabs-pane-kind-terminal = Terminal
vertical-tabs-pane-kind-code = Code
vertical-tabs-pane-kind-code-diff = Code-Diff
vertical-tabs-pane-kind-file = Datei
vertical-tabs-pane-kind-notebook = Notizbuch
vertical-tabs-pane-kind-workflow = Workflow
vertical-tabs-pane-kind-environment-variables = Umgebungsvariablen
vertical-tabs-pane-kind-environments = Umgebungen
vertical-tabs-pane-kind-rules = Regeln
vertical-tabs-pane-kind-plan = Plan
vertical-tabs-pane-kind-execution-profile = Ausführungsprofil
vertical-tabs-pane-kind-other = Sonstiges

# ── Vertikale Tabs: Einstellungen ────────────────────────────────────────────
vertical-tabs-setting-view-as = Anzeigen als
vertical-tabs-setting-panes = Panes
vertical-tabs-setting-tabs = Tabs
vertical-tabs-setting-tab-item = Tab-Element
vertical-tabs-setting-focused-session = Fokussierte Session
vertical-tabs-setting-summary = Zusammenfassung
vertical-tabs-setting-density = Dichte
vertical-tabs-setting-pane-title-as = Pane-Titel als
vertical-tabs-setting-command-conversation = Befehl / Unterhaltung
vertical-tabs-setting-working-directory = Arbeitsverzeichnis
vertical-tabs-setting-branch = Branch
vertical-tabs-setting-additional-metadata = Zusätzliche Metadaten
vertical-tabs-setting-show = Anzeigen
vertical-tabs-setting-pr-link-requires-gh = Erfordert eine installierte und authentifizierte GitHub-CLI
vertical-tabs-setting-pr-link = PR-Link
vertical-tabs-setting-diff-stats = Diff-Statistik
vertical-tabs-setting-show-details-on-hover = Details beim Überfahren anzeigen

# ── WS2: deutsche Werte für primäre Flächen (bisher harte Literale / EN-Fallback) ──
# Tab-Config- / New-Worktree-Modals
session-config-get-warping = Los geht's
session-config-modal-title = Erstelle deine erste Tab-Konfiguration
session-config-modal-subtitle-with-type = Richte einen wiederverwendbaren Startpunkt für deine Tabs ein. Wähle ein Repo, einen Session-Typ und optional einen Worktree. Nutze es, wann immer du einen neuen Tab mit diesem Setup öffnen willst.
session-config-modal-subtitle-no-type = Richte einen wiederverwendbaren Startpunkt für deine Tabs ein. Wähle ein Repo und optional einen Worktree, und nutze es, wann immer du einen neuen Tab mit diesem Setup öffnen willst.
new-worktree-title = Neuer Worktree
new-worktree-select-repo = Repository wählen
new-worktree-select-branch = Branch wählen
new-worktree-autogenerate = Worktree-Branchname automatisch erzeugen
new-worktree-name-label = Worktree-Branchname
new-worktree-name-placeholder = mein-feature-branch
new-worktree-invalid-name = Name darf nur Buchstaben, Zahlen, Bindestriche und Unterstriche enthalten
cockpit-spawn-card-remote-dir-placeholder = Remote-Verzeichnis (absoluter Pfad) — leer = Home des Hosts
# Native-Modal + KI-Einstellungsseite (WS2)
native-modal-dont-show-again = Nicht mehr anzeigen.
settings-ai-agent-tips-show = Agent-Tipps anzeigen
settings-ai-agent-tips-hide = Agent-Tipps ausblenden
settings-ai-use-agent-footer-show = „Use Agent“-Leiste anzeigen
settings-ai-use-agent-footer-hide = „Use Agent“-Leiste ausblenden
settings-ai-resets-at = Zurücksetzung: { $time }
settings-ai-context-window-label = Kontextfenster (Tokens)
# Git-Dialog (Commit / Push / Veröffentlichen / PR) — WS2
git-dialog-error-nothing-to-commit = Keine Änderungen zum Committen.
git-dialog-error-identity = Git-Identität nicht konfiguriert. Setze user.name und user.email.
git-dialog-error-non-fast-forward = Remote hat neue Änderungen — erst pullen, dann pushen.
git-dialog-error-no-remote = Kein Remote für diesen Branch konfiguriert.
git-dialog-error-auth = Authentifizierung fehlgeschlagen. Prüfe deine Git-Zugangsdaten.
git-dialog-error-network = Netzwerkfehler. Prüfe deine Verbindung.
git-dialog-error-repo-not-found = Remote-Repository nicht gefunden.
git-dialog-error-gh-missing = GitHub-CLI (gh) nicht installiert. Siehe https://cli.github.com/.
git-dialog-error-gh-unauth = GitHub-CLI nicht authentifiziert. Führe `gh auth login` aus.
git-dialog-error-generic = Git-Operation fehlgeschlagen.
git-dialog-title-commit = Änderungen committen
git-dialog-title-publish = Branch veröffentlichen
git-dialog-title-push = Änderungen pushen
git-dialog-title-create-pr = Pull Request erstellen
git-dialog-loading-commit = Wird committet…
git-dialog-loading-publish = Wird veröffentlicht…
git-dialog-loading-push = Wird gepusht…
git-dialog-loading-create-pr = Wird erstellt…
git-dialog-commit-need-message = Commit-Nachricht eingeben
git-dialog-branch-label = Branch
git-dialog-changes-label = Änderungen
git-dialog-file-count = { $count ->
        [one] { $count } Datei
       *[other] { $count } Dateien
    }
git-dialog-committing = Committe…
git-dialog-commit-and-push = Committen und pushen
git-dialog-commit-and-publish = Committen und veröffentlichen
git-dialog-enter-commit-message = Commit-Nachricht eingeben
git-dialog-toast-committed = Änderungen erfolgreich committet.
git-dialog-toast-committed-pushed = Änderungen committet und gepusht.
git-dialog-include-unstaged = Ungestagte einbeziehen
git-dialog-creating = Erstelle…
git-dialog-default-branch = Default-Branch
git-dialog-publishing = Veröffentliche…
git-dialog-pushing = Pushe…
git-dialog-toast-published = Branch erfolgreich veröffentlicht.
git-dialog-toast-pushed = Änderungen erfolgreich gepusht.
git-dialog-included-commits = Enthaltene Commits
# Fehlender DE-Key (EN-Fallback): SSH-Manager „connecting…"
workspace-left-panel-ssh-manager-connecting = Verbinde…

# --- „+"-Menü: neue Session (waren EN-Fallback, mischten die Sprache) ---
# Worktree-/Tab-Label bewusst kurz gehalten, damit sie im 268px-Menü nicht abschneiden.
workspace-new-worktree-config = Worktree-Konfig
workspace-new-tab-config = Tab-Konfig
workspace-reopen-closed-session = Geschlossene Session öffnen

# --- Terminal-Startzustand (Willkommens-Chips) ---
terminal-zero-state-title = Neue Terminal-Session
terminal-zero-state-start-agent = eine neue Agent-Unterhaltung starten
terminal-zero-state-cycle-history = durch frühere Befehle und Unterhaltungen blättern
terminal-zero-state-open-code-review = Code-Review öffnen
terminal-zero-state-autodetect-prompts = Agent-Prompts in Terminal-Sessions automatisch erkennen
terminal-zero-state-dismiss = Nicht mehr anzeigen

# --- Eingabe-Platzhalter & Hinweise (waren EN-Fallback) ---
terminal-input-ai-command-search-hint = „#“ für KI-Befehlsvorschläge tippen
terminal-input-run-commands-hint = Befehle ausführen
terminal-input-agent-hint-deploy-react-vercel = Frag Zaplex – z. B. Deploye meine React-App zu Vercel und richte die Umgebungsvariablen ein
terminal-input-agent-hint-debug-python-ci = Frag Zaplex – z. B. Hilf mir zu debuggen, warum meine Python-Tests in der CI fehlschlagen
terminal-input-agent-hint-setup-microservice = Frag Zaplex – z. B. Richte einen neuen Microservice mit Docker ein und erstelle die Deployment-Pipeline
terminal-input-agent-hint-fix-node-memory-leak = Frag Zaplex – z. B. Finde und behebe das Speicherleck in meiner Node.js-Anwendung
terminal-input-agent-hint-backup-postgres = Frag Zaplex – z. B. Erstelle ein Backup-Skript für meine PostgreSQL-Datenbank und plane es zeitlich
terminal-input-agent-hint-migrate-mysql-postgres = Frag Zaplex – z. B. Hilf mir, meine Daten von MySQL zu PostgreSQL zu migrieren
terminal-input-agent-hint-monitor-aws = Frag Zaplex – z. B. Richte Monitoring und Alerts für meine AWS-Infrastruktur ein
terminal-input-agent-hint-build-fastapi = Frag Zaplex – z. B. Baue eine REST-API für meine Mobile-App mit FastAPI
terminal-input-agent-hint-optimize-sql = Frag Zaplex – z. B. Hilf mir, meine langsamen SQL-Abfragen zu optimieren
terminal-input-agent-hint-github-actions = Frag Zaplex – z. B. Erstelle einen GitHub-Actions-Workflow, der beim Merge automatisch deployt
terminal-input-agent-hint-redis-cache = Frag Zaplex – z. B. Richte Redis-Caching für meine Web-Anwendung ein
terminal-input-agent-hint-kubernetes-pods = Frag Zaplex – z. B. Hilf mir herauszufinden, warum meine Kubernetes-Pods ständig abstürzen
terminal-input-agent-hint-bigquery-pipeline = Frag Zaplex – z. B. Baue eine Daten-Pipeline, die CSV-Dateien verarbeitet und nach BigQuery lädt
terminal-input-agent-hint-ssl-https = Frag Zaplex – z. B. Richte SSL-Zertifikate ein und konfiguriere HTTPS für meine Domain
terminal-input-agent-hint-refactor-legacy-code = Frag Zaplex – z. B. Hilf mir, diesen Legacy-Code auf moderne Design-Patterns umzustellen
terminal-input-agent-hint-unit-tests = Frag Zaplex – z. B. Erstelle Unit-Tests für meinen Authentifizierungs-Service
terminal-input-agent-hint-elk-logs = Frag Zaplex – z. B. Richte Log-Aggregation mit dem ELK-Stack für mein verteiltes System ein
terminal-input-agent-hint-oauth-express = Frag Zaplex – z. B. Hilf mir, OAuth2-Authentifizierung in meiner Express.js-App umzusetzen
terminal-input-agent-hint-optimize-docker = Frag Zaplex – z. B. Optimiere meine Docker-Images, um Build-Zeiten und Größe zu reduzieren
terminal-input-agent-hint-ab-testing = Frag Zaplex – z. B. Richte eine A/B-Testing-Infrastruktur für meine Web-Anwendung ein
terminal-input-steer-agent-hint = Den laufenden Agenten steuern
terminal-input-steer-agent-backspace-hint = Den laufenden Agenten steuern, oder Rücktaste zum Verlassen
terminal-input-follow-up-hint = Nachfrage stellen
terminal-input-follow-up-backspace-hint = Nachfrage stellen, oder Rücktaste zum Verlassen

# --- Server-Dateibrowser: Kontextmenü (war komplett EN-Fallback) ---
server-file-browser-menu-refresh = Aktualisieren
server-file-browser-menu-new = Neu
server-file-browser-menu-new-file = Neue Datei
server-file-browser-menu-new-folder = Neuer Ordner
server-file-browser-menu-upload = Hochladen
server-file-browser-menu-upload-file = Datei hochladen
server-file-browser-menu-upload-folder = Ordner hochladen
server-file-browser-menu-download = Herunterladen
server-file-browser-menu-copy-path = Pfad kopieren
server-file-browser-menu-copy-filename = Dateiname kopieren
server-file-browser-menu-rename = Umbenennen
server-file-browser-menu-delete = Löschen
server-file-browser-menu-cd-to-terminal = Im Terminal öffnen
server-file-browser-menu-terminal = Terminal
server-file-browser-menu-other = Weitere

# --- Linke Seitenleiste: Panels + SSH-Manager + Terminal-Eingabe (waren EN-Fallback) ---
terminal-input-a11y-helper = Gib deinen Shell-Befehl ein und drücke Enter zum Ausführen. Drücke Cmd-Auf, um zur Ausgabe zuvor ausgeführter Befehle zu navigieren. Drücke Cmd-L, um die Befehlseingabe erneut zu fokussieren.
terminal-input-a11y-label = Befehlseingabe.
terminal-input-choose-agent-model = Agent-Modell wählen
terminal-input-cli-agent-rich-input-hint = Sag dem Agenten, was er bauen soll…
terminal-input-cloud-agent-hint = Einen Agenten starten
terminal-input-enter-prompt-for-agent = Prompt für { $agent } eingeben…
terminal-input-no-skills-found = Keine Skills gefunden
workspace-left-panel-agent-conversations = Agent-Unterhaltungen
workspace-left-panel-close-panel = Panel schließen
workspace-left-panel-global-search = Globale Suche
workspace-left-panel-project-explorer = Projekt-Explorer
workspace-left-panel-server-file-browser = Server-Dateien
workspace-left-panel-skill-manager = Skill-Manager
workspace-left-panel-ssh-manager = SSH-Manager
workspace-left-panel-ssh-manager-add-cancel = Abbrechen
workspace-left-panel-ssh-manager-add-heading = Host hinzufügen
workspace-left-panel-ssh-manager-auth-key = Privater Schlüssel
workspace-left-panel-ssh-manager-auth-onekey = OneKey
workspace-left-panel-ssh-manager-auth-password = Passwort
workspace-left-panel-ssh-manager-candidates-add = Zum SSH-Manager hinzufügen
workspace-left-panel-ssh-manager-candidates-added = Hinzugefügt
workspace-left-panel-ssh-manager-candidates-empty = Keine importierbaren Hosts in { $path }
workspace-left-panel-ssh-manager-candidates-error = SSH-Konfiguration unter { $path } konnte nicht gelesen werden: { $error }
workspace-left-panel-ssh-manager-candidates-header = Aus { $path }
workspace-left-panel-ssh-manager-candidates-not-found = Keine SSH-Konfiguration unter { $path } gefunden
workspace-left-panel-ssh-manager-candidates-refresh = Aus ~/.ssh/config aktualisieren
workspace-left-panel-ssh-manager-connect = Verbinden
workspace-left-panel-ssh-manager-detail-auth = Authentifizierung
workspace-left-panel-ssh-manager-detail-empty = Wähle einen Server, um seine Details zu sehen.
workspace-left-panel-ssh-manager-detail-host = Host
workspace-left-panel-ssh-manager-detail-key-path = Schlüsselpfad
workspace-left-panel-ssh-manager-detail-port = Port
workspace-left-panel-ssh-manager-detail-resilience = Session-Persistenz
workspace-left-panel-ssh-manager-detail-ring-ceiling = Scrollback-Puffer
workspace-left-panel-ssh-manager-detail-user = Benutzer
workspace-left-panel-ssh-manager-error-host-required = Host darf nicht leer sein.
workspace-left-panel-ssh-manager-error-name-required = Name darf nicht leer sein.
workspace-left-panel-ssh-manager-error-port-invalid = Port muss eine Zahl zwischen 1 und 65535 sein.
workspace-left-panel-ssh-manager-field-group = Gruppe
workspace-left-panel-ssh-manager-field-name = Name
workspace-left-panel-ssh-manager-group-root = Wurzel
workspace-left-panel-ssh-manager-menu-clone = Klonen
workspace-left-panel-ssh-manager-menu-connect = Verbinden
workspace-left-panel-ssh-manager-menu-delete = Löschen
workspace-left-panel-ssh-manager-menu-edit = Bearbeiten
workspace-left-panel-ssh-manager-menu-new-folder = Neuer Ordner
workspace-left-panel-ssh-manager-menu-new-server = Neuer SSH-Server
workspace-left-panel-ssh-manager-menu-rename = Umbenennen
workspace-left-panel-ssh-manager-menu-sessions = Laufende Sessions
workspace-left-panel-ssh-manager-menu-sftp = Dateimanager
workspace-left-panel-ssh-manager-notes = Notizen
workspace-left-panel-ssh-manager-notes-placeholder = Notizen
workspace-left-panel-ssh-manager-onekey-add = Hinzufügen
workspace-left-panel-ssh-manager-onekey-credential = Zugangsdaten
workspace-left-panel-ssh-manager-onekey-delete = Löschen
workspace-left-panel-ssh-manager-onekey-key-path = Schlüsselpfad
workspace-left-panel-ssh-manager-onekey-key-path-required = Für Zugangsdaten mit privatem Schlüssel ist ein Schlüsselpfad erforderlich.
workspace-left-panel-ssh-manager-onekey-label = Name der Zugangsdaten
workspace-left-panel-ssh-manager-onekey-label-required = Name der Zugangsdaten darf nicht leer sein.
workspace-left-panel-ssh-manager-onekey-manage = OneKey verwalten
workspace-left-panel-ssh-manager-onekey-manager-title = OneKey-Manager
workspace-left-panel-ssh-manager-onekey-new = Neue Zugangsdaten
workspace-left-panel-ssh-manager-onekey-password = Passwort der Zugangsdaten
workspace-left-panel-ssh-manager-onekey-password-required = Neue OneKey-Zugangsdaten benötigen ein Passwort.
workspace-left-panel-ssh-manager-onekey-save = Speichern
workspace-left-panel-ssh-manager-onekey-save-before-connect = Speichere die OneKey-Zugangsdaten vor dem Verbinden.
workspace-left-panel-ssh-manager-onekey-secret = Passwort
workspace-left-panel-ssh-manager-onekey-select = Zugangsdaten wählen
workspace-left-panel-ssh-manager-onekey-select-required = Wähle OneKey-Zugangsdaten.
workspace-left-panel-ssh-manager-onekey-type = Typ
workspace-left-panel-ssh-manager-onekey-type-key = Privater Schlüssel
workspace-left-panel-ssh-manager-onekey-type-password = Passwort
workspace-left-panel-ssh-manager-onekey-user = Benutzer der Zugangsdaten
workspace-left-panel-ssh-manager-pane-folder-body = Ordner. Wähle einen Server in diesem Ordner, um seine Details zu sehen, oder mach einen Rechtsklick auf den Ordner für Erstellen-/Löschen-Aktionen.
workspace-left-panel-ssh-manager-pane-hint = Das Bearbeiten von Feldern und „Verbinden“ kommen in der nächsten Iteration. Vorerst zeigt dieser Bereich die gespeicherte Konfiguration; passe sie über den SQLite-Speicher oder den kommenden Editor an.
workspace-left-panel-ssh-manager-passphrase = Passphrase
workspace-left-panel-ssh-manager-placeholder = SSH-Manager — kommt bald
workspace-left-panel-ssh-manager-resilience-off = Standard
workspace-left-panel-ssh-manager-resilience-on = Dauerhaft
workspace-left-panel-ssh-manager-root-password = Root-Passwort
workspace-left-panel-ssh-manager-root-password-placeholder = Passwort für den Wechsel zu Root
workspace-left-panel-ssh-manager-save = Speichern
workspace-left-panel-ssh-manager-server-missing = Server nicht gefunden. Er wurde möglicherweise in einem anderen Fenster gelöscht.
workspace-left-panel-ssh-manager-sessions-empty = Keine laufenden Sessions
workspace-left-panel-ssh-manager-sessions-loading = Lade Sessions…
workspace-left-panel-ssh-manager-sessions-needs-key = Laufende Sessions benötigen schlüsselbasierte Authentifizierung
workspace-left-panel-ssh-manager-sessions-not-persistent = „Session-Persistenz“ auf „Dauerhaft“ setzen, um dies zu nutzen
workspace-left-panel-ssh-manager-startup-command = Startbefehl
workspace-left-panel-ssh-manager-startup-command-placeholder = Befehl, der nach der Verbindung ausgeführt wird
workspace-left-panel-ssh-manager-status-offline = Offline
workspace-left-panel-ssh-manager-status-online = Online
workspace-left-panel-ssh-manager-status-saved = ✓ Gespeichert
workspace-left-panel-ssh-manager-status-unknown = Unbekannt
workspace-left-panel-ssh-manager-test = Testen
workspace-left-panel-ssh-manager-testing = Teste…
workspace-left-panel-warp-drive = Zaplex Drive
