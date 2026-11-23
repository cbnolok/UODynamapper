"""The pictures of "RIFE in-betweens from the BC7 frames, looked at" from `classic_span rife-dump` clips.

Usage: python3 -I -B classic_rife_sheets.py <dump dir> <out dir> <today tsv> <don't care tsv> <record>:<pair>:<tag> ...

The dump folder holds, per record, the clip from the lossless frames (`lossless`), from today's BC7 UltraFast package
(`ultrafast`, the bench built as it is) and from the "don't care" kernel's (`dontcare`, the bench built with
dont_care.patch, whose runtime encode of the in-betweens is "don't care" too); the TSVs are `rife-rank`'s of the two
builds. Each in-between is the game's: RIFE's, then BC7 UltraFast as the renderer stores it. Everything is drawn over
a flat grey as the renderer draws the Classic frames (alpha from one half up opaque, the rest left out).

Per subject `<tag>`, writes `<tag>-sheet.png` (the pair's two frames; its in-between from each input at 1x and 3x
nearest; the difference from the lossless one, x4, the texels opaque in one only in magenta; RIFE's flow t->0 from the
lossless and the UltraFast frames, hue the direction and brightness the length, and their distance, white at
`FLOW_WHITE` pixels), `<tag>-flicker.mp4` (the in-between toggled lossless, UltraFast, lossless, don't care every
`TOGGLE_SECONDS`, at 1x and 3x side by side) and `<tag>-sequence-<n>x.mp4` (the whole clip, frames and in-betweens at
the game's timing, lossless, UltraFast and don't care side by side, at 1x and at 3x nearest, or 2x where 3x would pass
`MAX_VIDEO_WIDTH`), and `overview.png`, every subject's in-betweens and differences at 3x.
"""
import csv
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

INPUTS = ["lossless", "ultrafast", "dontcare"]
LABELS = {"lossless": "lossless (RGBA8 package)", "ultrafast": "BC7 UltraFast (today)", "dontcare": "BC7 \"don't care\""}
BACKGROUND = 128
CUTOFF = 128
NEAREST = 3
DIFFERENCE_GAIN = 4
FLIP = (255, 0, 255)
FLOW_WHITE = 2.0
GAP = 10
LABEL_HEIGHT = 18
TEXT = (255, 255, 255)
SHEET_BACKGROUND = (32, 32, 32)
TOGGLE_SECONDS = 0.7
TOGGLES = ["lossless", "ultrafast", "lossless", "dontcare"]
TOGGLE_ROUNDS = 3
SEQUENCE_SECONDS = 6.0
MAX_VIDEO_WIDTH = 1900
MAGNIFIED_SCALES = [NEAREST, 2]
OUTPUT_FPS = 25
CRF = "28"
FONT = ImageFont.load_default(size=13)


def load(folder, name, width, height):
    return np.frombuffer((folder / f"{name}.rgba").read_bytes(), np.uint8).reshape(height, width, 4)


def shown(rgba):
    """RGB over the grey, opaque from the cutoff up, and the opaque mask."""
    opaque = rgba[..., 3] >= CUTOFF
    rgb = np.full(rgba.shape[:2] + (3,), BACKGROUND, np.uint8)
    rgb[opaque] = rgba[..., :3][opaque]
    return rgb, opaque


def meta(folder):
    rows = list(csv.reader((folder / "meta.tsv").read_text().splitlines()[1:], delimiter="\t"))
    frame_secs = float(rows[0][1])
    items = {(kind, int(k)): tuple(int(v) for v in rest) for kind, k, *rest in rows[1:]}
    return frame_secs, items


def image(rgb):
    return Image.fromarray(rgb)


def nearest(picture, factor):
    return picture.resize((picture.width * factor, picture.height * factor), Image.NEAREST)


def difference(a, b):
    (rgb_a, opaque_a), (rgb_b, opaque_b) = shown(a), shown(b)
    gray = np.minimum(np.abs(rgb_a.astype(int) - rgb_b.astype(int)).max(axis=2) * DIFFERENCE_GAIN, 255).astype(np.uint8)
    out = np.repeat(gray[..., None], 3, axis=2)
    out[opaque_a != opaque_b] = FLIP
    return out


