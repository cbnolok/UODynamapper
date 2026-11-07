#!/usr/bin/env python3
"""Compare boundary discontinuities at frame edges and ordinary exported tile edges.

This is a statistical diagnostic, not a pass/fail rendering oracle: real scene
edges can also cause large discontinuities. Inputs are tiles/<level>/<x>/<y>.*.
"""
import argparse
import json
import random
from pathlib import Path

import numpy as np
from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tile_level", type=Path)
    parser.add_argument("--frame-tiles", type=int, required=True)
    parser.add_argument("--samples", type=int, default=4000)
    args = parser.parse_args()
    if args.frame_tiles <= 0 or args.samples <= 0:
        parser.error("frame-tiles and samples must be positive")
    tiles = {}
    for folder in sorted(args.tile_level.iterdir()):
        if not folder.is_dir() or not folder.name.isdigit():
            continue
        for path in sorted(folder.iterdir()):
            if path.stem.isdigit() and path.suffix.lower() in {".webp", ".png"}:
                tiles[(int(folder.name), int(path.stem))] = path
    pairs = [(key, (key[0] + 1, key[1]), "x") for key in tiles if (key[0] + 1, key[1]) in tiles]
    pairs += [(key, (key[0], key[1] + 1), "y") for key in tiles if (key[0], key[1] + 1) in tiles]
    if not pairs:
        parser.error("no adjacent tiles found")
    random.Random(1).shuffle(pairs)
    stats = {True: [], False: []}
    worst = []
    for first, second, axis in pairs[:args.samples]:
        with Image.open(tiles[first]) as image:
            a = np.asarray(image.convert("RGB"), dtype=np.int16)
        with Image.open(tiles[second]) as image:
            b = np.asarray(image.convert("RGB"), dtype=np.int16)
        if a.shape != b.shape or min(a.shape[:2]) < 2:
            parser.error("adjacent tiles must have equal sizes of at least 2x2")
        if axis == "x":
            edge, inner = np.abs(a[:, -1] - b[:, 0]).mean(), np.abs(a[:, -2] - a[:, -1]).mean()
        else:
            edge, inner = np.abs(a[-1] - b[0]).mean(), np.abs(a[-2] - a[-1]).mean()
        frame = (second[0] if axis == "x" else second[1]) % args.frame_tiles == 0
        excess = float(edge - inner)
        stats[frame].append(excess)
        worst.append((excess, frame, first, second, axis))
    summary = {}
    for frame, values in stats.items():
        summary["frame" if frame else "tile"] = {
            "boundaries": len(values),
            "mean_excess": float(np.mean(values)) if values else None,
            "p99_excess": float(np.percentile(values, 99)) if values else None,
        }
    summary["worst"] = sorted(worst, reverse=True)[:8]
    print(json.dumps(summary, sort_keys=True))


if __name__ == "__main__":
    main()
