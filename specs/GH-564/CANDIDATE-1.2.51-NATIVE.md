# Installierte Abnahme und Exploration von 1.2.51

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562). App- und GUI-Suite-Quelle:
`421125b94239368c718d30784e7c3d17448b3506`, Version 1.2.51. Das unveränderte,
signierte und notarisierte DMG stammt aus
[Batch 41056697950](https://github.com/byte5ai/zaplex/actions/runs/41056697950).
DMG-SHA256: `20afd5f5d8900f34c5429044eb142ced8787d7059f539f2599c33b7fa5fc0195`.
Es wurde kein Release oder Tag erstellt.

## CI und unabhängige installierte Prüfung

Der vollständige Batch ist erfolgreich, ohne Übernahme alter Produktgates.
Alle **16 unterschiedlichen installierten GUI-Pflichtfälle** bestehen, ohne
Fehler, übersprungene oder erwartete Fehler. Die tatsächlichen XCTest-Fall-IDs
stimmen exakt mit dem vor Ausführung auf diese Quelle gebundenen Bestand überein.
App-/Testquelle, installierte Version und DMG-Digest passen zusammen. Die
Installation des unveränderten Pakets prüft `codesign --deep --strict`.
[GUI-Nachweis](evidence/candidate-151-native/installed-gui.json).

Die Mac-Paketsuite enthält **4.395 bestandene Fälle**, ohne Fehler oder Skips.
Der neue Inbox-Kollisionsfall ist im originalen JUnit genau einmal enthalten.
[Paketnachweis](evidence/candidate-151-native/package-tests.json).
In der frühen App-Prüfung ist derselbe Fall ebenfalls genau einmal erfolgreich
ausgeführt; die 27 positiven Ergebnisblöcke melden insgesamt 814 bestandene
Testausführungen. Diese teilweise überlappenden Auswahlen werden nicht als
disjunkte Gesamtmenge addiert.
[App- und Kürzelnachweis](evidence/candidate-151-native/inbox-shortcut-ci.json).
Linux-Daemon-Regressionen, echtes SFTP, Sicherheitsprüfung, Compile und regulärer
Binary-Clippy sind ebenfalls erfolgreich.

## Echte Inbox und getrennte Kürzel

[Exploration 41075900378](https://github.com/byte5ai/zaplex/actions/runs/41075900378)
prüft Englisch am unveränderten DMG;
[41077486365](https://github.com/byte5ai/zaplex/actions/runs/41077486365) Deutsch
nach tatsächlicher Sprachwahl und vollständigem eigenem App-Neustart. Die danach
erfasste neue Shell-Baseline belegt keine Persistenz über den Neustart.
Der spätere Exploration-Harness
`cdda5e7b114ae978371ae1a12293a21e1f990066` unterscheidet sich von der App-/Suite-Quelle
ausschließlich durch zehn GH-564-Dokumentationsdateien; App, Ressourcen, Workflows
und GUI-Skripte sind unabhängig bytegleich verglichen. Beide Quellen bleiben im
Nachweis getrennt erfasst.

Vor Ausführung festgelegte Erwartungen: Ctrl+Cmd+O öffnet die echte Inbox statt
eines Dateidialogs. In jedem Theme und jeder Größe ist das X vor/mit Hover sichtbar;
ein tatsächlicher Klick schließt nur die Inbox. Anschließend bestätigt frische
Ausgabe dieselbe Quellshell-PID, denselben nicht exportierten Marker und dasselbe
Arbeitsverzeichnis. Nach dem Minimum wird für vollständig lesbare Ausgabe wieder
auf Normalgröße vergrößert.

| Variante | 1024×678 | 480×320 |
|---|---|---|
| EN / Zaplex Dark | „Open items“ und vollständiger Leerzustand sichtbar; X bei Ruhe/Hover, Klick und frische ursprüngliche Shell bestätigt (003–006). | Derselbe Ablauf bestätigt (007–010). |
| EN / Light | Derselbe Ablauf nach echtem Themewechsel bestätigt (014–017). | Derselbe Ablauf bestätigt (018–021). |
| DE / Zaplex Dark | „Offene Punkte“ und vollständiger Leerzustand sichtbar; X bei Ruhe/Hover, Klick und frische ursprüngliche Shell bestätigt (007–010). | Derselbe Ablauf bestätigt (011–014). |
| DE / Light | Derselbe Ablauf nach echtem Themewechsel bestätigt (018–021). | Derselbe Ablauf bestätigt (022–025). |

Cmd+Shift+O öffnet weiterhin den echten nativen Repository-Dateidialog. Das aktive
Dialogfenster wurde über das tatsächliche Fensterinventar bestimmt und explizit
aufgenommen (022). Escape schließt ohne Auswahl; das Hauptfenster wird wieder
aktiv, und frische Ausgabe bestätigt dieselbe ursprüngliche Shell (023).
Die deutsche Sitzung bestätigt denselben Dialog- und Rückweg mit expliziter
Fensterwahl und frischer gleicher Shell (026–027). Damit sind beide Standardkürzel
in DE und EN tatsächlich getrennt bedienbar. Die Inbox ist zusätzlich als zweiter
echter Aufrufer der gemeinsamen Modal-Schließen-Factory in allen acht Varianten
geprüft; daraus folgt keine pauschale Abnahme aller Modale.

23 englische Originale wurden einzeln betrachtet. Alle 30 lokal gesicherten PNGs
stimmen bytegenau mit dem abgeschlossenen Workflow-Artefakt überein. Die Sitzung
wurde nach knapp sieben Minuten eigener Bedienung ausdrücklich beendet, ohne
Ablauf des Operatorfensters.
[EN-Fallurteile, Artefaktherkunft und Einzelhashes](evidence/candidate-151-native/41075900378.json).

27 deutsche Originale wurden zusätzlich einzeln angesehen; alle 34 lokal
gesicherten PNGs stimmen bytegenau mit dem abgeschlossenen Workflow-Artefakt
überein. Auch diese Sitzung wurde nach weniger als acht Minuten ausdrücklich
beendet, ohne Ablauf des Operatorfensters. Beide Sitzungen umfassen zusammen
50 einzeln betrachtete Originale.
[DE-Fallurteile, Artefaktherkunft und Einzelhashes](evidence/candidate-151-native/41077486365.json).

## Tatsächlicher Terminal-Worktree-Einstieg

[Exploration 41079791577](https://github.com/byte5ai/zaplex/actions/runs/41079791577)
prüft EN / Zaplex Dark / 1024×678 am selben DMG. Vor der Aktion wurden der
vorhandene öffentliche Projektcheckout, sein Ursprung, `main`, HEAD
`cdda5e7b114ae978371ae1a12293a21e1f990066`, sauberer Git-Status und genau ein
Worktree unabhängig geprüft. Es wurde kein neues Repository erzeugt oder geklont.
Nach dem Wechsel in dieses Repository wurde die Quellshell-Baseline erfasst.

Das sichtbare Plusmenü öffnet den Worktree-Dialog. Die native Ordnerauswahl wählt
das bestehende Repository; Open startet die tatsächliche Worktree-Erzeugung im
neuen Tab. Die frische Git-/Shellprobe bestätigt einen eigenen Shellprozess,
eigenes Verzeichnis, neuen Branch `barrel-siesta`, denselben Basis-HEAD und dasselbe
gemeinsame Git-Verzeichnis. Der ursprüngliche nicht exportierte Marker ist in
dieser neuen Shell nicht gesetzt. Die Tabtitel unterscheiden Ausgangsrepository
und neuen Worktree lesbar.

Nach Rückkehr in den ursprünglichen Tab bestätigt frische Ausgabe dieselbe
Quellshell-PID, Marker und CWD. Die im Workflow gesicherten Git-Vorher-/Nachher-
Dateien wurden unabhängig verglichen: HEAD und sauberer Status bytegleich,
ursprünglicher Worktree-Eintrag unverändert, genau ein zusätzlicher Worktree am
gleichen Basis-HEAD. Das ist ein funktionaler Terminal-Worktree-Pass in dieser
Variante; kein verwalteter Agenten-Fork oder Abo-Nachweis.

**UI-Abweichung [#749](https://github.com/byte5ai/zaplex/issues/749):** Das
Repository-Feld bleibt nach wirksamer Auswahl leer, obwohl Open aktiviert wird.
Der Benutzer kann das tatsächlich gewählte Ziel vor dem Start nicht sehen.
Die Ursache ist im echten `RepoPicker` bestätigt: Eine leere Itemliste kann die
anschließend gewünschte Auswahl nicht anzeigen, während der Worktree-Dialog den
Pfad separat korrekt an Submit weitergibt. Konkrete Reproduktion, Priorität P2
und ein Testauftrag stehen im Korrekturissue. Hier wird ausdrücklich kein
vollständiger UX-Pass für den Worktree-Dialog behauptet.

Das verkürzte englische Elternmenü zeigt dagegen beim Hover den vollständigen
Tooltip „New worktree config“ (007). Das erfüllt das bestehende vollständiges-
Label-oder-Tooltip-Kriterium von #711; kein neuer Defekt wird daraus abgeleitet.

Ein flüchtiges natives „Go to Folder“-Fenster hat keine unabhängige AX-Fenster-
Identität. Die zunächst ans Elternfenster gerichtete Pfadeingabe wurde vom
Operator wegen unbestätigten Fokus abgelehnt. Erst die frische Bildschirmaufnahme,
gezielte dokumentierte Vordergrundeingabe und sichtbares Pfad-Rücklesen erlaubten
das anschließende Return. Die beiden Bildschirmaufnahmen zeigen einen OS-Banner
außerhalb des geprüften Eingabefelds; sie belegen keine vollständige Toolbar-Parität.

19 Originale wurden einzeln betrachtet; alle 26 lokal gesicherten PNGs sind
bytegleich mit dem abgeschlossenen Workflow-Artefakt. Der ausdrückliche Abschluss
erfolgte nach rund neun Minuten, ohne Ablauf des Operatorfensters.
[Fallurteile, unabhängiger Git-Vergleich und Einzelhashes](evidence/candidate-151-native/41079791577.json).

## Aussagegrenzen

Die native Prüfung betrifft die echte leere Inbox und den bestehenden Repository-
Einstieg. Wartende Agenten, gefüllte Kontenansichten, eigene Nutzerbelegungen und
Live-Abonnementstarts werden daraus nicht abgeleitet. Die isolierten Abo-Testprofile
fehlen weiterhin; API-Profile, unangemeldete CLIs und synthetische Fixtures ersetzen
sie nicht. Die drei Sitzungen umfassen 69 einzeln betrachtete Originale; deren
gesamte 90 lokal gesicherte PNGs wurden gegen die Originalartefakte verglichen.
Eine vollständige
Produktmatrix- oder Releasefreigabe folgt aus diesen gezielten Fällen nicht.
