"""The CPU spikes of each walk (key press to release): the leaf spans of at least SPIKE_US, by label, GPU waits apart.
Usage: spikes.py TRACE KEYLOG [NAME,NAME,... one per walk]"""
import collections
import sys
from pathlib import Path

# `python3 -I` leaves the script's folder off the path.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from trace_file import CONTAINERS, read_spans, read_walks

SPIKE_US = 4000
LONG_SPIKE_US = 10_000
TOP_LABELS = 18
LABEL_CHARS = 110
# Waits for the GPU or the compositor, not work of the app's.
GPU_WAITS = ('Surface::get_current_texture', 'Device::wait', 'system:bevy_render::view::window::prepare_windows')

spans = read_spans(sys.argv[1])
names = sys.argv[3].split(',') if len(sys.argv) > 3 else None
for index, walk in enumerate(read_walks(sys.argv[2])):
    name = names[index] if names else f'walk {index}'
    stats = collections.defaultdict(list)
    for span in spans:
        if walk[0] <= span.start <= walk[1] and span.dur >= SPIKE_US and not span.label.startswith(CONTAINERS) \
                and span.label not in GPU_WAITS:
            stats[span.label].append(span.dur)
    print(f'== {name} ({(walk[1] - walk[0]) / 1e6:.1f} s): spikes >= {SPIKE_US / 1000:g} ms, '
          f'>= {LONG_SPIKE_US / 1000:g} ms, max ms, sum ms, label')
    for label, durs in sorted(stats.items(), key=lambda item: -sum(item[1]))[:TOP_LABELS]:
        print(f'  {len(durs):4d} {sum(d >= LONG_SPIKE_US for d in durs):4d} {max(durs) / 1000:7.1f} '
              f'{sum(durs) / 1000:8.1f}  {label[:LABEL_CHARS]}')
