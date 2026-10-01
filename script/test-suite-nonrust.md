# Tatsächliche Non-Rust-Ausführung für #472

`test-suite-nonrust-plan.json` legt den buildfreien Teil der Messsuite fest:
alle Testkommandos der Composite-Action `static-pr-checks`, die buildfreien
PR-Vorprüfungen und die Regressionen dieses Messadapters. Jede Suite läuft genau
einmal unter Linux; wiederholte PR-Jobs vergrößern den Nenner nicht. Native
plattformgebundene Prüfungen bleiben separate Abnahmegates. Der Plan ersetzt
keinen bestehenden PR-Check.

Die Hashes der beiden ursprünglichen Workflow-/Actionquellen binden das Inventar.
Ändert sich eine davon, muss der Plan geprüft und aktualisiert werden. Unbekannte
oder fehlende Kommandos dürfen nicht durch ein unverändertes altes Inventar
verschwinden. Hilfsprogramme zur Installation sowie der CI-Änderungsdetektor sind
explizit keine Tests. Reine Quell-/Policyguards laufen weiter und erhalten einen
Prozessstatus, aber keine erfundene Testfallzahl.

## Fallgrenzen

| Suite | Bereits vorhandene Grenze |
| --- | --- |
| unittest-Suiten | Geladene Test-ID und tatsächliches Result-Ereignis; Discovery dient nur dem Vollständigkeitsabgleich. Skips zählen nicht als erfolgreiche Ausführung. Subtests bleiben beim Framework-Elternfall. |
| Release-Tag-Eingaben | Je bisherigem akzeptiertem oder abgewiesenem Tag. |
| Docker-Selbsttest | Je positiver/negativer Fixturedatei. |
| Release-Selbsttest | Je bereits vorhandenem Eingabe-/Erwartungspaar; lesbare IDs nennen den bestehenden Vertrag. Beide Nachfolgejob-Varianten bleiben getrennt. |
| Lizenz-Selbsttest | Drei getrennt erzeugte Konfigurationspaare. |
| Clippy-Gate-Selbsttest | Sieben vorhandene Dependency-Outcome-/Build-Flag-Tupel. Die übrigen Quellprüfungen sind Guards. |
| Versionsupdate | Die vorhandene erfolgreiche Aktualisierung und der anschließende abgewiesene Prerelease; einzelne Zieldateien sind keine zusätzlichen Fälle. |
| Release-Readiness | Bereits benannte Negativfixtures, beide positiven Matrixphasen und die jeweiligen vorhandenen Duplikat-/Marker-Eingaben. |
| SSH-tmux | Drei bestehende HOME-Szenarien, Fish/Bash-Körpergleichheit sowie je existierender Fehlerhelper-Körper. Syntax-/Quellguards bleiben Guards. Fish ist für die vollständige CI-Messung Pflicht. |
| Rust-Evidence-/Projektionsfixtures | Die bestehenden konkreten Artefakt-, Quellen-, Mutations- und Eingabeszenarien. Mehrere Assertions am selben Szenario bleiben zusammen; Schleifenparameter bleiben getrennt. |
| Cockpit-Selbsttest | Bereits erzeugte dynamische `cases` plus die tatsächlich ausgeführten unittest-Kinder. `source-mutation-harness` ist nur Container und zählt nicht nochmals. Die Prüfung, dass fehlende Runtime-Evidenz `not-run` ergibt, ist eine erfolgreich ausgeführte Negativfixture und kein ausgeführter nativer Audit. |

Die optionalen Marker ändern weder Eingaben noch Assertions. Ohne Messumgebung
schreiben sie nichts. Sie melden erst ein abgeschlossenes vorhandenes Szenario;
Prozessfehler, fehlende erwartete IDs oder doppelte Ereignisse machen die gesamte
Messung ungültig. Die Fallliste im Plan ist eine Vollständigkeitsanforderung,
niemals der Ausführungszähler. Gezählt werden ausschließlich überprüfte Ereignisse.

## Erhebung und Zusammenführung

Der `coverage`-Dispatch startet einen separaten buildfreien Linux-Job. Er verwendet
die gepinnten binären Parserpakete, installiert Fish und archiviert auch bei einem
Fehler alle bereits erzeugten Prozesslogs und Ereignisse. Kein Cargo-Aufruf ist Teil
dieses Jobs. `collect` verlangt den tatsächlichen CI-SHA, getrackte unveränderte
Adapter-/Plan-/Helper-/Suitequellen und aktivierte Python-Assertions.

Pro Suite werden Kommando, Exitcode, vollständiges stdout/stderr, Ereignisdatei,
Discovery und gegebenenfalls Cockpit-JSON archiviert. SHA-256 bindet die Originaldateien.
`validate` liest diese erneut, prüft Pfadgrenzen, Quellen, Kommandos und Vollständigkeit
und bildet erst daraus die Testzahlen. Erfasst wird Suite-Walltime; einzelne
Markerfallzeiten werden nicht behauptet. Laufzeiten paralleler CI-Jobs sind nicht
additiv.

Nach Download beider Artefakte desselben CI-Laufs und Versuchs:

```sh
PARSER_PYTHON=/path/to/pinned-parser-environment/bin/python
"$PARSER_PYTHON" script/test-suite-evidence /path/to/rust-artifact \
  --source-root /clean/checkout/of/measured-commit \
  --nonrust /path/to/nonrust-artifact > /tmp/combined-baseline.json
```

Den Kandidaten analog mit `--baseline /tmp/combined-baseline.json` auswerten.
Baseline und Kandidat benötigen denselben Messadapter, Suiteplan, Tool-/Runnerkontext
und nachgewiesene Produktionsprojektion. In dieser Bereinigungsphase bleiben die
Non-Rust-Tests unverändert: ihre Quellhashes, erfolgreichen IDs, Skips und Guards
müssen in beiden echten Ausführungen übereinstimmen. Eine spätere Non-Rust-Bereinigung
benötigt eine eigene geprüfte Änderung dieses Vertrags.

Die Gesamtquote verwendet `(Rust + NonRust)` in beiden Nennern. Ein Rust-only-Bericht
liefert weiter Diagnostik, aber `numeric_thresholds_met: null` und
`measurement_complete: false`. `rust_numeric_thresholds_met` ist ausdrücklich nur
die kleinere Rust-Teilmetrik. Ohne vollständige Belege gibt es keine Gesamtfreigabe.

Die ≤2-Prozentpunkte-Grenze bleibt auf die instrumentierten Linux-Rust-
Produktionsquellen bezogen. Unveränderte Scriptprüfungen liefern keine zusätzliche
Rust-Coverage. Zahlen allein ersetzen auch weiterhin keine inhaltliche Begründung,
warum entfernte Tests redundant sind. Die Einführung dieses Adapters ist selbst
keine Suiteverkleinerung.
