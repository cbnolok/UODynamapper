"""Runs the renderer's frame benchmark, logging each key press and release as `KEY <unix_us> <state>` to $KEYLOG, so the
span trace's walks can be found. Arguments are frame_benchmark.py's; set DYNAMAPPER_SPAN_TRACE as well (README.md)."""
import os
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO / 'skills/renderer-reference-validation/scripts'))
import frame_benchmark as fb  # noqa: E402

original_request = fb.BenchLaunch.request


def request(self, payload):
    if payload.get('command') == 'key':
        with open(os.environ['KEYLOG'], 'a') as log:
            log.write(f"KEY {int(time.time() * 1e6)} {payload['state']}\n")
    return original_request(self, payload)


fb.BenchLaunch.request = request
fb.main(sys.argv[1:])
