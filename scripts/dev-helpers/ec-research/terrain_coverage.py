#!/usr/bin/env python3
"""Reconstruct original EC 32x32 terrain masks from exported ownership evidence.

This models UOSA.exe 461060's byte output, not its final render or resampling.
Input comes from the inspect_ec_terrain_coverage Rust example. Pillow is needed
only to export PNGs; the reconstruction and its tests use the standard library.
"""

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import struct


SIDE = 32
# These are the executable's double constants, promoted from float values.
CARDINAL_BASE = float.fromhex("0x1.e666660000000p-1")
DIAGONAL_BASE = float.fromhex("0x1.3333340000000p-1")


def coverage_weights(parameter):
    # The original parameter is a serialized float, used as a double exponent.
    parameter = struct.unpack("<f", struct.pack("<f", parameter))[0]
    if not math.isfinite(parameter):
        raise ValueError("non-finite terrain blend parameter")
    if parameter <= 0:
        return 0, 0
    # 4611e4..461209 / 46122d..46124f set x87 rounding to truncation.
    return tuple(math.trunc(255.0 * base ** parameter)
                 for base in (CARDINAL_BASE, DIAGONAL_BASE))


def layer_sort_key(material):
    shader = (material.get("texture") or {}).get("shader_name")
    if shader == "UOWaterTerrainLayer":
        return material["id"] * 1000
    if shader in ("UODefaultTerrainLayer", "UOBumpMapTerrainLayer", "UOTerrainShader"):
        return material["id"]
    raise ValueError(f"unverified terrain layer class: {shader!r}")


def reconstruct_coverage(cells, material_id, parameter):
    """Return row-major mask bytes for a 32-square chunk plus one-cell border."""
    if len(cells) != SIDE + 2 or any(len(row) != SIDE + 2 for row in cells):
        raise ValueError("ownership must contain 34 rows of 34 material IDs")
    parameter = struct.unpack("<f", struct.pack("<f", parameter))[0]
    cardinal, diagonal = coverage_weights(parameter)
    ownership = []
    for y in range(SIDE):
        row = []
        for x in range(SIDE):
            owned = cells[y + 1][x + 1] == material_id
            if parameter > 0:
                # At chunk edges, the membership test also requires all adjacent
                # out-of-chunk cells (three at a corner) to belong to this layer.
                dxs = (-1, 0) if x == 0 else (0, 1) if x == SIDE - 1 else (0,)
                dys = (-1, 0) if y == 0 else (0, 1) if y == SIDE - 1 else (0,)
                owned = owned and all(cells[y + 1 + dy][x + 1 + dx] == material_id
                                     for dy in dys for dx in dxs)
            row.append(owned)
        ownership.append(row)

    pixels = bytearray()
    for y in range(SIDE):
        for x in range(SIDE):
            value = 255 if ownership[y][x] else 0
            # 4612d5..46134f requires BOTH neighbors in an axis to be in bounds.
            # It is not ordinary dilation with independently clamped neighbors.
            horizontal = 0 < x < SIDE - 1
            vertical = 0 < y < SIDE - 1
            if not value and horizontal and (ownership[y][x - 1] or ownership[y][x + 1]):
                value = cardinal
            if not value and vertical and (ownership[y - 1][x] or ownership[y + 1][x]):
                value = cardinal
            if not value and horizontal and vertical and any(
                    ownership[y + dy][x + dx] for dy in (-1, 1) for dx in (-1, 1)):
                value = diagonal
            pixels.append(value)
    return bytes(pixels)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory")
    args = parser.parse_args()
    evidence = json.loads(args.input.read_text())
    cells = evidence["cells"]
    ids = {material for row in cells[1:-1] for material in row[1:-1]}
    materials = {entry["id"]: entry for entry in evidence["materials"]}
    masks = []
    for material_id in ids:
        material = materials[material_id]
        parameter = material["coverage_exponent"]
        pixels = reconstruct_coverage(cells, material_id, parameter)
        masks.append((layer_sort_key(material), material_id, parameter, pixels))
    from PIL import Image
    args.output.mkdir()
    report = {key: evidence[key] for key in ("map", "chunk_origin", "ownership_source")}
    report["scope"] = "reconstructed raw coverage bytes; no final-render or filtering claim"
    report["layers"] = []
    for sort_key, material_id, parameter, pixels in sorted(masks):
        (args.output / f"material-{material_id}.rgba").write_bytes(
            bytes(channel for value in pixels for channel in (value,) * 4))
        Image.frombytes("L", (SIDE, SIDE), pixels).save(args.output / f"material-{material_id}.png")
        report["layers"].append(dict(material_id=material_id, sort_key=sort_key,
            blend_parameter=parameter, cardinal_byte=coverage_weights(parameter)[0],
            diagonal_byte=coverage_weights(parameter)[1], byte_counts=dict(sorted(Counter(pixels).items()))))
    (args.output / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"saved {len(masks)} reconstructed terrain masks to {args.output}")


if __name__ == "__main__":
    main()
