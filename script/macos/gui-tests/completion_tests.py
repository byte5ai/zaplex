import copy
import unittest

from completion import CASES, combine


SOURCE = "a" * 40
ORIGINAL = {"id": 123, "run_attempt": 1, "head_sha": SOURCE}
RESUMED = {"id": 456, "run_attempt": 2, "head_sha": "b" * 40}
FAILED = "NavigationTests/testSplitDown"


def evidence(cases, suite, revision):
    passed = all(result == "Passed" for result in cases.values())
    counts = {"passedTests": list(cases.values()).count("Passed"),
              "failedTests": list(cases.values()).count("Failed"), "skippedTests": 0}
    return {
        "acceptance": {"schema_version": 1, "suite": suite, "counts": counts,
                       "test_process_exit": 0 if passed else 65,
                       "app_source_sha": SOURCE, "test_source_sha": revision,
                       "dmg_sha256": "c" * 64, "required_cases": sorted(cases),
                       "executed_cases_match": passed,
                       "selected_tests": "pass" if passed else "fail",
                       "local_gui": "fail" if suite == "full" else "not-run"},
        "summary": dict(counts, totalTestCount=len(cases), expectedFailures=0,
                        result="Passed" if passed else "Failed"),
        "test-cases": {"testNodes": [{"nodeType": "Test Suite", "children": [
            {"nodeType": "Test Case", "nodeIdentifier": name + "()", "result": result}
            for name, result in sorted(cases.items())]}]},
        "app-info": {"WarpVersion": "v1.2.34"},
    }


class CompletionTests(unittest.TestCase):
    def setUp(self):
        self.before = evidence({name: "Failed" if name == FAILED else "Passed" for name in CASES},
                               "full", SOURCE)
        self.after = evidence({FAILED: "Passed"}, "selected", RESUMED["head_sha"])

    def combine(self):
        return combine(self.before, self.after, SOURCE, ORIGINAL, RESUMED)

    def test_exact_failed_case_completes_all_cases_without_rewriting_originals(self):
        originals = copy.deepcopy((self.before, self.after))
        result = self.combine()
        self.assertEqual(result["local_gui"], "pass")
        self.assertEqual(result["product_acceptance"], "not-run")
        self.assertEqual(len(result["cases"]), 15)
        self.assertEqual({case["name"] for case in result["cases"]}, CASES)
        self.assertEqual([case["name"] for case in result["cases"] if case["run_id"] == 456], [FAILED])
        self.assertEqual((self.before, self.after), originals)

    def test_case_tree_cannot_omit_duplicate_skip_or_replace_a_case(self):
        for target in ("before", "after"):
            for change in ("missing", "duplicate", "skipped", "unknown", "unfinished"):
                with self.subTest(target=target, change=change):
                    self.setUp()
                    nodes = getattr(self, target)["test-cases"]["testNodes"][0]["children"]
                    if change == "missing":
                        nodes.pop()
                    elif change == "duplicate":
                        nodes.append(copy.deepcopy(nodes[0]))
                    elif change == "unknown":
                        nodes[0]["nodeIdentifier"] = "Other/testCase()"
                    else:
                        nodes[0]["result"] = "Skipped" if change == "skipped" else "Not Run"
                    with self.assertRaises(ValueError):
                        self.combine()

    def test_different_app_test_revision_digest_or_version_is_rejected(self):
        for target in ("before", "after"):
            for key, value in (("app_source_sha", "d" * 40), ("test_source_sha", "e" * 40),
                               ("dmg_sha256", "f" * 64), ("dmg_sha256", "")):
                with self.subTest(target=target, key=key):
                    self.setUp()
                    getattr(self, target)["acceptance"][key] = value
                    with self.assertRaises(ValueError):
                        self.combine()
        self.setUp()
        self.after["app-info"]["WarpVersion"] = "v1.2.35"
        with self.assertRaises(ValueError):
            self.combine()

    def test_counts_verdicts_and_exit_must_agree_with_the_tree(self):
        for target in ("before", "after"):
            for section, key, value in (
                ("summary", "passedTests", 999), ("summary", "totalTestCount", 99),
                ("summary", "skippedTests", 1), ("summary", "expectedFailures", 1),
                ("summary", "result", "Unknown"), ("acceptance", "test_process_exit", True),
                ("acceptance", "required_cases", []), ("acceptance", "suite", "navigation"),
                ("acceptance", "counts", {}), ("acceptance", "selected_tests", "unknown"),
            ):
                with self.subTest(target=target, section=section, key=key):
                    self.setUp()
                    getattr(self, target)[section][key] = value
                    with self.assertRaises(ValueError):
                        self.combine()

    def test_partial_retry_or_repetition_of_passed_cases_does_not_complete(self):
        another = next(name for name in CASES if name != FAILED)
        for cases in ({another: "Passed"}, {FAILED: "Passed", another: "Passed"},
                      {FAILED: "Failed"}, {}):
            self.after = evidence(cases, "selected", RESUMED["head_sha"])
            with self.assertRaises(ValueError):
                self.combine()
        self.before = evidence({name: "Failed" if name in (FAILED, another) else "Passed" for name in CASES},
                               "full", SOURCE)
        self.after = evidence({FAILED: "Passed"}, "selected", RESUMED["head_sha"])
        with self.assertRaises(ValueError):
            self.combine()

    def test_invalid_same_or_unbound_run_identity_is_rejected(self):
        for run in (dict(RESUMED, id=123), dict(RESUMED, id=True),
                    dict(RESUMED, run_attempt=0), dict(RESUMED, head_sha="main")):
            with self.assertRaises(ValueError):
                combine(self.before, self.after, SOURCE, ORIGINAL, run)
        with self.assertRaises(ValueError):
            combine(self.before, self.after, SOURCE, dict(ORIGINAL, head_sha="f" * 40), RESUMED)


if __name__ == "__main__":
    unittest.main()
