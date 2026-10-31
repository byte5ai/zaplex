# 1.2.57: übersetzte Repository-Aktion und vollständiger Theme-Hinweis

Die beiden Textkorrekturen sind gemergt. Der vollständige neue CI-Batch und
alle 16 installierten GUI-Pflichtfälle sind unabhängig bestätigt. Die gezielten
englischen und deutschen Umfänge sind am selben qualifizierten DMG bestanden;
zwei frühere englische Fehlversuche bleiben getrennt bewahrt.
Eine vollständige UI-/UX- oder Produktabnahme folgt aus den CI-Ergebnissen nicht.

App und Harness: `51b6aa63fb8fe485bb4f3b2635883facf3514c07`, Version 1.2.57.
Tatsächlicher neuer DMG-SHA-256:
`8715686dee80ffdd61921a32e31f4e080dcffc339d09249ca365de38edc517b4`.

## Korrekturen und lokale Prüfung

[PR #762](https://github.com/byte5ai/zaplex/pull/762) behebt die auf 1.2.56
tatsächlich beobachteten Fehler:

- [#759](https://github.com/byte5ai/zaplex/issues/759): Der eigene
  Repository-Picker verwendet für die Hinzufügen-Zeile den vorhandenen
  übersetzten Schlüssel `workspace-add-new-repo`.
- [#760](https://github.com/byte5ai/zaplex/issues/760): Die Erklärung zur
  Betriebssystem-Theme-Synchronisierung verwendet einen umbrechenden Absatz.

Der [lokale Nachweis](evidence/text-labels-759-760-local.json) bestätigt
13 vorhandene gezielte Tests, `cargo check --locked`, striktes Clippy,
13 Phase-A-Prüfungen und die EN-/DE-Katalogparität. Es wurden keine neuen
Testfälle ergänzt. Die vorbestehenden Formatierungsabweichungen der
Appearance-Datei bleiben getrennt ausgewiesen.

## Vorab festgelegte Erwartungen und vollständige CI-Ergebnisse

Die Inventare und nativen Umfänge wurden vor dem neuen Batch
[öffentlich mit SHA-256 festgehalten](https://github.com/byte5ai/zaplex/issues/564#issuecomment-6543879721).
Die Paketfallmenge stammt aus dem unabhängig verifizierten Original-JUnit
von 1.2.56, die frühen gezielten Fälle aus dem tatsächlichen lokalen Lauf
auf 1.2.57. Testquellen und GUI-Pflichtfälle wurden nicht geändert.

[Batch 41205026965](https://github.com/byte5ai/zaplex/actions/runs/41205026965)
hat 16 ausgewählte Stufen erfolgreich abgeschlossen; zwei nicht ausgewählte
optionale Stufen wurden nicht ausgeführt. Alle allgemeinen/macOS-Jobs laufen
auf `macrunner`, nur der mitgelieferte Linux-Daemon und explizite Linux-Tests
auf Linux.
Die drei tatsächlich ausgewählten Linux-Jobs sind Daemon-Regressionen,
der musl-Daemon-Bau und die isolierte Linux-OpenSSH/SFTP-Serverprüfung samt
SafeFileServer-Commit-Wiederherstellung. Diese Zuordnung ist in der vorhandenen
[Runner-Prüfung](../../script/ci-runner-policy-tests) ausdrücklich erfasst;
die Linux-Dateiserverpfade verwenden unter anderem `renameat2`.

- Alle 13 gezielten Fälle jeweils genau einmal; 27 positive Ergebnisblöcke
  mit 827 erfolgreichen Ausführungen, ohne Fehler oder ignorierte Tests.
  827 bezeichnet keine Menge unterschiedlicher Tests.
- Exakt 4.408 eindeutige Paketfälle, ohne Fehler, Skips oder
  Flaky-/Rerun-Fehlereinträge; Original-JUnit und Archivdigest unabhängig
  gegen das eingefrorene Inventar geprüft.
- Alle 16 eingefrorenen GUI-Pflichtfälle am unverändert installierten neuen
  DMG bestanden; kein Fehler, Skip oder erwarteter Fehler. Tatsächliche
  Version, App-/Testquelle, DMG- und Archivdigest sowie strenge
  Codesign-Prüfung wurden unabhängig zugeordnet.

Die maschinenlesbaren Nachweise liegen unter
[evidence/candidate-157-native](evidence/candidate-157-native).

## Vorab festgelegte native Nachprüfung

Die eingefrorenen Umfänge legen zwei frische Sitzungen fest:
Englisch mit echter Worktree-Erstellung sowie Deutsch nach tatsächlichem
Sprachwechsel und App-Neustart. Beide umfassen Dark/Light bei 1024×678 und
480×678 Punkten, die übersetzte volle Hinzufügen-Zeile, echte native
Ordnerdialoge, den vollständigen Theme-Hinweis und unassistierte Shellproben
nach Abbrechen und Escape. Das exklusive 480-Sekunden-Budget gilt pro Sitzung
ab vor der ersten nativen Abfrage bis einschließlich des Abschlusses.

Der [erste englische Lauf 41222001064](https://github.com/byte5ai/zaplex/actions/runs/41222001064)
bleibt **fehlgeschlagen**: 480,582967 Sekunden ab vor der ersten nativen
Abfrage überschreiten die unveränderte exklusive 480-Sekunden-Grenze. Kein
Kulanzabzug; die 477,438916 Sekunden zwischen den aufgezeichneten Aktionen
ersetzen diesen vollständigen Zeitraum nicht. Alle eingefrorenen sichtbaren
Produktpunkte wurden beobachtet: die vier Theme-/Breitenkombinationen,
native Ordnerdialoge, vollständiger englischer Theme-Hinweis, unassistierte
Shellerhaltung nach Abbrechen/Escape und echter neuer Worktree mit Rückkehr
zur Quelle. 26 Originalbilder wurden betrachtet, alle 34 lokalen Bilder
und alle fünf tatsächlichen Rohfaktensätze sind bytegleich zum Archiv.
Der [getrennte Nachweis](evidence/candidate-157-native/english-labels-layout-and-worktree-budget-failed-attempt.json)
bewahrt den fehlgeschlagenen Umfang unverändert.

Die Wiederholungen verwenden dasselbe qualifizierte DMG und dieselben
eingefrorenen Kriterien. Eingabe-/Snapshotfolgen und ausdrückliche Beendigung
wurden verbunden; alle verwendeten Originalbilder wurden vor dem Produkturteil
betrachtet. Keine Prüfung oder Eingabeguard entfällt, kein erfolgreicher Build
oder installierter GUI-Fall wurde wiederholt.

Der [zweite englische Versuch 41223758751](https://github.com/byte5ai/zaplex/actions/runs/41223758751)
bleibt ebenfalls **unvollständig und fehlgeschlagen**: 523,052686 Sekunden,
24 betrachtete Bilder und alle 31 lokalen Bilder archivverifiziert. Die drei
ursprünglichen Shell-/Git-Snapshots bestätigen den unassistierten Abbrechen-
und Escape-Rückweg. Die Probe im neuen Tab wurde begonnen, während dort noch
der initiale Git-/LFS-Befehl lief; der vollständige Entwurf kam verändert an.
Der bestehende Eingabeguard verhinderte Return. Nur der eigene Entwurf wurde
mit Ctrl-U ohne Return gelöscht und die Sitzung ausdrücklich beendet.
Neue Worktree-/Rückkehr-Snapshots fehlen; dafür wird kein Pass abgeleitet.
Der [separate Nachweis](evidence/candidate-157-native/english-labels-layout-and-worktree-input-incomplete-attempt.json)
bewahrt diese Grenzen.

## Bestandener gezielter englischer Umfang

[Lauf 41226779501](https://github.com/byte5ai/zaplex/actions/runs/41226779501)
bestätigt alle unveränderten eingefrorenen EN-Kriterien am selben DMG:

| Tatsächlich beobachteter Teilfall | Ergebnis |
| --- | --- |
| Dark/Light, 1024×678 und 480×678 | Sichtbares aktuelles Repository, vollständiger lesbarer Pfadtooltip, kanonisches `+ Add new repo` und Hover über die ganze Zeile |
| Klick am rechten Zeilenrand, alle vier Varianten | Je echter nativer Ordnerdialog; Abbruch erhält die Auswahl |
| Vollständiger englischer OS-Theme-Hinweis | Mit geöffneter Theme-Seitenleiste in Dark und Light bis „when your system does.“ sichtbar |
| Normales Dark-Abbrechen und schmales Light-Escape | Frische unassistierte Proben erhalten Original-PID 1480, WT157EN-Marker, cwd, erwarteten HEAD, gemeinsames Git-Verzeichnis, sauberen Status und einen Worktree |
| Reale Erstellung über Öffnen in der App | Neuer eigener Branch und Worktree `badger-sundowner`; tatsächlich laufender Initialbefehl beobachtet, dann dessen Abschluss und leerer bereiter Prompt |
| Ohne Prompt-Klick in den bereiten neuen Tab tippen | Neue Shell PID 3893 ohne Quellmarker, cwd gleich neuem Worktree; erwarteter HEAD/gemeinsames Git-Verzeichnis, sauberer Status, exakt zwei Worktrees |
| Ursprünglichen Tab wählen und frisch tippen | PID 1480, Marker, cwd und saubere Git-Daten erhalten; neuer Worktree genau einmal und mit eigenem Branch registriert |

387,999472 Sekunden ab vor der ersten nativen Abfrage und 385,189910 Sekunden
zwischen den aufgezeichneten Aktionen liegen beide unter der unveränderten
exklusiven 480-Sekunden-Grenze. Ausdrücklich beendet, kein Ablauf. 27 neue
Originalbilder wurden tatsächlich betrachtet; alle 36 lokalen Bilder und alle
fünf tatsächlichen Rohfaktensätze sind bytegleich zum Originalarchiv.
Der [unabhängig qualifizierte Nachweis](evidence/candidate-157-native/english-labels-layout-and-worktree.json)
enthält die tatsächlichen Zeiten, Bildhashes und Git-/Shellvergleiche.

Die bekannten Schritte wurden für diese gezielte native Regression verbunden
und ihre frischen Bilder vor dem Produkturteil betrachtet. Vor der neuen
Worktree-Eingabe wurde anhand der tatsächlichen App auf den vollständigen
Abschluss des initialen Git-/LFS-Befehls und einen leeren Prompt gewartet;
es gab keinen unterstützenden Prompt-Klick. Das ist keine neue vollständige
explorative Produktabnahme und ändert den Status beider Fehlversuche nicht.

## Bestandener gezielter deutscher Umfang

[Lauf 41228055254](https://github.com/byte5ai/zaplex/actions/runs/41228055254)
verwendet dasselbe DMG und die unveränderten vorab veröffentlichten DE-Kriterien.
Deutsch wurde im sichtbaren Sprachmenü gewählt; die eigene App wurde vollständig
beendet und neu gestartet. Der Prozess wechselte von PID 1386 zu PID 1901.
Die neue Baseline gehört zur **post-restart Shell PID 1951** mit nicht
exportiertem WT157DE-Marker. Ein Shellerhalt über den Neustart wird nicht behauptet.

| Tatsächlich beobachteter Teilfall | Ergebnis |
| --- | --- |
| Dark/Light, 1024×678 und 480×678 | Repository und vollständiger Pfadtooltip lesbar; `+ Neues Repository hinzufügen` vollständig sichtbar, ganze Zeile hervorgehoben |
| Klick am rechten Zeilenrand, alle vier Varianten | Je echter nativer Ordnerdialog; Abbruch erhält die Auswahl |
| Deutscher OS-Theme-Hinweis mit geöffneter Seitenleiste | In Dark und Light vollständig, auf zwei Zeilen einschließlich des letzten Wortes „tut.“; kein Abschneiden |
| Normales Dark-Abbrechen | Ohne Prompt-Klick frische Ausgabe aus derselben post-restart Shell PID 1951; Marker, cwd und alle sauberen Git-/Worktree-Fakten erhalten |
| Tatsächliches Escape aus dem schmalen Light-Modal | Modal geschlossen, ohne Prompt-Klick frische gleiche Shell-/Git-Probe; weiterhin exakt ein Worktree |

354,524799 Sekunden ab vor der ersten nativen Abfrage einschließlich Sprachwahl,
Neustart, Proben und Abschluss; 352,317697 Sekunden zwischen den aufgezeichneten
Aktionen. Beide liegen unter der unveränderten exklusiven 480-Sekunden-Grenze.
Ausdrücklich beendet, kein Ablauf. 25 Originalbilder wurden tatsächlich
betrachtet; alle 32 lokalen Bilder und alle drei tatsächlichen Rohfaktensätze
sind bytegleich zum Originalarchiv. Der
[unabhängig qualifizierte DE-Nachweis](evidence/candidate-157-native/german-labels-layout-and-focus.json)
bestätigt Herkunft, Budget und die unveränderten drei Quell-Snapshots.

Damit sind #759 und #760 im festgelegten nativen Umfang nachgeprüft. Die zwei
erfolgreichen Sitzungen liefern zusammen 52 betrachtete neue Originalbilder;
die 50 betrachteten Bilder der beiden Fehlversuche bleiben davon getrennt.

## Aussagegrenzen

Frühere fehlgeschlagene oder unvollständige Sitzungen bleiben unverändert
bewertet. Echte isolierte Abonnements für den verwalteten Agenten-Fork und
gefüllte Kontofälle fehlen weiterhin; dafür wird kein Pass abgeleitet.
