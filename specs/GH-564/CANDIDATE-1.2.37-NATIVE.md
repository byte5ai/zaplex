# Native Abnahme des Kandidaten 1.2.37

Auftrag: [#564](https://github.com/byte5ai/zaplex/issues/564), übergreifend
[#562](https://github.com/byte5ai/zaplex/issues/562).

App- und Testquellstand: `a0705b38a92c8fe2877d43492f5190d372a053de`,
PR [#705](https://github.com/byte5ai/zaplex/pull/705). Dieser Stand korrigiert
die deutschen Einstellungswerte (#701), Kontext-Chip-Farben nach Themewechsel
(#702) und die Farben des Ausschlusslisten-Editors (#704).

## Nativer Fixture-Nachweis

Der [CI-Batch 40140544738](https://github.com/byte5ai/zaplex/actions/runs/40140544738)
hat das macOS-Compile-Gate samt nativen Fixture-Prüfungen bestanden.
Das abgeschlossene Jobprotokoll bestätigt sechs Plattformtests, einen
Agenten-Layouttest, sechs Workspace-Fälle und einen Header-Einstiegstest,
jeweils ohne Fehler oder übersprungene Tests.
Der vollständige Batch und die installierte DMG-Abnahme sind zum Zeitpunkt
dieses Abschnitts noch nicht abgeschlossen.

Alle **44 Originalbilder** wurden einzeln betrachtet. Die heruntergeladenen
ZIP-Hashes stimmen mit den GitHub-Artefakt-Digests überein; zusätzlich stimmen
alle 41 Workspace-Bildhashes mit dem vom Lauf erzeugten Manifest überein.
Das [Bildverzeichnis](evidence/candidate-137-native/manifest.json) enthält
Artefakt-IDs, SHA-256, Bildmaße und Beobachtungen pro Original.

| Umfang | Beobachtung und Aussagegrenze |
|---|---|
| 3 Agentenbilder | Freigabe-, beendeter und wiederherstellbarer Zustand zeigen erreichbare Aktionen. Lange Identitätswerte sind gekürzt. Keine echte Provider-Sitzung. |
| 7 Verbindungsbilder | Gefüllte Listen bei 250/320/480 Punkten in Hell/Dunkel zeigen feste Aktionsplätze und vollständigen Host-Tooltip; ein Favoriten-Flyout ist erfasst. Deterministische Testdaten, kein echter Verbindungsaufbau. |
| 16 Leerzustände | Deutsche und englische Verbindungs- und Favoritenanleitungen sind in den dargestellten Breiten/Themes vollständig lesbar. |
| 14 Dateimanagerbilder | DE/EN, Hell/Dunkel, 1180/760 Punkte: alle acht Aktionen in zwei beziehungsweise vier Reihen, unterscheidbare aktive/inaktive Pane, vollständiger Dateiname im Tooltip und Neuer-Ordner-Dialog in der fokussierten Pane. Nach Rückkehr ist der Terminalentwurf sichtbar. Kein Nachweis einer einreihigen Leiste oder vollständiger Bedienvarianten. |
| 2 Pane-Bilder | Vorher/Nachher zeigen dieselben beschrifteten Panes nebeneinander beziehungsweise übereinander und den erhaltenen Entwurf. Kein Bewegungsbeleg. |
| 2 Header-Bilder | Der Einstieg öffnet das ursprüngliche linke Verzeichnis neben dem unveränderten Geschwisterverzeichnis; der Fokus wechselt nach links. |

In diesen dargestellten Zuständen wurde keine neue sichtbare Regression
festgestellt. Die Bilder enthalten **nicht** die gezielte native Nachprüfung
der drei Einstellungsfehler. Die bestehende [Produktmatrix](MATRIX.md) bleibt
für nicht beobachtete Kombinationen offen.

## Vorab festgelegte Nachprüfung am installierten DMG

- DE/EN nach tatsächlicher Sprachauswahl und vollständigem App-Neustart:
  Keine/None, Standard/Default, Agenten/Agents und Links/Rechts bzw. Left/Right;
  physische Tastennamen bleiben erhalten.
- Dark→Light→Dark: Kontext-Chips, eingegebener Ausschlusslistenwert und
  Platzhalter bleiben lesbar; Auswahl, Eingabe und gewählte Toolbar-Elemente
  bleiben erhalten. Relevante deaktivierte Zustände getrennt prüfen.
- Geänderte Kontrollen und Menüeinträge bei 480×320 erreichen; anschließend
  frische Shell-PID, nicht exportierten Marker und CWD nachweisen.
- Weitere direkte Exploration der Dateimanager-Aktionsleiste: eine, zwei und
  vier Reihen, fokussierte Pane und unveränderte künstliche Dateiinhalte.

Je Interaktion werden Zustand vorher/nachher, Aktion, Erwartung und Beobachtung
festgehalten. Standbilder gelten nur als Zustandsbelege. Es erfolgt keine
Release-Freigabe aus diesen Teilnachweisen.
