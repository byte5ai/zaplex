# Native Sichtprüfung des Kandidaten 1.2.27

Der [macOS-Job der Fortsetzung 40048210472](https://github.com/byte5ai/zaplex/actions/runs/40048210472/job/120734924721)
prüft Quellcommit `70d6b1f8fbf248cd51a0c57a227026d2ba9a0270`.
Compilecheck, fünf Maus-Routingtests, ein Agentenfall, sechs Workspace-Fälle und
der zusätzliche Header-Einstieg bestehen jeweils ohne Fehler oder ignorierte
Fälle. Am 24.10.2026 wurden alle **44 Originalaufnahmen** einzeln betrachtet;
die Workspace-Digests stimmen mit dem quellgebundenen CI-Beleg überein.
Das [Manifest](evidence/candidate-127-native/manifest.json) nennt Artefakte,
Originalpfade, tatsächliche Bildgrößen und SHA-256 jedes Bildes. Rohbilder mit
temporären Laufzeitpfaden bleiben privat archiviert.

Prüfmaßstab bleiben die [Matrix](MATRIX.md) und die bestehenden
[UI-Referenzen](../../docs/ui/README.md). Die Aufnahmen sind keine neue Sollvorgabe.

| Bereich | Varianten | Beobachtung |
|---|---|---|
| Agentenansicht | 3 Bilder, EN/dunkel, 1180×640 und 540×670 | Freigabeaktionen bleiben getrennt. Beendete Sitzung bietet neue Unterhaltung und Shell-Rückweg. Recovery-Aktionen einschließlich Agent/Konto/Modell/Verzeichnis sind in der schmalen Ansicht sichtbar. Lange Identitäten und Meldungen bleiben gekürzt; volle Tooltips nicht geprüft. |
| Verbindungen und Favoriten-Flyout | 7 Bilder, Hell/Dunkel, Sidebar 250/320/480; Flyout bei 900×640 | Host-/Sitzungshierarchie, feste Aktionsplätze und umbrochene Metadaten bleiben erkennbar. Das Flyout öffnet am rechten Rand nach links und zeigt drei Folgeaktionen. Lange Eltern- und Menülabels bleiben verkürzt; keine vollständige Menüabnahme. |
| Leere Verbindungen/Favoriten | 16 Bilder, DE/EN, Hell/Dunkel, Sidebar 250/320/480 | Vollständiger Hinweis auf Ordner und Plus auch bei 250 Punkten. Favoritenhinweis enthält die Sternaktion und überlagert die Folgeaktionen nicht. |
| Zwei Dateimanager | 14 Bilder, DE/EN, Hell/Dunkel, 1180×640 und 760×640 | Acht beschriftete Aktionen in zwei/vier Reihen; aktive Pane und Cursor unterscheidbar. Filterhinweise bleiben nach Dark→Light in beiden Panes sichtbar. Voller Dateiname im Tooltip, Ordnerdialog in der fokussierten Pane, Rückkehr zu beiden Terminals mit erhaltenem Entwurf. |
| Inaktiver Header-Einstieg | 2 Bilder, EN/dunkel, 1024×640 | Vorher rechter Dateimanager aktiv, linkes Terminal inaktiv. Danach zeigt der linke Dateimanager seine eigene Datei und den Unterordner; rechte Nachbardatei und Cursor bleiben erhalten. Beide Seiten sind lokal. |
| Pane-Anordnung | 2 Bilder, EN/dunkel, 1180×640 | Unterschiedliche Identitätssuffixe trotz gleichen CWD; nach Drag vertikale Anordnung mit erhaltenem Entwurf und explizitem Tabtitel. |

## Nachprüfung der Korrekturen

Der konkrete Platzhalterbefund aus [#644](https://github.com/byte5ai/zaplex/issues/644)
ist in normaler und schmaler Ansicht für DE und EN nativ korrigiert: Der vorher
nach Dark→Light verschwundene Filterhinweis ist lesbar. Die Bilder zeigen weder
eingegebenen Filtertext noch einen während des Themewechsels offenen Rename-
oder Ordnerentwurf; diese Varianten bleiben für die installierte App vorgesehen.

Der native Header-Fall bestätigt für die lokale Variante von
[#645](https://github.com/byte5ai/zaplex/issues/645) die richtige Zielpane und den
Erhalt des Nachbarn. Das ersetzt nicht den ursprünglichen Ablauf mit einer
inaktiven echten Remote-Pane. Die Alias-Transferkorrektur
[#646](https://github.com/byte5ai/zaplex/issues/646) ist durch diese Bilder nicht
nachgewiesen; dafür sind reale Transfers mit unabhängigem Inhaltsvergleich nötig.

## Herkunft der übrigen Gates und Grenzen

[40045250146](https://github.com/byte5ai/zaplex/actions/runs/40045250146) auf
`b5e996a524809ec13c7642e6b37c4d892caf7762` besteht Sicherheitsprüfung und beide
Linux-Gates; macOS scheiterte dort vor dem Compiler am Rust-Suchpfad. Die oben
genannte Fortsetzung ergänzt macOS sowie **4415 bestandene Pakettests bei 13
übersprungenen Tests** und **4 bestandene Dokumentationstests bei 4 ignorierten**.
Der Rust-Setup-Fix ändert keine Produkt-/Cargo-Eingaben.

Der grüne Gesamtstatus von 40048210472 ist keine vollständige Batch-Abnahme:
Live-SFTP und damit der DMG-Bau wurden durch einen Ablaufdefekt übersprungen.
[PR #653](https://github.com/byte5ai/zaplex/pull/653) korrigiert die Bedingungen und
ergänzt einen verpflichtenden Abschlusscheck. Die
[Fortsetzung 40049854378](https://github.com/byte5ai/zaplex/actions/runs/40049854378)
auf `b270ee372f8548b83363916dc8fbddc1ee7b5884` führt nur die fehlenden Stufen aus;
die Live-Vorprüfung hat beide Ursprungsläufe und die unveränderten Prüfeingaben
bestätigt. Zum Stand dieser Sichtprüfung läuft diese Fortsetzung noch.

Die Integrationsfälle verwenden konstruierte Host-/Agentenzustände und echte
isolierte lokale PTYs/Dateien. Sie belegen keine echte Provider-Anmeldung,
Abonnement-/Kontingent-Routen, Remote-Persistenz, vollständige Tooltips, OS Reduced
Motion oder Restart-Restore. Eine signierte installierte DMG, alle Pflichtfälle
der GUI-Suite und die echte Remote-Exploration sind hier noch nicht nachgewiesen.
Ein Release oder Tag wurde nicht erstellt.
