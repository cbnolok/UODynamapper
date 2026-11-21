"""Joins the strides of the RGBA and BC7 packages on body and action and prints the spread of their differences.

Usage: python3 -I -B compare_strides.py <dir> cc ec, where <dir> holds `udd-cli tool strides export` tables named
rgba_cc.tsv, bc7_cc.tsv, rgba_ec.tsv and bc7_ec.tsv.
"""
import sys

NONE = "-"
WORST = 12
THRESHOLDS = [0.01, 0.02, 0.05, 0.1, 0.25]


def load(path):
    rows = {}
    for line in open(path):
        if line.startswith("#") or line.startswith("source"):
            continue
        f = line.rstrip("\n").split("\t")
        rows[(f[1], f[2])] = (f[6], f[7], f[3:6])
    return rows


def number(v):
    return None if v == NONE else float(v)


def quantile(sorted_values, q):
    if not sorted_values:
        return float("nan")
    i = q * (len(sorted_values) - 1)
    lo = int(i)
    hi = min(lo + 1, len(sorted_values) - 1)
    return sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * (i - lo)


for source in sys.argv[2:]:
    rgba = load(f"{sys.argv[1]}/rgba_{source}.tsv")
    bc7 = load(f"{sys.argv[1]}/bc7_{source}.tsv")
    keys = sorted(set(rgba) | set(bc7), key=lambda k: (int(k[0]), int(k[1])))
    both, only_rgba, only_bc7, same_hash, diffs = 0, [], [], 0, []
    for k in keys:
        a = number(rgba[k][0]) if k in rgba else None
        b = number(bc7[k][0]) if k in bc7 else None
        if a is not None and b is not None:
            both += 1
            if rgba[k][1] == bc7[k][1]:
                same_hash += 1
            diffs.append((b - a, abs(b - a) / a, k, a, b, rgba[k][2], bc7[k][2]))
        elif a is not None:
            only_rgba.append(k)
        elif b is not None:
            only_bc7.append(k)
    absolute = sorted(abs(d[0]) for d in diffs)
    relative = sorted(d[1] for d in diffs)
    print(f"{source}: RGBA {sum(1 for k in rgba if number(rgba[k][0]) is not None)} strides, "
          f"BC7 {sum(1 for k in bc7 if number(bc7[k][0]) is not None)}; both {both} "
          f"(same frame hash {same_hash}), RGBA only {len(only_rgba)}, BC7 only {len(only_bc7)}")
    print("  |difference| tiles: median {:.4f}, p90 {:.4f}, p99 {:.4f}, max {:.4f}; mean signed {:+.4f}".format(
        quantile(absolute, 0.5), quantile(absolute, 0.9), quantile(absolute, 0.99), absolute[-1],
        sum(d[0] for d in diffs) / len(diffs)))
    print("  relative: median {:.2%}, p90 {:.2%}, p99 {:.2%}, max {:.2%}".format(
        quantile(relative, 0.5), quantile(relative, 0.9), quantile(relative, 0.99), relative[-1]))
    print("  " + ", ".join(f"> {t:.0%}: {sum(1 for r in relative if r > t)}" for t in THRESHOLDS))
    exact = sum(1 for d in diffs if d[0] == 0)
    print(f"  identical strides: {exact}")
    print("  worst (body action: RGBA -> BC7, facings S/SW/W RGBA | BC7):")
    for d in sorted(diffs, key=lambda d: -d[1])[:WORST]:
        print(f"    {d[2][0]:>5} {d[2][1]:>3}: {d[3]:.3f} -> {d[4]:.3f} ({d[0] / d[3]:+.1%})  "
              f"{'/'.join(d[5])} | {'/'.join(d[6])}")
    if only_rgba:
        print(f"  RGBA only: {only_rgba[:20]}")
    if only_bc7:
        print(f"  BC7 only: {only_bc7[:20]}")
    # Per facing: the measurement itself, before the median picks one.
    facing_diffs, lost, gained = [], 0, 0
    for k in set(rgba) & set(bc7):
        if number(rgba[k][0]) is None or number(bc7[k][0]) is None:
            continue
        for a, b in zip(rgba[k][2], bc7[k][2]):
            a, b = number(a), number(b)
            if a is not None and b is not None:
                facing_diffs.append(abs(b - a) / max(abs(a), 1e-6))
            elif a is not None:
                lost += 1
            elif b is not None:
                gained += 1
    facing_diffs.sort()
    print(f"  facings: {len(facing_diffs)} measured in both, relative |difference| median "
          f"{quantile(facing_diffs, 0.5):.2%}, p90 {quantile(facing_diffs, 0.9):.2%}, p99 {quantile(facing_diffs, 0.99):.2%}; "
          f"> 10%: {sum(1 for r in facing_diffs if r > 0.1)}; measured only in RGBA {lost}, only in BC7 {gained}")
