#!/usr/bin/env python3
"""Compare mask orientations against neighboring cells in an original packed map.

Inputs come from inspect_ec_scene_cells and inspect_ec_item_aux. This measures
coverage continuity, not original-client pixel parity. Requires Pillow and numpy.
"""

import argparse
import csv
from pathlib import Path
import re

import numpy as np
from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cells", type=Path, required=True)
    parser.add_argument("--refs", type=Path, required=True)
    parser.add_argument("--images", type=Path, required=True)
    args = parser.parse_args()
    sources = {}
    item = None
    for line in args.refs.read_text().splitlines():
        if match := re.match(r"item=(\d+) ", line):
            item = int(match[1])
            sources[item] = None
        elif match := re.match(r"\s+source=3 id=(\d+) ", line):
            if item is None:
                raise ValueError("source reference without item")
            sources[item] = int(match[1])
    masks = {}
    for item, source in sources.items():
        if source is None:
            masks[item] = np.ones((64, 64))
        else:
            with Image.open(args.images / f"aux-{source}.png") as image:
                masks[item] = np.asarray(image.getchannel("A").resize((64, 64))) / 255.0
    with args.cells.open(newline="") as stream:
        rows = {(int(row["x"]), int(row["y"])): row for row in csv.DictReader(stream)}
    results = []
    for mirror in (False, True):
        for rotation in range(4):
            tiles = {}
            for xy, row in rows.items():
                coverage = np.full((64, 64), float(row["land_wet"] == "true"))
                for water in filter(None, row["wet_items"].split(";")):
                    mask = masks[int(water.split("@")[0])]
                    if mirror:
                        mask = np.fliplr(mask)
                    coverage = np.maximum(coverage, np.rot90(mask, rotation))
                tiles[xy] = coverage
            differences = []
            for (x, y), tile in tiles.items():
                if (x + 1, y) in tiles:
                    differences.append(np.mean(abs(tile[:, -1] - tiles[x + 1, y][:, 0])))
                if (x, y + 1) in tiles:
                    differences.append(np.mean(abs(tile[-1, :] - tiles[x, y + 1][0, :])))
            if not differences:
                raise ValueError("no adjacent cells to compare")
            results.append((float(np.mean(differences)), mirror, rotation))
    print("mean_alpha_edge_difference,mirror_x,quarter_turns")
    for difference, mirror, rotation in sorted(results):
        print(f"{difference:.9f},{str(mirror).lower()},{rotation}")


if __name__ == "__main__":
    main()
