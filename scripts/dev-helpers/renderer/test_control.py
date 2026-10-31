"""Readiness guards for native captures; no application or desktop required."""
import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('control', Path(__file__).with_name('control.py'))
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)


class SceneReadinessTests(unittest.TestCase):
    def test_terrain_ready_does_not_admit_pending_or_changing_art(self):
        ready = {'position': [2834, 987, 0, 0], 'rendered_position': [2834, 987, 0, 0],
                 'terrain': {'metadata_pending': False, 'metadata_revision': 8,
                             'submitted_metadata_revision': 8},
                 'art': {'map_id': 0, 'visible_chunks': 16, 'atlas_misses': 0,
                         'pending_pages': 0, 'emitted_instances': 100}}
        missing = copy.deepcopy(ready)
        missing['art']['atlas_misses'] = 2
        pending = copy.deepcopy(ready)
        pending['art']['pending_pages'] = 1
        changed = copy.deepcopy(ready)
        changed['art']['emitted_instances'] = 99
        sequence = [missing, pending, changed, ready, ready, ready]
        with patch.object(control, 'request', side_effect=sequence) as request, patch.object(control.time, 'sleep'):
            self.assertEqual(control.wait_for_scene('/unused', 10, ready['position']), ready)
            self.assertEqual(request.call_count, len(sequence))

    def test_unsettled_scene_times_out(self):
        with patch.object(control.time, 'monotonic', side_effect=[0, 0, 2]), \
                patch.object(control.time, 'sleep'), \
                patch.object(control, 'request', return_value={'position': [0, 0, 0, 0]}):
            with self.assertRaises(TimeoutError):
                control.wait_for_scene('/unused', 1)


if __name__ == '__main__':
    unittest.main()
