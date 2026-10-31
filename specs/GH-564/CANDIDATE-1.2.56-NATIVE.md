# 1.2.56: Repository-Auswahl, Abbruchfokus und echter Worktree

Der vollständige CI-Batch sowie die nativen Umfänge A und B sind bestanden.
Der deutsche Umfang C bleibt unvollständig und überschreitet zusätzlich sein
Zeitbudget. Er belegt die neuen Textfehler #759/#760; eine vollständige
UI-/UX- oder Produktabnahme folgt daraus nicht.

App und Harness: `73016d8854d2fb784c1b247fad48d468948671d5`, Version 1.2.56.
Tatsächlicher DMG-SHA-256:
`2e0dbbbdd1f67a41e2ccf73507cac7bba92e5c986a7f1b74d377c2ed520e7e02`.

## Korrektur und vollständiger CI-Nachweis

[#757](https://github.com/byte5ai/zaplex/issues/757) war ein tatsächlich offenes
Modal nach Escape aus der Repository-Auswahl. [PR #758](https://github.com/byte5ai/zaplex/pull/758)
gibt den Fokus nach Schließen des Suchfilters an den noch sichtbaren Picker
zurück, sofern dessen Kind den Fokus hält. Bereits anderswo liegender Fokus
wird dabei erhalten. Drei Regressionen verwenden echte Presenter-/KeyDown-
Weiterleitung; der ursprüngliche Fehler ist vor der Korrektur rot nachgewiesen.
Der [lokale Nachweis](evidence/worktree-picker-escape-757-local.json) enthält
Red/Green, Compile, striktes Clippy und die gezielten Prüfungen samt Grenzen.

[Batch 41184401648](https://github.com/byte5ai/zaplex/actions/runs/41184401648)
bestätigt 16 ausgewählte erfolgreiche Stufen; zwei nicht ausgewählte optionale
Stufen wurden nicht ausgeführt. Alle allgemeinen/macOS-Jobs laufen auf
`macrunner`, nur der Linux-Daemon und explizite Linux-Tests auf Linux.

Die frühen App-Regressionen treffen alle 13 gezielten Fälle jeweils einmal.
27 positive Ergebnisblöcke enthalten 827 erfolgreiche Testausführungen ohne
Fehler oder ignorierte Tests; dies ist keine Zahl unterschiedlicher Tests.
Die Paketsuite trifft exakt die vorab eingefrorenen 4.408 eindeutigen Fälle,
ohne Fehler, Skip oder Flaky-/Rerun-Fehlereinträge. Das unverändert installierte
DMG besteht alle 16 vorab festgelegten GUI-Fälle. Archiv-/DMG-Hashes,
App-/Harness-SHA, tatsächliche Version und strenge Codesign-Prüfung wurden
unabhängig bestätigt. Inventare und Nachweise liegen unter
[evidence/candidate-156-native](evidence/candidate-156-native).

## Native Prüfung A: Ordnerwechsel und Tastaturfokus

[Lauf 41194592925](https://github.com/byte5ai/zaplex/actions/runs/41194592925),
EN / Zaplex Dark / tatsächliche 1024×678 Punkte. Der Umfang ist vor Start
[öffentlich mit SHA-256 festgelegt](https://github.com/byte5ai/zaplex/issues/564#issuecomment-6541101187).

| Tatsächlich beobachteter Teilfall | Ergebnis |
| --- | --- |
| Aktuelles Git-Repository vor Bearbeitung | Sichtbarer Pfad; erwarteter HEAD, sauberer Checkout, ein Worktree |
| Echte erfolglose Suche und rechter Footer-Klick | Ganze Zeile hervorgehoben; echter nativer Ordnerdialog |
| Nativen Dialog abbrechen | Repository-Anzeige bytegleich zum vorherigen Bild |
| Worktree-Modal mit Abbrechen schließen | Unassistierte Eingabe erreicht dieselbe Shell PID 1454 und ihren nicht exportierten WT156A-Marker |
| Echten langen Git-Unterordner über zentralen Footer-Klick wählen | Neuer nicht leerer Pfad, voller lesbarer Tooltip, Branch `main`, Öffnen erreichbar |
| Tatsächliches Escape nach Ordnerwahl | Modal geschlossen; unassistierte Eingabe erreicht wieder dieselbe Original-Shell |
| Frische Git-/Shellvergleiche nach beiden Abbrüchen | PID, Marker, cwd, HEAD, gemeinsames Git-Verzeichnis, sauberer Status und ein Worktree unverändert |

380,487 Sekunden ab tatsächlichem Beginn; 377,898 Sekunden zwischen den
aufgezeichneten Aktionen. Beide Werte liegen unter dem unveränderten exklusiven
480-Sekunden-Budget. Ausdrücklich beendet, kein Ablauf des Interaktionsfensters.
17 Originalbilder wurden tatsächlich betrachtet; alle 21 lokalen Bilder und
drei live abgefragte Rohfaktensätze sind bytegleich zum Originalarchiv.
Der [maschinenlesbare Nachweis](evidence/candidate-156-native/repository-selection-and-focus.json)
enthält Bildhashes, genaue Zeiten und Git-/Shellvergleich.

## Native Prüfung B: Breiten, Light und tatsächliche Worktree-Erstellung

[Lauf 41196038594](https://github.com/byte5ai/zaplex/actions/runs/41196038594),
EN / Zaplex Dark und Light / tatsächliche 1024×678 und 480×678 Punkte.
Unveränderte vorab veröffentlichte B-Kriterien und dasselbe qualifizierte DMG.

| Tatsächlich beobachteter Teilfall | Ergebnis |
| --- | --- |
| Dark bei 480 Punkten | Repository und voller Tooltip lesbar; Footer über die volle Breite hervorgehoben, rechter Klick öffnet nativen Dialog |
| Dark: nativer Abbruch und Escape aus dem Modal | Auswahl erhalten; unassistierte Eingabe in ursprünglicher Shell PID 1434, WT156B-Marker und alle Git-/Shellfakten unverändert |
| Light bei 1024 und 480 Punkten | Repository und voller Tooltip lesbar; voller Footer-Hover und rechter Klick öffnen jeweils den echten nativen Dialog; Abbruch erhält die Auswahl |
| Rückkehr zu 1024 Punkten, Öffnen über die App | Echter neuer Worktree mit eigenem Branch und neuem Terminal-Tab |
| Ohne Prompt-Klick im neuen Tab tippen | Frische Ausgabe aus Shell PID 4427; kein geerbter WT156B-Marker, cwd ist der neue Worktree |
| Gitvergleich im neuen Tab | Erwarteter HEAD, dasselbe gemeinsame Git-Verzeichnis, sauberer Status, exakt zwei Worktrees |
| Zum ursprünglichen Tab zurückkehren und tippen | Frisch PID 1434, WT156B-Marker und cwd/HEAD/Git-Verzeichnis unverändert; Quelle sauber, neuer Worktree genau einmal registriert |

419,591 Sekunden ab tatsächlichem Beginn, unter dem unveränderten exklusiven
480-Sekunden-Budget. Ausdrücklich beendet, kein Ablauf. 22 Originalbilder wurden
tatsächlich betrachtet; sämtliche lokalen Bilder sind bytegleich zum Archiv.
Alle vier archivierten tatsächlichen Shell-/Git-Snapshots wurden unabhängig
verglichen; drei lagen zusätzlich live lokal vor und sind bytegleich. Die letzte
Live-Abfrage erfolgte nach ausdrücklichem Ende und wurde verweigert; verwendet
werden die frische betrachtete Rückkehrausgabe und der originale Archivdatensatz.

Ein privater Operator-Helfer lieferte zunächst eine vor Versand verweigerte
Resize-Option; sie wurde korrigiert. Nach Theme-Auswahl blieb Settings zunächst
offen, was auf dem Originalbild erkannt und vor Modalöffnung korrigiert wurde.
Diese Operatorhandlungen ergeben keinen zusätzlichen Produkt-Pass.
Der [maschinenlesbare B-Nachweis](evidence/candidate-156-native/layout-and-real-worktree.json)
enthält die getrennten Beobachtungen und Integritätsprüfungen.

Ein [erster B-Versuch](https://github.com/byte5ai/zaplex/actions/runs/41195308301)
bleibt ausdrücklich **unvollständig**: Ein Operator-Kontextwechsel verbrauchte
Zeit; Light und Worktree-Erstellung wurden nicht ausgeführt. Der Versuch wurde
nach 363,924 Sekunden beendet, statt das Budget zu verlängern. Seine belegten
Dark-/Escape-Teilfälle, neun betrachteten Bilder und fehlenden Teile sind im
[separaten Nachweis](evidence/candidate-156-native/layout-and-worktree-incomplete-attempt.json)
erhalten. Der frische vollständige B-Lauf ändert diesen früheren Status nicht.

## Native Prüfung C: deutsche Teilnachweise und neue Textfehler

[Lauf 41196800714](https://github.com/byte5ai/zaplex/actions/runs/41196800714),
App und Harness unverändert. Deutsch wurde über die sichtbare Einstellung
gewählt und die eigene App vollständig beendet und neu gestartet. Die folgende
Baseline gehört zur **neuen Shell PID 1839** mit nicht exportiertem WT156C-Marker;
ein Shellerhalt über den Neustart wird nicht behauptet.

| Tatsächlich beobachteter Teilfall | Ergebnis |
| --- | --- |
| Dark, 1024×678 und 480×678 | Repository und voller Tooltip lesbar; ganze Footer-Zeile hervorgehoben; rechter Klick öffnet echten nativen Ordnerdialog; Abbruch erhält Auswahl |
| Dark/Normal: Modal mit Abbrechen schließen | Ohne Prompt-Klick frische Ausgabe aus derselben post-restart Shell und unveränderte saubere Git-Fakten |
| Light, 1024×678 | Repository und voller Tooltip lesbar; ganze Footer-Zeile und nativer Ordnerabbruch funktionieren |
| Light/Normal: tatsächliches Escape | Modal geschlossen; ohne Prompt-Klick frische Ausgabe aus derselben post-restart Shell |
| Vergleich aller drei tatsächlichen Rohfaktensätze | PID, Marker, cwd, HEAD, gemeinsames Git-Verzeichnis, sauberer Status und ein Worktree unverändert |
| Hinzufügen-Beschriftung in der deutschen App | **Fail #759:** eigener Picker-Footer zeigt weiterhin `+ Add new repo...` in Dark/Normal, Dark/schmal und Light/Normal |
| Theme-Auswahl mit zusätzlicher Seitenleiste | **Fail #760:** längere deutsche Erklärung zur Betriebssystem-Synchronisierung bei 1024 Punkten rechts sichtbar abgeschnitten |
| Light, 480×678 | **Not-run:** nach Korrektur der deutschen Menükoordinaten blieb kein ausreichendes Operatorbudget |

Die Repository-Beschriftung umgeht den bereits vorhandenen Fluent-Schlüssel:
`RepoPicker` enthält eine feste englische Konstante. Das benachbarte
Repository-Menü nutzt den übersetzten Schlüssel bereits. Die Theme-Erklärung
verwendet einen ausdrücklich nicht umbrechenden `Span` statt des vorhandenen
`Paragraph`. Befunde, Reproduktion und Korrekturaufträge stehen in
[#759](https://github.com/byte5ai/zaplex/issues/759) und
[#760](https://github.com/byte5ai/zaplex/issues/760).

**Kein C-Pass, auch wegen des Zeitbudgets:** 488,022 Sekunden ab tatsächlichem
Beginn bzw. 485,756 Sekunden zwischen den aufgezeichneten Aktionen überschreiten
die unveränderte exklusive 480-Sekunden-Grenze. Der Abschluss kam acht Sekunden
zu spät; das Budget wird nicht erweitert. Der Lauf wurde ausdrücklich beendet,
ohne Ablauf seines längeren Infrastruktur-Interaktionsfensters. Infrastruktur-
Erfolg ersetzt weder den fehlenden Light/schmal-Fall noch die native Abnahme.

Der private Bedienhelfer verwendete zunächst englische Menükoordinaten.
Originalbilder zeigten dabei kein Modal. Eine harmlose Suche landete im
Terminalentwurf und wurde mit Ctrl-U **ohne Return** gelöscht. Erst frische
deutsche Menüaufnahmen lieferten die richtigen Positionen; nur die folgenden
tatsächlich sichtbaren Modals zählen als Dialogbeleg. Diese Operatorfehler
bleiben vom eigentlichen Übersetzungsfehler getrennt.

24 Originalbilder wurden tatsächlich betrachtet; sämtliche 32 lokalen Bilder
sind bytegleich zum Originalarchiv. Alle drei archivierten tatsächlichen
Git-/Shell-Snapshots wurden unabhängig verglichen, die zusätzlich live abgefragte
Baseline ist bytegleich. Der
[maschinenlesbare C-Nachweis](evidence/candidate-156-native/german-layout-and-focus.json)
bewahrt die Fehler, fehlende Variante und exakten Zeiten. Der folgende
Korrekturbatch ändert den Status dieses 1.2.56-Laufs nicht.

## Aussagegrenzen

Die englischen Prüfungen bestätigen einen Terminal-Worktree. Ein echter
verwalteter Agenten-Worktree-Fork und gefüllte Abo-/Kontofälle sind ohne reale
isolierte Profile weiterhin nicht geprüft; Simulationen ergeben dafür keinen
Pass. Die vollständige Matrix von #564 und die übergreifende Produktabnahme
von #562 bleiben getrennt. Kein Release oder Tag wurde gestartet.
