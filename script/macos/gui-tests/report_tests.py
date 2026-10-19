import unittest

from report import assess, validate_selection


class AcceptanceReportTests(unittest.TestCase):
    def test_selection_rejects_empty_unknown_duplicate_and_option_injection(self):
        for value in ("", "NavigationTests/testUnknown", "--skip-testing:ZaplexUITests",
                      "NavigationTests/testSplitUp,NavigationTests/testSplitUp"):
            self.assertFalse(validate_selection(value))
        self.assertTrue(validate_selection("NavigationTests/testSplitUp,NavigationTests/testSplitDown"))

    def result(self, **changes):
        summary = {"passedTests": 14, "failedTests": 0, "skippedTests": 0}
        summary.update(changes)
        return assess(summary, "full", 14, 0)

    def test_green_local_suite_cannot_claim_remote_visual_or_agent_acceptance(self):
        result = self.result()
        self.assertEqual(result["local_gui"], "pass")
        for key in ("remote_and_providers", "visual_reference_comparison",
                    "exploratory_agent", "product_acceptance"):
            self.assertEqual(result[key], "not-run")

    def test_empty_skipped_failed_incomplete_and_extra_tests_fail(self):
        for changes in ({"passedTests": 0}, {"skippedTests": 1}, {"failedTests": 1},
                        {"passedTests": 13}, {"passedTests": 15}, {"passedTests": "14"},
                        {"failedTests": False}):
            with self.subTest(changes=changes):
                self.assertEqual(self.result(**changes)["selected_tests"], "fail")
        self.assertEqual(assess({}, "full", 14, 0)["selected_tests"], "fail")

    def test_exit_failure_cannot_be_hidden_by_green_summary(self):
        summary = {"passedTests": 14, "failedTests": 0, "skippedTests": 0}
        self.assertEqual(assess(summary, "full", 14, 65)["local_gui"], "fail")

    def test_subset_and_changed_expected_count_cannot_pass_full_gate(self):
        summary = {"passedTests": 3, "failedTests": 0, "skippedTests": 0}
        result = assess(summary, "terminal", 3, 0)
        self.assertEqual(result["selected_tests"], "pass")
        self.assertEqual(result["local_gui"], "not-run")
        self.assertEqual(assess(summary, "full", 3, 0)["selected_tests"], "fail")
        self.assertEqual(assess(summary, "unknown", 3, 0)["selected_tests"], "fail")


if __name__ == "__main__":
    unittest.main()
