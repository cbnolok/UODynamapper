#!/usr/bin/env python3
"""Compare repeat frames or the overlap of frames at known pixel offsets.

Accepts normal images, or raw RGBA files with --size. This only compares pixels;
the capture driver must separately verify scene readiness and clean shader logs.
"""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image


def load(path, size):
    if path.suffix.lower() in {".rgba", ".raw"}:
        if size is None:
            raise ValueError("raw RGBA input requires --size WIDTH HEIGHT")
        width, height = size
        pixels = np.fromfile(path, dtype=np.uint8)
        if width <= 0 or height <= 0 or pixels.size != width * height * 4:
            raise ValueError(f"{path}: raw byte count does not match --size")
        return pixels.reshape(height, width, 4)
    with Image.open(path) as image:
        return np.asarray(image.convert("RGBA"))


def compare(left, right, offset_x=0, offset_y=0, guard=0, min_content=0.001, overlap=False):
    if guard < 0 or not 0 <= min_content <= 1:
        raise ValueError("guard must be nonnegative and min-content must be in 0..1")
    left_height, left_width = left.shape[:2]
    right_height, right_width = right.shape[:2]
    if not overlap and offset_x == 0 and offset_y == 0 and left.shape[:2] != right.shape[:2]:
        raise ValueError("repeat frames must have equal dimensions; use --overlap for an intentional partial comparison")
    x0, y0 = max(guard, offset_x + guard), max(guard, offset_y + guard)
    x1 = min(left_width - guard, offset_x + right_width - guard)
    y1 = min(left_height - guard, offset_y + right_height - guard)
    if x1 <= x0 or y1 <= y0:
        raise ValueError("frames have no overlap after excluding their guards")
    a = left[y0:y1, x0:x1]
    b = right[y0 - offset_y:y1 - offset_y, x0 - offset_x:x1 - offset_x]
    difference = np.abs(a.astype(np.int16) - b.astype(np.int16))
    content = [float(np.any(frame[..., :3] != 0, axis=2).mean()) for frame in (a, b)]
    differing = int(np.any(difference != 0, axis=2).sum())
    return {
        "overlap": [x0, y0, x1, y1],
        "pixels": int(a.shape[0] * a.shape[1]),
        "differing_pixels": differing,
        "max_channel_delta": int(difference.max()),
        "nonblack_fraction": content,
        "passed": differing == 0 and min(content) >= min_content,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--size", type=int, nargs=2, metavar=("WIDTH", "HEIGHT"))
    parser.add_argument("--offset-x", type=int, default=0, help="candidate origin relative to reference, in pixels")
    parser.add_argument("--offset-y", type=int, default=0)
    parser.add_argument("--overlap", action="store_true", help="allow differing frame dimensions at zero offset")
    parser.add_argument("--guard", type=int, default=0, help="exclude this many pixels at each frame's edges")
    parser.add_argument("--min-content", type=float, default=0.001, help="minimum nonblack fraction in each compared region")
    args = parser.parse_args()
    try:
        result = compare(load(args.reference, args.size), load(args.candidate, args.size),
                         args.offset_x, args.offset_y, args.guard, args.min_content, args.overlap)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps(result, sort_keys=True))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
