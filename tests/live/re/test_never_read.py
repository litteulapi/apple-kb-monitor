#!/usr/bin/env python3
"""The RE samplers never GET 0x4C or 0xFE by default (docs/FEATURES.md), WITHOUT hardware."""
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import sample_reports  # noqa: E402
import sample_reports_scan  # noqa: E402

NEVER = {0x4C, 0xFE}


class TestNeverRead(unittest.TestCase):
    def test_defaults_skip_never_read_ids(self):
        self.assertFalse(NEVER & set(sample_reports.IDS))
        self.assertFalse(NEVER & set(sample_reports_scan.ids_for(False, False)))
        self.assertFalse(NEVER & set(sample_reports_scan.ids_for(True, False)))
        self.assertEqual(len(sample_reports_scan.ids_for(True, False)), 254)

    def test_flag_opts_in(self):
        self.assertTrue(NEVER <= set(sample_reports_scan.ids_for(True, True)))
        self.assertTrue(NEVER <= set(sample_reports_scan.ids_for(False, True)))

    def test_ids_refused_before_opening_the_device(self):
        for ids in ("47,fe", "4c"):
            argv = ["sample_reports.py", "/dev/hidraw-none", "--ids", ids]
            with mock.patch.object(sys, "argv", argv), \
                    mock.patch("os.open", side_effect=AssertionError("opened")), \
                    mock.patch("sys.stderr"):
                with self.assertRaises(SystemExit) as e:
                    sample_reports.main()
            self.assertEqual(e.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
