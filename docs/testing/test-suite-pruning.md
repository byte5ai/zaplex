# Testsuite verkleinern: Messung und Auswahl (#472)

Die Abnahme verlangt mindestens 20 % weniger wenig nützliche Tests, ausdrücklich
auch Rust, bei höchstens zwei Prozentpunkten Verlust der **Produktionscode**-
Zeilenabdeckung. Annotationen zu zählen oder Fälle in eine Schleife zu verschieben
ist dafür kein Nachweis. Fehlgeschlagene, ausgefilterte oder ignorierte Tests zählen
nicht als eingesparte erfolgreiche Ausführung.

## Vorbereitete Änderungen

Die Reparaturen aus `772cf3911` sind übernommen: `GitignoreBuilder::add_line`
erhält eine Referenz; `MultilineString` wird vor `matches` als `str` gelesen; die
`VecDeque`-Erwartungswerte sind eindeutig. Die vorhandenen Assertions bleiben erhalten.

| Auswahl | Quellort / erhaltene Prüfung | Entscheidung |
| --- | --- | --- |
| `test_create_version` | `crates/warp_util/src/content_version_test.rs`: `test_versions_equal` und `test_versions_not_equal` rufen denselben `ContentVersion::new()` auf und prüfen zusätzlich Gleichheit bzw. Eindeutigkeit. | Einen reinen Konstruktoraufruf ohne Assertion entfernt. Voraussichtlich eine Rust-Ausführung weniger; noch nicht gemessen. |
| Fünf reine Enum-Konstruktionstests | `app/src/sftp_manager/browser_unit_tests.rs`: `test_action_{cancel_transfer,confirm_move,set_search_filter,clear_search_filter,download_save_as}` konstruierten eine Variante und matchten unmittelbar genau diese Variante; kein Action-Handler wurde ausgeführt. | Entfernt: ausschließlich Rust-Sprachmechanik, keine Produktwirkung. Echte Handler-, Transfer- und Navigationsprüfungen bleiben. |
| Zwei schwächere Permission-Tests | `app/src/sftp_manager/sftp_ops_tests.rs`: `test_bool_to_rwx_length` und `test_bool_to_rwx_valid_chars`. | Entfernt: die acht erhaltenen exakten Ergebnisprüfungen decken alle acht möglichen Eingaben ab und implizieren Länge und erlaubte Zeichen. |
| Doppelte Cancelled-Ausgabe | `app/src/sftp_manager/sftp_ops_tests.rs`: `test_sftp_ops_error_cancelled_consistent`. | Entfernt: `test_sftp_ops_error_display_cancelled` prüft denselben exakten Ausgabestring; der zusätzliche Selbstvergleich prüfte keinen weiteren Pfad. |
| Sechs Worktree-Sidecar-Platzhalter | `app/src/workspace/view_test.rs`: hover, pointer entry, close via selection, search Enter, search navigation/Escape, hide linked worktrees. | Entfernt. Alle waren explizit ignoriert und enthielten ausschließlich `unimplemented!` für das abgeschaffte `PersistedWorkspace`; keine Assertions gehen verloren. **Null eingesparte reguläre Ausführungen.** |
| Zwei App-ID-Tests | `app/src/app_id_test.rs` und `crates/warp_core/src/app_id_test.rs`. | Identischer Inhalt, aber nur die Core-Datei ist im statisch gefundenen Modulbaum referenziert. Keine getestete Reduktion beansprucht; Dateien unverändert. |
| `test_priority_normalization` | `crates/warp_completer/src/completer/suggest/priority/priority_test.rs` und `crates/warp_completer/src/signatures/v2/signatures_test.rs`. | **Behalten.** Identischer Testkörper prüft zwei verschiedene `Priority`-Implementierungen. |
| Fehlende Plugin-Datei / Versionsdatei | `app/src/terminal/cli_agent_sessions/plugin_manager/{claude,gemini}_tests.rs`. | **Behalten.** Ähnliche Testkörper prüfen unterschiedliche Provider, Dateipfade und Parser. |
| Kurze SFTP- und Session-Tests | `app/src/sftp_manager/*_tests.rs`, `crates/zaplex_remote_session/src/types_tests.rs`. | Nicht allein wegen Kürze entfernen: Identität, Konflikte und Verbindungszustände sind eigenständige Verträge. |

