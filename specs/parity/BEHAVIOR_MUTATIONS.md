# Ausführbare Verhaltensregressionen für GH-169

Die Matrix-Selbsttests verändern ungültige Deklarationen und synthetische Nachweise. Sie prüfen
nur den Prüfmechanismus. Ein solcher grüner Selbsttest belegt nicht, dass Rust-Tests eine echte
Produktregression erkennen.

Der vollständige, separat freizugebende CI-Audit führt deshalb nach den unveränderten Cargo-Suites
zusätzlich `script/cockpit-parity-audit run-mutations` aus. Dieser Befehl kompiliert und führt
Rust-Tests aus; er gehört **nicht** zur buildfreien Prüfung.

| Dimension | Absichtlicher Fehler im Produktionscode | Unveränderter Verhaltenstest |
| --- | --- | --- |
| Provider | Auswahl des falschen Providers beim Routing auf das freieste Konto | `routing::tests::filters_by_provider` |
| Host | Verschiedene stabile Host-IDs gleicher Länge erhalten denselben Schlüssel | `conductor::tests::host_ident_keys_by_stable_identity_not_label` |
| Status | Abgeschlossener Codex-Turn wird als laufend klassifiziert | `codex_sessions::tests::completed_turn_is_waiting_with_model_effort_ctx` |

Jeder Fall läuft in einem temporären, abgetrennten Checkout der aufgezeichneten Zaplex-SHA:

1. Der genaue bestehende Test muss mit unverändertem Code erfolgreich laufen: ein Test ausgeführt,
   keiner ignoriert. Der Filter verwendet den vollständigen Modulnamen und `--exact`.
2. Genau eine überprüfte Stelle im Produktionscode wird verändert. Testcode bleibt unverändert.
3. Derselbe Test muss mit einer Assertionsverletzung scheitern: genau ein fehlgeschlagener Test,
   Exit-Code 101. Compilerfehler, Zeitüberschreitung, Prozessfehler, andere Tests oder ein leerer
   Filter gelten ausdrücklich nicht als erkannte Regression.
4. Die Quelldatei wird auch bei einem Ausführungsfehler wiederhergestellt. Der temporäre Checkout
   wird entfernt; der aufrufende Arbeitsbaum wird nicht verändert.

Der temporäre Checkout verwendet über einen absoluten `CARGO_TARGET_DIR` denselben Cargo-Cache
wie die unveränderten Suites. Ein bereits gesetzter relativer Wert wird gegen den ursprünglichen
Arbeitsbaum aufgelöst. Die Umgebung des aufrufenden Prozesses wird nicht verändert.

Der Bericht `source-mutations.json` enthält Zaplex-SHA, Zeitpunkte, Testnamen, ursprüngliche
Quelldatei-Hashes, Befehle, Prozessresultate und getrennte Logpfade für Original und Mutation.
Die Zusammenführung liest die ursprünglichen Quellbytes direkt aus der festgehaltenen Git-SHA,
vergleicht ihre Hashes und die exakten Befehle, verlangt beide archivierten Logdateien samt
übereinstimmendem Hash und wertet deren tatsächliche Testausgaben erneut aus. Fehlende oder
veränderte Belege können nicht durch einen behaupteten `pass` ersetzt werden.
Alle drei Fälle müssen bestehen und zur aktuellen Audit-SHA gehören. Fehlt der Bericht oder bleibt
ein Fall unbewiesen, ist auch der zusammengesetzte automatisierte Audit rot. Logs und Bericht
werden im normalen Audit-Artefakt archiviert.

## Buildfreie Prüfung des Prüfmechanismus

`python3 specs/parity/test_mutation_gate.py` prüft das Auswerten der Testausgaben und die
Orchestrierung mit einem simulierten Runner: ungültige Kompilierung, leere/falsche Testfilter,
Zeitüberschreitung, fehlgeschlagener Originaltest, Wiederherstellung nach Ausnahme, eindeutige
Quellanker, absolute Cache-Pfade, unveränderte Umgebung sowie SHA-/Befehls-/Log-Bindung. Der bestehende `self-test` führt diese Prüfung ebenfalls aus.
Diese Python-Prüfung startet kein Cargo und ist kein Nachweis für tatsächlich erkannte
Rust-Regressionsmutationen.

Ein grüner automatisierter Audit ersetzt weiterhin nicht den echten Zwei-Host-Smoke-Test aus
`COCKPIT_RUNTIME_SMOKE.md`. GH-160 ist erst mit verlinktem vollständigem Audit und erforderlicher
nativer Abnahme verifiziert.
