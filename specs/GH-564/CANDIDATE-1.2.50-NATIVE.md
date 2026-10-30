# Installierte Abnahme und Exploration von 1.2.50

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562). App- und Testquelle:
`b3d5c4cde4b372e6770aed97a1c43accf04bfe70`. Alle Explorationen verwenden das
unveränderte, signierte und notarisierte DMG aus
[Batch 40992840627](https://github.com/byte5ai/zaplex/actions/runs/40992840627).
DMG-SHA256: `077ff57b221b01c70faf19d33a3f7921fa33f1d250873d04362adc5ba82712dc`.
Es wurde kein Release oder Tag erstellt.

## CI und installierte Pflichtfälle

Der vollständige Batch ist erfolgreich. Die Mac-Paketsuite enthält 4.394
bestandene Fälle, ohne Fehler oder übersprungene Fälle. Die gezielten App-Schritte
melden zusammen 813 bestandene Testausführungen in 27 Ergebnisblöcken; diese Zahl
bezeichnet keine 813 unterschiedlichen, überschneidungsfreien Tests.
Linux-Daemon und echtes SFTP bestehen ebenfalls.
[Fallzahlen und Quellnachweise](evidence/candidate-150-native/ci-results.json).

Alle **16 unterschiedlichen installierten GUI-Pflichtfälle** bestehen ohne Fehler
oder übersprungene Fälle. Der originale XCTest-Fallbaum wurde unabhängig gegen
den vor Ausführung gesicherten Pflichtbestand geprüft. App-/Testquelle, installierte
Version und DMG-Hash stimmen überein. Die Installation prüft die unveränderte
Signatur mit `codesign --deep --strict`.
[Unabhängiger Nachweis](evidence/candidate-150-native/installed-gui.json).

Die zwei Kontrastregressionen für die tatsächliche gemeinsame Schließen-Factory
bestehen jeweils einmal in der frühen App-Prüfung, zusätzlich zur späteren
vollständigen Paketsuite. PR #745 ergänzt diese frühe Auswahl; die Tests wären
bereits ohne diese Ergänzung in der Paketsuite ausgeführt worden.
[CI-Fälle und Loghash](evidence/candidate-150-native/modal-close-ci.json).

## Sichtbares Schließen-Symbol und erhaltene Shell

Die vorab festgelegte Nachprüfung von #743 umfasst Ruhe, Hover und echten Klick
auf das sichtbare obere X, jeweils bei 1024×678 und 480×320 Punkten. Nach jedem
Schließen folgt eine frische Prüfung von PID, nicht exportiertem Marker und CWD.

[Exploration 41034730295](https://github.com/byte5ai/zaplex/actions/runs/41034730295)
prüft Englisch. [41038876716](https://github.com/byte5ai/zaplex/actions/runs/41038876716)
prüft Deutsch nach vollständigem App-Neustart mit neuer Shell-Baseline; daraus
folgt keine Shellpersistenz über den Neustart.

| Variante | 1024×678 | 480×320 |
|---|---|---|
| EN / Zaplex Dark | X bei Ruhe/Hover kontrastreich, Klick schließt, frische ursprüngliche Shell (004–009). | Dasselbe Ergebnis (015–020); Shellausgabe nach Rückvergrößerung vollständig lesbar. |
| EN / Light | X sichtbar, Hover und Klick wirksam, frische ursprüngliche Shell (026–030). | Dasselbe Ergebnis, nochmals ohne den zuvor beobachteten fremden Tooltip aufgenommen (032–042). |
| DE / Zaplex Dark | X bei Ruhe/Hover sichtbar; nach frischer Beobachtung und expliziter Fensterwahl geschlossen, frische ursprüngliche Shell (007–012). | Ein explizit adressierter Klick schließt; frische ursprüngliche Shell (013–019). |
| DE / Light | X sichtbar, Hover und ein explizit adressierter Klick wirksam, frische ursprüngliche Shell (024–029). | Sichtbarkeit, Hover, Fehler und Optionen in der ersten Sitzung belegt (030–034). Die gezielte Fortsetzung bestätigt zusätzlich sichtbares X vor/mit Hover, tatsächliches Schließen und frische ursprüngliche Shell (012–015 des Folgelaufs). |

Die echte Codex-Fehlermeldung ist in allen vier Sprach-/Themevarianten sowohl
normal als auch am Minimum vollständig lesbar beziehungsweise vertikal erreichbar.
Vier Claude-Modelloptionen und fünf Effort-Stufen bleiben erreichbar. In EN/Light
bei normaler Größe wurde zusätzlich tatsächlich auf das gesperrte Starten geklickt;
Vorher-/Nachher-Aufnahmen sind bytegleich (028/029). Das ist kein erfolgreicher
angemeldeter Agentenstart.

40 englische und 34 deutsche Originale wurden einzeln betrachtet. Alle 50 lokal
gesicherten englischen beziehungsweise 38 deutschen Aufnahmen wurden zusätzlich
bytegenau gegen das jeweilige abgeschlossene Workflow-Artefakt geprüft. Die zweite
Zahl bezeichnet keine zusätzliche visuelle Abnahme.
[EN-Fallurteile und Einzelhashes](evidence/candidate-150-native/41034730295.json),
[DE-Fallurteile und Einzelhashes](evidence/candidate-150-native/41038876716.json).

Die deutsche Sitzung lief vor dem letzten X-Klick am Minimum ab; abgelehnte
Eingaben zählen nicht als ausgeführt. Die kurze
[Fortsetzung 41051951751](https://github.com/byte5ai/zaplex/actions/runs/41051951751)
verwendet denselben unveränderten 1.2.50-DMG/Quellstand, stellt Deutsch und Light
erneut über die echte UI ein und erfasst nach dem App-Neustart eine neue Baseline.
Ein explizit ans aktuelle Fenster gerichteter Klick auf das zuvor sichtbar
angehoverte X schließt nur die Karte. Die anschließend frisch ausgeführte Ausgabe
bestätigt dieselbe Shell-PID, denselben nicht exportierten Marker und dasselbe CWD.
15 weitere Originale wurden einzeln angesehen; alle 18 lokal gesicherten PNGs
stimmen bytegenau mit dem Workflow-Artefakt überein. Der Lauf wurde unmittelbar
danach ausdrücklich beendet, ohne Ablauf des Interaktionsfensters.
[Fortsetzungsnachweis](evidence/candidate-150-native/41051951751.json).

Damit ist #743 an der Agentenkarte in allen acht festgelegten Kombinationen aus
Sprache, Theme und Größe nativ nachgeprüft. Die drei Sitzungen umfassen zusammen
89 einzeln betrachtete Originale. Das ist keine pauschale Abnahme aller gemeinsamen
Modale oder der vollständigen Produktmatrix.

## Zusätzlicher nativer Befund: konkurrierende Kürzel

Cmd+Shift+O öffnet in EN/Dark den nativen Repository-Dateidialog statt der
vorgesehenen Inbox. Die doppelte Standardbelegung ist im Quellcode bestätigt;
Escape schließt den Dialog, ohne ein Repository auszuwählen oder anzulegen.
Originalaufnahme 012 der englischen Sitzung und Hash stehen im EN-Nachweis.
Der Fehler wird als [#746](https://github.com/byte5ai/zaplex/issues/746) verfolgt;
die Korrektur in [PR #747](https://github.com/byte5ai/zaplex/pull/747) verwendet
für die Inbox unter macOS Ctrl+Cmd+O und erhält den Repository-Einstieg.
Der [lokale Regressionstest](evidence/candidate-150-native/inbox-shortcut-local.json)
scheitert mit der alten Standardbelegung und besteht nach der Korrektur; Test und
Repository-Resolver sind in beiden Läufen bytegleich. Compile, regulärer
Binary-Clippy, Phase A und Workflow-Prüfung bestehen ebenfalls.
Die native Abnahme benötigt das korrigierte DMG. Die Inbox ist auf 1.2.50
kein bestandener zweiter Aufrufer der gemeinsamen Modal-Factory.

## Statische native Fixture-Ansichten

44 verschiedene Originaldateien des aktuellen Batches wurden einzeln angesehen:
drei Agenten- und 41 Workspace-Ansichten. Quelle und Einzelhashes sind im
[Fixture-Nachweis](evidence/candidate-150-native/fixtures.json) gebunden.

| Bereich | Beobachtung und Grenze |
|---|---|
| Agentenansicht | Sichtbare Aktionen und Recovery in drei simulierten Zuständen. Kein echtes Abo und kein Live-Provider-Pass. |
| Verbindungen | Vollständiger langer Host-Tooltip bei Sidebarbreiten 250/320/480 in Hell/Dunkel. Sessionnamen bleiben gekürzt; vollständige Identitätsinteraktion nicht bewiesen. |
| Leerzustände | Ordner- und Plus-Anleitung in DE/EN, Hell/Dunkel und allen drei Sidebarbreiten sichtbar. |
| Favoriten-Flyout | Elternzeile und Flyout sichtbar; Kürzungen und Tooltipüberlagerung ergeben keinen vollständigen Tastatur-/Identitätspass. |
| Dateimanager | DE/EN bei 1180/760: acht Aktionen in zwei/vier Reihen, Fokus, Ordnerdialog-Caret, voller Dateiname per Tooltip und erhaltener Terminalentwurf. Kein neuer Transfer-/Dateioperationspass. |
| Pane-Anordnung | Vorher-/Nachher zeigen erhaltenen Entwurf und eigenen Tabtitel nach Umordnung. Keine kontinuierliche Bewegungs-/Videoabnahme. |

## Eingabegrenzen und Bewertung

In EN wurden veraltete Koordinaten nach Kartenverschiebung beziehungsweise
Fensterverkleinerung vor weiterer Eingabe korrigiert; der Rückleseschutz verhinderte
Return in der falschen Oberfläche. Ein standardmäßiger Screenshot wählte das
inaktive Hauptfenster, während die explizite Fensterwahl den nativen Dateidialog
zeigte. Diese Operatorfälle werden nicht als Produktfehler gezählt.

In DE/Dark normal blieb ein erster Klick ohne bestätigte Wirkung; der nach
frischer Aufnahme explizit ans richtige Fenster gerichtete Klick schloss die Karte.
Die Ursache des ersten Versuchs ist ungeklärt. Daraus folgt keine allgemeine
Behauptung zuverlässiger Ein-Klick-Bedienung für diesen Versuch.

Nach einem Themewechsel blieb in EN ein fremder „Close panel“-Tooltip sichtbar.
Er wurde durch einen echten Connections-Hover ersetzt und verschwand danach.
Die kontrollierte deutsche Folge mit expliziter Mausbewegung reproduzierte das
nicht. Produktursache und Beitrag der Eingabefolge sind damit ungeklärt.

Die fehlenden isolierten Abo-Testprofile begrenzen weiterhin echte verwaltete
Agent-/Agenten-Worktree-Starts und gefüllte Kontenansichten. Unangemeldete CLI-Fehler,
API-Testprofile und synthetische Fixtures ersetzen diesen Nachweis nicht.
Die gesamte Prüfmatrix, weitere Remotevarianten und Reduced Motion sind durch
diesen gezielten Kandidatenbericht nicht vollständig abgenommen.
