#!/usr/bin/env python3
"""
Render a facet overview texture as a parchment-style hand-drawn map.

The tool accepts DDS or ordinary image inputs. DDS decoding is delegated to
ImageMagick because Pillow support for compressed DDS variants is inconsistent
across installations. Output is always written as PNG.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter, ImageOps


DEFAULT_PALETTE = {
    "paper": np.array([179, 146, 96], dtype=np.float32),
    "water": np.array([91, 116, 109], dtype=np.float32),
    "grass": np.array([139, 150, 103], dtype=np.float32),
    "desert": np.array([206, 184, 133], dtype=np.float32),
    "mountain": np.array([180, 171, 139], dtype=np.float32),
    "snow": np.array([223, 209, 166], dtype=np.float32),
    "dark": np.array([126, 74, 31], dtype=np.float32),
    "road": np.array([102, 56, 20], dtype=np.float32),
    "shore": np.array([224, 216, 166], dtype=np.float32),
    "ink": np.array([70, 48, 32], dtype=np.float32),
}


@dataclass(frozen=True)
class StyleOptions:
    seed: int
    simplify_radius: int
    halo_radius: float
    paper_strength: float
    detail_strength: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stylize a UO facet overview texture into a paper-map PNG."
    )
    parser.add_argument("source", type=Path, help="Source facet image, usually facetXX.dds.")
    parser.add_argument("output", type=Path, help="PNG output path.")
    parser.add_argument(
        "--style-reference",
        type=Path,
        help="Optional aligned paper-map reference used to sample the output palette.",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=4,
        help="Deterministic paper texture seed.",
    )
    parser.add_argument(
        "--simplify-radius",
        type=int,
        default=5,
        help="Median-filter radius for broad cartographic simplification.",
    )
    parser.add_argument(
        "--halo-radius",
        type=float,
        default=18.0,
        help="Coastline wash radius in pixels.",
    )
    parser.add_argument(
        "--paper-strength",
        type=float,
        default=0.20,
        help="Strength of generated paper grain, from 0.0 to about 0.5.",
    )
    parser.add_argument(
        "--detail-strength",
        type=float,
        default=0.78,
        help="Strength used when redrawing roads and high-contrast map details.",
    )
    return parser.parse_args()


def load_image(path: Path) -> Image.Image:
    if path.suffix.lower() != ".dds":
        return Image.open(path).convert("RGB")

    magick = shutil.which("magick")
    if magick is None:
        raise RuntimeError("ImageMagick `magick` is required to decode DDS inputs.")

    with tempfile.TemporaryDirectory() as tmpdir:
        png_path = Path(tmpdir) / "decoded.png"
        subprocess.run(
            [magick, str(path), str(png_path)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return Image.open(png_path).convert("RGB")


def image_to_float(image: Image.Image) -> np.ndarray:
    return np.asarray(image.convert("RGB"), dtype=np.float32) / 255.0


def classify_source(rgb: np.ndarray) -> dict[str, np.ndarray]:
    r = rgb[:, :, 0]
    g = rgb[:, :, 1]
    b = rgb[:, :, 2]
    max_channel = np.max(rgb, axis=2)
    min_channel = np.min(rgb, axis=2)
    saturation = (max_channel - min_channel) / np.maximum(max_channel, 1.0 / 255.0)
    luma = 0.2126 * r + 0.7152 * g + 0.0722 * b

    void = (luma < 0.085) & (saturation < 0.12)
    water = ~void & (b > 0.22) & (g > 0.16) & (r < 0.20) & (b >= g * 0.90) & (saturation > 0.22)
    snow = ~void & ~water & (luma > 0.72) & (saturation < 0.22)
    desert = ~void & ~water & ~snow & (r > 0.42) & (g > 0.32) & (b < 0.35) & (r >= g * 0.92)
    mountain = ~void & ~water & ~snow & ~desert & (saturation < 0.24) & (luma > 0.20) & (luma < 0.72)
    green_terrain = (g > r * 1.08) & (g >= b * 0.95)
    dark = ~void & ~water & ~snow & ~green_terrain & (luma < 0.18)
    grass = ~(void | water | snow | desert | mountain | dark)

    road = ~void & ~water & (luma < 0.27) & (r > b * 0.95) & (g > b * 0.72)
    detail = ~void & ~water & ((luma < 0.13) | ((saturation > 0.55) & (luma < 0.58)))

    return {
        "paper": void,
        "water": water,
        "snow": snow,
        "desert": desert,
        "mountain": mountain,
        "dark": dark,
        "grass": grass,
        "road": road,
        "detail": detail,
        "land": ~(void | water),
        "luma": luma,
    }


def median_color(rgb: np.ndarray, mask: np.ndarray, fallback: np.ndarray) -> np.ndarray:
    if int(mask.sum()) < 64:
        return fallback
    return np.median(rgb[mask] * 255.0, axis=0).astype(np.float32)


def quantile_color(
    rgb: np.ndarray,
    mask: np.ndarray,
    quantile: float,
    fallback: np.ndarray,
) -> np.ndarray:
    pixels = rgb[mask]
    if len(pixels) < 64:
        return fallback

    luma = pixels @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    center = np.quantile(luma, quantile)
    band = np.abs(luma - center) <= 0.035
    if int(band.sum()) < 32:
        band = np.abs(luma - center) <= 0.065
    return np.median(pixels[band] * 255.0, axis=0).astype(np.float32)


def color_luma(color: np.ndarray) -> float:
    return float(color @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32))


def scale_to_luma(color: np.ndarray, target_luma: float) -> np.ndarray:
    current_luma = color_luma(color)
    if current_luma <= 0.0:
        return color
    return np.clip(color * (target_luma / current_luma), 0.0, 255.0)


def enforce_palette_order(palette: dict[str, np.ndarray]) -> dict[str, np.ndarray]:
    mountain_luma = color_luma(palette["mountain"])
    snow_luma = color_luma(palette["snow"])
    if mountain_luma >= snow_luma - 18.0:
        palette["mountain"] = scale_to_luma(palette["mountain"], max(80.0, snow_luma - 30.0))

    mountain_luma = color_luma(palette["mountain"])
    desert_luma = color_luma(palette["desert"])
    if desert_luma <= mountain_luma + 8.0:
        palette["desert"] = scale_to_luma(palette["desert"], min(230.0, mountain_luma + 18.0))

    return palette


def sample_palette(
    reference: Image.Image | None,
    masks: dict[str, np.ndarray],
) -> dict[str, np.ndarray]:
    if reference is None:
        return enforce_palette_order(dict(DEFAULT_PALETTE))

    ref_rgb = image_to_float(reference)
    palette = dict(DEFAULT_PALETTE)
    for name in ("paper", "water", "grass", "desert", "dark"):
        palette[name] = median_color(ref_rgb, masks[name], DEFAULT_PALETTE[name])

    palette["mountain"] = quantile_color(ref_rgb, masks["mountain"], 0.35, DEFAULT_PALETTE["mountain"])
    palette["snow"] = quantile_color(ref_rgb, masks["snow"], 0.72, DEFAULT_PALETTE["snow"])
    palette["road"] = median_color(ref_rgb, masks["road"], DEFAULT_PALETTE["road"])
    shore_mask = dilate_mask(masks["land"], 15) & ~masks["land"]
    palette["shore"] = median_color(ref_rgb, shore_mask, DEFAULT_PALETTE["shore"])
    return enforce_palette_order(palette)


def mask_image(mask: np.ndarray) -> Image.Image:
    return Image.fromarray(mask.astype(np.uint8) * 255, mode="L")


def filtered_mask(mask: np.ndarray, size: int) -> np.ndarray:
    if size <= 1:
        return mask
    if size % 2 == 0:
        size += 1
    image = mask_image(mask).filter(ImageFilter.MaxFilter(size=size))
    return np.asarray(image, dtype=np.uint8) > 0


def dilate_mask(mask: np.ndarray, size: int) -> np.ndarray:
    if size % 2 == 0:
        size += 1
    return np.asarray(mask_image(mask).filter(ImageFilter.MaxFilter(size=size)), dtype=np.uint8) > 0


def blend(base: np.ndarray, color: np.ndarray, alpha: np.ndarray) -> np.ndarray:
    alpha3 = alpha[:, :, None].astype(np.float32)
    return base * (1.0 - alpha3) + color[None, None, :] * alpha3


def add_paper_grain(rgb: np.ndarray, seed: int, strength: float) -> np.ndarray:
    if strength <= 0.0:
        return rgb

    height, width, _ = rgb.shape
    rng = np.random.default_rng(seed)
    fine = rng.normal(0.0, 1.0, size=(height, width)).astype(np.float32)

    coarse_size = (max(2, height // 32), max(2, width // 32))
    coarse = rng.normal(0.0, 1.0, size=coarse_size).astype(np.float32)
    coarse_image = Image.fromarray(normalize_u8(coarse), mode="L").resize(
        (width, height),
        Image.Resampling.BICUBIC,
    )
    coarse = (np.asarray(coarse_image, dtype=np.float32) / 255.0 - 0.5) * 2.0

    grain = 0.35 * fine + 0.65 * coarse
    grain = grain / max(float(np.std(grain)), 1e-6)
    factor = 1.0 + grain[:, :, None] * strength * 0.12
    return np.clip(rgb * factor, 0.0, 255.0)


def normalize_u8(values: np.ndarray) -> np.ndarray:
    low = float(values.min())
    high = float(values.max())
    if high <= low:
        return np.zeros(values.shape, dtype=np.uint8)
    return np.clip((values - low) / (high - low) * 255.0, 0.0, 255.0).astype(np.uint8)


def stylize(source: Image.Image, reference: Image.Image | None, options: StyleOptions) -> Image.Image:
    source_rgb = image_to_float(source)
    masks = classify_source(source_rgb)
    palette = sample_palette(reference, masks)
    height, width, _ = source_rgb.shape

    out = np.zeros((height, width, 3), dtype=np.float32)
    for name in ("paper", "water", "grass", "desert", "mountain", "snow", "dark"):
        out[masks[name]] = palette[name]

    luma = masks["luma"]
    class_luma = np.zeros((height, width), dtype=np.float32)
    for name in ("paper", "water", "grass", "desert", "mountain", "snow", "dark"):
        class_values = luma[masks[name]]
        class_luma[masks[name]] = float(np.median(class_values)) if class_values.size else 0.5
    relief = np.clip((luma - class_luma) * 105.0, -24.0, 30.0)
    relief[masks["paper"]] = 0.0
    out = np.clip(out + relief[:, :, None], 0.0, 255.0)

    simplified = Image.fromarray(out.astype(np.uint8), mode="RGB").filter(
        ImageFilter.MedianFilter(size=max(1, options.simplify_radius | 1))
    )
    simplified_rgb = np.asarray(simplified, dtype=np.float32)
    out = out * 0.32 + simplified_rgb * 0.68

    land = masks["land"]
    paper = masks["paper"]
    land_image = mask_image(land)
    land_blur = np.asarray(
        land_image.filter(ImageFilter.GaussianBlur(radius=options.halo_radius)),
        dtype=np.float32,
    ) / 255.0
    water_halo = np.clip(land_blur * masks["water"].astype(np.float32) * 1.9, 0.0, 0.72)
    out = blend(out, palette["shore"], water_halo)

    water_blur = np.asarray(
        ImageOps.invert(land_image).filter(ImageFilter.GaussianBlur(radius=options.halo_radius * 0.45)),
        dtype=np.float32,
    ) / 255.0
    inner_coast = np.clip(water_blur * land.astype(np.float32) * 1.1, 0.0, 0.36)
    out = blend(out, palette["shore"], inner_coast)

    road = filtered_mask(masks["road"], 3)
    road_alpha = np.asarray(
        mask_image(road).filter(ImageFilter.GaussianBlur(radius=0.45)),
        dtype=np.float32,
    ) / 255.0
    out = blend(out, palette["road"], np.clip(road_alpha * options.detail_strength, 0.0, 1.0))

    detail = filtered_mask(masks["detail"], 3)
    detail_alpha = np.asarray(
        mask_image(detail).filter(ImageFilter.GaussianBlur(radius=0.35)),
        dtype=np.float32,
    ) / 255.0
    out = blend(out, palette["ink"], np.clip(detail_alpha * options.detail_strength * 0.55, 0.0, 1.0))

    edge = land_image.filter(ImageFilter.FIND_EDGES).filter(ImageFilter.GaussianBlur(radius=0.55))
    edge_alpha = np.asarray(edge, dtype=np.float32) / 255.0
    edge_alpha[paper] *= 0.25
    out = blend(out, palette["ink"], np.clip(edge_alpha * 0.11, 0.0, 0.16))

    out = add_paper_grain(out, options.seed, options.paper_strength)
    return Image.fromarray(np.clip(out, 0.0, 255.0).astype(np.uint8), mode="RGB")


def main() -> int:
    args = parse_args()
    source = load_image(args.source)
    reference = load_image(args.style_reference) if args.style_reference else None

    if reference is not None and reference.size != source.size:
        raise ValueError(
            f"style reference size {reference.size} does not match source size {source.size}"
        )

    options = StyleOptions(
        seed=args.seed,
        simplify_radius=args.simplify_radius,
        halo_radius=args.halo_radius,
        paper_strength=args.paper_strength,
        detail_strength=args.detail_strength,
    )
    output = stylize(source, reference, options)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    output.save(args.output, format="PNG")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
