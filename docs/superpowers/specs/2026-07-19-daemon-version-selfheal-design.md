# Daemon-Versions-Selbstheilung (RC-Paket, 2026-07-19)

**Problem (RC-Abnahme):** Ein laufender Alt-Daemon wird vom Proxy versionslos
wiederverwendet (`proxy.rs`: PID-File lesen, `kill(pid,0)`, an dessen Socket
bridgen). Daemon-seitige Fixes wirken dann „nicht gefixt", und weder Client
noch UI können den Skew sehen. User-Auftrag: erkennen **bevor** man es sieht,
selbst heilen — ohne das Persistenz-Versprechen (Sessions überleben) zu
brechen.

**Baustand-Befund (gegen Code verifiziert, nicht geraten):**
- `InitializeResponse.server_version` existiert im Protokoll; der Daemon füllt
  es bereits (`ChannelState::app_version()`, leer für Source-Builds —
  `server_model.rs handle_initialize`).
- Der Client PRÜFT bereits (`manager.rs version_is_compatible`) — aber
  `should_enforce_remote_version_check` schaltet die Prüfung für
  `Channel::Oss` (= zaplex) pauschal AB. Historischer Grund: Source-Builds
  ohne Tag + wiederverwendetes Upstream-Release-Binary → Delete/Reinstall-
  Schleife. Für Release-DMGs (Tag vorhanden, versionierte Binary-Slots) gilt
  der Grund nicht.