Zusätzlich beobachtet `clicking_panel_background_does_not_toggle_transfer_panel`
jetzt tatsächlich die Toggle-Aktion; die bisher allein geprüfte Transferanzahl
blieb auch bei einem unerwünschten Toggle unverändert. Der Hintergrund-Recovery-Test
akzeptiert den legitimen Fall, dass sein Worker schon vor der ersten Beobachtung
fertig ist, und verlangt weiterhin den endgültigen erfolgreichen Zustand.
Die acht zusätzlichen Entfernungen sind statisch begründet; tatsächliche eingesparte
Ausführungen, Laufzeit und Coverage bleiben bis zur Messung unbekannt.

Diese Auswahl ist eine konkrete erste Bereinigung, **keine 20-%-Erfüllung**. Weitere
Entfernungen benötigen beobachtete Laufzeiten und Abdeckung sowie eine Zuordnung
jeder entfallenden Assertion zu einem erhaltenen Test. Auch ein langsamer Test
kann einzigartig und deshalb unverzichtbar sein.

## Vorbereitete CI-Messung (nur nach Build-Freigabe)

`Test (manual)` besitzt den standardmäßig ausgeschalteten Eingang `coverage`.
Er verwendet die vollständige Rust-Workspace-Auswahl mit Ausnahme von
`command-signatures-v2` und dem vorhandenen Nextest-Profil `ci`. Package und Filter
steuern weiterhin nur den normalen Pakettestlauf. Der Coverage-Lauf ist ausdrücklich
unselektiert und erzeugt:

- `provenance.json`: Commit, Run-URL/-Versuch, Runner, Rust-Version, Toolversionen,
  Hashes aller Workspace-Cargo-Manifeste, Cargo-/Nextest-Konfiguration und Toolchain,
  tatsächlicher Aufruf, relevante Compilerumgebung und ursprünglicher Checkout-Pfad;
- `junit.xml`: tatsächlich ausgeführte Tests und Laufzeiten;
- `rust-lcov.info`: vollständige LLVM-Zeilendaten zur anschließenden Produktionsprojektion;
- `source-projection-preflight.json`: AST-Diagnose vor dem ersten Cargo-Schritt;
- `test-outcome.txt` und bei erfolgreicher, vollständiger Messung `summary.json`.