def flow_picture(flow, mask, other=None):
    """Flow t->0 as hue and brightness, or with `other` the distance of both vectors, white at FLOW_WHITE."""
    if other is not None:
        distance = np.maximum(np.hypot(flow[0] - other[0], flow[1] - other[1]),
                              np.hypot(flow[2] - other[2], flow[3] - other[3]))
        gray = (np.minimum(distance / FLOW_WHITE, 1.0) * 255).astype(np.uint8)
        gray[~mask] = 0
        return np.repeat(gray[..., None], 3, axis=2)
    length = np.hypot(flow[0], flow[1])
    hue = ((np.arctan2(flow[1], flow[0]) / (2 * np.pi)) % 1.0 * 255).astype(np.uint8)
    value = (np.minimum(length / max(length[mask].max() if mask.any() else 1.0, 1e-6), 1.0) * 255).astype(np.uint8)
    value[~mask] = 0
    hsv = np.stack([hue, np.full_like(hue, 255), value], axis=2)
    return np.asarray(Image.fromarray(hsv, "HSV").convert("RGB"))


def row(pictures, labels):
    measure = ImageDraw.Draw(Image.new("RGB", (1, 1)))
    cells = [max(p.width, int(measure.textlength(label, font=FONT)) + 1) for p, label in zip(pictures, labels)]
    out = Image.new("RGB", (sum(cells) + GAP * (len(cells) + 1), max(p.height for p in pictures) + LABEL_HEIGHT + GAP),
                    SHEET_BACKGROUND)
    draw = ImageDraw.Draw(out)
    x = GAP
    for picture, label, cell in zip(pictures, labels, cells):
        draw.text((x, 2), label, fill=TEXT, font=FONT)
        out.paste(picture, (x, LABEL_HEIGHT))
        x += cell + GAP
    return out


def stack(rows, title=None):
    top = LABEL_HEIGHT + GAP if title else 0
    out = Image.new("RGB", (max(r.width for r in rows), sum(r.height for r in rows) + top), SHEET_BACKGROUND)
    if title:
        ImageDraw.Draw(out).text((GAP, 4), title, fill=TEXT, font=FONT)
    y = top
    for r in rows:
        out.paste(r, (0, y))
        y += r.height
    return out


def even(picture):
    out = Image.new("RGB", (picture.width + picture.width % 2, picture.height + picture.height % 2), SHEET_BACKGROUND)
    out.paste(picture, (0, 0))
    return out


def encode(frames_dir, fps, out):
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i", str(frames_dir / "%04d.png"),
                    "-r", str(OUTPUT_FPS), "-c:v", "libx264", "-crf", CRF, "-pix_fmt", "yuv420p", "-movflags",
                    "+faststart", str(out)], check=True)


def clip(folder):
    """The clip as the game plays it with RIFE x2: frame, in-between, frame, ... each with its anchor."""
    frame_secs, items = meta(folder)
    count = sum(1 for kind, _ in items if kind == "frame")
    sequence = []
    for k in range(count):
        for kind, name in [("frame", f"frame{k}"), ("between", f"between{k}-game")]:
            width, height, anchor_x, anchor_y = items[(kind, k)]
            sequence.append((load(folder, name, width, height), anchor_x, anchor_y))
    return frame_secs, sequence


def sequence_video(subject_dir, tag, title, out):
    clips = {i: clip(subject_dir / i) for i in INPUTS}
    frame_secs = clips["lossless"][0]
    sequence = clips["lossless"][1]
    left = max(ax for _, ax, _ in sequence)
    up = max(ay for _, _, ay in sequence)
    right = max(rgba.shape[1] - ax for rgba, ax, _ in sequence)
    down = max(rgba.shape[0] - ay for rgba, _, ay in sequence)
    canvas = (left + right, up + down)
    magnified = [s for s in MAGNIFIED_SCALES if len(INPUTS) * canvas[0] * s + (len(INPUTS) + 1) * GAP <= MAX_VIDEO_WIDTH]
    for scale in [1] + magnified[:1]:
        with tempfile.TemporaryDirectory(dir=out) as frames_dir:
            frames_dir = Path(frames_dir)
            steps = len(sequence)
            loops = max(1, round(SEQUENCE_SECONDS / (steps * frame_secs / 2)))
            for n in range(steps * loops):
                pictures = []
                for i in INPUTS:
                    rgba, ax, ay = clips[i][1][n % steps]
                    board = np.full((canvas[1], canvas[0], 3), BACKGROUND, np.uint8)
                    rgb, opaque = shown(rgba)
                    y0, x0 = up - ay, left - ax
                    region = board[y0:y0 + rgba.shape[0], x0:x0 + rgba.shape[1]]
                    region[opaque] = rgb[opaque]
                    pictures.append(nearest(image(board), scale))
                kind = "frame" if n % 2 == 0 else "in-between"
                picture = stack([row(pictures, [LABELS[i] for i in INPUTS])],
                                f"{title}: whole clip, RIFE x2 at the game's timing, {scale}x ({kind} {n % steps // 2})")
                even(picture).save(frames_dir / f"{n:04d}.png")
            encode(frames_dir, 2 / frame_secs, out / f"{tag}-sequence-{scale}x.mp4")


