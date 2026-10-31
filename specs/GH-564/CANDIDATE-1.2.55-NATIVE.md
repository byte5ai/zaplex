# 1.2.55: native Repository-Auswahl und Abbruchfokus

**Kein nativer Gesamt-Pass.** Der vollständige CI-Batch ist erfolgreich; die
anschließende native Prüfung reproduziert einen weiteren Escape-Fehler (#757)
und überschreitet zusätzlich ihr vorab festgelegtes Zeitbudget.

App und Harness: `6da694e0ac4b16b5962c747ad6c64c7aa691c397`, Version 1.2.55.
Tatsächlicher DMG-SHA-256:
`e028a04afcd695af8f220dc30973ced32a33924280dadff72fb297fcbd65caf2`.

## Vollständiger CI-Nachweis

[Batch 41166591979](https://github.com/byte5ai/zaplex/actions/runs/41166591979):
16 ausgewählte Stufen erfolgreich, zwei nicht ausgewählte optionale Stufen nicht
ausgeführt. Die frühen App-Regressionen enthalten zehn gezielte Fälle jeweils
einmal; 27 positive Ergebnisblöcke ergeben 824 erfolgreiche Testausführungen,
keine Fehler oder ignorierten Tests. Daraus wird keine Zahl unterschiedlicher
Tests abgeleitet.

Die Paketsuite trifft exakt das vorab eingefrorene Inventar von 4.405 eindeutigen
Fällen, ohne Fehler, Skip oder Flaky-/Rerun-Fehlereinträge. Die unverändert
installierte GUI besteht alle 16 vorher festgelegten Fälle. Archiv-/DMG-Hashes,
App-/Harness-SHA, tatsächliche Version und strenge Codesign-Prüfung sind unabhängig
bestätigt. Die Originalinventare und Nachweise liegen unter
[evidence/candidate-155-native](evidence/candidate-155-native).

## Native Prüfung A

[Explorationslauf 41178847206](https://github.com/byte5ai/zaplex/actions/runs/41178847206),
EN / Zaplex Dark / 1024×678 Punkte. Der Prüfumfang war vor Start öffentlich mit
SHA-256 festgelegt; seine Kriterien wurden nach den Ergebnissen nicht verändert.

| Beobachteter Teilfall | Ergebnis |
| --- | --- |
| Aktuelles Repository vor Bearbeitung | Sichtbar; tatsächlicher Quell-Checkout auf dem erwarteten HEAD |
| Echte Anfrage `no-such-repository`, Hover und rechter Klick auf die Hinzufügen-Zeile | Volle Zeile hervorgehoben; echter nativer Ordnerdialog geöffnet |
| Nativen Ordnerdialog abbrechen | Repository-Anzeige bytegleich zum vorherigen Bild |
| Worktree-Modal über Abbrechen schließen, danach ohne Prompt-Klick tippen | Frische Ausgabe aus derselben Shell PID 1445, nicht exportierter Marker erhalten |
| Git-/Shellvergleich vor und nach diesem Abbrechen | cwd, HEAD, Git-Verzeichnis und Worktree-Inventar gleich; sauber; ein Worktree |
| Über zentralen Footer-Klick einen vorhandenen Git-Unterordner wählen | Nicht leere neue Identität, geschlossener Suchfilter, Branch `main` und aktivierte Öffnen-Aktion |
| Vollständigen Pfad-Tooltip ansehen | Voller gewählter Pfad bis zum tatsächlichen Unterordner lesbar |
| Escape nach dieser Ordnerwahl | **Fehlgeschlagen:** Modal und Tooltip bleiben sichtbar |
| Unmittelbar danach ohne Zusatzklick tippen | **Fehlgeschlagen:** eindeutiger Befehl fehlt; bewachter Helfer stoppt vor Return |

Die drei Originalbilder `019`, `020` und `021` sind bytegleich. Escape wurde nach
412,637 Sekunden gedrückt; beide folgenden Fehlerbeobachtungen lagen noch innerhalb
des 480-Sekunden-Budgets. Der Escape-Pfad ist unter
[#757](https://github.com/byte5ai/zaplex/issues/757) belegt. Der direkte
Schließ-Fokus-Fix [#755](https://github.com/byte5ai/zaplex/issues/755) ist für den
beobachteten Mouse-Cancel-Teilfall bestätigt; daraus folgt keine Escape-Abnahme.
Nach dem fehlgeschlagenen Escape gibt es keinen neuen Shell-/Git-Snapshot.

## Budget und Nachweisgrenzen

Das exklusive 480-Sekunden-Budget wurde insgesamt überschritten: 506,818 Sekunden
zwischen den aufgezeichneten Aktionen, 509,609 Sekunden ab tatsächlichem Beginn.
**Der Umfang bleibt auch deshalb fehlgeschlagen.** Ein bestandener Teilfall oder
der erfolgreiche Infrastrukturjob ersetzt keinen bestandenen Gesamtumfang.

Der Lauf wurde ausdrücklich beendet, ohne Ablauf seines Interaktionsfensters;
anschließend war kein aktives Operatorziel für diesen Lauf mehr vorhanden.
21 Originalbilder wurden tatsächlich betrachtet, alle 24 lokalen Bilder sind
bytegleich zum archivierten Workflow-Artefakt `13034304103`. ZIP-SHA-256:
`5cd055589a94f89ed5cdefeaaeb5a660e1807b5abc61cff8e57f6b7cd865fa73`.

Der lange initiale Baseline-Befehl lieferte einen Typing-Timeout nach Versand.
Er wurde nicht erneut eingegeben: Vollständige tatsächliche Eingabe wurde beobachtet,
erst danach Return gesendet und frische Baseline-Ausgabe erfasst. Zwei verfrühte
lokale Bildleseversuche wurden nach Abschluss derselben Aufnahmeprozesse korrigiert;
aus fehlenden Bildern wurde kein Produkturteil abgeleitet.

Die vorbereiteten Umfänge B/C wurden auf diesem bekannten Fehlerstand nicht
begonnen. Tatsächliche Worktree-Erstellung und weitere Sprach-/Themen-/Breiten-
Varianten sind für 1.2.55 damit nicht nativ bestätigt. Echte verwaltete Agenten-
und Abo-Fälle sind weiterhin ohne reale isolierte Profile nicht geprüft.
Kein Release oder Tag wurde gestartet.

Der [maschinenlesbare Nachweis](evidence/candidate-155-native/repository-selection-and-escape.json)
enthält Bildhashes, genaue Zeitwerte, Vergleich der archivierten Shell-/Git-Fakten,
Prüfinventar und explizite Fehlschläge. Die Fortsetzung steht in
[#564](https://github.com/byte5ai/zaplex/issues/564#issuecomment-6540701644).
