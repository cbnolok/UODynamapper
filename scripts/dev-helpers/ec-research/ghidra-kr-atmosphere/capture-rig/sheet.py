"""Side-by-side sheet of the KR atmosphere captures and their colour statistics. Writes into the scratch folder.

Rows are views; columns are variant a (current KR-like afternoon), b (every term the client lacks switched off) and
c (b plus a hard 0.75 projected-shadow stand-in).
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

CAPS = Path('/home/claudio/rust-target/uodynamapper-scratch/kr-atmosphere/caps')
OUT = CAPS / 'kr_atmosphere_sheet.png'
VIEWS = [('slopes_6', 'open slopes (5650, 2950)'), ('town_britain', 'town (1434, 1699)'),
         ('forest_yew', 'forest (620, 1020)'), ('cliff_covetous', 'cliff / mountain (2460, 880)')]
VARIANTS = [('sheet-a', 'a: KR-like afternoon as now'), ('sheet-b', 'b: client-missing terms off'),
            ('sheet-c', 'c: b + 0.75 projected-shadow stand-in')]
CELL = (640, 480)
LABEL_MARGIN = (6, 5)
LABEL_COLOUR = (235, 235, 235)
BACKGROUND = (20, 20, 20)
PERCENTILE_DIVISOR = 10
LABEL_HEIGHT = 22
LUMA_WEIGHTS = (0.299, 0.587, 0.114)
# Skip the debug overlays in the statistics: the top band holds the position and FPS panels.
STATS_TOP_MARGIN = 130


def luma_stats(image):
    width, height = image.size
    pixels = list(image.crop((0, STATS_TOP_MARGIN, width, height)).get_flattened_data())
    luma = sorted(sum(w * v for w, v in zip(LUMA_WEIGHTS, p)) for p in pixels)
    count = len(luma)
    mean = [sum(p[c] for p in pixels) / count for c in range(3)]
    chroma = sum(max(p) - min(p) for p in pixels) / count
    return (mean, luma[count // PERCENTILE_DIVISOR], luma[count // 2],
            luma[count * (PERCENTILE_DIVISOR - 1) // PERCENTILE_DIVISOR], chroma)


def main():
    sheet = Image.new('RGB', (CELL[0] * len(VARIANTS), (CELL[1] + LABEL_HEIGHT) * len(VIEWS)), BACKGROUND)
    draw = ImageDraw.Draw(sheet)
    for row, (view, view_label) in enumerate(VIEWS):
        for column, (folder, variant_label) in enumerate(VARIANTS):
            image = Image.open(CAPS / folder / f'{view}.png').convert('RGB')
            mean, p10, p50, p90, chroma = luma_stats(image)
            print('%-15s %-8s mean (%3.0f,%3.0f,%3.0f) luma p10/p50/p90 %3.0f/%3.0f/%3.0f chroma %.0f' % (
                view, folder, mean[0], mean[1], mean[2], p10, p50, p90, chroma))
            x = column * CELL[0]
            y = row * (CELL[1] + LABEL_HEIGHT)
            sheet.paste(image.resize(CELL, Image.LANCZOS), (x, y + LABEL_HEIGHT))
            draw.text((x + LABEL_MARGIN[0], y + LABEL_MARGIN[1]), f'{view_label} | {variant_label}', fill=LABEL_COLOUR)
    sheet.save(OUT)
    print(OUT)


if __name__ == '__main__':
    sys.exit(main())