def subject(dump, out, record, pair, tag, ranks):
    subject_dir = dump / record
    _, items = meta(subject_dir / "lossless")
    width, height, _, _ = items[("between", pair)]
    betweens = {i: load(subject_dir / i, f"between{pair}-game", width, height) for i in INPUTS}
    flows = {i: np.frombuffer((subject_dir / i / f"flow{pair}.f32").read_bytes(), np.float32).reshape(4, height, width)
             for i in INPUTS}
    count = sum(1 for kind, _ in items if kind == "frame")
    frames = []
    for k in [pair, (pair + 1) % count]:
        w, h, _, _ = items[("frame", k)]
        frames.append(load(subject_dir / "lossless", f"frame{k}", w, h))
    db = {i: ranks[i].get((record, str(pair)), "?") for i in INPUTS[1:]}
    title = (f"{tag}: record {record} pair {pair} (frames {pair} -> {(pair + 1) % count}); game's in-between vs RIFE "
             f"from lossless: UltraFast {db['ultrafast']} dB, don't care {db['dontcare']} dB")
    mask = shown(betweens["lossless"])[1] | shown(betweens["ultrafast"])[1]
    in_betweens = [image(shown(betweens[i])[0]) for i in INPUTS]
    differences = [nearest(image(difference(betweens["lossless"], betweens[i])), NEAREST) for i in INPUTS[1:]]
    rows = [
        row([nearest(image(shown(f)[0]), NEAREST) for f in frames], [f"frame {pair}, 3x", "next frame, 3x"]),
        row(in_betweens, [LABELS[i] for i in INPUTS]),
        row([nearest(p, NEAREST) for p in in_betweens], [f"{LABELS[i]}, 3x" for i in INPUTS]),
        row(differences, [f"|{i} - lossless| x{DIFFERENCE_GAIN}, magenta: opaque in one" for i in INPUTS[1:]]),
        row([nearest(image(flow_picture(flows[i], mask)), NEAREST) for i in INPUTS[:2]]
            + [nearest(image(flow_picture(flows["ultrafast"], mask, flows["lossless"])), NEAREST)],
            ["flow t->0 from lossless", "flow t->0 from UltraFast", f"flow distance, white at {FLOW_WHITE} px"]),
    ]
    stack(rows, title).save(out / f"{tag}-sheet.png")

    with tempfile.TemporaryDirectory(dir=out) as frames_dir:
        frames_dir = Path(frames_dir)
        for n, state in enumerate(TOGGLES * TOGGLE_ROUNDS):
            one = image(shown(betweens[state])[0])
            picture = stack([row([one, nearest(one, NEAREST)], ["1x", f"{NEAREST}x nearest"])],
                            f"{tag} in-between {pair}: {LABELS[state]}")
            even(picture).save(frames_dir / f"{n:04d}.png")
        encode(frames_dir, 1 / TOGGLE_SECONDS, out / f"{tag}-flicker.mp4")

    sequence_video(subject_dir, tag, tag, out)
    return row([nearest(p, NEAREST) for p in in_betweens] + differences,
               [f"{tag}: lossless"] + [LABELS[i] for i in INPUTS[1:]] + [f"|{i} - lossless| x{DIFFERENCE_GAIN}"
                                                                         for i in INPUTS[1:]])


def ranks_of(path, name):
    with open(path) as f:
        return {(r["record"], r["pair"]): r["game_db"] for r in csv.DictReader(f, delimiter="\t") if r["input"] == name}


def main():
    dump, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    ranks = {"ultrafast": ranks_of(sys.argv[3], "ultrafast"), "dontcare": ranks_of(sys.argv[4], "dontcare")}
    overview = []
    for spec in sys.argv[5:]:
        record, pair, tag = spec.split(":", 2)
        overview.append(subject(dump, out, record, int(pair), tag, ranks))
        print(tag)
    stack(overview, "Every subject's in-between at 3x: lossless, UltraFast, don't care, and the differences").save(
        out / "overview.png")


main()
