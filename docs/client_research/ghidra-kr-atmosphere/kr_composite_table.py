"""Numeric tables of the Kingdom Reborn lighting composite and the baked terrain shade.

Usage: python3 -I kr_composite_table.py NIGHTMAP_PNG NIGHTSIGHTMAP_PNG

The two images are previews of TerrainTexture.uop entries 21 (night map) and 22 (night-sight map), converted
outside the repository. Nothing is written; the Markdown tables go to stdout. Formulas: results/uokr_atmosphere.md.
"""
import math
import sys

import numpy
from PIL import Image

# UOLightingPostProcessEffect::BuildCompositeQuad (UOKR 0x0041f5c0): c = BASE + SLOPE * (gamma - CENTRE).
GAMMA_BASE = 0.25
GAMMA_SLOPE = 0.2
GAMMA_CENTRE = 0.5
# UOLightingShader stage 1 is MODULATE4X.
MODULATE_4X = 4.0
# UOAtmosphere::OnGlobalLightLevel (UOKR 0x0056e8a0): level = 1 - L / 31 for L < 31.
LIGHT_LEVEL_STEPS = 31
SERVER_LEVELS = (0, 5, 10, 15, 20, 26, 30)
GAMMAS = (0.0, 0.5, 1.0)
DEFAULT_GAMMA = 0.5
# UOAtmosphere::InitPostProcessEffects (UOKR 0x0041ecd0) starts at 0.75 before the server sends a level.
INITIAL_LIGHT_LEVEL = 0.75
# UOShadowPostProcessEffect intensity after InitPostProcessEffects.
SHADOW_INTENSITY = 0.25
# UOTerrain::BuildTerrainMesh (UOKR 0x0044b7f0): grey = (N . D + 1) * 0.5, D at 0x00c0f3b8.
HALF = 0.5
LIGHT_ELEVATION_DEGREES = 45.0
SLOPES_DEGREES = (0.0, 22.5, 45.0, 67.5, 90.0)
PERCENTILES = (10, 50, 90)
BYTE_MAX = 255.0
# Dynamapper for comparison: tonemap_ec_kr_profile (assets/shaders/postprocess/tonemapping.wesl, profile 2)
# with the UOD-Painterly afternoon exposure of assets/defaults/shader_presets.toml. The former separate KR gain
# (kr_tonemap_params.x, 0.9) is folded into that exposure (1.06 * 0.9), so the curve is unchanged.
DYNAMAPPER_KR_WHITE_CUTOFF = 0.8
DYNAMAPPER_KR_EXPOSURE = 0.954
TONEMAP_INPUTS = (0.1, 0.25, 0.5, 0.75, 1.0)


def load_rgb(path):
    return numpy.asarray(Image.open(path).convert("RGB"), dtype=numpy.float64) / BYTE_MAX


def stats(image):
    flat = image.reshape(-1, 3)
    rows = {"mean": flat.mean(axis=0)}
    for p in PERCENTILES:
        rows[f"p{p}"] = numpy.percentile(flat, p, axis=0)
    rows["max"] = flat.max(axis=0)
    return rows


def composite(level, gamma, buffer_rgb):
    c = GAMMA_BASE + GAMMA_SLOPE * (gamma - GAMMA_CENTRE)
    return MODULATE_4X * (c * level + buffer_rgb * (1.0 - level))


def fmt(rgb):
    return "(" + ", ".join(f"{v:.3f}" for v in rgb) + ")"


def main():
    night = stats(load_rgb(sys.argv[1]))
    sight = stats(load_rgb(sys.argv[2]))

    print("## Light buffers (0..1)\n")
    print("| Map | " + " | ".join(night.keys()) + " |")
    print("|---|" + "---|" * len(night))
    for name, rows in (("night map", night), ("night-sight map", sight)):
        print(f"| {name} | " + " | ".join(fmt(v) for v in rows.values()) + " |")

    print(f"\n## Scene multiplier, gamma {DEFAULT_GAMMA} (output = clamp(multiplier * scene))\n")
    print("| Server level L | level | night map mean | night map p10 | night map p90 | night sight mean |")
    print("|---|---|---|---|---|---|")
    levels = [(str(L), 1.0 - L / LIGHT_LEVEL_STEPS) for L in SERVER_LEVELS]
    levels.insert(1, ("start", INITIAL_LIGHT_LEVEL))
    for label, level in levels:
        print(f"| {label} | {level:.3f} | {fmt(composite(level, DEFAULT_GAMMA, night['mean']))} | "
              f"{fmt(composite(level, DEFAULT_GAMMA, night['p10']))} | "
              f"{fmt(composite(level, DEFAULT_GAMMA, night['p90']))} | "
              f"{fmt(composite(level, DEFAULT_GAMMA, sight['mean']))} |")

    print("\n## Gamma slider at full light (level 1)\n")
    print("| gamma | multiplier |")
    print("|---|---|")
    for gamma in GAMMAS:
        print(f"| {gamma} | {composite(1.0, gamma, numpy.zeros(3))[0]:.3f} |")

    print(f"\n## Shadow: scene * {1.0 - SHADOW_INTENSITY:.2f} under a sprite shadow\n")

    print("## Baked terrain shade (N . D + 1) * 0.5, light 45 degrees above the -row (north) horizon\n")
    print("| slope | facing the light | facing east or west | facing away |")
    print("|---|---|---|---|")
    elevation = math.radians(LIGHT_ELEVATION_DEGREES)
    light = numpy.array([0.0, math.sin(elevation), math.cos(elevation)])
    for slope in SLOPES_DEGREES:
        s = math.radians(slope)
        toward = numpy.array([0.0, math.cos(s), math.sin(s)])
        side = numpy.array([math.sin(s), math.cos(s), 0.0])
        away = numpy.array([0.0, math.cos(s), -math.sin(s)])
        shades = [(float(n @ light) + 1.0) * HALF for n in (toward, side, away)]
        print(f"| {slope} | " + " | ".join(f"{v:.3f}" for v in shades) + " |")

    print("\n## Dynamapper KR tonemap against the client (identity at full light)\n")
    print("| input | Dynamapper KR | client |")
    print("|---|---|---|")
    for x in TONEMAP_INPUTS:
        print(f"| {x} | {dynamapper_kr_tonemap(x):.3f} | {x:.3f} |")


def dynamapper_kr_tonemap(x):
    out = x * DYNAMAPPER_KR_EXPOSURE
    out *= 1.0 + out / (DYNAMAPPER_KR_WHITE_CUTOFF * DYNAMAPPER_KR_WHITE_CUTOFF)
    return out / (1.0 + out)


if __name__ == "__main__":
    main()
