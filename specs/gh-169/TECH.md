# GH-169 — Technical design

## Components

### Versioned automated contract

`specs/parity/cockpit-matrix.json` remains the source of truth for required dimensions,
capabilities, named Rust evidence, reference probes, and deterministic UI screenshot contracts.
`script/cockpit-parity-audit validate` resolves every named test from source and rejects unknown,
missing, duplicated, or uncovered matrix entries.

`self-test` mutates provider, location, and status coverage; exact remote boundaries; and named test
evidence. It additionally exercises the runtime validator with a valid synthetic fixture and
fail-closed mutations. Synthetic images are used only inside the validator self-test and are never
reported as runtime evidence.

### Runtime evidence validator

`script/cockpit-parity-audit validate-runtime` takes an evidence directory and writes a
machine-readable report.

The directory is closed-world: it must contain only `observations.json`, `versions.txt`, and the
four documented PNGs. Symlinks, subdirectories, extra files, oversized JSON/images, malformed PNG
headers, implausible image dimensions, and duplicate image content fail validation.

`observations.json` binds the evidence to:

- schema version 1;
- a 40-character Zaplex commit SHA;
- an RFC3339 UTC execution time within the configured freshness window;
- exactly one local and one remote sanitized topology label;
- local Claude, local Codex, and one remote Claude-or-Codex provider observation;
- default and pinned account coverage;
- live, waiting, idle, and dormant state coverage;
- launch, reattach, transcript, usage, attention, lifecycle, and snapshot capability coverage;
- passing local-Claude, local-Codex, remote-host, reattach/lifecycle, machine-snapshot, and
  two-host-disconnect cases;
- the exact expected evidence filenames.

`versions.txt` is strict key/value data and repeats the Zaplex SHA. It records one-line Claude,
Codex, and remote-provider versions without account or host identity. The repeated SHA and remote
provider must match `observations.json`.

An absent directory produces `not-run` and exit 0 for ordinary CI reporting. `--require-pass`
turns `not-run` into a non-zero release-gate result. Existing but invalid evidence always returns
non-zero. `--expected-zaplex-revision` prevents evidence captured against another commit from
passing.

### Report assembly and enforcement

The workflow always creates `runtime-evidence.json`. On ordinary hosted CI the evidence directory
does not exist, so the component is truthfully `not-run`.

`assemble` incorporates that report and preserves two decisions:

- `status` is the automated parity result, so pull-request verification can pass without claiming
  a real authenticated runtime run;
- `release_gate_status` is `pass` only when the automated gate and runtime evidence both pass for
  the exact same Zaplex revision.

`enforce` checks the automated result by default. `enforce --require-runtime` is the release-gate
mode and rejects both `not-run` and `fail`.

## Security and privacy

- All host and account values are controlled aliases, not discovered identifiers.
- The validator rejects extra bundle contents and symlinks to keep unrelated local data out of the
  uploaded artifact.
- Screenshot contents require human sanitization; the validator checks only format, size,
  dimensions, and distinctness.
- Reference repositories are synchronized read-only and are never packaged or loaded by Zaplex.

## Verification

Static verification on development hosts:

```text
script/cockpit-parity-audit validate
script/cockpit-parity-audit self-test
python3 -m py_compile script/cockpit-parity-audit
git diff --check
```

Cargo checks/tests and Playwright rendering run only in GitHub Actions. A real runtime pass can be
produced only on a host with the built Zaplex revision, authenticated installed CLIs, and a real
remote connection, following `specs/parity/COCKPIT_RUNTIME_SMOKE.md`.
