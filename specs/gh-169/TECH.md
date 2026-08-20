# GH-169 — Technical design

## Components

### Versioned automated contract

`specs/parity/cockpit-matrix.json` remains the source of truth for required dimensions,
capabilities, named Rust evidence, reference probes, and deterministic UI screenshot contracts.
`script/cockpit-parity-audit validate` resolves every named test from source and rejects unknown,
missing, duplicated, ignored, or uncovered matrix entries. CI first executes each exact Cargo
filter with libtest `--list`, requires every catalogued function assigned to that suite to appear
exactly once, and then requires the real run summary to show every listed test passed with none
ignored. A successful Cargo process that matched zero tests is therefore a gate failure.

`self-test` mutates provider, location, and status coverage; exact remote boundaries; named,
mandatory, ignored, and zero-execution test evidence. It additionally exercises the runtime
validator and archive extractor with valid synthetic fixtures and fail-closed mutations. Synthetic
images are used only inside the validator self-test and are never reported as runtime evidence.

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

### Draft-release seed transport

`script/upload-cockpit-runtime-evidence` is the local bridge between real-machine evidence and a
hosted strict audit. It:

1. copies only the exact six files through no-follow, non-blocking, size-capped file descriptors
   into a private staging directory and rejects files that change during the copy;
2. rejects duplicate JSON/text keys, canonicalizes both text files, strips all ancillary PNG
   chunks, and validates only that metadata-free staging copy against the exact build SHA;
3. writes a deterministic uncompressed ZIP with fixed ordering, timestamps, and modes;
4. pins the destination to the verified `byte5ai/zaplex` origin, requires the SHA to equal current
   `main`, and refuses a real Git tag at the reserved evidence name;
5. creates or reuses only an empty draft release named `cockpit-runtime-evidence-<SHA>` with
   `target_commitish=<SHA>`, then uploads exactly `cockpit-runtime-smoke.zip` without replacement.

Existing draft releases must have the exact tag, target, draft state, and no assets. Any uncertainty
or collision fails before upload. The helper never publishes or deletes a release automatically.

The workflow's draft-release input is mutually exclusive with `runtime_evidence_run_id`. It is
accepted only on an exact `main` dispatch; the tag must equal
`cockpit-runtime-evidence-${GITHUB_SHA}` and the asset must equal `cockpit-runtime-smoke.zip`. A
small manual seed job has `contents: write`, checks out only that trusted main revision without
persisting credentials, lists drafts through the authenticated Releases API, and requires exactly
one asset and the exact draft target. It validates ZIP entries and then executes the same-revision
runtime validator before creating a one-day internal seed artifact. The main audit job has only
read permissions, rejects oversize draft or run artifacts from API metadata before downloading,
downloads run artifacts by the immutable checked artifact id, and passes both transports through
the same self-tested closed-world extractor with compressed, declared, and actually expanded size
limits. It validates the seed again and never places raw runtime files or untrusted input values
under the general `artifacts/` tree.

On `require_manual_runtime=true`, a separate gate requires `runtime-evidence.json` to be a clean
`pass` before `actions/upload-artifact` retains the exact six metadata-free files under the fixed
`cockpit-runtime-smoke` name. Invalid sources remain outside the general report artifact. The final
`enforce --require-runtime` remains authoritative.

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
- Draft evidence releases are private transport objects, never published product releases. They
  contain only the sanitized deterministic ZIP and should be deleted after the passing Actions
  report has been retained and linked.
- Reference repositories are synchronized read-only and are never packaged or loaded by Zaplex.

## Verification

Static verification on development hosts:

```text
script/cockpit-parity-audit validate
script/cockpit-parity-audit self-test
script/upload-cockpit-runtime-evidence --help
bash -n script/upload-cockpit-runtime-evidence
python3 -m py_compile script/cockpit-parity-audit
git diff --check
```

Cargo checks/tests and Playwright rendering run only in GitHub Actions. A real runtime pass can be
produced only on a host with the built Zaplex revision, authenticated installed CLIs, and a real
remote connection, following `specs/parity/COCKPIT_RUNTIME_SMOKE.md`.
