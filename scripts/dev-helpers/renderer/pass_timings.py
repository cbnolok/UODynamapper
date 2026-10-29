#!/usr/bin/env python3
"""Match built-in render-pass diagnostic logs to benchmark fixture windows."""
import argparse
from datetime import datetime, timedelta
import json
from pathlib import Path
import re
import statistics


def correlate(benchmark, log):
    reference = benchmark['fixtures'][0]['measurement_started_at_unix_seconds']
    day = datetime.fromtimestamp(reference).date()
    clock = None
    samples = []
    for line in re.sub(r'\x1b\[[0-9;]*m', '', log).splitlines():
        stamp = re.match(r'\s*(\d{2}:\d{2}:\d{2}) ', line)
        if stamp:
            local = datetime.combine(day, datetime.strptime(stamp[1], '%H:%M:%S').time())
            clock = min((local + timedelta(days=offset) for offset in [-1, 0, 1]),
                        key=lambda value: abs(value.timestamp() - reference)).timestamp()
        metric = re.search(r'Render \| total gpu ([\d.]+) ms \| main3d ([\d.]+) ms \| upscale ([\d.]+) ms', line)
        if metric and clock is not None:
            sample = {'logged_at_unix_seconds': clock,
                      'reported_gpu_ms': float(metric[1]),
                      'main_opaque_gpu_ms': float(metric[2]),
                      'upscaling_gpu_ms': float(metric[3])}
            fragments = re.search(r'\| frag main3d ([\d.]+)([KM]?)', line)
            if fragments:
                scale = {'': 1, 'K': 1000, 'M': 1000000}[fragments[2]]
                sample['main_opaque_fragments'] = float(fragments[1]) * scale
            samples.append(sample)
    result = {'label': benchmark['label'],
              'diagnostics': 'smoothed hardware durations; rounded console values; not raw frame percentiles',
              'window_guard_seconds': 1, 'fixtures': []}
    for fixture in benchmark['fixtures']:
        # Console timestamps have one-second resolution. Exclude both boundary
        # seconds rather than assigning a neighboring fixture's dump to this one.
        start = fixture['measurement_started_at_unix_seconds'] + 1
        end = fixture['measurement_finished_at_unix_seconds'] - 1
        selected = [sample for sample in samples if start <= sample['logged_at_unix_seconds'] <= end]
        if len(selected) < 3:
            raise ValueError(f"insufficient GPU pass diagnostics for {fixture['name']}: {len(selected)}")
        medians = {key: statistics.median(sample[key] for sample in selected)
                   for key in ['reported_gpu_ms', 'main_opaque_gpu_ms', 'upscaling_gpu_ms']}
        fragments = [sample['main_opaque_fragments'] for sample in selected if 'main_opaque_fragments' in sample]
        if fragments:
            medians['main_opaque_fragments'] = statistics.median(fragments)
        result['fixtures'].append({'name': fixture['name'], 'position': fixture['position'],
            'median_sampled_fps': fixture['median_sampled_fps'], 'samples': selected,
            'medians': medians})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--benchmark', type=Path, required=True)
    parser.add_argument('--app-log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='new JSON report')
    args = parser.parse_args()
    result = correlate(json.loads(args.benchmark.read_text()), args.app_log.read_text())
    with args.output.open('x') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    for fixture in result['fixtures']:
        print(f"{fixture['name']}: {fixture['medians']}")


if __name__ == '__main__':
    main()
