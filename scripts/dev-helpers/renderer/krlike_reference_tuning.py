#!/usr/bin/env python3
"""Measure KR-like against the KR reference screenshots and render preset variants for tuning it.

Research helper for docs/agent_notes/krlike_reference_tuning.md. The screenshots in docs/images/kr_reference/ are
measured per region type (water, grass, sand, rock, snow, stone, wood, foliage) in hand-picked boxes; Dynamapper
views with the same content are captured with KR-like preset variants and measured the same way.

It reuses the launch machinery of shader_settings_audit.py (isolated assets and cache copies, KDE Desktop 4, the
F3 panel's Render Style and Time of Day buttons through the control socket). One launch holds up to four
variants, one in each time-of-day slot of their render style; the slot names are only storage.

A variant is a KR-like slot taken from a presets file and changed by `set` (flat `table.key` names, as in
shader_settings_audit.py). Variants are read from SCRATCH/variants.json:

    {"base.afternoon": {"presets": "base", "period": "afternoon", "set": {}},
     "water.amb": {"presets": "base", "period": "afternoon", "set": {"lighting.ambient_strength": 0.2}}}

`presets` is "base" (SCRATCH/baseline_presets.toml, the presets before tuning) or "repo" (the repository's
dynamapper/assets/defaults/shader_presets.toml). An optional `group` takes the slot from another render style
(classic, enhanced, painterly) instead of KR-like, for checks of the other styles.

Subcommands:
  refstats                     measure the reference boxes (SCRATCH/results/reference.json)
  run LAND V1,V2,.. VIEW,..    one launch per group of four variants, under the preview lock at low priority;
                               LAND is cc, ec or kr (art is always EC); captures in SCRATCH/caps/LAND/VARIANT/VIEW.png
  launch LAND V1,.. VIEW,..    the same in this process (wrap it in the preview lock yourself)
  stats LAND V1,.. [VIEW,..]   measure the capture boxes of the variants and print them beside the references
  sheet NAME LAND V1,.. VIEW,..
                               SCRATCH/sheets/NAME.png: one row per view, one column per variant
  refsheet NAME LAND V1,..     SCRATCH/sheets/NAME.png: per reference, the reference and its matched view in each
                               variant
  compare LAND V1,..           per region kind, the median luma and saturation of the references and each variant
  mockup LAND SOURCE NAME [--saturation S] [--knee K --slope M]
                               a variant NAME made from SOURCE's captures by a linear-space colour change: the
                               grading saturation (mix toward Rec. 709 luma), then a chroma knee (relative chroma
                               (max - min) / max above K scaled down to K + (c - K) * M by the same mix)

SCRATCH is /home/claudio/rust-target/uodynamapper-scratch/krlike-tuning unless KRLIKE_TUNING_SCRATCH is set. The
binary is a copy of a build of this repository placed in SCRATCH/rig/bin by hand.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

SCRIPT = Path(__file__).resolve()
REPO = Path(os.environ.get('DYNAMAPPER_REPO', SCRIPT.parents[3]))
SCRATCH = Path(os.environ.get('KRLIKE_TUNING_SCRATCH',
                              '/home/claudio/rust-target/uodynamapper-scratch/krlike-tuning'))
os.environ['SHADER_AUDIT_SCRATCH'] = str(SCRATCH)
_spec = importlib.util.spec_from_file_location('shader_settings_audit', SCRIPT.parent / 'shader_settings_audit.py')
audit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(audit)

REFERENCES = REPO / 'docs/images/kr_reference'
CAPS = SCRATCH / 'caps'
RESULTS = SCRATCH / 'results'
SHEETS = SCRATCH / 'sheets'
BASELINE_PRESETS = SCRATCH / 'baseline_presets.toml'
VARIANTS = SCRATCH / 'variants.json'
KR_GROUP = 'kr'
SLOTS_PER_LAUNCH = len(audit.TIMES)
LAND_SOURCES = ('cc', 'ec', 'kr')
LAUNCH_ID_DIGITS = 10
# The KR client's orthographic frustum is 26 world units wide (UOWorldViewer::SetDefaultFrustum), 18.4 tiles across:
# zoom 0.79 at a 1024-pixel window (see the KR atmosphere research, P2.7).
CLIENT_VIEW_ZOOM = 0.79
audit.ZOOM = CLIENT_VIEW_ZOOM

# (name, x, y, why). Map 0 (Felucca), teleported at z 0 like the audit views.
VIEWS = {
    'coast': (1445, 1790, 'Britain coast: open sea, shore statics, sand, docks'),
    'britain': (1434, 1699, 'Britain town: roofs, plaster and stone walls, roads, grass'),
    'yew': (620, 1020, 'Yew forest: foliage over grass'),
    'snow': (4050, 500, 'snow field and mountain'),
    'covetous': (2460, 880, 'Covetous cliffs: rock and mountain banks over grass'),
    'trinsic': (1867, 2780, 'Trinsic: sandstone and brick, a stone river bank, a wooden bridge'),
    'bucs': (2700, 2150, "Buccaneer's Den: a water inlet in grass banks, a roof, foliage"),
}

# Region boxes (x0, y0, x1, y1) in the image's own pixels. Each lists only pixels of one region type, picked by eye
# on the gridded images; the screenshots' HUD, logos and characters are left out.
REFERENCE_BOXES = {
    '6490.jpg': {
        'water': [(580, 278, 612, 318), (655, 268, 695, 296)],
        'snow': [(250, 450, 340, 650), (720, 600, 850, 700), (420, 60, 520, 140)],
        'rock': [(20, 250, 180, 450), (1000, 300, 1140, 450)],
        'grass': [(960, 560, 1100, 700)],
        'stone': [(470, 470, 600, 540), (650, 520, 760, 600)],
        'wood': [(380, 210, 420, 400), (850, 220, 890, 400)],
        'foliage': [(430, 160, 560, 250), (700, 150, 830, 240)],
    },
    '2712200822561889526.jpg': {
        'water': [(8, 600, 70, 690)],
        'grass': [(110, 40, 350, 140), (650, 20, 800, 120)],
        'stone': [(330, 330, 450, 450), (550, 390, 680, 470)],
        'wall': [(110, 570, 220, 680)],
        'foliage': [(10, 120, 80, 280)],
    },
    'gallery_74_1428_img.jpg': {
        'grass': [(0, 580, 200, 760), (480, 120, 600, 220)],
        'wall': [(110, 300, 230, 420), (470, 360, 560, 480)],
        'stone': [(0, 0, 100, 40), (220, 250, 330, 300)],
    },
    'UO_KR_screen.webp': {
        'dirt': [(200, 300, 420, 420), (180, 60, 330, 130)],
        'foliage': [(230, 460, 420, 560), (500, 360, 700, 480)],
        'wall': [(600, 130, 660, 280), (0, 400, 90, 500)],
    },
    'ultima-online-kingdom-reborn-20060825004646363-1647239.jpg': {
        'grass': [(0, 460, 100, 700), (100, 100, 200, 300)],
        'stone': [(250, 600, 420, 680), (780, 520, 880, 620)],
        'wall': [(120, 520, 220, 660), (520, 120, 580, 180)],
    },
    '350px-Lakeshirekr.jpg': {
        'grass': [(0, 200, 90, 300), (250, 100, 340, 200), (100, 280, 230, 340)],
        'foliage': [(20, 80, 70, 170), (170, 0, 240, 60)],
    },
    '254602-ultima-online-kingdom-reborn-base_16x9.jpg.jpg': {
        'grass': [(0, 60, 80, 150), (90, 160, 140, 200)],
        'wood': [(240, 120, 330, 170)],
        'foliage': [(0, 180, 120, 300)],
    },
    'sanctuary-warzone.jpg': {
        'dirt': [(300, 120, 400, 200), (420, 200, 490, 300)],
        'stone': [(220, 260, 320, 340)],
    },
}

# The view each reference is set beside on the reference sheet. The region boxes of the views' captures (window
# 1024 x 768, CLIENT_VIEW_ZOOM) are VIEW_BOXES, in the same form as REFERENCE_BOXES.
REFERENCE_VIEW = {
    '6490.jpg': 'snow',
    '2712200822561889526.jpg': 'coast',
    'gallery_74_1428_img.jpg': 'britain',
    'UO_KR_screen.webp': 'britain',
    'ultima-online-kingdom-reborn-20060825004646363-1647239.jpg': 'britain',
    '350px-Lakeshirekr.jpg': 'yew',
    '254602-ultima-online-kingdom-reborn-base_16x9.jpg.jpg': 'yew',
    'sanctuary-warzone.jpg': 'covetous',
}
VIEW_BOXES = {
    'coast': {
        'water': [[512, 256, 1000, 700]],
        'sand': [[200, 40, 420, 170], [75, 220, 150, 640]],
        'grass': [[0, 440, 60, 700]],
        'foliage': [[0, 250, 60, 380]],
    },
    'britain': {
        'stone': [[108, 120, 297, 278], [518, 452, 819, 634]],
        'roof': [[108, 381, 202, 523], [550, 104, 819, 223]],
        'grass': [[424, 452, 518, 555]],
        'foliage': [[305, 310, 368, 365], [384, 365, 439, 460]],
    },
    'yew': {
        'grass': [[108, 523, 234, 594], [692, 191, 779, 341], [487, 286, 582, 412]],
        'foliage': [[447, 81, 676, 152], [242, 373, 321, 436], [439, 460, 518, 523]],
    },
    'snow': {
        'snow': [[108, 318, 503, 634]],
        'dirt': [[740, 239, 898, 349]],
    },
    'covetous': {
        'rock': [[503, 239, 819, 555]],
        'dirt': [[226, 570, 384, 673]],
    },
    'trinsic': {
        'stone': [[64, 64, 320, 192], [600, 100, 880, 250], [200, 560, 450, 700]],
        'flag': [[420, 300, 620, 480]],
        'roof': [[0, 80, 120, 380]],
        'wall': [[384, 0, 440, 90], [520, 0, 600, 60]],
        'water': [[900, 440, 1010, 520], [650, 720, 720, 768]],
        'wood': [[680, 480, 880, 640]],
        'grass': [[60, 480, 170, 560]],
    },
    'bucs': {
        'water': [[110, 200, 400, 440], [300, 80, 560, 280]],
        'grass': [[0, 20, 100, 120], [460, 330, 560, 440]],
        'roof': [[700, 180, 1000, 400]],
        'wood': [[580, 480, 700, 640]],
        'foliage': [[480, 670, 680, 768], [540, 0, 760, 60]],
    },
}

# Region kinds pooled for the reference comparison (the median over the boxes of a kind, across images).
REGION_KINDS = {'water': ('water',), 'grass': ('grass',), 'foliage': ('foliage',), 'stone': ('stone', 'wall', 'flag'),
                'wood': ('wood',), 'ground': ('sand', 'dirt'), 'rock': ('rock',), 'snow': ('snow',)}
# sRGB transfer function, for mock-ups of linear-space shader terms applied to sRGB captures.
SRGB_LINEAR_KNEE = 0.04045
SRGB_ENCODED_KNEE = 0.0031308
SRGB_SLOPE = 12.92
SRGB_OFFSET = 0.055
SRGB_GAMMA = 2.4
THUMB_WIDTH = 512
LABEL_HEIGHT = 18
LOCAL_CONTRAST_SIGMA = 2.0


# ----------------------------------------------------------------------------------------------------------------
# Variants and launches
# ----------------------------------------------------------------------------------------------------------------

def load_variants():
    return json.loads(VARIANTS.read_text())


def presets_file(name):
    return BASELINE_PRESETS if name == 'base' else audit.FACTORY


def variant_slot(variant):
    with presets_file(variant['presets']).open('rb') as stream:
        presets = tomllib.load(stream)
    slot = audit.deterministic(audit.factory_slot(presets, variant.get('group', KR_GROUP), variant['period']))
    for key, value in variant['set'].items():
        assert key in slot, f'unknown setting {key}'
        slot[key] = value
    return slot


def launch_spec(land, names, views):
    variants = load_variants()
    configs = {name: {'id': name, 'mode': variants[name].get('group', KR_GROUP), 'slot': variant_slot(variants[name])}
               for name in names}
    graphics = {'art_texture_source': '"ec"', 'land_texture_source': json.dumps(land)}
    # The control socket lives in the run folder named by the id, and a socket path must fit SUN_LEN.
    digest = hashlib.sha1('+'.join(names).encode()).hexdigest()[:LAUNCH_ID_DIGITS]
    return {'id': f'{land}-{digest}', 'configs': names, 'graphics': graphics, 'water': 0,
            'views': views}, configs


def run(land, names, views):
    groups = [names[i:i + SLOTS_PER_LAUNCH] for i in range(0, len(names), SLOTS_PER_LAUNCH)]
    (SCRATCH / 'runs').mkdir(parents=True, exist_ok=True)
    for group in groups:
        log = SCRATCH / 'runs' / f'{launch_spec(land, group, views)[0]["id"]}.out'
        for attempt in range(audit.LAUNCH_ATTEMPTS):
            with log.open('w') as out:
                code = subprocess.call(['flock', str(audit.PREVIEW_LOCK), str(audit.LOWPRIO), sys.executable,
                                        str(SCRIPT), 'launch', land, ','.join(group), ','.join(views)],
                                       stdout=out, stderr=subprocess.STDOUT)
            print(f'{time.strftime("%H:%M:%S")} {land} {group} exit {code}', flush=True)
            if code == 0:
                break


def launch(land, names, views):
    spec, configs = launch_spec(land, names, views)
    runner = audit.Run(spec, configs, audit.load_factory())
    start = audit.VIEWS[0]
    with runner:
        runner.wait_in_game(position=[start[1], start[2], start[3], 0])
        runner.capture(runner.caps / '0_discard.png')
        for view in views:
            x, y, _ = VIEWS[view]
            runner.teleport((view, x, y, 0, ''))
            for name in names:
                runner.select(name)
                runner.capture(CAPS / land / name / f'{view}.png')
            if time.monotonic() - runner.started > audit.MAX_LAUNCH_SECONDS:
                raise TimeoutError(f'launch too long for the preview limit at {view}')


# ----------------------------------------------------------------------------------------------------------------
# Measurements
# ----------------------------------------------------------------------------------------------------------------

def load_rgb(path):
    import numpy as np
    from PIL import Image
    with Image.open(path) as image:
        return np.asarray(image.convert('RGB'), dtype=np.float32)


def local_contrast(image):
    """Mean absolute difference from a Gaussian blur of the luma: texture detail and sharpness at a few pixels."""
    import numpy as np
    luma = rec709(image)
    radius = int(3 * LOCAL_CONTRAST_SIGMA)
    taps = np.exp(-0.5 * (np.arange(-radius, radius + 1) / LOCAL_CONTRAST_SIGMA) ** 2)
    taps /= taps.sum()
    padded = np.pad(luma, radius, mode='edge')
    rows = sum(w * padded[:, i:i + luma.shape[1]] for i, w in enumerate(taps))
    blurred = sum(w * rows[i:i + luma.shape[0], :] for i, w in enumerate(taps))
    return np.abs(luma - blurred)


def rec709(image):
    return image[..., 0] * 0.2126 + image[..., 1] * 0.7152 + image[..., 2] * 0.0722


def region_stats(image, boxes):
    """Per region: mean RGB, luma mean and spread, HSV saturation, hue and local contrast over its boxes."""
    import colorsys
    import numpy as np
    detail = local_contrast(image)
    out = {}
    for region, rects in boxes.items():
        pixels = np.concatenate([image[y0:y1, x0:x1].reshape(-1, 3) for x0, y0, x1, y1 in rects])
        local = np.concatenate([detail[y0:y1, x0:x1].ravel() for x0, y0, x1, y1 in rects])
        mean = pixels.mean(axis=0)
        top = pixels.max(axis=1)
        saturation = np.where(top > 0, (top - pixels.min(axis=1)) / np.maximum(top, 1e-6), 0.0)
        hue, _, _ = colorsys.rgb_to_hsv(*(mean / 255.0))
        luma = rec709(pixels[None])[0]
        out[region] = {'rgb': [round(float(v), 1) for v in mean], 'luma': round(float(luma.mean()), 1),
                       'luma_sd': round(float(luma.std()), 1), 'sat': round(float(saturation.mean()), 3),
                       'hue': round(hue * 360.0), 'detail': round(float(local.mean()), 2)}
    return out


def frame_stats(image):
    import numpy as np
    luma = rec709(image)
    return {f'p{p}': round(float(np.percentile(luma, p)), 1) for p in (5, 25, 50, 75, 95)}


def refstats():
    RESULTS.mkdir(parents=True, exist_ok=True)
    out = {}
    for name, boxes in REFERENCE_BOXES.items():
        image = load_rgb(REFERENCES / name)
        out[name] = {'regions': region_stats(image, boxes), 'frame': frame_stats(image)}
    (RESULTS / 'reference.json').write_text(json.dumps(out, indent=1))
    print_table(out)


def capture_stats(land, names, views):
    boxes = VIEW_BOXES
    out = {}
    for name in names:
        for view in views:
            path = CAPS / land / name / f'{view}.png'
            if path.exists() and view in boxes:
                image = load_rgb(path)
                out[f'{land}/{name}/{view}'] = {'regions': region_stats(image, boxes[view]),
                                                'frame': frame_stats(image)}
    RESULTS.mkdir(parents=True, exist_ok=True)
    path = RESULTS / 'captures.json'
    merged = json.loads(path.read_text()) if path.exists() else {}
    merged.update(out)
    path.write_text(json.dumps(merged, indent=1))
    print_table(out)


def print_table(results):
    print(f'{"image":44} {"region":8} {"R":>6} {"G":>6} {"B":>6} {"luma":>6} {"sd":>5} {"sat":>5} {"hue":>4} '
          f'{"detail":>6}')
    for name, entry in results.items():
        for region, s in entry['regions'].items():
            r, g, b = s['rgb']
            print(f'{name[-44:]:44} {region:8} {r:6.1f} {g:6.1f} {b:6.1f} {s["luma"]:6.1f} {s["luma_sd"]:5.1f} '
                  f'{s["sat"]:5.2f} {s["hue"]:4d} {s["detail"]:6.2f}')
        print(f'{name[-44:]:44} {"frame":8} ' + ' '.join(f'{k} {v}' for k, v in entry['frame'].items()))


def kind_medians(entries):
    """entries: {name: {'regions': ...}} -> {kind: (median luma, median saturation, box count)}."""
    import numpy as np
    out = {}
    for kind, regions in REGION_KINDS.items():
        found = [entry['regions'][region] for entry in entries for region in regions if region in entry['regions']]
        if found:
            out[kind] = (float(np.median([s['luma'] for s in found])), float(np.median([s['sat'] for s in found])),
                         len(found))
    return out


def compare(land, names):
    references = json.loads((RESULTS / 'reference.json').read_text())
    captures = json.loads((RESULTS / 'captures.json').read_text())
    columns = [('reference', kind_medians(references.values()))]
    for name in names:
        prefix = f'{land}/{name}/'
        columns.append((name, kind_medians([v for k, v in captures.items() if k.startswith(prefix)])))
    print(f'{"kind":8} ' + ' '.join(f'{label[:20]:>20}' for label, _ in columns))
    for kind in REGION_KINDS:
        cells = []
        for _, medians in columns:
            luma, sat, count = medians.get(kind, (float('nan'), float('nan'), 0))
            cells.append(f'{luma:6.1f} {sat:5.2f} n{count:<3d}'.rjust(20))
        print(f'{kind:8} ' + ' '.join(cells))


def srgb_to_linear(x):
    import numpy as np
    return np.where(x <= SRGB_LINEAR_KNEE, x / SRGB_SLOPE, ((x + SRGB_OFFSET) / (1 + SRGB_OFFSET)) ** SRGB_GAMMA)


def linear_to_srgb(x):
    import numpy as np
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= SRGB_ENCODED_KNEE, x * SRGB_SLOPE, (1 + SRGB_OFFSET) * x ** (1 / SRGB_GAMMA) - SRGB_OFFSET)


def mock_colour(image, saturation, knee, slope):
    """The colour change of `mockup` on an sRGB capture (0..255)."""
    import numpy as np
    linear = srgb_to_linear(image / 255.0)
    luma = rec709(linear)[..., None]
    linear = luma + (linear - luma) * saturation
    if knee is not None:
        top = linear.max(axis=-1, keepdims=True)
        chroma = np.where(top > 0, (top - linear.min(axis=-1, keepdims=True)) / np.maximum(top, 1e-6), 0.0)
        target = np.where(chroma > knee, knee + (chroma - knee) * slope, chroma)
        linear = luma + (linear - luma) * np.where(chroma > 0, target / np.maximum(chroma, 1e-6), 1.0)
    return linear_to_srgb(linear) * 255.0


def mockup(land, source, name, saturation, knee, slope):
    from PIL import Image
    import numpy as np
    out = CAPS / land / name
    out.mkdir(parents=True, exist_ok=True)
    for path in sorted((CAPS / land / source).glob('*.png')):
        image = mock_colour(load_rgb(path), saturation, knee, slope)
        Image.fromarray(np.round(image).astype(np.uint8)).save(out / path.name)
    print(out)


# ----------------------------------------------------------------------------------------------------------------
# Sheets
# ----------------------------------------------------------------------------------------------------------------

def thumb(path):
    from PIL import Image
    with Image.open(path) as image:
        image = image.convert('RGB')
        return image.resize((THUMB_WIDTH, round(image.height * THUMB_WIDTH / image.width)))


def grid(rows, title):
    """rows: [[(label, image or None), ...], ...] -> one sheet with a label over every cell."""
    from PIL import Image, ImageDraw
    columns = max(len(row) for row in rows)
    heights = [max((cell.height for _, cell in row if cell is not None), default=0) + LABEL_HEIGHT for row in rows]
    sheet = Image.new('RGB', (columns * THUMB_WIDTH, sum(heights) + LABEL_HEIGHT), 'black')
    draw = ImageDraw.Draw(sheet)
    draw.text((4, 2), title, fill='white')
    y = LABEL_HEIGHT
    for row, height in zip(rows, heights):
        for column, (label, cell) in enumerate(row):
            x = column * THUMB_WIDTH
            draw.text((x + 4, y + 2), label, fill='yellow')
            if cell is not None:
                sheet.paste(cell, (x, y + LABEL_HEIGHT))
        y += height
    return sheet


def sheet(name, land, names, views):
    rows = []
    for view in views:
        row = []
        for variant in names:
            path = CAPS / land / variant / f'{view}.png'
            row.append((f'{view} {land} land: {variant}', thumb(path) if path.exists() else None))
        rows.append(row)
    SHEETS.mkdir(parents=True, exist_ok=True)
    grid(rows, f'{name}: KR-like, EC art, {land.upper()} land').save(SHEETS / f'{name}.png')
    print(SHEETS / f'{name}.png')


def refsheet(name, land, names):
    rows = []
    for reference, view in REFERENCE_VIEW.items():
        row = [(f'reference {reference[:40]}', thumb(REFERENCES / reference))]
        for variant in names:
            path = CAPS / land / variant / f'{view}.png'
            row.append((f'{view}: {variant}', thumb(path) if path.exists() else None))
        rows.append(row)
    SHEETS.mkdir(parents=True, exist_ok=True)
    grid(rows, f'{name}: KR reference beside KR-like ({land.upper()} land, EC art)').save(SHEETS / f'{name}.png')
    print(SHEETS / f'{name}.png')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('refstats')
    for command in ('run', 'launch'):
        p = sub.add_parser(command)
        p.add_argument('land', choices=LAND_SOURCES)
        p.add_argument('variants')
        p.add_argument('views')
    p = sub.add_parser('stats')
    p.add_argument('land', choices=LAND_SOURCES)
    p.add_argument('variants')
    p.add_argument('views', nargs='?', default=','.join(VIEWS))
    p = sub.add_parser('sheet')
    p.add_argument('name')
    p.add_argument('land', choices=LAND_SOURCES)
    p.add_argument('variants')
    p.add_argument('views')
    p = sub.add_parser('refsheet')
    p.add_argument('name')
    p.add_argument('land', choices=LAND_SOURCES)
    p.add_argument('variants')
    p = sub.add_parser('compare')
    p.add_argument('land', choices=LAND_SOURCES)
    p.add_argument('variants')
    p = sub.add_parser('mockup')
    p.add_argument('land', choices=LAND_SOURCES)
    p.add_argument('source')
    p.add_argument('name')
    p.add_argument('--saturation', type=float, default=1.0)
    p.add_argument('--knee', type=float)
    p.add_argument('--slope', type=float, default=1.0)
    args = parser.parse_args()
    split = lambda text: [item for item in text.split(',') if item]
    if args.command == 'refstats':
        refstats()
    elif args.command == 'run':
        run(args.land, split(args.variants), split(args.views))
    elif args.command == 'launch':
        launch(args.land, split(args.variants), split(args.views))
    elif args.command == 'stats':
        capture_stats(args.land, split(args.variants), split(args.views))
    elif args.command == 'sheet':
        sheet(args.name, args.land, split(args.variants), split(args.views))
    elif args.command == 'refsheet':
        refsheet(args.name, args.land, split(args.variants))
    elif args.command == 'compare':
        compare(args.land, split(args.variants))
    elif args.command == 'mockup':
        mockup(args.land, args.source, args.name, args.saturation, args.knee, args.slope)


if __name__ == '__main__':
    main()
