"""For each function named, the perf samples under it by direct callee and by leaf: where a span's time goes.
Usage: perf_callees.py PERF_SCRIPT_TXT FUNCTION...   (the text of `perf script -F tid,ip,sym`)"""
import collections
import re
import sys

TOP = 12
SYMBOL_CHARS = 110
# Generic-argument nesting stripped from symbols, innermost first.
GENERIC_DEPTH = 3
INDENTED_FRAME = ' ' * 8


def stacks(path):
    frames = []
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if not line.strip():
            if frames:
                yield frames
            frames = []
            continue
        if line.startswith('\t') or line.startswith(INDENTED_FRAME):
            parts = line.strip().split(None, 1)
            symbol = parts[1] if len(parts) > 1 else '?'
            symbol = re.sub(r'\+0x[0-9a-f]+$', '', symbol)
            symbol = re.sub(r'::h[0-9a-f]{16}$', '', symbol)
            frames.append(symbol)
    if frames:
        yield frames


def short(symbol):
    for _ in range(GENERIC_DEPTH):
        symbol = re.sub(r'<[^<>]*>', '', symbol)
    return symbol[-SYMBOL_CHARS:]


path, targets = sys.argv[1], sys.argv[2:]
total = collections.Counter()
callees = {target: collections.Counter() for target in targets}
leaves = {target: collections.Counter() for target in targets}
for frames in stacks(path):
    for target in targets:
        index = next((i for i, frame in enumerate(frames) if target in frame), None)
        if index is None:
            continue
        total[target] += 1
        callees[target][short(frames[index - 1]) if index > 0 else '(self)'] += 1
        leaves[target][short(frames[0])] += 1
for target in targets:
    samples = total[target]
    print(f'==== {target}: {samples} samples')
    for title, counter in (('direct callees', callees[target]), ('leaves', leaves[target])):
        print(f'  {title}:')
        for symbol, count in counter.most_common(TOP):
            print(f'   {100 * count / samples:5.1f}%  {symbol}')
