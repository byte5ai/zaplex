#!/usr/bin/env python3
"""Exercise suite argument selection without starting a macOS GUI or build."""

import os
from pathlib import Path
import subprocess
import unittest

RUN = Path(__file__).with_name("run")
SELECTION = RUN.read_text().split('mkdir -p "$art"', 1)[0]
PRINT = '\nprintf "%s\\n" "$suite" "$expected" "$#"; for arg in "$@"; do printf "%s\\n" "$arg"; done\n'


class LauncherTests(unittest.TestCase):
    def invoke(self, suite, selection=""):
        return subprocess.run(
            [os.environ.get("TEST_BASH", "bash"), "-c", SELECTION + PRINT,
             str(RUN), "/unused/evidence", suite, selection],
            capture_output=True, text=True, check=False,
        )

    def test_full_runs_all_fifteen_without_filter_or_empty_argument(self):
        result = self.invoke("full")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["full", "16", "0"])

    def test_diagnostic_suites_have_exact_filters(self):
        cases = {
            "terminal": ["3", "2", "-only-testing:ZaplexUITests/SmokeTests",
                         "-skip-testing:ZaplexUITests/SmokeTests/testNativeCloseButtonClosesWindow"],
            "navigation": ["8", "1", "-only-testing:ZaplexUITests/NavigationTests"],
            "surfaces": ["4", "1", "-only-testing:ZaplexUITests/SurfaceTests"],
        }
        for suite, expected in cases.items():
            with self.subTest(suite=suite):
                result = self.invoke(suite)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), [suite] + expected)

    def test_selected_cases_replace_the_suite_filter(self):
        cases = ["NavigationTests/testSplitRight", "NavigationTests/testSplitLeft"]
        result = self.invoke("navigation", ",".join(cases))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["selected", "2", "2"] +
                         [f"-only-testing:ZaplexUITests/{case}" for case in cases])

    def test_full_acceptance_cannot_be_narrowed(self):
        self.assertNotEqual(self.invoke("full", "NavigationTests/testSplitRight").returncode, 0)

    def test_invalid_or_duplicate_selection_is_rejected(self):
        for selection in ("NavigationTests/doesNotExist",
                          "NavigationTests/testSplitRight,NavigationTests/testSplitRight"):
            self.assertNotEqual(self.invoke("navigation", selection).returncode, 0)

    def test_unknown_suite_is_rejected(self):
        self.assertNotEqual(self.invoke("unknown").returncode, 0)


if __name__ == "__main__":
    unittest.main()
