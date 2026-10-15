"""Offline importer contract tests; no network or PDF framework required."""

import unittest

from import_oratory import parse_pdf


class ImportTests(unittest.TestCase):
    def test_preserves_q_not_bandwidth_and_nonconsecutive_source_ids(self):
        text = """Harman AE/OE 2018 Target
f2
900 Hz
Filter Settings
Filter Type Frequency Gain Q-Factor BW
Band 1 PEAK 31,5 Hz 7,9 dB 1,41 1,00 Band 2
Band 3 HIGH_SHELF 9000 Hz -2,0 dB 0,71 1,89
Band 4 Band 5 -8,6 dB
Adjust gain of band 3 to taste (airiness)
Deviation from Target
29.10.2019
"""
        eq, date, target, _, issues = parse_pdf(text)
        self.assertEqual(date, "2019-10-29")
        self.assertEqual(target, "Harman AE/OE 2018")
        self.assertEqual(eq, {
            "preamp_gain_db": -8.6,
            "filters": [
                {"id": 1, "kind": "peaking", "frequency_hz": 31.5,
                 "gain_db": 7.9, "quality_factor": 1.41},
                {"id": 3, "kind": "high_shelf", "frequency_hz": 9000.0,
                 "gain_db": -2.0, "quality_factor": 0.71, "control_name": "Airiness"},
            ],
        })
        self.assertEqual(issues, [
            "Source plot references unconfigured bands [2]; retained only the explicit table rows"
        ])

    def test_omits_invalid_and_unterminated_recommendations_without_losing_eq(self):
        eq, _, _, _, issues = parse_pdf("""Harman AE/OE 2018 Target
Filter Settings
Band 1 PEAK 2950 Hz -5,2 dB 2,5 0,57
Band 2 HIGH_SHELF 10000 Hz 2,0 dB 0,71 1,89
-2,0 dB
Adjust gain of band 1 to preference (Bass)
Adjust gain of band 7 to preference (treble presence)
Adjust gain of band 1 to preference (missing closing parenthesis
Adjust gain of band 2 to preference (airiness)
2025-05-17
""")
        self.assertEqual(len(eq["filters"]), 2)
        self.assertNotIn("control_name", eq["filters"][0])
        self.assertEqual(eq["filters"][1]["control_name"], "Airiness")
        self.assertEqual(len(issues), 3)

    def test_invalid_or_unsupported_bands_block_the_entire_preset(self):
        for band in [
            "Band 2 LOW_SHELF 110 Hz 10,0 dB -0,71",
            "Band 2 LOW_PASS 12000 Hz 0,0 dB 0,71",
            "Band 2 PEAK 200 Hz 1,0 dB",  # missing Q
            "Band 2\nPEAK 200 Hz 1,0 dB",  # missing Q with wrapped row
        ]:
            with self.subTest(band=band), self.assertRaises(ValueError):
                parse_pdf(f"""Harman AE/OE 2018 Target
Filter Settings
Band 1 PEAK 95 Hz -1,5 dB 1,4 1,01
{band}
-5,1 dB
16.01.21
""")

    def test_legacy_column_order_and_undated_source(self):
        eq, date, _, _, issues = parse_pdf("""Frequency Q-Factor Gain Filter Type
Band 1 8,5 Hz 0,5 10,0 dB PEAK
Band 2 170 Hz 0,3 -9,5 dB PEAK
-9,3 dB
Filter Settings
Harman 2015 Target
""")
        self.assertEqual(eq["filters"][0]["frequency_hz"], 8.5)
        self.assertEqual(eq["filters"][0]["quality_factor"], 0.5)
        self.assertEqual(eq["filters"][0]["gain_db"], 10.0)
        self.assertEqual(eq["preamp_gain_db"], -9.3)
        self.assertEqual(date, "undated")
        self.assertEqual(len(issues), 1)


if __name__ == "__main__":
    unittest.main()
