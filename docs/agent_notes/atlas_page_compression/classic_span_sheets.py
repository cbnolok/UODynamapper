"""The pictures of "BC7 frames as they show, at 1x and after SPAN" from `classic_span frames` crops.

Usage: python3 -I -B classic_span_sheets.py <crops dir> <out dir>

Each frame is drawn over a flat grey as the renderer draws it: alpha from one half up opaque, below that blended.
Writes, per frame name, `<name>-sheet.png` (rows: 1x; 3x nearest; |difference| x8 at 3x; SPAN 2x encoded as the
game does, at 2x nearest, and its difference), and two flicker sequences for ffmpeg, `flicker-1x/` (lossless and
UltraFast at 3x) and `flicker-span/` (SPAN of the lossless and of the UltraFast frame, each BC7, at 2x): every
subject side by side, one image per state.
"""
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

BACKGROUND = (128, 128, 128)
CUTOFF = 128
NEAREST_1X = 3
NEAREST_SPAN = 2
DIFFERENCE_GAIN = 8
GAP = 12
LABEL_HEIGHT = 18
TEXT = (255, 255, 255)
SHEET_BACKGROUND = (32, 32, 32)
INPUTS = ["lossless", "ultrafast", "veryfast", "bc1"]
SPANS = ["span-lossless", "span-ultrafast"]


def load(crops, name, variant):
    [path] = crops.glob(f"{name}_{variant}_*.rgba")
    width, height = (int(v) for v in path.stem.rsplit("_", 1)[1].split("x"))
    rgba = Image.frombytes("RGBA", (width, height), path.read_bytes())
    # The opaque pass draws alpha from one half up at full strength; the blended pass draws the rest.
    alpha = rgba.getchannel("A").point(lambda a: 255 if a >= CUTOFF else a)
    shown = Image.new("RGB", rgba.size, BACKGROUND)
    shown.paste(rgba.convert("RGB"), mask=alpha)
    return shown


def nearest(image, factor):
    return image.resize((image.width * factor, image.height * factor), Image.NEAREST)


def difference(a, b):
    return ImageChops.difference(a, b).point(lambda v: min(255, v * DIFFERENCE_GAIN))


def row(images, labels):
    measure = ImageDraw.Draw(Image.new("RGB", (1, 1)))
    cells = [max(i.width, int(measure.textlength(label)) + 1) for i, label in zip(images, labels)]
    width = sum(cells) + GAP * (len(images) + 1)
    height = max(i.height for i in images) + LABEL_HEIGHT + GAP
    out = Image.new("RGB", (width, height), SHEET_BACKGROUND)
    draw = ImageDraw.Draw(out)
    x = GAP
    for image, label, cell in zip(images, labels, cells):
        draw.text((x, 2), label, fill=TEXT)
        out.paste(image, (x, LABEL_HEIGHT))
        x += cell + GAP
    return out


def stack(rows):
    out = Image.new("RGB", (max(r.width for r in rows), sum(r.height for r in rows)), SHEET_BACKGROUND)
    y = 0
    for r in rows:
        out.paste(r, (0, y))
        y += r.height
    return out


def main():
    crops, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    names = sorted({p.name.split("_")[0] for p in crops.glob("*_lossless_*.rgba")})
    flicker_1x = {state: [] for state in INPUTS[:2]}
    flicker_span = {state: [] for state in SPANS}
    for name in names:
        shown = {v: load(crops, name, v) for v in INPUTS + SPANS}
        lossless = shown["lossless"]
        rows = [
            row([shown[v] for v in INPUTS], INPUTS),
            row([nearest(shown[v], NEAREST_1X) for v in INPUTS], [f"{v} {NEAREST_1X}x" for v in INPUTS]),
            row([nearest(difference(lossless, shown[v]), NEAREST_1X) for v in INPUTS[1:]],
                [f"|{v} - lossless| x{DIFFERENCE_GAIN}" for v in INPUTS[1:]]),
            row([nearest(shown[v], NEAREST_SPAN) for v in SPANS]
                + [nearest(difference(shown[SPANS[0]], shown[SPANS[1]]), NEAREST_SPAN)],
                ["SPAN 2x of lossless, BC7", "SPAN 2x of UltraFast, BC7", f"|difference| x{DIFFERENCE_GAIN}"]),
        ]
        stack(rows).save(out / f"{name}-sheet.png")
        for v in INPUTS[:2]:
            flicker_1x[v].append(nearest(shown[v], NEAREST_1X))
        for v in SPANS:
            flicker_span[v].append(nearest(shown[v], NEAREST_SPAN))
    for folder, states, title in [("flicker-1x", flicker_1x, "1x frames, 3x nearest"),
                                  ("flicker-span", flicker_span, "SPAN 2x, then BC7 as the game, 2x nearest")]:
        (out / folder).mkdir(exist_ok=True)
        for k, (state, images) in enumerate(states.items()):
            picture = row(images, names)
            canvas = Image.new("RGB", (picture.width, picture.height + LABEL_HEIGHT + GAP), SHEET_BACKGROUND)
            canvas.paste(picture, (0, LABEL_HEIGHT + GAP))
            ImageDraw.Draw(canvas).text((GAP, 4), f"{title}: {state}", fill=TEXT)
            # Even sides for libx264.
            even = canvas.crop((0, 0, canvas.width // 2 * 2, canvas.height // 2 * 2))
            even.save(out / folder / f"{k}.png")


main()
