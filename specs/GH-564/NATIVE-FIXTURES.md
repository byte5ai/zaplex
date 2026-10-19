# Native Fixture-Auswertung am 19.10.2026

**Keine UX-Gesamtfreigabe.** Zwei visuelle Fehler sind im aktuellen nativen
Integrationstest-Build nachgewiesen, obwohl dessen Tests erfolgreich sind.
Die freien Nutzerabläufe, das installierte DMG und echte Provider-/Remote-Zugänge
wurden damit nicht abgenommen.

## Herkunft und Aussagegrenzen

- [Batch 39246675914](https://github.com/byte5ai/zaplex/actions/runs/39246675914),
  Quell- und Teststand `1201fc4f3cafcbab7464343cad8b715d8cf3db2e`, Paketversion 1.2.1.
- Native macOS-App im Integrationstest mit echtem Display. Kein installiertes DMG;
  deshalb kein DMG-Digest. Die Aufnahmen sind keine Browser-Mockups.
- 15 unveränderte PNGs mit Herkunft, Abmessungen und SHA-256 im
  [Manifest](evidence/native-1201fc4f/manifest.json). Die 13 Workspace-Aufnahmen
  wurden gegen das vom CI-Job erzeugte `evidence.json` geprüft.
- Englisch; Connections explizit hell/dunkel und 250/320/480 Punkte Sidebarbreite.
  Übrige Aufnahmen zeigen die dunkle Darstellung, das Favoriten-Flyout die helle.
  Die im Dateinamen genannten Fenstermaße werden in den nativen Tests gesetzt
  und geprüft; keine Gleichsetzung mit einer AX-Messung des installierten DMGs.
- Host-/Sessioninventar und Agentenzustände sind gesetzte, bezeichnete Fixtures.
  Lokale Dateimanager und Panes verwenden echte isolierte Dateien und PTYs.
  Kein echter SSH-/Provider-Erfolg, keine unabhängige Exploration, kein Nachweis
  für Deutsch, Kontrastvariante oder Reduced Motion.

## Priorisierte Abweichungen

### NF01 — P1: Dateinamen verschwinden in zwei schmalen Dateipanes

**Korrektur:** [#572](https://github.com/byte5ai/zaplex/issues/572).
Bezug: Matrix F03/F08–F10 und Variante „Native Mindestbreite“ / lesbare Identität.

Soll: Dateiidentität bleibt vor optionalen Metadaten erkennbar. Ist: Bei
760×640 Punkten verschwinden „Name“, `projects` und `acceptance-file.txt`
vollständig; Größe und Änderungsdatum bleiben. Der aktive Rahmen und alle acht
F-Aktionen sind sichtbar, aber der Nutzer kann das Ziel nicht am Namen erkennen.
Bei 1180×640 sind dieselben Namen lesbar.

| Normal | Schmal |
|---|---|
| [1180×640](evidence/native-1201fc4f/native-file-managers-normal.png) | [760×640](evidence/native-1201fc4f/native-file-managers-narrow.png) |

Reproduktion: `test_native_workspace_file_managers_evidence`, zwei lokale
Dateipanes mit offener Sidebar, dann Resize von 1180 auf 760 Punkte. Auch der
[anschließende F7-Dialog](evidence/native-1201fc4f/native-file-manager-focused-action.png)
zeigt die verschwundenen Namen.

Quellbefund: `app/src/sftp_manager/file_list.rs` reserviert 80 Punkte für Größe,
120 für Datum sowie Icon/Abstände; der Name bekommt nur die verbleibende Breite.
Header und Dateizeilen haben denselben unbedingten Aufbau.

Testauftrag in #572: responsive Priorität für Namen und tatsächliche sichtbare
Namensbounds/Textdarstellung prüfen. Der vorhandene `file_manager_layout()`-Test
findet `acceptance-file.txt` nur im Datenmodell. Das beweist keine lesbaren Pixel.

### NF02 — P2: Handlungsanleitung im schmalen Agentenfenster abgeschnitten

**Korrektur:** bestehendes [#153](https://github.com/byte5ai/zaplex/issues/153)
für diesen verbleibenden Layoutpunkt wieder geöffnet. Bezug: lesbare
Handlungsanleitungen, Zustände und schmale Variante aus #564.

Im [540×670-Bild](evidence/native-1201fc4f/subscription-agent-narrow-540x670.png)
läuft „Start a new conversation or return to the she…“ rechts aus dem sichtbaren
Bereich. Auch die allgemeine Shortcut-Hilfe wird hart beschnitten. Der
Session-Ende-Text ist gekürzt; „New conversation“ und „Back to shell“ bleiben
sichtbar. Das [1180×640-Bild](evidence/native-1201fc4f/subscription-agent-normal-1180x640.png)
zeigt separat den gesetzten Approval-Zustand mit vier sichtbaren Aktionen.
Diese beiden Bilder sind unterschiedliche Zustände, keine Vorher-/Nachher-Folge
eines echten Providerlaufs.

Testauftrag in #153: Anleitung umbrechen oder vollständig zugänglich machen;
die tatsächlichen Textzeilen zusätzlich zu Container-/Buttonbounds prüfen.
Die erfolgreiche Geometrieprüfung allein erfasst abgeschnittene Texte nicht.

## Eng begrenzte positive Beobachtungen

| Gegenstand | Sichtbarer Beleg | Grenze |
|---|---|---|
| Connections-Hierarchie | Sechs Aufnahmen mit 250/320/480 Punkten, hell/dunkel: Aktionsslots bleiben rechts; lange Titel werden gekürzt, Recovery-Metadaten umbrechen. | Gesetztes Inventar; Tooltip-Vollständigkeit, echte Verbindung, DE und freie Bedienung nicht geprüft. |
| Favoriten-Flyout | `native-favorite-flyout.png`: seitliches Menü mit New agent/Edit/Remove; Elternmenü bleibt sichtbar. | Keine vollständige Identitäts-/Tooltipbewertung langer Favoriten, kein Verbindungsaufbau. |
| Dateiaktionen | Bei 1180 Punkten zwei, bei 760 vier Reihen; F2–F8/F10 und Zielzeile sichtbar, Fokuswechsel und F7-Dialog aufgenommen. | NF01 verhindert eine Freigabe der gesamten Dateiliste. |
| Rückkehr zur Shell | `native-file-managers-restored-terminals.png`: zwei Terminals und der erhaltene Entwurf `printf file-manager-draft`. | Kein Neustart-/Remote-Restore und keine freie Exploration. |
| Pane-Drag | `native-panes-side-by-side.png` und `native-panes-after-drag.png`: nebeneinander → übereinander; Entwurf bleibt sichtbar, expliziter Tabtitel steht oben. | Native Interaktionsassertions für diese feste lokale Folge; keine vollständige Vier-Richtungen-/Mehrhost-Abnahme. |

Diese Beobachtungen ersetzen keine vollständige `pass`-Zeile der
[vorab festgelegten Matrix](MATRIX.md). Der aktuelle Batch muss noch den
Test-DMG-/GUI-Teil abschließen; die unabhängige Exploration benötigt weiterhin
den bereits erfassten isolierten GUI-Zugang.
