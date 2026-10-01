# Entscheidungen zum Prompt-Audit (#473)

Die gültige Code-Map und die Entwicklungsregeln liegen im operativen Basic-Memory-Vault unter `projects/zaplex/`:

- `main/projects/zaplex/zaplex-code-map`
- `main/projects/zaplex/zaplex-engineering-conventions`

Sie ergänzen die vorhandenen Architektur-, Produkt-, UI- und Build-Notizen. Globale Hostregeln bleiben im zentralen Regelbestand. Die Überführung entfernt tote WARP-/RC-Verweise, `.claude/skills`, unbeständige Dateizahlen und die historische Sprachmigration. Die Workspace-Mitgliedschaft bestimmt `Cargo.toml`. Delegation richtet sich nach unabhängigen Aufgaben und getrennten Schreibbereichen.

Die Review-Skills behalten ihre Schnittstellen; ihre Formulierungen werden bereinigt. Der ausgelieferte PR-Kommentar-Skill wahrt die Nutzerautorisierung und erkennt bereits erteilte Arbeitsaufträge an. Das ist eine Präzisierung derselben Berechtigungsgrenze, keine neue Agentenfunktion und kein Versionssprung.

## Entscheidungen zu den Nebenbefunden

| Befund | Entscheidung und Grenze |
| --- | --- |
| L1: Structured Output | Zurückgestellt. Ein eigener Protokoll-Change muss Claude- und Codex-Ausgabepfade gemeinsam prüfen; die bestehenden JSON-Validierung und Vertrauensgrenzen bleiben erhalten. |
| L2: Zweiter Prompt-Satz | Zurückgestellt als eigener Code-Cleanup. Die nur testseitig aufgerufenen Hilfen werden in dieser Textbereinigung nicht verändert. |
| L3: Dogfood-Skill | Inaktiv belassen. Keine Wiederverdrahtung von Warp-internen Diensten; das Entfernen unbenutzter Upstream-Ressourcen gehört in einen separaten Ressourcen-Cleanup. |
| L4: Vendored Skills | Keine manuellen Modell-/API-Edits. Aktualisierung benötigt eine verifizierte Upstream-Revision und einen separaten Vendor-Diff; Aktualität ist hier nicht behauptet. |
| L5: Review-/Triage-Skills | Beibehalten als explizit aufrufbare lokale Werkzeuge mit vorhandenen Artefaktverträgen. Kein vorhandener Zaplex-CI-Aufrufer wird behauptet und kein neuer Workflow hinzugefügt. F2/F8 werden direkt korrigiert. |
| L6: CONTRIBUTING | Tote WARP-Links entfernen; auf vorhandene README und Skills verweisen. |
| L7: Modellpreise | Bestehendes ehrliches „unpriced“ beibehalten. Neue Preise brauchen aktuelle Primärbelege und gegebenenfalls ein anderes Tarifschema; keine aus Modellnamen geratenen Werte. |
| L8: System-Prompt-Parameter | Als separaten Harness-Cleanup zurückstellen; keine CLI-/Launch-Semantik in einer Prompt-Textbereinigung ändern. |

Die übrigen Nebenbefunde sind damit entschieden, nicht als implementiert ausgegeben. Build-, Release- und Native-UI-Nachweise werden durch diese Dokumentationsänderung nicht ersetzt.
