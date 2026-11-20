"""The long frames of one walk (key press to release) and the leaf spans inside them.
Usage: long_frames.py TRACE KEYLOG [WALK_INDEX, default the last]"""
import collections
import sys
from pathlib import Path

# `python3 -I` leaves the script's folder off the path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from trace_file import CONTAINERS, FRAME_LABEL, MAIN_APP_LABEL, RENDER_APP_LABEL, read_spans, read_walks

LONG_FRAME_US = 33_000
TOP_PER_FRAME = 5
TOP_OVERALL = 30
# A frame counts toward a label's hits when the label took at least this long in it.
HIT_US = 5000
LABEL_CHARS = 120

spans = read_spans(sys.argv[1])
walk = read_walks(sys.argv[2])[int(sys.argv[3]) if len(sys.argv) > 3 else -1]
frames = sorted(span.start for span in spans if span.label == FRAME_LABEL and walk[0] <= span.start <= walk[1])
gaps = list(zip(frames, frames[1:]))
long = [(a, b) for a, b in gaps if b - a > LONG_FRAME_US]
print(f'walk {(walk[1] - walk[0]) / 1e6:.1f} s, frames {len(gaps)}, long {len(long)}, '
      f'median {sorted(b - a for a, b in gaps)[len(gaps) // 2] / 1000:.2f} ms')
total, hits = collections.Counter(), collections.Counter()
for a, b in long:
    inside, main_busy, render_busy = collections.Counter(), 0, 0
    for span in spans:
        lo, hi = max(a, span.start), min(b, span.start + span.dur)
        if hi <= lo:
            continue
        if span.label == MAIN_APP_LABEL:
            main_busy += hi - lo
        elif span.label == RENDER_APP_LABEL:
            render_busy += hi - lo
        elif not span.label.startswith(CONTAINERS):
            inside[span.label] += hi - lo
    print(f'\n-- +{(a - walk[0]) / 1e6:6.2f} s: {(b - a) / 1000:5.1f} ms   main busy {main_busy / 1000:5.1f}  '
          f'render busy {render_busy / 1000:5.1f}')
    for label, us in inside.most_common(TOP_PER_FRAME):
        print(f'   {us / 1000:7.1f} ms  {label[:LABEL_CHARS]}')
    for label, us in inside.items():
        total[label] += us
        if us >= HIT_US:
            hits[label] += 1
print(f'\n== leaf spans over all long frames: ms summed, frames where it took >= {HIT_US / 1000:g} ms')
for label, us in total.most_common(TOP_OVERALL):
    print(f'{us / 1000:9.1f} ms {hits[label]:4d}  {label[:LABEL_CHARS]}')
