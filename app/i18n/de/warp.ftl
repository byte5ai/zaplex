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
cockpit-conductor-title = Nach Projekt — was dich braucht

# Session-Zeilen-Aktionen im Cockpit
cockpit-session-adopt = ▸ übernehmen
cockpit-session-transcript = ◇ Verlauf
cockpit-session-fork = ⑂ Fork
cockpit-session-fork-worktree = ⑂ +Worktree

# GitHub-Flows auf der freiesten Instanz
cockpit-flow-quick-issue = ⚡ GitHub-Issue entwerfen (freieste)
cockpit-flow-pr-review = ⚡ PR reviewen (freieste)
cockpit-flow-triage = ⚡ Issue triagieren (freieste)
cockpit-spawn-card-new-agent = ✧ Neuer Agent…

# ── SSH-Hosts (linkes Panel) ─────────────────────────────────────────────────
workspace-left-panel-ssh-manager-tree-empty = Noch keine SSH-Server. Klicke 📁 für einen Ordner, + für einen Server.
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
