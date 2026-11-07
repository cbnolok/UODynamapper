"""Capture a list of views under one variant of the KR-like afternoon preset. Run under the preview lock (see rig.py).

Usage: python3 views.py RUN_NAME VARIANT VIEW_SET
  VARIANT a = the preset as shipped, b = every term the client lacks switched off, c = b plus a shadow stand-in.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from rig import Run  # noqa: E402

STATUS_PRINT_CHARS = 3000

# (label, x, y, z, map); all on Felucca.
VIEW_SETS = {
    'sheet': [
        ('town_britain', 1434, 1699, 0, 0),
        ('forest_yew', 620, 1020, 0, 0),
        ('cliff_covetous', 2460, 880, 0, 0),
        ('town_minoc', 2518, 561, 0, 0),
        ('slopes_5', 3000, 400, 0, 0),
        ('slopes_6', 5650, 2950, 0, 0),
        ('slopes_7', 2280, 1170, 0, 0),
        ('slopes_8', 5800, 3100, 0, 0),
    ],
}

name, variant, view_set = sys.argv[1:4]
views = VIEW_SETS[view_set]
with Run(name, variant, position=views[0][1:]) as r:
    status = r.wait_in_game()
    print(json.dumps({'preset': status.get('render', {}).get('preset'), 'window': status.get('window')}))
    # The first capture after start-up is discarded: shader compilation and uploads may still be settling.
    r.capture('0_discard')
    for label, x, y, z, map_id in views:
        status = r.teleport(x, y, z, map_id)
        print(label, 'position', status.get('position'))
        r.capture(label)
    print(json.dumps({'render': r.status().get('render')})[:STATUS_PRINT_CHARS])
