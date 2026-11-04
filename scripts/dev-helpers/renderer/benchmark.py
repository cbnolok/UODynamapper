#!/usr/bin/env python3
"""Sample a running renderer's smoothed diagnostics across teleport fixtures."""
import argparse
import json
from pathlib import Path
import statistics
import time

from control import request


def fixture(value):
    try:
        name, x, y = value.split(':')
        return name, int(x), int(y)
    except ValueError as error:
        raise argparse.ArgumentTypeError('expected name:x:y') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path, help='new JSON report')
    parser.add_argument('--label', required=True, help='record build, preset, source and camera')
    parser.add_argument('--fixture', required=True, action='append', type=fixture)
    parser.add_argument('--map', type=int, default=0)
    parser.add_argument('--z', type=int, default=0)
    parser.add_argument('--settle', type=float, default=8)
    parser.add_argument('--seconds', type=float, default=20)
    parser.add_argument('--interval', type=float, default=0.25)
    args = parser.parse_args()
    if not (args.settle >= 0 and args.seconds > 0 and args.interval > 0):
        parser.error('settle must be nonnegative; seconds and interval must be positive')
    report = {'label': args.label, 'diagnostics': 'smoothed samples, not raw frame percentiles',
              'settle_seconds': args.settle, 'sample_seconds': args.seconds,
              'interval_seconds': args.interval, 'fixtures': []}
    # Reserve the destination before moving the app. Never overwrite evidence.
    with args.output.open('x') as output:
        for name, x, y in args.fixture:
            request(args.socket, {'command': 'teleport', 'x': x, 'y': y,
                                  'z': args.z, 'map': args.map}, 10)
            time.sleep(args.settle)
            samples = []
            started = time.monotonic()
            started_at = time.time()
            while time.monotonic() - started < args.seconds:
                status = request(args.socket, {'command': 'status'}, 10)
                expected = [x, y, args.z, args.map]
                if status['position'] != expected or status['rendered_position'] != expected:
                    raise RuntimeError(f'fixture moved or not rendered: {status}')
                status['sampled_at_unix_seconds'] = time.time()
                samples.append(status)
                time.sleep(args.interval)
            fps = [sample['fps'] for sample in samples if sample['fps'] is not None]
            if not fps:
                raise RuntimeError(f'no frame diagnostics for {name}')
            summary = {'name': name, 'position': expected, 'samples': samples,
                       'elapsed_seconds': time.monotonic() - started,
                       'measurement_started_at_unix_seconds': started_at,
                       'measurement_finished_at_unix_seconds': time.time(),
                       'median_sampled_fps': statistics.median(fps),
                       'min_sampled_fps': min(fps), 'max_sampled_fps': max(fps)}
            report['fixtures'].append(summary)
            print(f"{name}: median {summary['median_sampled_fps']:.1f} FPS "
                  f"(500 ms window samples {min(fps):.1f}..{max(fps):.1f})", flush=True)
        json.dump(report, output, indent=2)
        output.write('\n')


if __name__ == '__main__':
    main()
