"""stride_hand_compare.py TSV...: the strides of `udd-cli tool strides measure --report` tables (or its stdout lines)
against the strides counted by hand (docs/agent_notes/mobile_animation_motion.md, "Planted-foot measurement"): per clip
the facings, the stride and the difference; then the mean |diff| and the count within 0.1."""
import sys
import numpy as np

HAND = [('cc', 400, 0, 1.3), ('cc', 400, 2, 2.15), ('cc', 401, 0, 1.3), ('cc', 401, 2, 2.15),
        ('ec', 400, 1, 1.4), ('ec', 400, 3, 1.75), ('ec', 400, 15, 1.3), ('ec', 401, 1, 1.4), ('ec', 401, 3, 1.75),
        ('cc', 200, 0, 1.05), ('cc', 200, 1, 2.4), ('cc', 217, 0, 0.95), ('cc', 225, 0, 1.0),
        ('cc', 17, 0, 1.3), ('cc', 1, 0, 2.25), ('cc', 9, 0, 2.8),
        ('ec', 200, 22, 1.36), ('ec', 200, 24, 3.8), ('ec', 217, 22, 0.91), ('ec', 225, 22, 1.55),
        ('ec', 17, 22, 2.4), ('ec', 1, 22, 2.55), ('ec', 9, 22, 3.13)]
TOLERANCE = 0.1

rows = {}
for path in sys.argv[1:]:
    for line in open(path):
        f = line.rstrip('\n').split('\t')
        if len(f) < 8 or f[0] not in ('cc', 'ec'):
            continue
        rows[(f[0], int(f[1]), int(f[2]))] = f[3:7]
diffs = []
for src, body, action, hand in HAND:
    r = rows.get((src, body, action))
    if r is None:
        print(f'{src} {body:4d} a{action:<3d} hand {hand:.2f}  (no row)')
        continue
    stride = None if r[3] == '-' else float(r[3])
    d = '' if stride is None else f'{stride - hand:+.2f}'
    if stride is not None:
        diffs.append(abs(stride - hand))
    print(f'{src} {body:4d} a{action:<3d} hand {hand:.2f}  facings {r[0]:>6} {r[1]:>6} {r[2]:>6}  stride {r[3]:>6}  {d}')
d = np.array(diffs)
print(f'mean |diff| {d.mean():.3f}, max {d.max():.2f}, within {TOLERANCE}: {(d <= TOLERANCE + 1e-9).sum()}/{len(d)} '
      f'of {len(HAND)}')
