"""Combine actual failed/passed GUI evidence without inventing an XCTest run."""

import json
from pathlib import Path
import re
import runpy

report = runpy.run_path(str(Path(__file__).with_name("report.py")))
CASES = report["CASES"]


def read_evidence(root):
    return {name: json.loads((root / f"{name}.json").read_text())
            for name in ("acceptance", "summary", "test-cases", "app-info")}


def case_results(tree):
    found = {}

    def visit(node):
        if not isinstance(node, dict):
            raise ValueError("Invalid test node")
        if node.get("nodeType") == "Test Case":
            name = node.get("nodeIdentifier", "").removesuffix("()")
            result = node.get("result")
            if name not in CASES or name in found or result not in ("Passed", "Failed"):
                raise ValueError("Unknown, duplicate, skipped or unfinished test case")
            found[name] = result
        for child in node.get("children", []):
            visit(child)

    for node in tree.get("testNodes", []):
        visit(node)
    return found


def validate_evidence(evidence, source, test_source, suite):
    receipt, summary = (evidence[key] for key in ("acceptance", "summary"))
    cases = case_results(evidence["test-cases"])
    counts = {"passedTests": list(cases.values()).count("Passed"),
              "failedTests": list(cases.values()).count("Failed"), "skippedTests": 0}
    if (not cases or any(type(summary.get(key)) is not int or summary[key] != value
                         for key, value in counts.items())
            or receipt.get("counts") != counts or summary.get("expectedFailures", 0) != 0
            or summary.get("totalTestCount") != len(cases)):
        raise ValueError("Test tree, summary and receipt counts differ")
    passed = all(result == "Passed" for result in cases.values())
    if (type(receipt.get("test_process_exit")) is not int
            or receipt["test_process_exit"] != (0 if passed else 65)
            or summary.get("result") != ("Passed" if passed else "Failed")):
        raise ValueError("Process exit and summary disagree with executed cases")
    expected = {"schema_version": 1, "suite": suite,
                "app_source_sha": source, "test_source_sha": test_source,
                "required_cases": sorted(cases), "executed_cases_match": passed,
                "selected_tests": "pass" if passed else "fail",
                "local_gui": ("pass" if passed else "fail") if suite == "full" else "not-run"}
    if any(receipt.get(key) != value for key, value in expected.items()):
        raise ValueError("GUI receipt identity or verdict differs")
    digest = receipt.get("dmg_sha256", "")
    version = evidence["app-info"].get("WarpVersion", "")
    if not re.fullmatch(r"[a-f0-9]{64}", digest) or not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[\w.-]+)?", version):
        raise ValueError("Invalid installed artifact identity")
    return cases, digest, version


def combine(original, continuation, source, original_run, resumed_run):
    for run in (original_run, resumed_run):
        if (type(run.get("id")) is not int or run["id"] <= 0
                or type(run.get("run_attempt")) is not int or run["run_attempt"] <= 0
                or not re.fullmatch(r"[a-f0-9]{40}", run.get("head_sha", ""))):
            raise ValueError("Invalid evidence run identity")
    if original_run["id"] == resumed_run["id"] or original_run["head_sha"] != source:
        raise ValueError("Different original build and continuation are required")
    before, digest, version = validate_evidence(original, source, source, "full")
    after, resumed_digest, resumed_version = validate_evidence(
        continuation, source, resumed_run["head_sha"], "selected")
    failed = {name for name, result in before.items() if result == "Failed"}
    if (set(before) != CASES or not failed or set(after) != failed
            or any(result != "Passed" for result in after.values())):
        raise ValueError("Continuation must pass exactly all failed required cases")
    if digest != resumed_digest or version != resumed_version:
        raise ValueError("Installed DMG hash or version differs between runs")
    return {
        "schema_version": 1, "kind": "completed_gui_cases", "local_gui": "pass",
        "product_acceptance": "not-run", "remote_and_providers": "not-run",
        "visual_reference_comparison": "not-run", "exploratory_agent": "not-run",
        "app_source_sha": source, "dmg_sha256": digest, "version": version,
        "runs": [{key: run[key] for key in ("id", "run_attempt", "head_sha")}
                 for run in (original_run, resumed_run)],
        "cases": [{"name": name, "result": "Passed",
                   "run_id": resumed_run["id"] if name in failed else original_run["id"]}
                  for name in sorted(CASES)],
    }
