#!/usr/bin/env python3
"""
Merge modular WGSL shader parts into a single `land_base.wgsl`.
Run: python3 tools/merge_shaders.py
"""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
shaders = root / 'assets' / 'shaders' / 'worldmap'
parts = [
    'land_common.wgsl',
    'land_atlas.wgsl',
    'land_noise.wgsl',
    'land_interp.wgsl',
    'land_normals.wgsl',
    'land_sampling.wgsl',
    'land_lighting.wgsl',
    'land_shading.wgsl',
    'land_main.wgsl',
]

out = shaders / 'land_base.wgsl'
with out.open('w', encoding='utf-8') as f:
    for p in parts:
        src = shaders / p
        if not src.exists():
            raise SystemExit(f"Missing shader part: {src}")
        f.write(f"// --------- begin: {p} ---------\n")
        f.write(src.read_text(encoding='utf-8'))
        f.write(f"\n// --------- end: {p} ---------\n\n")

print(f"Wrote {out}")
