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
    return combine_sequence(original, [(continuation, resumed_run)], source, original_run)


def combine_sequence(original, continuations, source, original_run):
    runs = [original_run] + [run for _, run in continuations]
    if not continuations:
        raise ValueError("At least one GUI continuation is required")
    for run in runs:
        if (type(run.get("id")) is not int or run["id"] <= 0
                or type(run.get("run_attempt")) is not int or run["run_attempt"] <= 0
                or not re.fullmatch(r"[a-f0-9]{40}", run.get("head_sha", ""))):
            raise ValueError("Invalid evidence run identity")
    if len({run["id"] for run in runs}) != len(runs) or original_run["head_sha"] != source:
        raise ValueError("Different original build and continuation are required")
    before, digest, version = validate_evidence(original, source, source, "full")
    failed = {name for name, result in before.items() if result == "Failed"}
    if set(before) != CASES or not failed:
        raise ValueError("The original run must cover all required cases and contain failures")
    passed_in = {name: original_run["id"] for name in CASES - failed}
    for continuation, resumed_run in continuations:
        after, resumed_digest, resumed_version = validate_evidence(
            continuation, source, resumed_run["head_sha"], "selected")
        if not failed or set(after) != failed:
            raise ValueError("Each continuation must execute exactly the remaining failed cases")
        if digest != resumed_digest or version != resumed_version:
            raise ValueError("Installed DMG hash or version differs between runs")
        failed = {name for name, result in after.items() if result == "Failed"}
        if resumed_run.get("conclusion") != ("failure" if failed else "success"):
            raise ValueError("Continuation run verdict disagrees with its test results")
        passed_in.update({name: resumed_run["id"] for name in set(after) - failed})
    if failed:
        raise ValueError("The final continuation must pass every remaining case")
    return {
        "schema_version": 1, "kind": "completed_gui_cases", "local_gui": "pass",
        "product_acceptance": "not-run", "remote_and_providers": "not-run",
        "visual_reference_comparison": "not-run", "exploratory_agent": "not-run",
        "app_source_sha": source, "dmg_sha256": digest, "version": version,
        "runs": [{key: run[key] for key in ("id", "run_attempt", "head_sha")}
                 for run in runs],
        "cases": [{"name": name, "result": "Passed",
                   "run_id": passed_in[name]}
                  for name in sorted(CASES)],
    }
