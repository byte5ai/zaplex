#!/usr/bin/env python3
"""Report local GUI execution without claiming unexecuted product acceptance."""

import json
import re
import sys
from pathlib import Path


SUITES = {"full": 14, "terminal": 3, "navigation": 8, "surfaces": 2}
CASES = {
    "SmokeTests/testTerminalExecutesAndRendersOutput",
    "SmokeTests/testNewWindowRunsItsOwnShell",
    "SmokeTests/testNativeCloseButtonClosesWindow",
    "SmokeTests/testRelaunchAcceptsTerminalInput",
    "NavigationTests/testTabsKeepIndependentShells",
    "NavigationTests/testClosingTabPreservesOriginalShell",
    "NavigationTests/testSplitRight", "NavigationTests/testSplitLeft",
    "NavigationTests/testSplitDown", "NavigationTests/testSplitUp",
    "NavigationTests/testCancelSplitKeepsOriginalShell",
    "NavigationTests/testSettingsReturnToOriginalShell",
    "SurfaceTests/testFileManagerUsesPaneDirectoryAndReturnsToShell",
    "SurfaceTests/testConnectionsPanelKeepsTerminalUsable",
}


def validate_selection(selection):
    cases = selection.split(",")
    return len(cases) == len(set(cases)) and set(cases) <= CASES


def matching_test_cases(tree, expected):
    """Use Xcode's executed test IDs, not only an aggregate green count."""
    found = []

    def visit(node):
        if not isinstance(node, dict):
            return
        if node.get("nodeType") == "Test Case":
            identifier = node.get("nodeIdentifier", "")
            if identifier.endswith("()"):
                identifier = identifier[:-2]
            found.append((identifier, node.get("result")))
        for child in node.get("children", []):
            visit(child)

    for node in tree.get("testNodes", []):
        visit(node)
    return (bool(expected) and len(found) == len(expected)
            and {identifier for identifier, _ in found} == set(expected)
            and all(result == "Passed" for _, result in found))


def expected_cases(suite, root):
    if suite == "full":
        return CASES
    if suite == "selected":
        selection = (root / "selected-cases.txt").read_text().strip()
        return set(selection.split(",")) if validate_selection(selection) else set()
    prefix = {"terminal": "SmokeTests/", "navigation": "NavigationTests/",
              "surfaces": "SurfaceTests/"}.get(suite)
    return {case for case in CASES if prefix and case.startswith(prefix)
            and (suite != "terminal" or case != "SmokeTests/testNativeCloseButtonClosesWindow")}


def assess(summary, suite, expected, exit_code):
    valid_counts = all(
        type(summary.get(key)) is int and summary[key] >= 0
        for key in ("passedTests", "failedTests", "skippedTests")
    )
    passed = (
        ((suite in SUITES and expected == SUITES[suite])
         or (suite == "selected" and 0 < expected <= len(CASES)))
        and exit_code == 0
        and valid_counts
        and summary["passedTests"] == expected
        and summary["failedTests"] == 0
        and summary["skippedTests"] == 0
        and summary.get("expectedFailures", 0) == 0
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
    if sys.argv[1] == "--validate-selection":
        return 0 if validate_selection(sys.argv[2]) else 1
    artifact_dir, suite, expected, exit_code = sys.argv[1:]
    root = Path(artifact_dir)
    try:
        summary = json.loads((root / "summary.json").read_text())
        if not isinstance(summary, dict):
            summary = {}
    except (OSError, ValueError):
        summary = {}
    report = assess(summary, suite, int(expected), int(exit_code))
    try:
        required_cases = expected_cases(suite, root)
        test_tree = json.loads((root / "test-cases.json").read_text())
        cases_match = (int(expected) == len(required_cases)
                       and isinstance(test_tree, dict)
                       and matching_test_cases(test_tree, required_cases))
    except (OSError, ValueError):
        cases_match = False
        required_cases = set()
    report["required_cases"] = sorted(required_cases)
    report["executed_cases_match"] = cases_match
    digest_file = root / "dmg.sha256"
    digest_parts = digest_file.read_text().split() if digest_file.is_file() else []
    digest = digest_parts[0] if digest_parts else ""
    report["dmg_sha256"] = digest if re.fullmatch(r"[a-f0-9]{64}", digest) else None
    for filename, key in (("test-source-sha.txt", "test_source_sha"),
                          ("app-source-sha.txt", "app_source_sha")):
        path = root / filename
        value = path.read_text().strip() if path.is_file() else None
        report[key] = value if value and re.fullmatch(r"[a-f0-9]{40}", value) else None
    if report["dmg_sha256"] is None or report["test_source_sha"] is None or not cases_match:
        report["selected_tests"] = "fail"
        report["local_gui"] = "fail" if suite == "full" else "not-run"
        report["product_acceptance"] = "fail"
        report["error"] = "Missing artifact identity or incomplete/mismatched executed test cases"
    (root / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["selected_tests"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
