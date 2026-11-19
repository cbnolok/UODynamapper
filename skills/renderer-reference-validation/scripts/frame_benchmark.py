#!/usr/bin/env python3
"""Frame time benchmark of one or more Dynamapper binaries: the land's frag_depth cost and any other GPU cost change.

Not a test: GPU timings vary with the machine, its clocks and whatever else runs, so it measures, it does not judge.
It reruns the benchmark of the ClassicUO draw order (docs/dev_wiki/rendering/DEPTH_SCALE_AND_KR_CLIENT.md, "Benchmark"):
a 1900 x 1000 window at zoom 0.5, no frame limiter, no vsync, the afternoon preset, two views:

  land-mountains   map 1, 2542, 488, z -5, statics off: the land's fragment cost alone
  city-britain     map 1, 1496, 1628, z 10: a dense city, land and statics

Each view runs in one or more motions: `still` (the default) keeps the player in place, which isolates the GPU cost;
`walk` holds the move-north key through warmup and sampling (20 tiles a second, so about 220 tiles), which adds
the CPU paths a still camera never runs: chunk streaming, statics collection, light collection, texture pinning and
uploads.

For each repetition, camera, style and binary, one launch through the visual harness's Launch (Desktop 4, the
exact-PID preview helper, isolated settings, the preview lock held for that launch only) visits both views and samples
the app's measured frame time (status.frame_ms, one sample per SAMPLE_INTERVAL_SECONDS, new frames only) and the
amdgpu busy percentage for SAMPLE_SECONDS each, after WARMUP_SECONDS. The binaries' order rotates with the
repetition, so none always runs first. Each launch's figure is the median of its samples; each cell of the table, the
median of the launches. It also counts the hitches (status.long_frames: the frames longer than 33, 66 and 100 ms) from
the press of the walk key to the end of the sampling, summed over the launches, with the longest frame sampled.

  frame_benchmark.py run OUT.json --binary NAME=BINARY[@TREE] ... [--repetitions N] [--camera ...] [--style ...]
                                   [--motion still|walk ...]
  frame_benchmark.py table OUT.json

`run` appends to OUT.json, so runs can be split and resumed; `table` prints the Markdown table, the first binary
named in OUT.json as the baseline. TREE is the checkout whose dynamapper/assets the launch copies (the shaders load
from it at runtime), the repository by default.
"""
import argparse
import json
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import visual_harness as vh  # noqa: E402

WINDOW = (1900, 1000)
ZOOM = 0.5
TIME_OF_DAY = 'afternoon'
WARMUP_SECONDS = 3.0
SAMPLE_SECONDS = 8.0
SAMPLE_INTERVAL_SECONDS = 0.5
DEFAULT_REPETITIONS = 3
GPU_BUSY = Path('/sys/class/drm/card0/device/gpu_busy_percent')
DEFAULT_ROOT = vh.SCRATCH_ROOT / 'frame-benchmark'
# (name, map, x, y, z, statics)
VIEWS = (('land-mountains', 1, 2542, 488, -5, False), ('city-britain', 1, 1496, 1628, 10, True))
# style -> (art, land)
STYLE_SOURCES = {'classic': ('cc', 'cc'), 'enhanced': ('ec', 'ec'), 'kr': ('ec', 'kr'), 'painterly': ('ec', 'ec')}
DEFAULT_STYLES = ('classic', 'kr')
STYLE_LABELS = {'classic': 'Classic', 'enhanced': 'Enhanced', 'kr': 'KR-like', 'painterly': 'UOD-Painterly'}
MOTIONS = ('still', 'walk')
# The key the walk holds; player_movement.rs moves north while it is pressed, one tile per step at 20 steps a second.
WALK_KEY = 'KeyW'


def view_fixture(camera, view):
    name, map_id, x, y, z, statics = view
    return vh.Fixture(name=name, guards='frame benchmark', tier='full', tags=(), map=map_id, x=x, y=y, z=z,
                      zoom=ZOOM, camera=camera, times=(TIME_OF_DAY,), styles=(), sources={}, quick_sources={},
                      settings={'world_rendering': {'enable_statics': statics}}, tolerance=None)


