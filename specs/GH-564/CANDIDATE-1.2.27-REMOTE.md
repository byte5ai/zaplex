# Installierte Remote-Abnahme des Kandidaten 1.2.27

Der [Batch 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378)
ist vollständig bestanden: Live-SFTP, signierte/notarisierte DMG, **15 von 15
Pflichtfällen der installierten GUI-Suite ohne Fehler oder Skip** und der
verpflichtende Abschlusscheck. Die unveränderten früheren Gates sind über die
im [nativen Bericht](CANDIDATE-1.2.27-NATIVE.md) beschriebene Quellenprüfung belegt.
Die GUI-Receipt wurde zusätzlich gegen Quellstand, Version und heruntergeladene
DMG unabhängig validiert. Es wurde kein Release oder Tag erstellt.

Die anschließende [Remote-Exploration 40054191997](https://github.com/byte5ai/zaplex/actions/runs/40054191997)
nutzt genau diese installierte DMG, EN/dunkel bei 1024×768. Die Erwartungen stehen
vor dem Lauf in der [Matrix](MATRIX.md); nächste Aktionen wurden aus der jeweils
frisch beobachteten Oberfläche gewählt. Der Workflow-Erfolg bescheinigt die
Infrastruktur und das Cleanup; Produktbefunde stammen aus den folgenden Abläufen.

- App-Quelle: `b270ee372f8548b83363916dc8fbddc1ee7b5884`
- GUI-Harness: `45935546251bbaa7ebc01d9b9ead22c5333941e9`
- DMG SHA256: `fcb082266c6b4f61fb8bb0c3d871e9ccafb8c229225e528f288bafb36f800644`
- [Manifest](evidence/candidate-127-remote/manifest.json): 60 private Originalaufnahmen mit Hashes; Rohbilder enthalten Remote-Details und bleiben privat archiviert.

## Beobachtete Abläufe

| Fall | Ergebnis und unabhängiger Nachweis | Aufnahmen |
|---|---|---|
| SSH-Alias importieren und SFTP öffnen | Importierter Alias bleibt erhalten; persistente Remote-Shell verbindet. Vor SFTP-Vertrauen wurde der angezeigte ECDSA-Fingerabdruck über die bestehende SSH-Verbindung abgeglichen. | 015–016, 029, 031–032 |
| Inaktiver Remote-Header neben aktivem lokalem Dateimanager | Ordnerknopf öffnet den eigenen Remote-Host und dessen CWD. Lokales Alias-Ziel und Cursor im Nachbarn bleiben erhalten. | 023–024, 032 |
| Datei und verschachtelten Ordner Remote→Lokal kopieren | Ziel ist ein lokaler Symlink-Pfad. Beide Inhalte stimmen per unabhängigem SHA256-Vergleich mit dem vorher erfassten Remote-Original überein. Remote-Quellen bleiben erhalten. Ziel wird ohne Fokuswechsel oder Refresh angezeigt. | 018, 035, 037, 043, 050 |
| Bestehenden Ordner erneut kopieren | Sichtbarer Skip mit Hinweis auf bestehenden Inhalt; unveränderte Prüfsummen. Das ist kein Nachweis für geänderte Konfliktinhalte oder Dialogabbruch. | 038, 043 |
| Separate Datei Lokal→Remote verschieben | Quelle und Ziel sind sichtbar zugeordnet. Remote-Inhalt entspricht dem unabhängig berechneten Erwartungshash; lokale Quelle fehlt. Beide Ansichten aktualisieren sich automatisch. | 045–046, 050 |
| F10 zurück zu beiden Shells | Lokale PID, nicht exportierter Marker und logischer Alias-Pfad bleiben erhalten; ebenso Remote-PID, Marker und CWD. | 018, 022, 050, 058 |

Damit sind die ursprüngliche Remote-Variante des Headerfehlers
[#645](https://github.com/byte5ai/zaplex/issues/645), reale Alias-Transfers nach
[#646](https://github.com/byte5ai/zaplex/issues/646), die Alias-SFTP-Verbindung nach
[#639](https://github.com/byte5ai/zaplex/issues/639) und die automatische
Zielaktualisierung nach [#640](https://github.com/byte5ai/zaplex/issues/640)
in diesen konkreten Varianten am installierten Produkt nachgewiesen.

## Grenzen und Fortsetzung

Die 15 Pflichtfälle und diese Exploration ersetzen keine vollständige
Produktabnahme. Nicht ausgeführt wurden hier ein laufender Agent bei
Transportabbruch, echte Abonnement-/Kontorouten, eingegebene Filter-/Rename-/
Ordnerentwürfe beim Theme-/Sprachwechsel, unterbrochene Moves, Mehrfachauswahl
und weitere Matrixvarianten. Installiert sind Codex 0.163.0 und Claude Code
2.1.306; Versions-/Hilfsausgabe allein belegt keinen Agentenlauf. Die verfügbaren
API-Testprofile sind kein Abonnementnachweis.

Das begrenzte GUI-Fenster wurde ausdrücklich beendet. Der Cleanup-Beleg meldet
`owned-processes-stopped-and-data-trashed`. Keine Produktdaten außerhalb der
isolierten Testumgebung wurden verändert.
