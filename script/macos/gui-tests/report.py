#!/usr/bin/env python3
"""Report local GUI execution without claiming unexecuted product acceptance."""

import json
import re
import sys
from pathlib import Path


SUITES = {"full": 12, "terminal": 3, "navigation": 8}


def assess(summary, suite, expected, exit_code):
    valid_counts = all(
        type(summary.get(key)) is int and summary[key] >= 0
        for key in ("passedTests", "failedTests", "skippedTests")
    )
    passed = (
        suite in SUITES
        and expected == SUITES[suite]
        and exit_code == 0
        and valid_counts
        and summary["passedTests"] == expected
        and summary["failedTests"] == 0
        and summary["skippedTests"] == 0
    )
    return {
        "schema_version": 1,
        "suite": suite,
        "selected_tests": "pass" if passed else "fail",
        "local_gui": ("pass" if passed else "fail") if suite == "full" else "not-run",
        "remote_and_providers": "not-run",
        "visual_reference_comparison": "not-run",
        "exploratory_agent": "not-run",
        "product_acceptance": "not-run" if passed else "fail",
        "counts": {key: summary.get(key) for key in ("passedTests", "failedTests", "skippedTests")},
        "test_process_exit": exit_code,
    }


def main():
    artifact_dir, suite, expected, exit_code = sys.argv[1:]
    root = Path(artifact_dir)
    try:
        summary = json.loads((root / "summary.json").read_text())
        if not isinstance(summary, dict):
            summary = {}
    except (OSError, ValueError):
        summary = {}
    report = assess(summary, suite, int(expected), int(exit_code))
    for filename, key in (("test-source-sha.txt", "test_source_sha"),
                          ("app-source-sha.txt", "app_source_sha")):
        path = root / filename
        value = path.read_text().strip() if path.is_file() else None
        report[key] = value if value and re.fullmatch(r"[a-f0-9]{40}", value) else None
    (root / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["selected_tests"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
