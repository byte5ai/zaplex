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

# ── SSH-Hosts (linkes Panel) ─────────────────────────────────────────────────
workspace-left-panel-ssh-manager-tree-empty = Noch keine SSH-Server. Klicke 📁 für einen Ordner, + für einen Server.
workspace-left-panel-ssh-manager-add-blank = Leeren Server anlegen
workspace-left-panel-ssh-manager-discover-tailscale = Tailscale-Hosts entdecken
