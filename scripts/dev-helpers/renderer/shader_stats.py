#!/usr/bin/env python3
"""Extract Mesa RADV compiler statistics from an owned preview's app log."""
import argparse
import json
from pathlib import Path
import re


def extract(log):
    log = re.sub(r'\x1b\[[0-9;]*m', '', log)
    entries = []
    for match in re.finditer(r'\*\*\* SHADER STATS \*\*\*(.*?)\*{10,}', log, re.S):
        preceding = log[:match.start()].rstrip().splitlines()
        stage = preceding[-1].removesuffix(':') if preceding else 'unknown'
        fields = {}
        for line in match[1].splitlines():
            field = re.fullmatch(r'([^:]+):\s*(\d+)', line.strip())
            if field:
                key = re.sub(r'[^a-z0-9]+', '_', field[1].lower()).strip('_')
                fields[key] = int(field[2])
        if 'code_size' in fields:
            entries.append({'stage': stage, **fields})
    if not entries:
        raise ValueError('no RADV shader statistics found in preview log')
    return entries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='new JSON report')
    parser.add_argument('--label', required=True, help='identify build and shader variant')
    args = parser.parse_args()
    try:
        entries = extract(args.app_log.read_text())
    except ValueError as error:
        parser.error(f'{error}; RADV may omit statistics for reused pipeline cache entries')
    report = {'label': args.label, 'app_log': str(args.app_log),
              'diagnostics': 'compiler statistics, not executed instruction counts or GPU durations',
              'shaders': entries}
    with args.output.open('x') as output:
        json.dump(report, output, indent=2)
        output.write('\n')
    for entry in sorted(entries, key=lambda value: value['code_size'], reverse=True)[:5]:
        print(json.dumps(entry))


if __name__ == '__main__':
    main()
