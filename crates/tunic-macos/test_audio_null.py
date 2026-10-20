import tempfile
import unittest
import wave
from pathlib import Path

from audio_null import compare, stimulus


class AudioNullTest(unittest.TestCase):
    def test_latency_is_removed_but_gain_channel_swaps_and_dropouts_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            reference = Path(directory) / "reference.wav"
            recorded = Path(directory) / "recorded.wav"
            stimulus(reference, 8000)
            with wave.open(str(reference), "rb") as source:
                params = source.getparams()
                data = source.readframes(source.getnframes())

            def capture(payload):
                with wave.open(str(recorded), "wb") as output:
                    output.setparams(params)
                    output.writeframes(b"\0" * (73 * 6) + payload)

            capture(data)
            result = compare(reference, recorded)
            self.assertTrue(result["passed"])
            self.assertEqual(result["latency_frames"], 73)
            self.assertEqual(result["channels"][0]["peak_error"], 0)
            self.assertEqual(result["channels"][1]["peak_error"], 0)

            capture(b"".join(data[index + 3:index + 6] + data[index:index + 3]
                             for index in range(0, len(data), 6)))
            self.assertFalse(compare(reference, recorded)["passed"])

            capture(b"".join((int.from_bytes(data[index:index + 3], "little", signed=True) // 2)
                             .to_bytes(3, "little", signed=True) for index in range(0, len(data), 3)))
            result = compare(reference, recorded)
            self.assertFalse(result["passed"])
            self.assertAlmostEqual(result["channels"][0]["gain_db"], -6.0206, places=3)

            # Drop one frame midway; a latency fit must not hide this discontinuity.
            middle = 3 * 8000 * 6
            capture(data[:middle] + data[middle + 6:])
            self.assertFalse(compare(reference, recorded)["passed"])
            capture(b"\0" * len(data))
            with self.assertRaisesRegex(ValueError, "silent"):
                compare(reference, recorded)


if __name__ == "__main__":
    unittest.main()