Auch bei Fehlern werden verfügbare Diagnoseartefakte hochgeladen. Der fehlgeschlagene
Testschritt bleibt fehlgeschlagen: Es gibt kein `--ignore-run-fail`. Ein abgebrochener
Compile-Schritt liefert keine gültige Baseline. Der bekannte
[erste Baseline-Lauf](https://github.com/byte5ai/zaplex/actions/runs/36125552465)
endete vor der Testausführung und besitzt keine Artefakte.

Die AST-Projektion verwendet `tree-sitter==0.25.2` und `tree-sitter-rust==0.24.2`.
`script/test-suite-evidence-requirements.txt` bindet deren Linux-Binärpakete per Hash;
`--only-binary=:all:` verbietet einen Parser-Quellbuild. Installation nur in einem
separaten virtuellen Environment:

```sh
PARSER_VENV=/tmp/zaplex-suite-parser
python3 -m venv "$PARSER_VENV"
"$PARSER_VENV/bin/python" -m pip install --only-binary=:all: --require-hashes \
  -r script/test-suite-evidence-requirements.txt
```

Nach zwei freigegebenen vergleichbaren Läufen können die heruntergeladenen
Artefakte ohne Build ausgewertet werden:

```sh
"$PARSER_VENV/bin/python" script/test-suite-evidence /path/to/baseline-artifact \
  --source-root /clean/checkout/of/baseline-sha > /tmp/suite-baseline.json
"$PARSER_VENV/bin/python" script/test-suite-evidence /path/to/candidate-artifact \
  --source-root /clean/checkout/of/candidate-sha --baseline /tmp/suite-baseline.json
```

Die Auswertung zählt JUnit-Testfälle statt Rust-Annotationen, listet die langsamsten
50 Tests und entfernte/neue Ausführungs-IDs auf und berechnet Laufzeit- sowie
Abdeckungsdifferenzen. Ungültige, fehlgeschlagene, leere oder doppelte Testdaten werden
abgewiesen. JUnit-Gesamtzähler müssen mit den tatsächlich vorhandenen Testfällen
übereinstimmen; ein abgeschnittener Bericht wird nicht als Testsuite-Verkleinerung
akzeptiert. Die gelesenen Quellen müssen zum aufgezeichneten Git-SHA gehören und
unverändert sein. Ein anderer Runner, andere Tools/Configs oder ein veränderter Satz
instrumentierter Produktionszeilen macht den direkten Vergleich ungültig.
Die Projektion entfernt AST-bestimmte Testregionen und Kommentare, nummeriert die
verbleibenden nichtleeren Produktionszeilen und bindet deren vollständigen Inhalt
pro Datei per SHA-256. Reine Testlöschungen dürfen physische Zeilennummern verschieben;
der Vergleich bleibt möglich. Eine Produktionsänderung oder geänderte
Projektionspolicy macht den Vergleich dagegen ungültig.

## Grenzen der Messung und noch erforderliche Abnahme

Die erste Baseline für die größere Bereinigung muss **nach** den Compile-Reparaturen
und dieser ersten Bereinigung erstellt werden. Das verhindert, dass zuvor überhaupt
nicht ausführbare oder ignorierte Tests die Einsparung künstlich erhöhen.

Ein numerisch guter Vergleich allein schließt #472 nicht:

1. Die AST-Policy entfernt `#[cfg(test)]`, annotierte Testfunktionen und die
   explizit als Testinfrastruktur definierten Features `test-util` und
   `integration_tests`, einschließlich verschachtelter und externer Testmodule.
   Normale unbekannte Features werden nicht erfunden. Eine Kombination mit
   unbekannter Produktionszugehörigkeit, testabhängige generierte Traits,
   unexpandierte Testmakros oder eine gemeinsame physische LCOV-Zeile von Test und
   Produktion bleiben unbewiesen. Auch separate Testdateien, `crates/integration`,
   Buildskripte und generierte `target`-Quellen sind explizite Policy-Ausschlüsse.
   Die Quote gilt für instrumentierte Linux-Produktionsquellen dieser Policy,
   nicht für nie kompilierte Plattformzweige.
   Der reale Repository-Vorabscan hat außerdem Grammatikgrenzen des Parsers bei
   Attributen in Rust-Strukturmustern gefunden (unter anderem die großen
   Workspace-/Terminal-Dateien). Das ist kein nachgewiesener Rust-Compilefehler.
   Betroffene Coverage-Zeilen erscheinen unter `unresolved_production_lines`;
   der vollständige Prozentwert bleibt `null`, der Status `unproven`.
   `partial_known_line_coverage_percent` ist ausdrücklich nur eine Teilstatistik.
   Ein solcher Bericht kann die 2-Prozentpunkte-Grenze **nicht** erfüllen; dafür
   müssen die betreffenden AST-/LLVM-Regionen zuerst eindeutig zugeordnet werden.
   Die Vorabdiagnose steht schon vor Cargo bereit, sodass diese Grenze nicht erst
   nach einem teuren Messlauf sichtbar wird.
2. Python- und andere Skripttests müssen mit tatsächlichen CI-Ergebnissen in dieselbe
   Gesamtbilanz aufgenommen werden. Rust-only Nextest ist nicht die Gesamtsuite.
3. Produktionsänderungen, geänderte Features/Filter, übersprungene Tests und
   zusammengelegte Testfälle dürfen keine vermeintliche Einsparung erzeugen.
4. Jede weitere Entfernung braucht eine konkrete Redundanzbegründung, erhaltene
   Assertions und einen vergleichbaren tatsächlichen Kandidatenlauf.

Kein Coverage-Lauf wurde durch diese Vorbereitung ausgelöst. Für die neue
Zusammenstellung sind ausgeführte Testzahl, Zeiteinsparung und Produktionscoverage
noch **unbekannt**; die 20-%-/2-Prozentpunkte-Abnahme steht aus.
