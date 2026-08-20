# Cockpit parity runtime smoke

This is the manual half of the GH-160/GH-169 parity gate. The automated matrix validates fixtures,
focused Rust tests, the UI contract, and freshly synchronized reference revisions. This procedure
validates the installed provider CLIs and the real local/remote transport that hermetic CI cannot
represent.

Use a disposable project with no secrets in its name or transcript. Record only sanitized host
labels, provider versions, session states, and screenshots. Never attach provider tokens, process
environments, full transcripts, account files, or personal filesystem paths.

## Preconditions

- A Zaplex build produced from the revision recorded in the audit report.
- `claude --version` and `codex --version` succeed on the local host.
- One registered remote host is reachable through Zaplex and has at least one of the two CLIs.
- The local host and that remote host form the required two-host topology.
- At least one default account and one explicitly pinned account are available across the run.

Create a sanitized evidence directory and record the versions:

```text
runtime-smoke/
  versions.txt
  observations.json
  local-claude.png
  local-codex.png
  remote-host.png
  reattach.png
```

`versions.txt` contains the Zaplex revision plus the output of `claude --version` and
`codex --version`. Replace real host and account names with stable labels such as `local`,
`remote-a`, `account-default`, and `account-pinned`.

## Local Claude

1. Open a local terminal in the disposable project and launch Claude from the Cockpit with the
   default account.
2. Confirm that the tree shows `local → project → PTY session → Claude` exactly once.
3. Produce one running turn and one turn waiting for user input. Confirm the state glyph and the
   subtle whole-row waiting treatment without duplicate visible status text in the compact tree.
4. Open the account detail, verify that the session transcript resolves, and capture
   `local-claude.png` without transcript content.
5. Close the PTY, verify that the live leaf disappears and the substantial session remains
   resumable as dormant history in account detail.

## Local Codex

1. Launch Codex in the same project with an explicitly pinned account.
2. Confirm that the tree shows a separate Codex leaf under its exact PTY and that the account card
   headline is `Codex`, with the account identity only in the subline.
3. Produce a completed turn, open its transcript, and confirm that usage is attributed to the
   pinned account rather than the default account.
4. Capture `local-codex.png` without transcript content, then close and resume the dormant thread.

## Remote host

1. Connect one Zaplex terminal to `remote-a`. Confirm that a remote Cockpit root appears only after
   the connection is live.
2. Launch one installed provider CLI on that host. Verify the exact host, project, PTY, provider,
   account route, waiting state, and transcript route.
3. Capture `remote-host.png`, then open a second Zaplex session to the same host.
4. Disconnect the first session: the host root must remain. Disconnect the final session: the host
   root must disappear while local sessions remain visible.

## Reattach and lifecycle

1. Reopen one local dormant Claude session and one local dormant Codex session from account detail.
2. Reattach the still-live remote PTY by its stable binding; do not accept a new session inferred
   only from its working directory.
3. Capture `reattach.png` and confirm that no provider, account, host, or session boundary was
   crossed during reattach.

## Machine-readable snapshot

1. From a Zaplex-managed terminal in this run, execute `zaplex cockpit snapshot --json`. Confirm
   exit 0, `schema_version: 1`, both live hosts, and the same waiting/session counts as the UI.
2. Confirm the document contains no config roots, transcript text, credentials, tokens, or raw host
   ids. Usage that is not trustworthy must be `null` with `unknown` provenance.
3. From an ordinary shell without the Zaplex control environment, execute the same command. Confirm
   a local-only `degraded` document and exit 3 rather than a fabricated fully loaded remote view.

## Evidence record

Write `observations.json` with this shape. `pass` is allowed only after every assertion above was
observed on the Zaplex revision named in the automated audit report.

```json
{
  "schema_version": 1,
  "zaplex_revision": "<40-hex-sha>",
  "executed_at": "<RFC3339 UTC>",
  "topology": ["local", "remote-a"],
  "cases": {
    "local_claude": "pass|fail",
    "local_codex": "pass|fail",
    "remote_host": "pass|fail",
    "reattach_and_lifecycle": "pass|fail",
    "machine_snapshot": "pass|fail"
  },
  "evidence": [
    "versions.txt",
    "local-claude.png",
    "local-codex.png",
    "remote-host.png",
    "reattach.png"
  ],
  "notes": "<sanitized failure details only>"
}
```

Attach the evidence directory to the same CI/audit run or release-gate record. A missing manual
record is `not-run`, never an implicit pass.
