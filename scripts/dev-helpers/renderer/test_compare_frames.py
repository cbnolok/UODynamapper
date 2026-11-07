import unittest

import numpy as np

from compare_frames import compare


class CompareFramesTests(unittest.TestCase):
    def test_translation_overlap_and_repeat_match(self):
        scene = np.arange(6 * 10 * 4, dtype=np.uint8).reshape(6, 10, 4)
        left, right = scene[:, :8], scene[:, 2:]
        result = compare(left, right, offset_x=2, guard=1)
        self.assertTrue(result["passed"])
        self.assertEqual(result["overlap"], [3, 1, 7, 5])
        self.assertTrue(compare(left, left)["passed"])

    def test_difference_and_blank_frames_fail(self):
        left = np.full((4, 4, 4), 255, dtype=np.uint8)
        right = left.copy()
        right[1, 1, 0] = 0
        self.assertEqual(compare(left, right)["differing_pixels"], 1)
        self.assertFalse(compare(left, right)["passed"])
        blank = np.zeros_like(left)
        self.assertFalse(compare(blank, blank)["passed"])

    def test_invalid_overlap_is_rejected(self):
        frame = np.ones((4, 4, 4), dtype=np.uint8)
        for options in [{"offset_x": 4}, {"guard": 2}, {"guard": -1}]:
            with self.assertRaises(ValueError):
                compare(frame, frame, **options)

    def test_repeat_rejects_a_truncated_candidate(self):
        reference = np.full((4, 4, 4), 255, dtype=np.uint8)
        truncated = reference[:1, :1]
        with self.assertRaisesRegex(ValueError, "equal dimensions"):
            compare(reference, truncated)
        self.assertTrue(compare(reference, truncated, overlap=True)["passed"])
        self.assertTrue(compare(reference, truncated, offset_x=1)["passed"])


if __name__ == "__main__":
    unittest.main()
