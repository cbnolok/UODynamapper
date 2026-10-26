"""Check recovered terrain byte/edge rules against small independent fixtures."""

import unittest

from terrain_coverage import coverage_weights, layer_sort_key, reconstruct_coverage


class CoverageTests(unittest.TestCase):
    def test_source_parameter_bytes_and_truncation(self):
        self.assertEqual(coverage_weights(0), (0, 0))
        self.assertEqual(coverage_weights(-1), (0, 0))
        self.assertEqual(coverage_weights(1), (242, 153))
        self.assertEqual(coverage_weights(5), (197, 19))
        self.assertEqual(coverage_weights(0.15), (253, 236))
        with self.assertRaises(ValueError):
            coverage_weights(float("nan"))

    def test_isolated_membership_cardinals_and_diagonals(self):
        cells = [[0] * 34 for _ in range(34)]
        cells[17][17] = 1
        mask = reconstruct_coverage(cells, 1, 1)
        for y in range(32):
            for x in range(32):
                dx, dy = abs(x - 16), abs(y - 16)
                expected = 0
                if dx == dy == 0:
                    expected = 255
                elif dx + dy == 1:
                    expected = 242
                elif dx == dy == 1:
                    expected = 153
                self.assertEqual(mask[y * 32 + x], expected, (x, y))
        hard = reconstruct_coverage(cells, 1, 0)
        self.assertEqual(hard.count(255), 1)
        self.assertEqual(hard.count(0), 1023)

    def test_chunk_edge_membership_requires_external_ownership(self):
        cells = [[1] * 34 for _ in range(34)]
        self.assertEqual(reconstruct_coverage(cells, 1, 1), bytes([255] * 1024))
        # An entirely different western chunk suppresses the western mask edge.
        for row in cells:
            row[0] = 0
        mask = reconstruct_coverage(cells, 1, 1)
        self.assertTrue(all(mask[y * 32] == 0 for y in range(32)))
        self.assertTrue(all(mask[y * 32 + 1] == 255 for y in range(32)))
        self.assertEqual(reconstruct_coverage(cells, 1, 0), bytes([255] * 1024))
        # The source stores float32: a positive double can serialize as zero.
        self.assertEqual(reconstruct_coverage(cells, 1, 1e-50), bytes([255] * 1024))
        # A mismatching diagonal outside the chunk affects only its corner.
        cells = [[1] * 34 for _ in range(34)]
        cells[0][0] = 0
        mask = reconstruct_coverage(cells, 1, 1)
        self.assertEqual(mask[0], 0)
        self.assertEqual(mask.count(0), 1)

    def test_edge_dilation_requires_both_axis_neighbors(self):
        cells = [[0] * 34 for _ in range(34)]
        cells[17][2] = 1  # x=1; cannot dilate horizontally onto x=0.
        mask = reconstruct_coverage(cells, 1, 1)
        self.assertEqual(mask[16 * 32], 0)
        self.assertEqual(mask[16 * 32 + 2], 242)
        self.assertEqual(mask[15 * 32 + 2], 153)
        with self.assertRaises(ValueError):
            reconstruct_coverage([[0]], 1, 1)

    def test_original_layer_sort_keys(self):
        def material(shader, id):
            return dict(id=id, texture=dict(shader_name=shader))
        self.assertEqual(layer_sort_key(material("UOWaterTerrainLayer", 5)), 5000)
        self.assertEqual(layer_sort_key(material("UODefaultTerrainLayer", 70)), 70)
        self.assertEqual(layer_sort_key(material("UOBumpMapTerrainLayer", 12)), 12)
        with self.assertRaises(ValueError):
            layer_sort_key(material("UnverifiedShader", 1))


if __name__ == "__main__":
    unittest.main()
