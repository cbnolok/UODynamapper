"""Colour statistics of the KR reference screenshots. Read only; prints one line per image.

Usage: python3 -I kr_reference_stats.py docs/images/kr_reference
"""
import os
import sys

from PIL import Image

LUMA_WEIGHTS = (0.299, 0.587, 0.114)
MAX_SAMPLES = 200000
PERCENTILE_DIVISOR = 10
NAME_WIDTH = 40


def stats(path):
    image = Image.open(path).convert("RGB")
    pixels = list(image.get_flattened_data())
    step = max(1, len(pixels) // MAX_SAMPLES)
    pixels = pixels[::step]
    count = len(pixels)
    mean = [sum(pixel[channel] for pixel in pixels) / count for channel in range(3)]
    luma = sorted(sum(weight * value for weight, value in zip(LUMA_WEIGHTS, pixel)) for pixel in pixels)
    # Blue/red ratio of the darker half, where the night map would dominate.
    dark = [pixel for pixel in pixels
            if sum(weight * value for weight, value in zip(LUMA_WEIGHTS, pixel)) <= luma[count // 2]]
    dark_r = sum(pixel[0] for pixel in dark) / max(len(dark), 1)
    dark_b = sum(pixel[2] for pixel in dark) / max(len(dark), 1)
    chroma = sum(max(pixel) - min(pixel) for pixel in pixels) / count
    return (image.size, mean, luma[count // PERCENTILE_DIVISOR], luma[count // 2],
            luma[count * (PERCENTILE_DIVISOR - 1) // PERCENTILE_DIVISOR],
            mean[2] / max(mean[0], 1), dark_b / max(dark_r, 1), chroma)


def main(folder):
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        try:
            size, mean, p10, p50, p90, ratio, dark_ratio, chroma = stats(path)
        except OSError:
            continue
        print("%-40s %4dx%-4d mean (%3.0f,%3.0f,%3.0f) luma p10/p50/p90 %3.0f/%3.0f/%3.0f B/R %.2f dark B/R %.2f chroma %.0f" % (
            name[:NAME_WIDTH], size[0], size[1], mean[0], mean[1], mean[2], p10, p50, p90, ratio, dark_ratio, chroma))


if __name__ == "__main__":
    main(sys.argv[1])
