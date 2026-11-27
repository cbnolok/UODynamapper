#!/usr/bin/env python3
"""Sheets, composited errors and the crowd's backlog of the BC7 profiles on SPAN 2x frames.

Reads the folder `block_encode::tests::span_frames::evaluate_span_frame_profiles` writes (each 2x frame as
`<sample>_source.png`, its decodings as `<sample>_<Profile>.png`, `frames.tsv` and `clips.tsv`) and writes into
`--out`:

- `<creature>.png`: for a standing and a walking frame, columns source | UltraFast | VeryFast | Fast | Basic | Slow;
  rows: the frame at 1:1 (a 2x frame drawn at a 2x view, one texel a pixel) over the ground, its |diff| x8, then two
  4x nearest crops (the outline: the window with the most alpha edge; the detail: the window where UltraFast errs
  most inside the opaque texels), each with its |diff| x8 row.
- `errors.md`: the error of the shown image, blended in linear light as the GPU blends, over mid-grey and over the
  ground image (`--ground`, tiled under the frame; without it a grass colour).
- `crowd.md`: the GPU time the 2x frames of a crowd's walk clips take (SPAN by the game's cost model, encode as
  measured) and how long the shared 2 ms a frame budget takes to drain it.

Run with `python3 -I -B span_bc7_sheets.py <folder> --out <sheets> [--ground ground.png]`.
"""

import argparse
import csv
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

PROFILES = ["UltraFast", "VeryFast", "Fast", "Basic", "Slow"]
COLUMNS = ["source"] + PROFILES
GREY = 128
# A Trammel grass texel, when no ground image is given.
GRASS = (74, 98, 41)
DIFF_GAIN = 8
CROP = 48
ZOOM = 4
GAP = 6
LABEL_HEIGHT = 14
BACKGROUND = (32, 32, 32)
LABEL_COLOUR = (230, 230, 230)
CHANNEL_MAX = 255.0
# upscales/render.rs's cost model: a dispatch's fixed part, its multiply-adds, the prepare and finish around the
# network's dispatches, and the timed scale over the model (1.05-1.3 on the Radeon 880M, mobile_animation_motion.md).
DISPATCH_MS = 0.027
MS_PER_GIGA_MULTIPLY_ADD = 1.0
NETWORK_DISPATCHES = 22
FRAME_STEP_DISPATCHES = NETWORK_DISPATCHES + 2
COST_SCALE = 1.15
BUDGET_MS = 2.0
CROWD = 32
FRAME_RATES = (60, 144)


