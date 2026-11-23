"""The pictures of "BC1 for the Classic creatures" from `classic_big dump` clips.

Usage: python3 -I -B classic_big_sheets.py <dump dir> <out dir> <inputs> <name>[:<frame>][:<crop x>,<crop y>] ...

Each frame is drawn over a flat grey as the renderer draws it: alpha from one half up opaque. `<inputs>` names the
dumped inputs to show, comma separated, lossless first (`lossless,bc7,bc1-high`; a folder dumped with
`CLASSIC_BIG_RUNTIME=bc1-gpu` and renamed, such as `bc7+bc1`, is an input too). Per subject `<name>` (a folder of the
dump), with its frame (the middle one by default) and the top left corner of a `CROP`-texel crop (centred by default):

- `<name>-sheet.png`: per input, the frame at 1x; the crop at 3x nearest; its |difference| from lossless x8; the crop
  of the SPAN 2x frame (after the game's runtime encode) at 2x nearest; and its difference from lossless's.
- `<name>-flicker.mp4`: lossless and each other input in turn, 0.7 s each: the frame at 1x, the crop at 3x and the
  SPAN crop at 2x, labelled.
- `<name>-motion.mp4`: the clip looping at the game's pace, the inputs side by side at 2x nearest: the package
  frames, then the clip with its RIFE x2 in-betweens, then the SPAN 2x frames at 1x of their size.
"""
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

BACKGROUND = (128, 128, 128)
SHEET_BACKGROUND = (32, 32, 32)
TEXT = (255, 255, 255)
CUTOFF = 128
CROP = 128
CROP_ZOOM = 3
SPAN_SCALE = 2
SPAN_ZOOM = 2
MOTION_ZOOM = 2
DIFFERENCE_GAIN = 8
GAP = 12
LABEL_HEIGHT = 18
STATE_SECS = 0.7
OUTPUT_FPS = 25
CRF = "28"
# The Classic client's time per frame (dynamapper clips.rs `CLASSIC_FRAME_SECS`), and RIFE x2's.
FRAME_SECS = 0.08
RIFE_FACTOR = 2
LOOPS = 3
LOSSLESS = "lossless"


def frames(folder):
    """The folder's frames in order: (image over grey, anchor x, anchor y)."""
    shown = []
    for path in sorted(folder.glob("*.rgba")):
        _, size, anchor_x, anchor_y = path.stem.split("_")
        width, height = (int(v) for v in size.split("x"))
        rgba = Image.frombytes("RGBA", (width, height), path.read_bytes())
        alpha = rgba.getchannel("A").point(lambda a: 255 if a >= CUTOFF else 0)
        image = Image.new("RGB", rgba.size, BACKGROUND)
        image.paste(rgba.convert("RGB"), mask=alpha)
        shown.append((image, int(anchor_x), int(anchor_y)))
    return shown


def nearest(image, factor):
    return image.resize((image.width * factor, image.height * factor), Image.NEAREST)


def difference(a, b):
    return ImageChops.difference(a, b).point(lambda v: min(255, v * DIFFERENCE_GAIN))


def labelled(image, text):
    out = Image.new("RGB", (image.width, image.height + LABEL_HEIGHT), SHEET_BACKGROUND)
    out.paste(image, (0, LABEL_HEIGHT))
    ImageDraw.Draw(out).text((2, 2), text, fill=TEXT)
    return out


def row(images, background=SHEET_BACKGROUND):
    width = sum(i.width for i in images) + GAP * (len(images) - 1)
    out = Image.new("RGB", (width, max(i.height for i in images)), background)
    x = 0
    for image in images:
        out.paste(image, (x, 0))
        x += image.width + GAP
    return out


def column(images):
    out = Image.new("RGB", (max(i.width for i in images), sum(i.height for i in images) + GAP * (len(images) - 1)),
                    SHEET_BACKGROUND)
    y = 0
    for image in images:
        out.paste(image, (0, y))
        y += image.height + GAP
    return out