class BenchLaunch(vh.Launch):
    """A live launch, uncapped: no frame limiter, no vsync."""

    def __init__(self, ctx, launch_id, captures, motion):
        super().__init__(ctx, launch_id, captures, live=True)
        self.motion = motion
        prefs = self.dir / 'assets/settings/user_preferences.toml'
        prefs.write_text(vh.toml_set(prefs.read_text(), 'performance', 'frame_limit_enabled', False)[0])
        graphics = self.dir / 'assets/settings/graphics.toml'
        graphics.write_text(vh.toml_set(graphics.read_text(), 'graphics', 'vsync', False)[0])

    def sample(self):
        frames, busy, longest = [], [], []
        end = time.monotonic() + SAMPLE_SECONDS
        last = None
        while time.monotonic() < end:
            status = self.request({'command': 'status'})
            total = status.get('total_frames')
            if status.get('frame_ms') and total != last:
                frames.append(status['frame_ms'])
                if status.get('longest_frame_ms'):
                    longest.append(status['longest_frame_ms'])
                last = total
            try:
                busy.append(int(GPU_BUSY.read_text()))
            except (OSError, ValueError):
                pass
            time.sleep(SAMPLE_INTERVAL_SECONDS)
        return frames, busy, longest

    def bench(self):
        results = {}
        with self.preview_lock():
            self.start()
            try:
                self.wait_ready(self.captures[0])
                state = vh.live_state(self.ctx.files, self.captures[0])
                for capture in self.captures:
                    self.switch(capture, state)
                    self.wait_ready(capture)
                    walking = self.motion == 'walk'
                    long_before = self.request({'command': 'status'}).get('long_frames') or {}
                    if walking:
                        self.request({'command': 'key', 'key': WALK_KEY, 'state': 'press'})
                    try:
                        time.sleep(WARMUP_SECONDS)
                        frames, busy, longest = self.sample()
                    finally:
                        if walking:
                            self.request({'command': 'key', 'key': WALK_KEY, 'state': 'release'})
                    if not frames:
                        raise vh.HarnessError(f'{capture.key}: no frame time sampled')
                    long_after = self.request({'command': 'status'}).get('long_frames') or {}
                    # The hitches from the press of the walk key to the end of the sampling, warmup included.
                    long_frames = {key: count - long_before.get(key, 0) for key, count in long_after.items()}
                    results[capture.fixture.name] = {'frame_ms': frames, 'gpu_busy': busy,
                                                     'long_frames': long_frames, 'longest_ms': longest}
                    vh.log(f'  {capture.key}: frame {statistics.median(frames):.2f} ms, '
                           f'gpu busy {statistics.median(busy) if busy else "-"}%, long frames {long_frames}, '
                           f'longest {max(longest, default=0):.1f} ms')
            finally:
                self.stop()
        return results


def parse_binary(text):
    name, separator, rest = text.partition('=')
    if not separator or not name or not rest:
        raise argparse.ArgumentTypeError('NAME=BINARY[@TREE]')
    binary, _, tree = rest.partition('@')
    binary, tree = Path(binary), Path(tree) if tree else vh.REPO
    if not binary.is_file():
        raise argparse.ArgumentTypeError(f'{binary}: no such binary')
    if not (tree / 'dynamapper/assets').is_dir():
        raise argparse.ArgumentTypeError(f'{tree}: no dynamapper/assets')
    return name, binary, tree


class LaunchArgs:
    """The fields of the harness's arguments a Context reads."""

    def __init__(self, root, packages, binary, tree):
        self.root, self.window, self.packages, self.binary, self.tree = root, WINDOW, packages, binary, tree


def run(args):
    if 'DISPLAY' not in vh.os.environ:
        raise vh.HarnessError('no DISPLAY: the preview needs the X11/XWayland display')
    names = [name for name, _, _ in args.binary]
    if len(set(names)) != len(names):
        raise vh.HarnessError('two binaries share a name')
    rows = json.loads(args.out.read_text()) if args.out.exists() else []
    sockets = args.root / 'sk'
    sockets.mkdir(parents=True, exist_ok=True)
    # control.py binds its reply sockets under tempfile's folder: keep that path short and off the RAM disk.
    vh.tempfile.tempdir = str(sockets)
    styles = args.style or DEFAULT_STYLES
    cameras = args.camera or vh.CAMERAS
    first = max((row['repetition'] for row in rows), default=-1) + 1
    motions = args.motion or MOTIONS[:1]
    for repetition in range(first, first + args.repetitions):
        shift = repetition % len(args.binary)
        for camera in cameras:
            for style in styles:
                art, land = STYLE_SOURCES[style]
                for motion in motions:
                    for name, binary, tree in args.binary[shift:] + args.binary[:shift]:
                        ctx = vh.Context(LaunchArgs(args.root, args.packages, binary, tree), f'bench-{name}')
                        captures = [vh.Capture(view_fixture(camera, view), style, TIME_OF_DAY, art, land)
                                    for view in VIEWS]
                        launch_id = f'r{repetition}-{camera}-{style}-{motion}-{name}'
                        vh.log(launch_id)
                        for view, data in BenchLaunch(ctx, launch_id, captures, motion).bench().items():
                            rows.append({'repetition': repetition, 'camera': camera, 'style': style, 'art': art,
                                         'land': land, 'motion': motion, 'binary': name, 'view': view, **data})
                        args.out.write_text(json.dumps(rows, indent=1))