def srgb_to_linear(byte):
    value = byte / CHANNEL_MAX
    return np.where(value <= 0.04045, value / 12.92, ((value + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(linear):
    linear = np.clip(linear, 0.0, 1.0)
    value = np.where(linear <= 0.0031308, linear * 12.92, 1.055 * linear ** (1 / 2.4) - 0.055)
    return np.round(value * CHANNEL_MAX)


def load(path):
    return np.asarray(Image.open(path).convert("RGBA"), dtype=np.float64)


def ground_under(ground, height, width):
    if ground is None:
        return np.broadcast_to(np.array(GRASS, dtype=np.float64), (height, width, 3))
    reps = (math.ceil(height / ground.shape[0]), math.ceil(width / ground.shape[1]), 1)
    return np.tile(ground, reps)[:height, :width]


def shown(rgba, under):
    alpha = rgba[..., 3:4] / CHANNEL_MAX
    return linear_to_srgb(srgb_to_linear(rgba[..., :3]) * alpha + srgb_to_linear(under) * (1 - alpha))


def psnr(squared, count):
    return math.inf if squared == 0 else 10 * math.log10(CHANNEL_MAX ** 2 / (squared / count))


def window(score, side):
    """The top left of the `side` square window with the largest sum of `score`."""
    height, width = score.shape
    side_y, side_x = min(side, height), min(side, width)
    summed = np.pad(score, ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    sums = summed[side_y:, side_x:] - summed[:-side_y, side_x:] - summed[side_y:, :-side_x] + summed[:-side_y, :-side_x]
    y, x = np.unravel_index(np.argmax(sums), sums.shape)
    return y, x, side_y, side_x


def nearest(image, zoom):
    return image.repeat(zoom, 0).repeat(zoom, 1)


def labelled_row(draw, sheet, top, label, panels):
    """A row's label, then each panel with its column's name above it."""
    draw.text((GAP, top), label, fill=LABEL_COLOUR)
    left = GAP
    for column, panel in zip(COLUMNS, panels):
        draw.text((left, top + LABEL_HEIGHT), column, fill=LABEL_COLOUR)
        sheet.paste(Image.fromarray(panel.astype(np.uint8)), (left, top + 2 * LABEL_HEIGHT))
        left += panel.shape[1] + GAP
    return top + 2 * LABEL_HEIGHT + panels[0].shape[0] + GAP


def creature_sheet(folder, samples, ground, path):
    blocks = []
    for sample in samples:
        source = load(folder / f"{sample}_source.png")
        decoded = [load(folder / f"{sample}_{profile}.png") for profile in PROFILES]
        under = ground_under(ground, *source.shape[:2])
        images = [shown(rgba, under) for rgba in [source] + decoded]
        diffs = [np.zeros_like(images[0])] + [np.minimum(np.abs(image - images[0]) * DIFF_GAIN, CHANNEL_MAX)
                                              for image in images[1:]]
        alpha = source[..., 3]
        edge = np.abs(np.diff(alpha, axis=0, prepend=0)) + np.abs(np.diff(alpha, axis=1, prepend=0))
        outline = window(edge, CROP)
        detail = window(np.abs(images[1] - images[0]).sum(2) * (alpha == CHANNEL_MAX), CROP)
        rows = [(f"{sample}: 1:1 over the ground", images), ("|diff| x8", diffs)]
        for name, (y, x, h, w) in (("outline", outline), ("detail", detail)):
            rows.append((f"{name} crop 4x at {x},{y}", [nearest(image[y:y + h, x:x + w], ZOOM) for image in images]))
            rows.append(("|diff| x8", [nearest(image[y:y + h, x:x + w], ZOOM) for image in diffs]))
        blocks.append(rows)
    width = max(sum(panel.shape[1] + GAP for panel in panels) + GAP for rows in blocks for _, panels in rows)
    height = GAP + sum(2 * LABEL_HEIGHT + panels[0].shape[0] + GAP for rows in blocks for _, panels in rows)
    sheet = Image.new("RGB", (width, height), BACKGROUND)
    draw = ImageDraw.Draw(sheet)
    top = GAP
    for rows in blocks:
        for label, panels in rows:
            top = labelled_row(draw, sheet, top, label, panels)
    sheet.save(path)


def errors_table(folder, frames, ground):
    totals = {}
    for row in frames:
        source = load(folder / f"{row['sample']}_source.png")
        decoded = load(folder / f"{row['sample']}_{row['profile']}.png")
        for backdrop, under in (("grey", np.full(source.shape[:2] + (3,), GREY, dtype=np.float64)),
                                ("ground", ground_under(ground, *source.shape[:2]))):
            error = np.abs(shown(decoded, under) - shown(source, under))
            total = totals.setdefault((row["creature"], row["profile"], backdrop), [0.0, 0, 0.0])
            total[0] += float((error ** 2).sum())
            total[1] += error.size
            total[2] = max(total[2], float(error.max()))
    lines = ["| creature | profile | over grey PSNR dB | max | over ground PSNR dB | max |", "|---|---|---:|---:|---:|---:|"]
    creatures = list(dict.fromkeys(row["creature"] for row in frames))
    for creature in creatures:
        for profile in PROFILES:
            grey = totals[(creature, profile, "grey")]
            land = totals[(creature, profile, "ground")]
            lines.append(f"| {creature} | {profile} | {psnr(grey[0], grey[1]):.2f} | {grey[2]:.0f} | "
                         f"{psnr(land[0], land[1]):.2f} | {land[2]:.0f} |")
    return "\n".join(lines)


def encode_fit(frames):
    """Per profile, the least-squares ms = a + b * megapixels over every frame encoded alone."""
    fits = {}
    for profile in PROFILES:
        rows = [row for row in frames if row["profile"] == profile]
        pixels = np.array([int(row["width"]) * int(row["height"]) / 1e6 for row in rows])
        ms = np.array([float(row["alone_ms"]) for row in rows])
        slope, intercept = np.polyfit(pixels, ms, 1)
        fits[profile] = (intercept, slope)
    return fits


def crowd_table(clips, fits):
    lines = [f"Cost of the 2x frames of {CROWD} creatures each showing the clip (SPAN by the cost model x{COST_SCALE}, "
             f"encode by the fit of the frames timed alone); drain at the {BUDGET_MS} ms a frame budget.", "",
             "| clip | frames | SPAN ms/frame | profile | encode ms/frame (largest) | encode share | backlog GPU s | "
             + " | ".join(f"drain s at {rate} fps" for rate in FRAME_RATES) + " |",
             "|---|---:|---:|---|---:|---:|---:|" + "---:|" * len(FRAME_RATES)]
    fit_lines = ["| profile | ms fixed | ms per 2x megapixel |", "|---|---:|---:|"]
    for profile in PROFILES:
        fit_lines.append(f"| {profile} | {fits[profile][0]:.3f} | {fits[profile][1]:.2f} |")
    for clip in clips:
        pixels = [int(value) for value in clip["frames_2x_px"].split(",")]
        span = int(clip["passes_per_frame"]) * (FRAME_STEP_DISPATCHES * DISPATCH_MS
                                                 + int(clip["multiply_adds_per_pass"]) / 1e9 * MS_PER_GIGA_MULTIPLY_ADD)
        span *= COST_SCALE
        for profile in PROFILES:
            intercept, slope = fits[profile]
            encodes = [intercept + slope * value / 1e6 for value in pixels]
            per_frame = span + sum(encodes) / len(encodes)
            backlog = CROWD * len(pixels) * per_frame
            drains = " | ".join(f"{backlog / BUDGET_MS / rate:.1f}" for rate in FRAME_RATES)
            lines.append(f"| {clip['creature']} {clip['movement']} | {len(pixels)} | {span:.2f} | {profile} | "
                         f"{sum(encodes) / len(encodes):.2f} ({max(encodes):.2f}) | "
                         f"{sum(encodes) / len(encodes) / per_frame:.0%} | {backlog / 1e3:.2f} | {drains} |")
    return "\n".join(fit_lines) + "\n\n" + "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("folder", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--ground", type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    ground = load(args.ground)[..., :3] if args.ground else None
    with open(args.folder / "frames.tsv", newline="") as file:
        frames = list(csv.DictReader(file, delimiter="\t"))
    with open(args.folder / "clips.tsv", newline="") as file:
        clips = list(csv.DictReader(file, delimiter="\t"))
    samples = list(dict.fromkeys(row["sample"] for row in frames))
    for creature in dict.fromkeys(row["creature"] for row in frames):
        own = [sample for sample in samples if sample.startswith(creature + "_")]
        chosen = [next(sample for sample in own if "_stand_" in sample)]
        walks = [sample for sample in own if "_walk_" in sample]
        chosen.append(walks[len(walks) // 2])
        creature_sheet(args.folder, chosen, ground, args.out / f"{creature}.png")
    (args.out / "errors.md").write_text(errors_table(args.folder, frames, ground) + "\n")
    (args.out / "crowd.md").write_text(crowd_table(clips, encode_fit(frames)) + "\n")
    print((args.out / "errors.md").read_text())
    print((args.out / "crowd.md").read_text())


if __name__ == "__main__":
    main()