def views(subject, inputs, index, corner):
    """Per input: the 1x frame, the crop at 3x and the SPAN crop at 2x, and the crop box."""
    out = {}
    for name in inputs:
        image = frames(subject / name / "1x")[index][0]
        span = frames(subject / name / "span")[index]
        x, y = corner if corner else (max(0, (image.width - CROP) // 2), max(0, (image.height - CROP) // 2))
        box = (x, y, min(image.width, x + CROP), min(image.height, y + CROP))
        # The SPAN canvas holds the frame at 2x from its own anchor: the frame's texel (0, 0) lies where the anchors
        # meet.
        frame_anchor = frames(subject / name / "1x")[index][1:]
        dx = span[1] - SPAN_SCALE * frame_anchor[0]
        dy = span[2] - SPAN_SCALE * frame_anchor[1]
        span_box = tuple(SPAN_SCALE * v + d for v, d in zip(box, (dx, dy, dx, dy)))
        out[name] = (image, nearest(image.crop(box), CROP_ZOOM), nearest(span[0].crop(span_box), SPAN_ZOOM))
    return out


def sheet(subject, inputs, index, corner, out):
    shown = views(subject, inputs, index, corner)
    rows = []
    for name in inputs:
        image, crop, span = shown[name]
        cells = [labelled(image, f"{name} 1x"), labelled(crop, "crop 3x")]
        if name != LOSSLESS:
            cells.append(labelled(difference(shown[LOSSLESS][1], crop), f"|diff| x{DIFFERENCE_GAIN}"))
        cells.append(labelled(span, "SPAN 2x, runtime encode, at 2x"))
        if name != LOSSLESS:
            cells.append(labelled(difference(shown[LOSSLESS][2], span), f"|diff| x{DIFFERENCE_GAIN}"))
        rows.append(row(cells))
    column(rows).save(out)


def encode(folder, fps, out):
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i", str(folder / "%04d.png"),
                    "-r", str(OUTPUT_FPS), "-c:v", "libx264", "-crf", CRF, "-pix_fmt", "yuv420p", "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-movflags", "+faststart", str(out)], check=True)


def flicker(subject, inputs, index, corner, out):
    shown = views(subject, inputs, index, corner)
    order = [state for name in inputs[1:] for state in (LOSSLESS, name)]
    with tempfile.TemporaryDirectory(dir=out.parent) as temporary:
        folder = Path(temporary)
        for k, name in enumerate(order):
            image, crop, span = shown[name]
            labelled(row([image, crop, span]), f"{name}: 1x | crop 3x | SPAN 2x crop at 2x").save(
                folder / f"{k:04d}.png")
        encode(folder, 1 / STATE_SECS, out)


def placed(clip, zoom):
    """The clip's frames on one canvas by their anchors, at `zoom` nearest."""
    left = max(a for _, a, _ in clip)
    up = max(b for _, _, b in clip)
    right = max(i.width - a for i, a, _ in clip)
    down = max(i.height - b for i, _, b in clip)
    out = []
    for image, a, b in clip:
        canvas = Image.new("RGB", (left + right, up + down), BACKGROUND)
        canvas.paste(image, (left - a, up - b))
        out.append(nearest(canvas, zoom))
    return out


def motion(subject, inputs, out):
    parts = [("1x", FRAME_SECS, MOTION_ZOOM, "frames, 2x nearest"),
             ("rife", FRAME_SECS / RIFE_FACTOR, MOTION_ZOOM, "RIFE x2, 2x nearest"),
             ("span", FRAME_SECS, 1, "SPAN 2x (runtime encode)")]
    with tempfile.TemporaryDirectory(dir=out.parent) as temporary:
        folder = Path(temporary)
        count = 0
        for kind, secs, zoom, title in parts:
            clips = [placed(frames(subject / name / kind), zoom) for name in inputs]
            repeat = round(secs * OUTPUT_FPS)
            for _ in range(LOOPS):
                for k in range(len(clips[0])):
                    image = labelled(row([labelled(clips[i][k], name) for i, name in enumerate(inputs)]), title)
                    for _ in range(max(1, repeat)):
                        image.save(folder / f"{count:04d}.png")
                        count += 1
        encode(folder, OUTPUT_FPS, out)


def main():
    dump, out_dir, inputs = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3].split(",")
    assert inputs[0] == LOSSLESS, "lossless first, the reference"
    out_dir.mkdir(parents=True, exist_ok=True)
    for spec in sys.argv[4:]:
        fields = spec.split(":")
        name = fields[0]
        subject = dump / name
        count = len(frames(subject / LOSSLESS / "1x"))
        index = int(fields[1]) if len(fields) > 1 and fields[1] else count // 2
        corner = tuple(int(v) for v in fields[2].split(",")) if len(fields) > 2 else None
        sheet(subject, inputs, index, corner, out_dir / f"{name}-sheet.png")
        flicker(subject, inputs, index, corner, out_dir / f"{name}-flicker.mp4")
        motion(subject, inputs, out_dir / f"{name}-motion.mp4")
        print(name, index, corner)


main()