- Der Mismatch-Pfad löscht das **Binary auf Platte** („stale binary") — die
  Recovery-Semantik der Upstream-Channels. Bei zaplex-Release-Builds ist das
  On-Disk-Binary per Konstruktion das richtige (Install-Slot und Socket-Name
  keyen auf denselben Tag); stale ist der **laufende Prozess**. Löschen wäre
  das falsche Objekt.
- **Idle-Exit existiert vollständig:** `GRACE_PERIOD` (10 min) bei
  0 Verbindungen + 0 Sessions (`deregister_connection`/`start_grace_timer`),
  plus GC-Sweep alle 5 min (`gc_sessions` + `maybe_arm_grace_after_gc`,
  unconditional pro Tick) mit 24-h-Reap für verlassene Sessions und
  Ring-RAM-Cap. Lücke: die direkten Session-Ende-Pfade
  (`handle_close_session`, `on_session_reader_eof`) armieren den Timer nicht
  selbst — Retirement passiert erst über den nächsten GC-Tick (≤ 5 min
  später). Kein Leak, aber unnötige Latenz.

## Mechanik (drei Teile, alle klein)

### 1. Versionierter Rendezvous-Pfad (der strukturelle Fix)
`server-<tag>.sock` / `server-<tag>.pid` statt `server.sock`/`server.pid` —
EINE gemeinsame Namensquelle `setup::daemon_runtime_filename(ext)`, benutzt
von Proxy (bind/lookup), Daemon (bind/cleanup) und den Client-Accessors in
`ssh_transport.rs`. Keying: `Channel::Oss` **und** `app_version() = Some(tag)`
→ versioniert; sonst (Source-Builds, Nicht-Oss-Channels) unverändert legacy —
der `deploy_remote_server`-Dev-Loop und Upstream-Verhalten bleiben unberührt
(bewusst enger gekeyt als `remote_server_binary`, das die meisten Nicht-Oss-
Binary-Pfade versioniert). Das Versions-Segment ist **längenbegrenzt** (max.
20 Zeichen; längere oder unsaubere Tags kollabieren deterministisch zu
`prefix-<fnv64>`): der Socket-Pfad muss in `sockaddr_un.sun_path` (104/108
Bytes) passen — codex-Fund im Review.

Wirkung: Ein v0.rc3-Client KANN keinen v0.rc2-Daemon mehr erwischen — der
Proxy (selbst das versionierte Binary) findet nur den arteigenen Socket und
spawnt sonst frisch. Der Alt-Daemon bleibt nur unter seinem Alt-Namen
erreichbar (also für Clients seines eigenen Releases) und stirbt über
Grace/GC, sobald seine letzte Session endet.

### 2. Handshake-Enforcement einschalten (Gürtel + Hosenträger)
`should_enforce_remote_version_check`: `Oss → app_version().is_some()`
(Release erzwingt, Source-Build überspringt weiter). Mismatch-Pfad: für Oss
KEIN Binary-Löschen (Upstream-Zweig unverändert), sondern Connect-Fehler mit
klarer Meldung → läuft über den bestehenden `SessionConnectionFailed`-Pfad in
den Tab (`on_connect_failed`) und den Classic-SSH-Fallback. Mit Teil 1 ist
das ein Assert auf „unmöglich" — feuert es doch, war es ein Pfad-Bug, und
zaplex sagt es, statt still alten Code zu servieren.

### 3. Idle-Exit-Härtung (Latenz raus)
`maybe_arm_grace_after_gc` → `maybe_arm_grace_when_idle`; zusätzlich zu GC
auch von `handle_close_session` und `on_session_reader_eof` aufgerufen (ctx
wird durchgereicht; beide Call-Sites haben es). Verhalten unverändert, nur
sofort statt ≤ 5 min später.

## Übergang / Follow-up: versionsübergreifende Adoption

Der ursprüngliche Stand isolierte alte Daemons korrekt, machte deren noch
laufende Sessions für neue Clients aber unsichtbar. Der Adopt-Sidebar-Follow-up
schließt diese Lücke, ohne die Versionsisolierung zurückzunehmen:

- Der aktuelle, vertrauenswürdige Proxy listet ausschließlich Socket-Dateien im
  aktuellen Identity-Verzeichnis. Beliebige Pfade, Symlinks und Traversal sind
  ausgeschlossen; die Anzahl der Runtime-Einträge ist begrenzt.
- Der Standardpfad bleibt unverändert: Nur der Socket des aktuellen Releases
  darf bei Bedarf einen neuen Daemon starten. Ein explizit ausgewählter alter
  Socket ist strikt **connect-only** und wird niemals neu angelegt oder gelöscht.
- Beim Inventarabruf wird `InitializeResponse.server_version` des jeweiligen
  Daemons gespeichert. Anzeige, Klick, Tab und Reconnect tragen Socketname und
  diese exakte Version gemeinsam weiter; der Manager prüft gegen die beobachtete
  Daemon-Version statt den Versionscheck pauschal abzuschalten.
- Neue RPCs bleiben capability-gated. Alte Daemons ohne Agent-Inventar liefern
  weiterhin eine neutrale Zaplex-Session-Kennung; Generation 0 behält den
  bestehenden ID-only-Attach-Pfad.
- PTY-Handover zwischen Daemon-Prozessen (FD-Passing) bleibt out of scope: Die
  Session wird nicht migriert, sondern bis zu ihrem Ende am besitzenden alten
  Daemon bedient.

## Abnahme
1. Neuer Daemon + neuer Client → Socket heißt `server-<tag>.sock`, Connect
   wie gehabt (Regression: Bootstrap, Reattach, Adopt).
2. Alt-Daemon läuft weiter → neuer Connect erreicht ihn NICHT (frischer
   Daemon), Alt-Daemon exit nach Ende seiner Sessions (Log: grace timer).
3. Dev-Loop (`cargo run` + `deploy_remote_server`): unverändert
   `server.sock`, keine Enforcement-Fehler.
4. Session schließen als letzte bei 0 Clients → Grace-Timer-Log sofort,
   nicht erst nach GC-Tick.
5. Neuer Client + Alt-Daemon mit laufenden Sessions → beide Runtime-Sockets
   werden getrennt inventarisiert; ein Klick verbindet exakt den Alt-Daemon und
   Reconnects bleiben an dessen beobachtete Version gebunden.
6. Staler oder manipulierter Alt-Socket → keine Daemon-Neugründung, keine
   Pfadauflösung außerhalb des Identity-Verzeichnisses und kein stiller Fallback
   auf den aktuellen Daemon.
