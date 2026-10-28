# Native Abnahme des Kandidaten 1.2.41

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

## Identität und GUI-Ergebnis

App-Quellstand: `0d7f5b1c1d2d4de18115508e8c66442786f98101`.
DMG-SHA256: `4abf08c30dfbf393a79044a4198ebaa3745637550425497473013f7d0d55c5b2`.
Der [Artefakt-Batch](https://github.com/byte5ai/zaplex/actions/runs/40732407949)
baut App, Linux-Daemon und signiertes/notarisiertes DMG erfolgreich. Von 16
installierten GUI-Fällen bestehen 15. Der verbleibende Tab-Fall scheitert bereits
an der OCR im Setup: Das Original zeigt `490663`, erkannt wird `499663`.

Die [Diagnose](https://github.com/byte5ai/zaplex/actions/runs/40743849365)
vergleicht elf unveränderte Originalbilder in 16 festen Konfigurationen. Nur
2×/hohe Interpolation/PSM11 erfüllt sämtliche positiven und negativen Erwartungen.
[PR #719](https://github.com/byte5ai/zaplex/pull/719) korrigiert ausschließlich
die Test-OCR, ohne Ziffernersetzung, unscharfen Vergleich oder erwartungsabhängigen
Fallback. Die bisherigen Regressionen bleiben erhalten, der konkrete Fehlfall
kommt hinzu. [Diagnosebeleg](evidence/ocr-141/diagnosis.json).

Die [gezielte Fortsetzung](https://github.com/byte5ai/zaplex/actions/runs/40744866505)
besteht mit derselben App und demselben DMG sowohl die OCR-Regressionen als auch
`NavigationTests/testTabsKeepIndependentShells`. Der
[Produktionsvalidator](evidence/candidate-141-native/gui-completion.json) bestätigt
**16/16 GUI-Fälle aus Original und Fortsetzung**, ohne die 15 bestandenen Fälle
oder den App-/Daemon-/DMG-Bau zu wiederholen. Harness-Revision:
`a973f67804df2933ff66eba203f075f15d39bf6b`. Kein Release oder Tag erstellt.

## Direkte native Prüfung: EN/Dark

Die [vorab festgelegten Kriterien](evidence/candidate-141-native/native-plan.md)
beziehen sich auf Startmenü L1–L6 und den unabhängigen Fehlerbefund #714.
[Sollreferenzen](evidence/candidate-141-native/design-reference.json).

[Exploration 40745799604](https://github.com/byte5ai/zaplex/actions/runs/40745799604)
endet ausdrücklich per Abschlussmarker, ohne Zeitablauf. Der macOS-Job läuft von
18:31:34 bis 18:44 UTC. Alle 20 Originalaufnahmen wurden einzeln betrachtet und
bytegenau mit dem hochgeladenen Artefakt verglichen. Das
[Manifest](evidence/candidate-141-native/40745799604/manifest.json), unveränderte
Bilder, AX-Metadaten und Aktionen sind dauerhaft gesichert.

| Kriterium | Ergebnis und Beleg |
|---|---|
| #714, Mindestbreite 480×678 | Pass: Elternmenü und linkes Untermenü überlappen sich nicht; vollständiger Tooltip „New worktree config“ sichtbar, Suchfeld und Aktion erreichbar (06). |
| Suchfokus | Pass: frische Eingabe `worktree141` erscheint im Suchfeld (07). |
| Größenwechsel 480→1024→480 bei offenem Menü | Pass: Query und vollständiger Tooltip bleiben erhalten, keine Überlappung oder abgeschnittene Aktion (08/09). Die bestehende linke Platzierung bleibt bestehen; dynamischer Wechsel nach rechts wird daraus nicht behauptet. |
| Neu geöffnet bei 1024×678 | Pass: Untermenü rechts, vollständiger Tooltip und Elternmenü gleichzeitig sichtbar (11). |
| Aktion / Rückweg | „Add new repo“ öffnet den leeren New-worktree-Dialog, Open ist deaktiviert, Cancel erreichbar. Cancel schließt den Dialog (12/13). Keine Konfiguration oder Repository angelegt. |
| Escape und Shellerhalt | Escape schließt das Menü (10). Frische Ausgabe nach allen Abbrüchen bestätigt dieselbe PID1272, den nicht exportierten Marker `native141` und dasselbe Arbeitsverzeichnis (03/18). |
| EN/Light, Deutsch, Außenklick | In dieser kurzen Sitzung nicht ausgeführt; separat fortsetzen. |
| A03: tatsächlicher Agenteneinstieg ohne Konto | In dieser Sitzung nicht ausgeführt. |

Die erste Baseline-Eingabe enthielt einen vom Operator als Return interpretierten
Zeilenumbruch und ist **kein** Zustandsnachweis (01). Erst 02/03 bilden die gültige
Baseline. Gemeldete Tippfehler mit `mutation_dispatched` wurden durch neue
Aufnahmen vor Return kontrolliert; die sichtbare Eingabe war angekommen.
Settings und Theme-Picker wurden geöffnet (13–15), aber kein Themewechsel
vorgenommen. Standbilder belegen keine kontinuierliche Hover-Bewegung.

Die Gesamtmatrix bleibt offen. Diese gezielte Nachprüfung ist keine pauschale
UI-/UX-Freigabe und ersetzt weder echte Provider-Varianten noch Bewegungsbelege.

## Zweite kurze Sitzung: EN/Light, DE/Hell/Dunkel und Agenteneinstieg

[Exploration 40748184338](https://github.com/byte5ai/zaplex/actions/runs/40748184338)
verwendet dasselbe unveränderte DMG. Der macOS-Job läuft von 18:47:28 bis
19:00:43 UTC (**13 Minuten 15 Sekunden**), nach einer Pause zum vorherigen Job.
Abschlussmarker und geschlossene Operatorsitzung sind archiviert; kein Zeitablauf.
Alle **52 Originalbilder** wurden einzeln betrachtet und bytegenau mit dem
hochgeladenen Artefakt verglichen. [Manifest](evidence/candidate-141-native/40748184338/manifest.json).
Die Nummer32 hat wegen eines zurückgewiesenen Operator-Startbefehls kein Bild;
Fehler-JSON und Aktion bleiben erhalten. Der folgende korrekte Start gelingt (33).

| Kriterium | Ergebnis und Beleg |
|---|---|
| #714, EN/Light bei 480×678 | Pass für Geometrie und Lesbarkeit: linker Flyout neben Elternzeile, vollständiger Tooltip, Suche und Aktion sichtbar (10/14). |
| EN/Light Suchfeld / offener Größenwechsel | Nach erneutem Hover ist frische Eingabe sichtbar; Query bleibt bei 480→1024→480 erhalten, keine Überlappung (14–16). Der erste Klick-/Tippversuch lieferte nur einen Teil der Eingabe und ließ das Kindmenü verschwinden (11/13); dessen Ursache ist nicht abschließend getrennt von der Operator-Fokussteuerung. Daher kein uneingeschränkter Maus-Fokus-Pass. |
| EN/Light Außenklick | Menü geschlossen, kein neuer Tab/keine Pane (17). Frische Ausgabe nach Menü- und Agentenkartenabbruch: PID1296, nicht exportierter Marker native141b und Arbeitsverzeichnis unverändert (02/27). |
| DE/Light nach vollständigem Neustart, 480×678 | Pass: vollständiger Worktree-Titel/Tooltip, Suchfeld mit deutsch141 und vollständige Repository-Aktion neben dem Elternmenü (37/38). |
| DE/Light Größenwechsel / Escape | Offenes Menü bei 480→1024→480 ohne Überlappung und mit erhaltenem Query; Escape schließt es (39–41). |
| DE/Dark bei 480×678 | Pass für Geometrie, Lesbarkeit und Suche: vollständiger Titel/Tooltip, dunkel141 und Repository-Aktion sichtbar (47/48); Außenklick schließt (49). Größenwechsel in DE/Dark nicht erneut durchgeführt. |
| Frischer deutscher Shell-Rückweg | Nach Neustart neue Baseline PID3045/native141de; dieselben Werte und CWD nach deutschen Menüabbrüchen und Themewechsel frisch ausgegeben (35/51). Kein Shellerhalt über App-Neustart behauptet. |
| A03, tatsächlicher Agenteneinstieg | Per Hover auf New agent und Return öffnet sich die echte Startkarte (21/22). Der vorherige direkte Klick lässt nur die Terminalansicht sichtbar (18/19); daraus wird kein Produktfehler abgeleitet. |
| A03, Modellwahl / Erkennungsfehler | **Fail #720:** Claude-Modelloptionen laufen schon bei 1024×678 rechts heraus (22). Die tatsächliche Codex-Erkennungsfehlermeldung wird bei 1024 und 480 ebenfalls rechts abgeschnitten (23/24). Der Start ist im Codex-Fehlerzustand deaktiviert; Escape führt zurück (25/27). Kein Agent oder Login gestartet. |

Das [Korrekturissue #720](https://github.com/byte5ai/zaplex/issues/720) enthält
Soll/Ist, Reproduktion, Schweregrad und unabhängige Abnahmekriterien. Die sichtbare
Codex-Meldung nennt die installierte CLI und eine fehlende Kontoauthentifizierung;
ihre vollständige Ursache und ein erfolgreicher Login wurden nicht geprüft.
Claude zeigt Modelloptionen und „CLI default login“; daraus folgt keine bestätigte
Anmeldung oder erfolgreiche Provider-Interaktion. Die frühere A03-Lücke ist damit
für Einstieg, Fehlerdarstellung und Abbruch teilweise untersucht, nicht insgesamt
bestanden. Andere Kontozustände, tatsächlicher Modellstart, weitere Varianten und
Bewegungsnachweise bleiben getrennt.
