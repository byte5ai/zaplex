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

The directory is closed-world: do not add logs, transcripts, config files, nested directories, or
symlinks. `versions.txt` uses exactly these one-line keys (replace the example values, not the keys):

```text
zaplex_revision=<40-character lowercase commit SHA>
claude_version=<sanitized output of claude --version>
codex_version=<sanitized output of codex --version>
remote_provider=claude|codex
remote_provider_version=<sanitized remote CLI version>
```

Use only the stable host aliases `local` and `remote-a`. Do not record real host or account names.

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

Write `observations.json` with exactly this shape. `pass` is allowed only after every assertion
above was observed on the Zaplex revision named in the automated audit report. The remote provider
may be Claude or Codex, but it must match `versions.txt`.

```json
{
  "schema_version": 1,
  "zaplex_revision": "<40-hex-sha>",
  "executed_at": "<RFC3339 UTC>",
  "topology": [
    {"label": "local", "kind": "local"},
    {"label": "remote-a", "kind": "remote"}
  ],
  "providers": {
    "local_claude": {"provider": "claude", "host": "local"},
    "local_codex": {"provider": "codex", "host": "local"},
    "remote_agent": {"provider": "claude", "host": "remote-a"}
  },
  "account_modes": ["default", "pinned"],
  "states": ["live", "waiting", "idle", "dormant"],
  "capabilities": [
    "launch",
    "reattach",
    "transcript",
    "usage",
    "attention",
    "lifecycle",
    "snapshot"
  ],
  "cases": {
    "local_claude": "pass",
    "local_codex": "pass",
    "remote_host": "pass",
    "reattach_and_lifecycle": "pass",
    "machine_snapshot": "pass",
    "two_host_disconnect": "pass"
  },
  "evidence": [
    "versions.txt",
    "local-claude.png",
    "local-codex.png",
    "remote-host.png",
    "reattach.png"
  ]
}
```

Inspect every image before upload. Raw screenshots commonly contain ancillary color, density, or
capture-tool PNG chunks, so the strict metadata-free validator is intentionally not run directly
against the capture directory. The upload helper below copies into private staging, removes those
chunks without changing the source files, canonicalizes the text records, and then runs the
authoritative exact-revision validation before any release is created or asset uploaded. That
validator checks the closed file set, schema, revision, freshness, topology, coverage, cases, PNG
structure/dimensions, and that all four screenshots differ. It cannot detect secrets rendered
inside otherwise valid pixels; visual inspection therefore remains mandatory.

Concretely, the helper runs `cockpit-parity-audit validate-runtime --require-pass` against its
sanitized staging copy. Run that subcommand directly only when inspecting an already canonicalized
bundle downloaded from a passing audit, never as the first step on raw capture-tool PNGs.

## Seed the strict audit

The checked-in upload helper copies only the exact six closed-world files through no-follow,
non-blocking, size-capped file descriptors, canonicalizes the JSON/text files, strips ancillary PNG
metadata, repeats strict validation, creates a deterministic ZIP, and uploads it using the
repository-approved `gho "$(getpat)"` authentication path:

```text
script/upload-cockpit-runtime-evidence \
  --evidence-dir runtime-smoke \
  --zaplex-revision <40-hex-sha>
```

The only accepted destination is a draft release named
`cockpit-runtime-evidence-<40-hex-sha>` whose `target_commitish` is that exact current `main` SHA.
Its only asset is `cockpit-runtime-smoke.zip`. The helper pins the repository to the verified
`byte5ai/zaplex` origin and refuses published releases, real Git tags, mismatched targets, existing
assets, replacement, and mixed release contents. Draft releases keep the
sanitized bundle out of the public release list; authorized repository users can still inspect it,
so the screenshot review above remains mandatory.

The helper prints the exact workflow inputs. Dispatch the Cockpit parity workflow on `main` with
the printed draft-release tag and `require_manual_runtime=true`. The workflow rejects a simultaneous
run-id and draft-release source. A minimal manual seed job receives temporary contents-write access
solely to list and download the exact draft; it runs only for `refs/heads/main`, does not persist
checkout credentials, validates the ZIP and same-revision bundle, and hands a one-day internal
artifact to the normal read-only audit job. The existing source-run artifact input remains
supported.

After validation, a strict dispatch re-uploads the six metadata-free files as an Actions artifact
named `cockpit-runtime-smoke`; unvalidated runtime files are never included in the general audit
artifact, and its failure report never reflects untrusted input values. Draft and source-run
artifacts are size-checked through GitHub metadata before download; source-run artifacts are then
downloaded by the checked immutable artifact id, and both transports use the same bounded
closed-world ZIP extractor. The assembled report passes
`release_gate_status` only when automated and
runtime evidence use the same Zaplex revision. Download and inspect
`cockpit-parity-<GITHUB_SHA>/cockpit-parity-report.json`; both `status` and
`release_gate_status` must be `pass` before linking the run from GH-160/GH-169.

After the report is retained and linked, delete the temporary draft release through the normal
GitHub UI or the authenticated repository helper. Never publish it. Without either a source-run
artifact or the SHA-bound draft asset, `manual_runtime` and `release_gate_status` remain `not-run`;
they are never an implicit pass.