def threshold_ms(key):
    """The milliseconds of a `long_frames` key (`over_66ms`), for ordering them."""
    return int(key.removeprefix('over_').removesuffix('ms'))


def table(args):
    rows = json.loads(args.out.read_text())
    binaries = list(dict.fromkeys(row['binary'] for row in rows))
    cells = {}
    for row in rows:
        key = (row['camera'], row['style'], row['view'], row.get('motion', 'still'), row['binary'])
        cell = cells.setdefault(key, {'frame': [], 'busy': [], 'long': {}, 'longest': []})
        cell['frame'].append(statistics.median(row['frame_ms']))
        for threshold, count in row.get('long_frames', {}).items():
            cell['long'][threshold] = cell['long'].get(threshold, 0) + count
        cell['longest'] += row.get('longest_ms', [])
        if row['gpu_busy']:
            cell['busy'].append(statistics.median(row['gpu_busy']))
    base = binaries[0]
    header = ['Camera', 'Style', 'View', 'Motion'] + [f'Frame ms {name} (range)' for name in binaries]
    header += [f'{name} vs {base}' for name in binaries[1:]] + ['GPU busy ' + ' / '.join(binaries)]
    header += [f'Frames over 33/66/100 ms {name} (longest ms)' for name in binaries]
    print('| ' + ' | '.join(header) + ' |')
    print('|' + '---|' * len(header))
    sources = {row['style']: f"{row['art']}/{row['land']}" for row in rows}
    for camera, style, view, motion in sorted({key[:4] for key in cells}):
        found = [cells.get((camera, style, view, motion, name)) for name in binaries]
        medians = [statistics.median(cell['frame']) if cell else None for cell in found]
        line = [camera, f'{STYLE_LABELS[style]} {sources[style]}', view, motion]
        line += [f"{median:.2f} ({min(cell['frame']):.2f}-{max(cell['frame']):.2f})" if cell else '-'
                 for median, cell in zip(medians, found)]
        line += [f'{(median - medians[0]) / medians[0] * 100:+.1f}%' if median and medians[0] else '-'
                 for median in medians[1:]]
        line.append(' / '.join(f"{statistics.median(cell['busy']):.0f}%" if cell and cell['busy'] else '-'
                               for cell in found))
        line += [f"{'/'.join(str(cell['long'][key]) for key in sorted(cell['long'], key=threshold_ms))} "
                 f"({max(cell['longest']):.0f})"
                 if cell and cell['long'] and cell['longest'] else '-' for cell in found]
        print('| ' + ' | '.join(line) + ' |')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    run_parser = commands.add_parser('run', help='launch and sample the binaries, appending to OUT.json')
    run_parser.add_argument('out', type=Path)
    run_parser.add_argument('--binary', action='append', type=parse_binary, required=True,
                            metavar='NAME=BINARY[@TREE]')
    run_parser.add_argument('--repetitions', type=int, default=DEFAULT_REPETITIONS)
    run_parser.add_argument('--camera', action='append', choices=vh.CAMERAS)
    run_parser.add_argument('--style', action='append', choices=tuple(STYLE_SOURCES))
    run_parser.add_argument('--motion', action='append', choices=MOTIONS, help='still (default) and/or walk')
    run_parser.add_argument('--packages', type=Path, default=vh.DEFAULT_PACKAGES)
    run_parser.add_argument('--root', type=Path, default=DEFAULT_ROOT, help=f'work folders (default {DEFAULT_ROOT})')
    table_parser = commands.add_parser('table', help='the Markdown table of OUT.json')
    table_parser.add_argument('out', type=Path)
    args = parser.parse_args(argv)
    try:
        run(args) if args.command == 'run' else table(args)
    except vh.HarnessError as error:
        print(f'error: {error}', file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
