import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from report import CASES, assess, matching_test_cases, validate_selection


FULL_COUNT = 16


class AcceptanceReportTests(unittest.TestCase):
    def test_duplicate_wrong_failed_or_absent_case_cannot_replace_a_required_case(self):
        one = {"nodeType": "Test Case", "nodeIdentifier": "Example/testOne()", "result": "Passed"}
        two = {"nodeType": "Test Case", "nodeIdentifier": "Example/testTwo()", "result": "Passed"}
        expected = {"Example/testOne", "Example/testTwo"}
        def tree(nodes):
            return {"testNodes": [{"nodeType": "Test Suite", "children": nodes}]}
        self.assertTrue(matching_test_cases(tree([one, two]), expected))
        for nodes in ([], [one], [one, one], [one, two, two],
                      [one, dict(two, result="Skipped")], [one, dict(two, result="Failed")],
                      [one, dict(two, nodeIdentifier="Example/testOther()")]):
            self.assertFalse(matching_test_cases(tree(nodes), expected))

    def test_report_requires_artifact_identity_even_with_green_tests(self):
        script = Path(__file__).with_name("report.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "summary.json").write_text(json.dumps({
                "passedTests": FULL_COUNT, "failedTests": 0, "skippedTests": 0}))
            (root / "test-cases.json").write_text(json.dumps({"testNodes": [
                {"nodeType": "Test Case", "nodeIdentifier": case + "()", "result": "Passed"}
                for case in CASES]}))
            for digest, revision, expected_exit in (
                ("", "", 1), ("a" * 64, "", 1),
                ("invalid", "b" * 40, 1), ("a" * 64, "b" * 40, 0),
            ):
                with self.subTest(digest=digest, revision=revision):
                    (root / "dmg.sha256").write_text(digest)
                    (root / "test-source-sha.txt").write_text(revision)
                    result = subprocess.run([sys.executable, str(script), str(root),
                                             "full", str(FULL_COUNT), "0"], capture_output=True)
                    self.assertEqual(result.returncode, expected_exit, result.stderr)
                    report = json.loads((root / "acceptance.json").read_text())
                    self.assertEqual(report["local_gui"], "pass" if expected_exit == 0 else "fail")
                    self.assertNotEqual(report["product_acceptance"], "pass")

    def test_selection_rejects_empty_unknown_duplicate_and_option_injection(self):
        for value in ("", "NavigationTests/testUnknown", "--skip-testing:ZaplexUITests",
                      "NavigationTests/testSplitUp,NavigationTests/testSplitUp"):
            self.assertFalse(validate_selection(value))
        self.assertTrue(validate_selection("NavigationTests/testSplitUp,NavigationTests/testSplitDown"))

    def result(self, **changes):
        summary = {"passedTests": FULL_COUNT, "failedTests": 0, "skippedTests": 0}
        summary.update(changes)
        return assess(summary, "full", FULL_COUNT, 0)

    def test_green_local_suite_cannot_claim_remote_visual_or_agent_acceptance(self):
        result = self.result()
        self.assertEqual(result["local_gui"], "pass")
        for key in ("remote_and_providers", "visual_reference_comparison",
                    "exploratory_agent", "product_acceptance"):
            self.assertEqual(result[key], "not-run")

    def test_empty_skipped_failed_incomplete_and_extra_tests_fail(self):
        for changes in ({"passedTests": 0}, {"skippedTests": 1}, {"failedTests": 1},
                        {"passedTests": FULL_COUNT - 1}, {"passedTests": FULL_COUNT + 1}, {"passedTests": str(FULL_COUNT)},
                        {"failedTests": False}, {"expectedFailures": 1}):
            with self.subTest(changes=changes):
                self.assertEqual(self.result(**changes)["selected_tests"], "fail")
        self.assertEqual(assess({}, "full", FULL_COUNT, 0)["selected_tests"], "fail")

    def test_exit_failure_cannot_be_hidden_by_green_summary(self):
        summary = {"passedTests": FULL_COUNT, "failedTests": 0, "skippedTests": 0}
        self.assertEqual(assess(summary, "full", FULL_COUNT, 65)["local_gui"], "fail")

    def test_subset_and_changed_expected_count_cannot_pass_full_gate(self):
        summary = {"passedTests": 3, "failedTests": 0, "skippedTests": 0}
        result = assess(summary, "terminal", 3, 0)
        self.assertEqual(result["selected_tests"], "pass")
        self.assertEqual(result["local_gui"], "not-run")
        self.assertEqual(assess(summary, "full", 3, 0)["selected_tests"], "fail")
        self.assertEqual(assess(summary, "unknown", 3, 0)["selected_tests"], "fail")


if __name__ == "__main__":
    unittest.main()
