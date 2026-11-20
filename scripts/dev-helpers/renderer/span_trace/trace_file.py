"""Reads a span trace (`DYNAMAPPER_SPAN_TRACE`, app/span_trace.rs) and the key log `bench.py` writes beside it."""
from collections import namedtuple

Span = namedtuple('Span', 'thread start dur label')
# Spans that only contain others: a frame, a schedule, an app, the render driver. Leaf analyses skip them.
CONTAINERS = ('update', 'schedule:', 'sub_app', 'main_app', 'system:bevy_render::run_render_schedule', 'system:bevy_app',
              'system:bevy_time::fixed', 'main_render_schedule', 'camera_schedule',
              'system:bevy_core_pipeline::schedule::camera_driver')
FRAME_LABEL = 'update'
MAIN_APP_LABEL = 'main_app'
RENDER_APP_LABEL = 'sub_app:RenderApp'


def read_spans(path):
    """The spans, their starts in Unix microseconds."""
    spans, t0 = [], 0
    for line in open(path):
        parts = line.split()
        if parts and parts[0] == 'T0_UNIX_US':
            t0 = int(parts[1])
        elif len(parts) >= 4:
            spans.append(Span(parts[0], t0 + int(parts[1]), int(parts[2]), parts[3]))
    return spans


def read_walks(path):
    """Each walk of the key log: (press, release) in Unix microseconds."""
    keys = [(int(line.split()[1]), line.split()[2]) for line in open(path) if line.startswith('KEY')]
    presses = [time for time, state in keys if state == 'press']
    releases = [time for time, state in keys if state == 'release']
    return list(zip(presses, releases))
