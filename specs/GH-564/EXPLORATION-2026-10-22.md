# Explorative native Teilprüfung am 22.10.2026

Geprüft wird das unveränderte installierte DMG **1.2.2**, App-Commit
`cadcb19130bbe0630f39bdd61b4e183ff154981a`, aus Run 39295080993,
SHA-256 `313303cc2dd14b820cd9c01bb931b555300864914b7fc746f4c6841e4889bae3`.
Dies ist kein Nachweis für die späteren Korrekturen auf main. Designreferenz:
`6c956383ad560913dc31fa383104dc489104d561`; Aufgaben und Erwartungen standen
vorab in MATRIX.md und im [Prüfauftrag](https://github.com/byte5ai/zaplex/issues/564#issuecomment-6336962219).
Englisch, Dark, Fenster 1024×678 Punkte bei (0,30); Originalbilder 1024×678 Pixel.
Reduced Motion, Light und schmale Breite wurden in diesen Sitzungen nicht geprüft.

## Sitzungen und sichtbare Belege

[Run 39752441133](https://github.com/byte5ai/zaplex/actions/runs/39752441133)
verwendet Harness `fa45c58ef04d7eed1306fbdad9ebd490a36e4f2d`.
[Manifest mit neun Originalbildern](evidence/exploration-39752441133/manifest.json),
[Aktionsprotokoll](evidence/exploration-39752441133/actions.json).

| Bilder | Beobachtung | Aussage |
|---|---|---|
| [01](evidence/exploration-39752441133/01-start.png) → [02](evidence/exploration-39752441133/02-fixture-input.png) → [03](evidence/exploration-39752441133/03-fixture-ready.png) | Frische Shell erzeugt künstliche Dateien und zeigt `READY:7319:/tmp/zaplex-acceptance-39752441133`. | Tatsächliches Arbeitsverzeichnis und Shellmarker vor dem Dateimanager-Einstieg. |
| [04](evidence/exploration-39752441133/04-file-manager.png) | Vordergrundklick auf das sichtbare Ordnersymbol bei (982,53) relativ zum Fenster; Dateimanager bleibt aus. | Bedienbefund #604; noch keine bewiesene Ursache und kein Nachweis auf aktuellem main. |
| [05](evidence/exploration-39752441133/05-file-manager-correct-coordinates.png) | Zusätzlicher Klick bei (982,83) relativ zum Fenster. | Dieser Punkt liegt unterhalb des Symbols. Der historische Dateiname ist irreführend; dies ist ausschließlich eine Koordinatenprobe und keine zweite gültige Reproduktion. |
| [06](evidence/exploration-39752441133/06-pane-menu.png) → [07](evidence/exploration-39752441133/07-file-manager-shortcut.png) | Pane-Menü öffnet sich; ein veralteter `hotkey`-Werkzeugbefehl wird abgewiesen. | App reagiert auf den Menüklick. Aus dem abgewiesenen Werkzeugbefehl folgt keine Appaussage. |
| [08](evidence/exploration-39752441133/08-file-manager-keyboard.png) → [09](evidence/exploration-39752441133/09-nested-directory.png) | `press cmd+shift+e` öffnet Dateimanager im richtigen Verzeichnis mit `nested` und `ORIGINAL.txt`; Enter öffnet `nested` mit `CHILD.txt`. | Tastatureinstieg und Navigation in den Unterordner beobachtet. Die anschließende Rückkehr nach oben wurde wegen abgelaufener Bedienzeit nicht zugestellt. |

[Run 39755300902](https://github.com/byte5ai/zaplex/actions/runs/39755300902)
verwendet Harness `c367aea68ae254fbd744bc5fd3b8849fbc4fb6ae`.
[Manifest mit sechs Originalbildern](evidence/exploration-39755300902/manifest.json),
[Aktionsprotokoll](evidence/exploration-39755300902/actions.json).

| Bilder | Beobachtung | Aussage |
|---|---|---|
| [01](evidence/exploration-39755300902/01-start-retry.png) → [02](evidence/exploration-39755300902/02-fixture-input.png) → [03](evidence/exploration-39755300902/03-fixture-ready.png) | Nach einmalig unvollständiger AX-Aufnahme wird die App sichtbar; neue isolierte Dateien und Shellmarker `8426` werden erzeugt. | Frische unabhängige Sitzung, kein fortgesetzter Zustand aus dem ersten Run. |
| [04](evidence/exploration-39755300902/04-file-manager.png) → [05](evidence/exploration-39755300902/05-rename-dialog.png) → [06](evidence/exploration-39755300902/06-rename-input.png) | Tastatureinstieg zeigt richtiges Verzeichnis. F2 öffnet „Rename“ für `ORIGINAL.txt`; Eingabe `RENAMED.txt` ist vollständig sichtbar. | Auffindbarer Dialog und editierbarer Name. Der Bestätigungsklick wurde nach VM-Ende abgewiesen; keine abgeschlossene Umbenennung, kein Inhaltsvergleich und kein erfolgreicher Abbruch daraus ableitbar. |

Alle 15 Originalbilder wurden einzeln angesehen und ohne Bildbearbeitung archiviert.
Teilweise meldete `type` Exit 1 trotz sichtbarer vollständiger Eingabe; vor jedem
weiteren Schritt wurde der sichtbare Zustand geprüft. Die zehnminütigen Bedienfenster
endeten in beiden Sitzungen, bevor alle vorgesehenen Aufgaben abgeschlossen waren.
Beide Jobs sind infrastrukturell erfolgreich beendet; das ist kein Produkt-Pass.

## Befund und Abgrenzung

[#604](https://github.com/byte5ai/zaplex/issues/604) verfolgt den Unterschied zwischen
Ordner-Klick und Tastatureinstieg. PR #605 ergänzt einen nativen Regressionstest,
der den gerenderten Button klickt; frühere Dateimanager-Tests lösten die Workspace-Aktion
intern aus. Bis zur Ausführung ist weder die Ursache noch eine Produktkorrektur bewiesen.
F01/F04/F08 haben die oben abgegrenzten Teilbeobachtungen, keine vollständige Abnahme.
Transfer, Konfliktauflösung, Mindestbreiten, alle Rückwege und Remote-Varianten sind
von diesen lokalen Sitzungen nicht abgedeckt. #564/#562 erhalten keine Gesamtfreigabe.

Ein weiterer [Bereitstellungslauf 39759939367](https://github.com/byte5ai/zaplex/actions/runs/39759939367)
endete nach Ablauf des Bedienfensters ebenfalls infrastrukturell erfolgreich.
Darin wurde keine weitere Interaktion ausgeführt; er erweitert die Produktbelege nicht.
