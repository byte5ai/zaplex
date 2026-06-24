# Implementierungsplan: Nativer persistenter Remote-Session-Layer

> **Status:** Plan (read-only, noch kein Code), erstellt 2026-06-24.
> **Quelle der Entscheidung:** [zaplex-concept.md §3.5](../../zaplex-concept.md) (Architektur-Entscheidung „Option 1 — nativer Persistenz-Layer") + Track-A-Upstream-Spike.
> **Direktive:** Nur nachhaltige, technisch saubere Lösungen — keine Quick-Wins/Hacks (siehe Projekt-Memory `no-quick-wins-sustainable-only`).
> **Gilt:** zaplex besitzt die Remote-Session-Persistenz selbst; **kein** Compose mit User-tmux/byobu/screen.

---

## 1. Ziel & Nicht-Ziele

**Ziel:** Ein in zaplex eingebauter Remote-Session-Layer, der
1. **mehrere interaktive Sessions pro Host** verwaltet (in zaplex sichtbar, keine externen tmux-Window-Namen),
2. Sessions über **Verbindungsabbrüche** (Deckel zu, Akku leer, Netz-/Tailscale-Roaming) **am Leben hält** (host-resident), inkl. **nahtlosem Re-Attach mit erhaltener Historie** — auch über **App-Restart** hinweg,
3. **Maus-Bedienung garantiert** (harte Anforderung §3.5 Lücke 3), weil zaplex die Remote-Session-Struktur end-to-end besitzt,
4. perspektivisch **mosh-Eigenschaften** liefert (UDP-Transport mit State-Sync, predictive local echo, IP-Roaming),
5. damit **externes `byobu` + `mosh` überflüssig** macht.

**Nicht-Ziele:**
- Kein Compose mit vom User betriebenen Multiplexern (explizit verworfen, §3.5).
- Kein Ersatz für die **Agent-Fleet-Persistenz** (`claude remote-control` etc., §3.2/§4.4) — aber: **gemeinsame Primitive** anstreben (siehe §10).
- Windows-Remote-Host ist v1-out-of-scope (wie beim bestehenden remote-server: bash/zsh, glibc ≥ 2.31).

---

## 2. Architektonischer Kernbefund (prägt den gesamten Plan)

Das bestehende `remote-server`-Subsystem ist **kein** PTY-/Session-Multiplexer:

- Die **interaktive Shell** läuft heute über einen **separaten SSH-PTY-Kanal**; der `remote-server`-Daemon spricht ein **strukturiertes Protobuf-RPC** (Repo-Metadaten, File-Sync, Command-Exec) — PTY-Bytes fließen **nicht** durch das Protokoll. (`crates/remote_server/proto/remote_server.proto`, `crates/remote_server/src/protocol.rs:72`; PTY-Pfad: `app/src/terminal/local_tty/event_loop.rs`.)
- **Aber:** Der Daemon ist genau die richtige **Infrastruktur**, auf der wir aufsetzen — er ist
  - **identity-scoped & host-resident** (`app/src/remote_server/unix/mod.rs:run_daemon`, Unix-Socket `~/.warp-*/remote-server/{identity}/server.sock`, Mode 0600, PID-flock),
  - **überlebt den SSH-Drop und den Client-App-Restart** (Socket + Daemon bleiben; nächster Proxy verbindet sich wieder — `app/src/remote_server/unix/proxy.rs`, `setsid()`),
  - **multi-connection** (mehrere Proxies/Tabs teilen sich einen Daemon).

**Konsequenz:** Der native Layer macht aus dem Daemon einen **PTY-besitzenden Session-Host** (tmux-Äquivalent, aber zaplex-eigen und in Blocks/Maus integriert): der Daemon **spawnt die User-Shell selbst**, hält pro Session einen **Output-Ring-Buffer** und streamt PTY-Bytes über einen **neuen Protokoll-Kanal** an den Client — mit **Attach/Detach/Resume + Replay**. Persistenz lebt damit **server-seitig im Daemon**, nicht in der flüchtigen Client-SSH-Verbindung.

Das ist die saubere, tragfähige Lösung: Persistenz, Blocks und Maus funktionieren, weil **zaplex die Remote-Session besitzt** — statt sie an einen fremden Multiplexer oder an die Lebensdauer eines SSH-Kanals zu koppeln.

---

## 3. Zielarchitektur

```
┌─ zaplex Client (App) ─────────────────────────────────────────────┐
│  Terminal UI  ── Blocks/Grid ◄── ansi::Processor ◄── Session-Bytes │
│       │ Maus/Resize/Input ──────────────────────────► (Input-Msg)  │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │ zaplex_remote_session (NEU, client-seitiger Teil)            │ │
│  │  - Session-Attach/Detach/Resume-Client                       │ │
│  │  - Replay-Consumer (Buffer → Grid beim Re-Attach)            │ │
│  │  - erweitert RemoteServerManager-Zustandsmaschine            │ │
│  └──────────────────────────────────────────────────────────────┘ │
└───────────────────────────┬───────────────────────────────────────┘
        Phase B2: über bestehende SSH-ControlMaster + Proxy-stdio
        Phase B3: alternativ über nativen UDP-Transport (mosh-grade)
                            │
┌─ Remote Host ─────────────▼───────────────────────────────────────┐
│  zaplex Session-Host-Daemon (erweiterter remote-server-daemon)     │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │ zaplex_remote_session (NEU, server-seitiger Teil)            │ │
│  │  SessionRegistry: SessionId → {PTY, Shell-Child, RingBuffer, │ │
│  │                                 size, seq, last_attached}     │ │
│  │  - spawnt/besitzt die User-Shell-PTYs                         │ │
│  │  - Output-Ring-Buffer pro Session (Replay-Quelle)            │ │
│  │  - Attach/Detach/Resume, Resize, Input-Forwarding            │ │
│  │  - Lifecycle/GC, RAM-Ceiling pro Session                     │ │
│  └──────────────────────────────────────────────────────────────┘ │
│  bestehend: ServerModel (Repo/File/Command-RPC), Socket, PID-flock │
└────────────────────────────────────────────────────────────────────┘
```

**Neuer Crate:** `crates/zaplex_remote_session` (erstes `zaplex_*`-Crate; Workspace-Glob `crates/*` greift automatisch, Registrierung in `Cargo.toml [workspace.dependencies]`). Enthält client- und server-seitige Logik hinter Feature-Gates, plus die geteilten Session-Typen. **Touchpoint-Disziplin (§8.3):** Änderungen an `crates/remote_server` und `app/src/terminal/*` minimal und additiv halten; die Substanz lebt im neuen Crate.

**Runtime/Stack (vorhanden, wiederverwenden):** tokio 1.47, prost 0.14 (Protobuf), serde/bincode, thiserror, diesel/SQLite (`crates/persistence`).

---

## 4. Protokoll-Erweiterung

Additiv zum bestehenden Envelope (`ClientMessage`/`ServerMessage`, 4-Byte-LE-Length-Prefix + Protobuf, 64 MB-Cap, `protocol.rs`). Push-Messages haben leere `request_id` — der PTY-Stream nutzt diesen Push-Pfad.

**Neue Client→Server-Messages:**
- `OpenSession { cwd, shell, env, size }` → Daemon spawnt PTY+Shell, legt `SessionId` an, returnt `SessionOpened { session_id }`.
- `AttachSession { session_id, last_seq }` → Daemon liefert `SessionAttached { size, base_seq, replay: bytes[] }` (Replay ab `last_seq` aus dem Ring-Buffer) und beginnt Live-Streaming.
- `DetachSession { session_id }` → Client koppelt ab, **Session läuft weiter** (Default bei Disconnect).
- `SessionInput { session_id, bytes }` → Tastatur/Maus-Bytes in die PTY (inkl. SGR-Mouse-Reports).
- `ResizeSession { session_id, size }` → `TIOCSWINSZ` server-seitig (schließt die heutige Lücke: Remote-Resize-Protokoll fehlt aktuell).
- `CloseSession { session_id }` → Shell beenden, Session entsorgen.
- `ListSessions {}` → `SessionList [{session_id, title, cwd, alive, last_attached}]` (für Multi-Session-UI + Adopt).

**Neue Server→Client-Pushes:**
- `SessionOutput { session_id, seq, bytes }` → Live-PTY-Bytes (monoton steigende `seq` für Replay-Korrelation).
- `SessionExited { session_id, exit_status }`.

**Versionierung:** Die neuen Messages sind protobuf-additiv (neue Feldnummern) → alte/neue Binaries bleiben dekodierbar; die exakte Version-Handshake-Logik (`Initialize`/`InitializeResponse{server_version}`, exact-match → Reinstall, `manager.rs`) erzwingt aber ohnehin gleiche Release-Tags zwischen Client und Daemon. Capability-Negotiation: `InitializeResponse` um `features: []` ergänzen, damit der Client weiß, ob der Daemon den Session-Host kann.

---

## 5. Server-seitiger Session-Host (Phase B2-Kern)

Lebt im erweiterten Daemon (`app/src/remote_server/unix/mod.rs` → ruft in `zaplex_remote_session::server`).

**SessionRegistry** (`HashMap<SessionId, Session>`, persistent über die Daemon-Lebenszeit):
- `Session { pty: Pty, child: Child, ring: OutputRing, size: Winsize, seq: u64, title, cwd, attached: Option<ConnId>, last_attached: Instant }`.
- **PTY-Ownership:** Der Daemon allokiert das PTY und spawnt die User-Login-Shell selbst (analog `local_tty/unix.rs` PTY-Setup: `posix_openpt`/`ptsname`, Child mit Slave-FD; `TIOCSWINSZ` für Resize). Damit ist die Session **vom SSH-Kanal entkoppelt** — SSH-Drop killt sie nicht.
- **Output-Ring-Buffer:** bounded pro Session (byte- **und** zeilenbasiert), monotone `seq`. Quelle für Replay beim Re-Attach. Default-Ceiling konfigurierbar (siehe §7, RAM-Ceiling) — anlehnend an, aber unabhängig von der Client-Scrollback-Grenze (`BlockSize::max_block_scroll_limit`, ~10k Zeilen).
- **Reader-Task pro Session:** liest PTY → schreibt in Ring (`seq++`) → pusht `SessionOutput` an die *aktuell attachte* Connection (falls vorhanden). Kein Attach = Bytes laufen weiter in den Ring (Session bleibt produktiv).
- **Lifecycle/GC:** Session endet, wenn (a) Shell exitet (`SessionExited`), (b) `CloseSession`, oder (c) **Detached-Idle-Timeout** überschritten **und** keine Persistenz gewünscht. Default: detachte Sessions **bleiben** (das ist der Sinn); ein konfigurierbares max. Detached-Alter (z. B. 24 h) als Speicher-Schutz, plus harter Per-Session-RAM-Ceiling.
- **Crash-/Reboot-Robustheit:** Sessions sind PTYs lebender Prozesse → ein Host-Reboot beendet sie zwangsläufig (wie tmux). Über Reboot hinweg „überleben" ist **kein** Ziel (mosh/tmux können das auch nicht). Über Daemon-Restart hinweg: die Registry ist In-Memory; ein Daemon-Crash beendet die Kind-Shells. Härtung optional via `setsid` + Re-Parenting in v2 — **nicht** v1 (sauber abgegrenzt, kein Hack).

---

## 6. Client-seitige Integration

**Byte-Flow-Anschluss:** Der bestehende Pfad ist `PTY/Transport-read → ansi::Processor → Handler → TerminalModel → BlockList/Grid` (`event_loop.rs`, `terminal_model.rs`, `blocks.rs`). Der native Layer ersetzt für eine persistente Remote-Session die Byte-Quelle: statt direktem SSH-PTY kommen die Bytes aus `SessionOutput`-Pushes. Sauberste Einhängung: eine `remote_tty`-Variante (es gibt bereits `app/src/terminal/remote_tty/terminal_manager.rs` mit async-channel-Eventloop) als „Session-attached"-Quelle.

**Re-Attach + Replay:** Beim (Re-)Verbinden sendet der Client `AttachSession{ last_seq }`; die `replay`-Bytes werden **durch denselben `ansi::Processor`** gespeist → Grid/Blocks rekonstruieren sich deterministisch. `last_seq` persistiert client-seitig pro Session (siehe §7), damit nach App-Restart nur das Delta nachgeladen wird (oder voller Ring, wenn `last_seq` zu alt).

**Maus-Garantie (harte Anforderung):** Da der Daemon das echte PTY besitzt, läuft der **standardmäßige SGR-Mouse-Weg** end-to-end: die Remote-Shell/App setzt `DECSET 1006/1000/1002/1003`, der Client erkennt das über den normalen `ansi`-Pfad, und Maus-Events gehen via `SessionInput` zurück in dieselbe PTY (`should_intercept_mouse` `app/src/terminal/alt_screen/mod.rs:11`, `write_user_bytes_to_pty` `pty_controller.rs:40`). **Keine** Multiplexer-Schachtelung dazwischen → der Warp-Maus-Bug (tmux-Nesting) entfällt strukturell.

**Resize:** UI-Resize → `ResizeSession` → `TIOCSWINSZ` im Daemon. Schließt die heute fehlende Remote-Resize-Lücke.

**Multi-Session-UI & Adopt:** `ListSessions` speist die Sidebar/Session-Inventar; „Adopt-by-session-id" (§3.2) wird damit für Remote-Sessions echt: Enter auf eine laufende Session → `AttachSession` → Block mit voller Historie.

---

## 7. Persistenz & Settings

**Client-seitige Session-Persistenz (über App-Restart):** kleine Tabelle `remote_sessions` in `crates/persistence` (diesel-Migration unter `crates/persistence/migrations/`, schema.rs auto-generiert via `diesel.toml`): `{ session_id, host_node_id, identity_key, title, cwd, last_seq, last_attached_at }`. Beim Start: pro Host `ListSessions` → mit persistierten Einträgen abgleichen → wieder-attachbar anzeigen.

**Per-Verbindung-Setting (aus §3.5):** `SshServerInfo` (`crates/warp_ssh_manager/src/types.rs:99`) + Tabelle `ssh_servers` (`crates/persistence/src/model.rs:1463`) + CRUD (`repository.rs`) additiv um `session_resilience: Off | PersistOnly | PersistPlusMosh` erweitern (Feld + Migration + Row/NewRow + Mapping). Default `Off` (konservativ). Globales Default + Feature-Gate über `maybe_define_setting!` (`app/src/terminal/warpify/settings.rs`-Muster) und `warp_features` (`SSHTmuxWrapper`-Muster).

**RAM-Ceiling pro Session:** Heute existiert **kein** RAM-Governor (nur zeilenbasierte Scrollback-Grenze `BlockSize::max_block_scroll_limit`). Der Ring-Buffer bekommt ein **byte-basiertes** Ceiling pro Session (Setting), und die Registry ein Gesamt-Ceiling pro Host — das ist zugleich der erste Baustein des in §3.2 genannten „RAM-Governor für die Fleet" (gemeinsame Primitive, §10).

---

## 8. Phase B3 — mosh-grade UDP-Transport

Aufbauend auf B2 (das Session-Protokoll bleibt identisch; nur der **Transport** unter dem Protokoll wechselt von „SSH-ControlMaster + Proxy-stdio" auf nativen UDP).

- **Modell:** wie mosh — die **initiale SSH-Verbindung** startet den Daemon und übergibt einen **Session-Key** (AES-OCB/AEAD); danach spricht der Client per **UDP** mit dem Daemon. Kein offener Port ohne Key (mosh-Sicherheitsmodell als Referenz).
- **SSP-artiger State-Sync:** Der Client hält den zuletzt bestätigten `seq`; der Server sendet das nötige Delta. Roaming über IP-Wechsel ist „gratis", weil UDP zustandslos auf Adressebene ist und die Session über den Key, nicht die IP, identifiziert wird.
- **Predictive local echo:** clientseitige Vorhersage von Tastatur-Echo/Cursor (mosh-Heuristik) für latenzarmes Tippgefühl; bei Bestätigung/Abweichung korrigieren.
- **Abgrenzung:** B3 ist die **saubere Kür**, nicht der Einstieg. B2 liefert bereits Persistenz + Re-Attach (das Must-Have). B3 liefert Roaming + Latenz. Reihenfolge daher **B2 → B3** (mosh-Orchestrierung **B1** nur als Fallback, falls B3 sich verzögert — kein Quick-Win-Pfad, sondern explizit nachrangig).

---

## 9. Reconnect/Re-Attach-Semantik (Manager-Erweiterung)

Die bestehende Zustandsmaschine (`crates/remote_server/src/manager.rs`: `Connecting→Initializing→Connected→Reconnecting→Disconnected`, `MAX_RECONNECT_ATTEMPTS=2`, `RECONNECT_DELAY=2s`) ist für **transiente Blips** gebaut und verwirft Session-State beim Disconnect. Erweiterung:
- **Re-Attach statt nur Reconnect:** Nach Transport-Reconnect zusätzlich `AttachSession{last_seq}` → Replay. `SessionReconnected` trägt bereits den getauschten Client — Consumer müssen zusätzlich den Replay konsumieren.
- **Längeres/limitloses Re-Attach-Fenster:** Da die Session **server-seitig weiterlebt**, ist das 2-Versuch-Fenster nur noch für den *Transport*, nicht für die *Session*. Deckel-zu/Akku-leer = Client weg, Daemon+Session bleiben → späterer Connect re-attached.
- **State nicht mehr wegwerfen:** `session_bootstrap_info`/`last_navigated_path` (heute beim Disconnect gecleart, `manager.rs:923`) für persistente Sessions erhalten bzw. aus der Registry rehydrieren.

---

## 10. Gemeinsame Primitive mit der Agent-Fleet

§3.2/§4.4 nennen Agent-Fleet-Persistenz (`claude remote-control` in tmux). Der hier gebaute Session-Host **ist** die generische Persistenz-Primitive, die §3.4 für Codex („generische tmux-Persistenz") fordert — nur zaplex-eigen statt tmux. Konsequenz: Agent-Sessions laufen als **Sessions im selben Daemon** (eine `OpenSession`, deren Shell `claude`/`codex` startet), profitieren von Ring-Buffer/Re-Attach/RAM-Ceiling. **Nicht zwei Persistenz-Mechanismen bauen** — die Fleet adoptiert diese Primitive.

---

## 11. Build, Deploy, Versionierung

- Der Daemon ist **dasselbe App-Binary** mit Subcommand (`remote-server-daemon`/`-proxy`, `app/src/remote_server/mod.rs`); der neue Session-Host wird darin feature-gegated mitgeliefert — **kein** separates Deploy.
- **Install/Version:** vorhandener Pfad wiederverwenden (`ssh_transport.rs` install_binary + GitHub-Release/SCP-Fallback + Dev-Cross-Compile-musl). Version-Handshake erzwingt Client==Daemon-Tag; `features` im `InitializeResponse` für sanftes Degradieren, falls ein alter Daemon den Session-Host nicht kann.

---

## 12. Phasen / Meilensteine (jede Stufe sauber & abgeschlossen)

| Stufe | Inhalt | Abnahme |
|---|---|---|
| **0. Crate-Gerüst** | `crates/zaplex_remote_session` (lib, Feature-Gates, geteilte Typen), Workspace-Registrierung; Protokoll-Messages additiv definiert; `features`-Capability im Handshake | baut grün; Handshake meldet Capability; keine Verhaltensänderung |
| **1. Session-Host (Daemon)** | PTY-Ownership + Shell-Spawn + Ring-Buffer + Reader-Task + `OpenSession`/`SessionOutput`/`SessionInput`/`ResizeSession`/`CloseSession`/`SessionExited` | Headless-Test: Session öffnen, Befehl laufen lassen, Bytes korrekt gestreamt; Resize wirkt |
| **2. Client-Attach + Block-Integration** | `remote_tty`-Quelle aus `SessionOutput`; Input/Resize/Maus via `SessionInput`/`ResizeSession`; eine Remote-Session als Block bedienbar | manuell: voll bedienbare Remote-Session inkl. **Maus** (Klick/Selektion/Scroll) ohne externen Multiplexer |
| **3. Persistenz + Re-Attach** | `DetachSession`/`AttachSession{last_seq}` + Replay; Manager-Erweiterung; client-seitige `remote_sessions`-Tabelle; Deckel-zu/Akku-leer überlebt; App-Restart re-attached | manuell: Verbindung killen → später nahtlos mit Historie weiter; App neu starten → re-attach |
| **4. Multi-Session + Settings** | `ListSessions`, Sidebar/Adopt-by-id; `session_resilience`-Per-Host-Setting + Migration + UI; RAM-Ceiling pro Session/Host | mehrere Sessions/Host, Adopt funktioniert; Setting pro Host schaltbar; Ceiling greift |
| **5. (B3) mosh-grade Transport** | UDP-Transport unter identischem Session-Protokoll: AEAD-Key-Übergabe via SSH, SSP-State-Sync, Roaming, predictive echo | Roaming über IP-Wechsel; latenzarmes Echo; Security-Review |

Stufen 0–4 = **Must-Have-Persistenz (B2)**. Stufe 5 = **B3-Kür**. Jede Stufe ist einzeln abnehmbar und hinterlässt einen konsistenten Zustand.

---

## 13. Teststrategie

- **Unit:** Ring-Buffer (seq-Monotonie, Wrap, Replay-ab-seq, Ceiling-Eviction); Protokoll-Roundtrip (prost encode/decode, Framing).
- **Integration (headless Daemon):** Muster aus `crates/integration` (es gibt bereits SSH/tmux-Integrationstests) — Session öffnen, Output deterministisch prüfen, Detach/Attach/Replay, Resize (`stty size` im Remote prüfen), Shell-Exit→`SessionExited`.
- **Maus-Regression:** automatisiert prüfen, dass SGR-Mouse-Reports (`1006`) durch `SessionInput` end-to-end ankommen; manuelle Abnahme der Selektion.
- **Resilienz:** Transport hart killen (Prozess/Netz) → Session lebt, Re-Attach liefert lückenlose `seq`-Folge; App-Restart-Re-Attach.
- **B3-Security:** kein UDP-Zugriff ohne Key; Key-Rotation; Roaming-Fuzzing.

---

## 14. Risiken & offene Fragen

- **PTY-Ownership-Refactor:** Der Daemon muss robustes PTY-Handling bekommen (heute lebt PTY-Code client-seitig in `local_tty/unix.rs`). Sauber: gemeinsamen PTY-Kern in ein geteiltes Modul ziehen, das Client und Daemon nutzen — **nicht** duplizieren. Aufwand real, aber das ist der nachhaltige Weg.
- **Daemon-Speicher:** N persistente Sessions × Ring-Buffer → Host-RAM. Ceiling + Detached-Idle-Alter sind Pflicht, kein Nice-to-have.
- **Version-Lockstep:** Client==Daemon-Tag-Zwang erschwert gemischte Stände; `features`-Negotiation mildert, aber Roll-out-Reihenfolge bedenken.
- **B3-Komplexität & Security:** eigener AEAD-UDP-Transport ist anspruchsvoll; mosh als Referenz, ggf. Krypto-Review extern. Genau deshalb nachrangig zu B2.
- **Daemon-Crash/Host-Reboot:** Sessions überleben das (bewusst) **nicht** — ehrlich kommunizieren (§2.3), kein Fake-Versprechen.
- **Upstream-Touchpoints:** Erweiterungen an `crates/remote_server`/`app/src/terminal` rebase-fähig additiv halten; Substanz in `zaplex_remote_session`.

---

## 15. Anhang — Code-Seams (verifiziert)

| Bereich | Datei:Zeile |
|---|---|
| Transport-Trait, `Connection`, `connect()` | `crates/remote_server/src/transport.rs:35,63` · `app/src/remote_server/ssh_transport.rs:69,691` |
| Protokoll (Framing, Messages, ServerModel) | `crates/remote_server/proto/remote_server.proto` · `crates/remote_server/src/protocol.rs:16,72` |
| Manager-Zustandsmaschine + Reconnect | `crates/remote_server/src/manager.rs:31,177,923` |
| Daemon + Proxy (Socket, flock, setsid) | `app/src/remote_server/unix/mod.rs:26,116` · `app/src/remote_server/unix/proxy.rs:43` |
| Lokales PTY-Setup / Resize (TIOCSWINSZ) | `app/src/terminal/local_tty/unix.rs:639` · `app/src/terminal/local_tty/mod.rs:81` (PtyOptions) |
| Byte-Flow / EventLoop / ANSI | `app/src/terminal/local_tty/event_loop.rs:25,35` · `app/src/terminal/model/terminal_model.rs:453` |
| Blocks/Grid/Scrollback-Ceiling | `app/src/terminal/model/blocks.rs:262` · `app/src/terminal/terminal_manager.rs:51` |
| remote_tty (async-channel-Eventloop) | `app/src/terminal/remote_tty/terminal_manager.rs:42` |
| Maus (Intercept + Write-back) | `app/src/terminal/alt_screen/mod.rs:11` · `app/src/terminal/writeable_pty/pty_controller.rs:40` |
| SessionType / BootstrapSessionType | `app/src/terminal/model/session.rs:834,843` |
| SSH-Host-Settings + ssh_servers + CRUD | `crates/warp_ssh_manager/src/types.rs:99` · `crates/persistence/src/model.rs:1463` · `crates/warp_ssh_manager/src/repository.rs` |
| Settings-Makro / Feature-Flags | `app/src/terminal/warpify/settings.rs:46` · `crates/warp_features/src/lib.rs:124` |
| Workspace / Crate-Registrierung | `Cargo.toml:1` (`[workspace] members=["crates/*","app"]`, `[workspace.dependencies]`) |
| Persistenz-Migrationen / Schema | `crates/persistence/migrations/` · `diesel.toml` · `crates/persistence/src/schema.rs` |
