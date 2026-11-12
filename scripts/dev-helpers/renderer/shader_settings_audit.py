#!/usr/bin/env python3
"""Measure what each Dynamapper shader preset setting changes on screen.

Research helper for docs/agent_notes/shader_settings_audit.md. It renders fixed views in every preset and time of
day where a setting can act, once at the factory value (assets/defaults/shader_presets.toml) and once neutralised,
and reports the pixel change. No rendering code is changed: every variant is a value in the run's own cache copy of
settings/shaders/land.toml.

Batching: one app launch holds up to four variants per render style, one in each time-of-day slot. The script
writes them into the slots, starts on the first one and switches with the F3 panel's Render Style and Time of Day
buttons through the app-only control socket. Slot names are only storage; the variant's values are what render.

Subcommands:
  plan                 build the variant plan (SCRATCH/plan.json) and print the counts
  calibrate            one launch: captures the open F3 panel and candidate views, prints static light counts
  probe                one launch: finds the candidate view where static lights change the most pixels at night
  run [ID...]          run launches of the plan (all by default), each under the preview lock at low priority
  launch ID...         run launches in this process; wrap it in the preview lock yourself
  metrics              diff every capture against its baseline (SCRATCH/results/metrics.json)
  sheets               contact sheets per setting (SCRATCH/sheets/)
  report               markdown tables (SCRATCH/results/report.md)
  strong-plan [--settings A,B]
                       plan of the kept low-impact settings at strong values (SCRATCH/plan.json), optionally only
                       some of them, with the factory slots in EC and CC textures; then `run`
  strong-metrics       diff every strong capture against its condition's off column
  strong-report        markdown tables (SCRATCH/results/strong_report.md)
  strong-sheets        one side-by-side sheet per setting, style and texture set (SCRATCH/sheets/)
  strong-compare BEFORE
                       compare every capture with the same capture of another scratch folder (the same plan run
                       with an earlier build): SCRATCH/results/compare.md and one before/after sheet per style
                       and texture set of the factory slots (SCRATCH/sheets/compare/)

Typical session: plan, run, metrics, report, sheets. For strong values, in a scratch folder of its own:
strong-plan, run, strong-metrics, strong-report, strong-sheets. For a build's before and after, run the same
strong plan in two scratch folders, each with its own rig binary and pinned assets, then strong-compare in the
later one. `run` starts each launch as its own process under
the shared preview lock and lowprio.sh, so other agents' GPU runs can interleave; every app start goes to KDE
Desktop 4 only.

The binary is a copy of a build of this repository placed in SCRATCH/rig/bin by hand. The user's saved settings
are never read or written: every launch gets a private working directory with copies of assets/ and cache/.
"""
import argparse
import importlib.util
import json
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import tomllib
from pathlib import Path

SCRIPT = Path(__file__).resolve()
REPO = Path(os.environ.get('DYNAMAPPER_REPO', SCRIPT.parents[3]))
SCRATCH = Path(os.environ.get('SHADER_AUDIT_SCRATCH',
                              '/home/claudio/rust-target/uodynamapper-scratch/shader-settings-audit'))
BINARY = SCRATCH / 'rig/bin/dynamapper'
RUNS = SCRATCH / 'runs'
CAPS = SCRATCH / 'caps'
RESULTS = SCRATCH / 'results'
SHEETS = SCRATCH / 'sheets'
PLAN = SCRATCH / 'plan.json'
PACKAGES = Path('/home/claudio/test/uddp')
PREVIEW = REPO / 'skills/kde-x11-preview/scripts/preview.py'
DESKTOP = 'Desktop 4'
# Launches copy the assets from SCRATCH/assets when that folder exists, a copy pinned when the measurement started,
# so later edits to the working tree's shaders and presets cannot reach a measurement already under way.
PINNED_ASSETS = SCRATCH / 'assets'
ASSETS = PINNED_ASSETS if PINNED_ASSETS.is_dir() else REPO / 'dynamapper/assets'
FACTORY = ASSETS / 'defaults/shader_presets.toml'
BUNDLED_PRESETS = Path('settings/shaders/land.toml')

MODES = ('classic', 'enhanced', 'kr', 'painterly')
TIMES = ('morning', 'afternoon', 'night', 'cave')
SLOTS_PER_MODE = len(TIMES)
MODE_BUTTON = {'classic': 'Classic', 'enhanced': 'Enhanced', 'kr': 'KR-like', 'painterly': 'UOD-Painterly'}
# The Render Style buttons force each group's shading mode and post-process profile. KR-like and UOD-Painterly force
# the same pair, but KR-like lights the night through the client composite and loads a neutral scene multiplier
# (global_lighting 1) where UOD-Painterly keeps its own, so each group's values need that group's slots.
SLOT_GROUPS = {'classic': ('classic',), 'enhanced': ('enhanced',), 'kr': ('kr',), 'painterly': ('painterly',)}
TIME_BUTTON = {'morning': 'Morning', 'afternoon': 'Afternoon', 'night': 'Night', 'cave': 'Cave'}

# Well under two thirds of the 2880 x 1692 screen, or the preview comes up maximized.
WINDOW = (1024, 768)
ZOOM = 1.0
# Logical window pixels of the F3 panel's top-row buttons at WINDOW, ui_scale 1.15 (from `calibrate`).
PANEL_BUTTONS = {
    'Classic': (132, 149), 'Enhanced': (204, 149), 'KR-like': (278, 149), 'UOD-Painterly': (367, 149),
    'Morning': (569, 149), 'Afternoon': (650, 149), 'Night': (721, 149), 'Cave': (774, 149),
}

# (name, x, y, z, why). Map 0 (Felucca).
VIEWS = (
    ('slopes', 5650, 2950, 0, 'open terrain with slopes (Lost Lands)'),
    ('britain', 1434, 1699, 0, 'Britain town: buildings, roads, lamp posts'),
    ('yew', 620, 1020, 0, 'Yew forest: dense foliage statics'),
    ('covetous', 2460, 880, 0, 'Covetous cliffs: mountain banks and steep relief'),
    ('water', 1500, 1750, 0, 'Britain coast: open sea (terrain water), a shore and wooden docks'),
    ('lights', 1960, 2064, 0, 'a coastal building east of Britain whose wall lights throw the widest glow of the '
                              'probed spots (about 6% of the frame at Enhanced night)'),
)

# base-b captures these views twice; all six would not fit in one preview run.
REPEAT_VIEWS = ('slopes', 'britain')
# preview.py refuses --seconds above 300.
PREVIEW_SECONDS = 300
PLACEMENT_TIMEOUT_SECONDS = 60
SOCKET_TIMEOUT_SECONDS = 180
SOCKET_POLL_SECONDS = 0.2
HELPER_EXIT_SECONDS = 30
LOG_TAIL_CHARS = 3000
REQUEST_TIMEOUT_SECONDS = 60
SCENE_TIMEOUT_SECONDS = 240
SCENE_WAIT_SECONDS = 60
SCENE_RETRY_SECONDS = 0.5
# As control.wait_for_scene: three equal status snapshots 0.1 s apart.
SCENE_STABLE_SNAPSHOTS = 3
SCENE_POLL_SECONDS = 0.1
TELEPORT_SETTLE_SECONDS = 2.0
CAPTURE_TIMEOUT_SECONDS = 60
CAPTURE_POLL_SECONDS = 0.03
INPUT_TIMEOUT_SECONDS = 20
# egui lays a new window out in an invisible sizing pass; give it a few frames before clicking.
PANEL_OPEN_SETTLE_SECONDS = 0.25
CLICK_SETTLE_SECONDS = 0.08
# Window fade-out, uniform push and material refresh after the panel closes.
PANEL_CLOSE_SETTLE_SECONDS = 0.35
VERIFY_TOLERANCE = 1e-4
MAX_LAUNCH_SECONDS = 270

# Metrics. A pixel's change is its largest channel change, 0..255.
CHANGE_LEVEL = 4
PERCENTILE = 99
# Region masks: a pixel belongs to a region when removing that content changes it by more than CHANGE_LEVEL.
REGION_ORDER = ('water', 'lights', 'statics', 'land')
WHOLE_FRAME_SHARE = 0.5
REGION_REPORT_SHARE = 0.15
# Proposed low-impact threshold: below both in every reachable condition.
LOW_MEAN = 0.5
LOW_SHARE = 0.01

spec = importlib.util.spec_from_file_location('control', REPO / 'scripts/dev-helpers/renderer/control.py')
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)


# ----------------------------------------------------------------------------------------------------------------
# Presets
# ----------------------------------------------------------------------------------------------------------------

def load_factory():
    with FACTORY.open('rb') as stream:
        return tomllib.load(stream)


def flatten(slot):
    out = {}
    for key, value in slot.items():
        if isinstance(value, dict):
            for sub, item in value.items():
                out[f'{key}.{sub}'] = item
        else:
            out[key] = value
    return out


def unflatten(flat):
    out = {}
    for key, value in flat.items():
        if '.' in key:
            table, sub = key.split('.', 1)
            out.setdefault(table, {})[sub] = value
        else:
            out[key] = value
    return out


def toml_value(value):
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        text = repr(value)
        return text if any(c in text for c in '.en') else text + '.0'
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(item) for item in value) + ']'
    return json.dumps(value)


def presets_toml(slots, default_preset):
    """slots: {(mode, time): flat slot}. Writes the full LandShaderModePresets structure."""
    lines = [f'default_preset = {json.dumps(default_preset)}', '']
    for mode in MODES:
        for period in TIMES:
            nested = unflatten(slots[(mode, period)])
            lines.append(f'[{mode}.{period}]')
            lines += [f'{k} = {toml_value(v)}' for k, v in nested.items() if not isinstance(v, dict)]
            lines.append('')
            for table, values in nested.items():
                if isinstance(values, dict):
                    lines.append(f'[{mode}.{period}.{table}]')
                    lines += [f'{k} = {toml_value(v)}' for k, v in values.items()]
                    lines.append('')
    return '\n'.join(lines)


def factory_slot(factory, mode, period):
    return flatten(factory[mode][period])


def deterministic(slot):
    """The baseline of every measurement: the factory slot with its two time-driven terms frozen.

    Water animation moves the water UVs every frame, and the KR fog's cloud veil drifts with time; both would make
    two captures of the same values differ. Each is measured on its own (see the water launch and fog_params.w)."""
    out = dict(slot)
    out['effects.enable_water_animation'] = 0
    fog = list(out['lighting.fog_params'])
    fog[3] = 0.0
    out['lighting.fog_params'] = fog
    return out


# ----------------------------------------------------------------------------------------------------------------
# Settings: reachability (from the shaders) and neutral values
# ----------------------------------------------------------------------------------------------------------------

def luma(rgb):
    return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]


def grey_same_luma(value):
    """Neutral tint: a grey of the same luminance (keeps a 4th component)."""
    grey = round(luma(value), 6)
    return [grey, grey, grey] + list(value[3:])


def shading(s):
    return s['effects.shading_mode']


def profile(s):
    return s['effects.post_process_profile']


def fragment(s):
    return shading(s) != 0


def tonemap_on(s):
    return s['lighting.enable_tonemap'] == 1


def grading_on(s):
    return s['lighting.enable_grading'] == 1 and s['lighting.grade_params'][0] > 0


def fog_on(s):
    fog = s['lighting.fog_params']
    return (s['lighting.enable_fog'] == 1 and s['lighting.fog_color'][3] > 0.001
            and (fog[0] >= 0.001 or fog[1] >= 0.001))


def gloom_on(s):
    return fragment(s) and s['lighting.enable_gloom'] == 1 and s['lighting.gloom_params'][0] >= 1e-4


def fake_normals_on(s):
    return s['effects.enable_art_fake_normals'] == 1


def projected_on(s):
    return s['effects.enable_art_projected_shadows'] == 1 and s['effects.art_projected_shadow_strength'] > 0


def blur_on(s):
    return (s['effects.enable_blur'] == 1 and s['effects.blur_strength'] > 0.001
            and s['effects.blur_radius'] > 0)


def wash_on(s):
    return (profile(s) == 2 and s['effects.enable_kr_color_wash'] == 1
            and s['effects.kr_color_wash_strength'] > 0)


# static_lights.rs: (max_alpha, daylight_alpha, darkness_gamma) per render style; the mask alpha is
# min(daylight + (1 - global_lighting)^gamma, daylight + max) * visibility.
STATIC_LIGHT_STYLE = {0: (None, 0.0, 1.0), 1: (0.50, 0.03, 1.12), 2: (0.20, 0.0, 1.50)}


def client_lit(s):
    """KR-like and Enhanced dim through the clients' night composite: no global_lighting, no light decals."""
    return 'global_lighting' not in s


def static_light_mask_visible(s):
    if client_lit(s):
        return False
    _, daylight, gamma = STATIC_LIGHT_STYLE[shading(s)]
    darkness = min(max(1.0 - s['global_lighting'], 0.0), 1.0)
    return daylight + darkness ** gamma > 0


def rim_on(s):
    scale = 0.25 if shading(s) == 1 else 1.0
    return fragment(s) and s['land_lighting.rim_strength'] * scale > 0.001


class Spec:
    """One measured setting (or one component of a vector setting).

    reach(slot) says whether the shaders can act on it in that slot; neutral(value, slot) gives its off or identity
    value. kind 'neutral' renders the variant at the neutral value; 'reverse' renders the factory value against a
    baseline that already holds the neutral one (the frozen fog veil); 'probe' turns on a setting every factory
    preset leaves off, to see what it would do."""

    def __init__(self, key, neutral, reach, how, comp=None, kind='neutral', modes=MODES, times=TIMES):
        self.key, self.comp, self.neutral, self.reach, self.how = key, comp, neutral, reach, how
        self.kind, self.modes, self.times = kind, modes, times
        self.name = key if comp is None else f'{key}.{"xyzw"[comp]}'

    def value(self, slot):
        # A slot may lack a key its style has no use for (global_lighting in the client-lit styles); reach is then
        # False, so the value is never rendered.
        value = slot.get(self.key)
        return value if self.comp is None or value is None else value[self.comp]

    def with_value(self, slot, value):
        out = dict(slot)
        if self.comp is None:
            out[self.key] = value
        else:
            vec = list(slot[self.key])
            vec[self.comp] = value
            out[self.key] = vec
        return out


def const(value):
    return lambda _value, _slot: value


ALWAYS = lambda s: True
SPECS = [
    Spec('global_lighting', const(1.0), lambda s: not client_lit(s), '1.0 (identity multiplier)'),
    Spec('effects.normal_mode', const(0), ALWAYS, '0 (geometric normals)'),
    Spec('effects.enable_blur', const(0), blur_on, '0'),
    Spec('effects.blur_radius', const(0.5), blur_on, '0.5 (the shader floor of half a screen pixel)',
         times=('afternoon',)),
    Spec('effects.enable_linear_filtering', const(0), ALWAYS, '0 (nearest texels)'),
    Spec('effects.enable_grunge', const(0), lambda s: s['effects.grunge_strength'] > 0.0001, '0'),
    Spec('effects.enable_kr_transition_grime', const(0),
         lambda s: profile(s) == 2 and s['effects.kr_transition_grime_strength'] > 0, '0'),
    Spec('effects.enable_kr_color_wash', const(0), wash_on, '0'),
    Spec('effects.kr_color_wash_scale', lambda v, s: min(v * 2.0, 160.0), wash_on,
         'twice the preset (no identity value; GUI maximum 160)'),
    Spec('effects.light_decal_intensity', const(0.0), ALWAYS, '0'),
    Spec('effects.static_light_decal_visibility', const(0.0), static_light_mask_visible, '0'),
    Spec('effects.enable_art_fake_normals', const(0), ALWAYS, '0'),
    Spec('effects.art_contact_shadow_strength', const(0.0), fake_normals_on, '0'),
    Spec('effects.kr_land_temperature_strength', const(0.0), lambda s: shading(s) == 2, '0'),
    Spec('effects.kr_land_relief_shadow_strength', const(0.0), lambda s: shading(s) == 2, '0'),
    Spec('effects.kr_land_shadow_mottle_strength', const(0.0), lambda s: shading(s) == 2, '0'),
    Spec('effects.kr_land_material_contrast_strength', const(0.0), lambda s: profile(s) == 2, '0'),
    Spec('effects.kr_art_temperature_strength', const(0.0), fake_normals_on, '0'),
    Spec('effects.enable_art_projected_shadows', const(0),
         lambda s: s['effects.art_projected_shadow_strength'] > 0, '0'),
    Spec('effects.art_projected_shadow_length', const(0.0), projected_on, '0 (shadow collapsed to the base)'),
    Spec('effects.art_projected_shadow_softness', const(0.05), projected_on, '0.05 (hardest edge, shader minimum)'),
    Spec('effects.art_shadow_strength', const(0.0), ALWAYS, '0'),
    Spec('effects.art_highlight_strength', const(0.0), ALWAYS, '0'),
    Spec('effects.art_depth_tint_strength', const(0.0), ALWAYS, '0'),
    Spec('lighting.enable_fog', const(0), fog_on, '0'),
    Spec('lighting.fog_color', lambda v, s: grey_same_luma(v), fog_on, 'grey of the same luminance (alpha kept)'),
    Spec('lighting.fog_params', const(0.0), lambda s: fog_on(s) and s['lighting.fog_params'][1] >= 0.001,
         '0 (distance density)', comp=0),
    Spec('lighting.fog_params', const(0.0), lambda s: fog_on(s) and s['lighting.fog_params'][0] >= 0.001,
         '0 (height density)', comp=1),
    Spec('lighting.fog_params', lambda v, s: v, fog_on,
         'cloud veil on at the preset value against the frozen baseline (0)', comp=3, kind='reverse'),
    Spec('lighting.fog_night_color', const(0.0), fog_on, '0 (night blend: day fog colour only)', comp=3),
    Spec('lighting.enable_tonemap', const(0), ALWAYS, '0'),
    Spec('lighting.exposure', const(1.0), tonemap_on, '1.0'),
    Spec('lighting.enable_grading', const(0), grading_on, '0'),
    Spec('lighting.grade_params', const(0.0),
         lambda s: shading(s) == 2 and s['lighting.grade_params'][3] >= 0.5, '0 (KR headroom reserve)', comp=1),
    Spec('lighting.grade_params', const(0.0), lambda s: fragment(s) and s['land_lighting.fill_strength'] > 0,
         '0 (fill chroma tint)', comp=2),
    Spec('lighting.grade_params', const(0.0), fragment, '0 (headroom limit off)', comp=3),
    Spec('lighting.grade_extra', const([0.0, 1.0, 1.0, 0.0]), grading_on,
         '[0, 1, 1, 0] (no vibrance, saturation 1, contrast 1, no split toning)'),
    Spec('lighting.grade_chroma_knee', const(1.0), grading_on, '1 (chroma knee off)'),
    Spec('lighting.grade_warm_color', lambda v, s: [1.0, 1.0, 1.0] + list(v[3:]),
         lambda s: grading_on(s) and s['lighting.grade_extra'][3] > 0, 'white'),
    Spec('lighting.grade_cool_color', lambda v, s: [1.0, 1.0, 1.0] + list(v[3:]),
         lambda s: grading_on(s) and s['lighting.grade_extra'][3] > 0, 'white'),
    Spec('lighting.enable_gloom', const(0), gloom_on, '0'),
    Spec('lighting.gloom_params', const(0.0), gloom_on, '0 (no shadow bias)', comp=2),
    Spec('lighting.gloom_color', const([0.0, 0.0, 0.0, 0.0]), gloom_on,
         '[0, 0, 0, 0] (tint falls back to atmosphere_tint, no desaturation)'),
    Spec('lighting.light_color', const([1.0, 1.0, 1.0]), ALWAYS, 'white'),
    Spec('lighting.atmosphere_tint', const([1.0, 1.0, 1.0]), ALWAYS, 'white (identity of the art depth tint)'),
    Spec('lighting.ambient_strength', const(0.0), ALWAYS, '0'),
    Spec('land_lighting.enable_bent', const(0), ALWAYS, '0'),
    Spec('land_lighting.diffuse_strength', const(0.0), ALWAYS, '0'),
    Spec('land_lighting.specular_strength', const(0.0), fragment, '0'),
    Spec('land_lighting.rim_strength', const(0.0), rim_on, '0'),
    Spec('land_lighting.fill_strength', const(0.0), fragment, '0'),
    Spec('land_lighting.sharpness_factor', const(1.0), lambda s: fragment(s) and s['land_lighting.sharpness_mix'] > 0,
         '1.0 (unshaped Lambert)'),
    Spec('land_lighting.sharpness_mix', const(0.0), fragment, '0'),
    Spec('land_lighting.diffuse_wrap', const(0.0), lambda s: shading(s) == 2, '0'),
    Spec('land_lighting.fill_sky_color', lambda v, s: grey_same_luma(v),
         lambda s: fragment(s) and s['land_lighting.fill_strength'] > 0, 'grey of the same luminance (alpha kept)'),
    Spec('land_lighting.fill_ground_color', lambda v, s: grey_same_luma(v),
         lambda s: fragment(s) and s['land_lighting.fill_strength'] > 0, 'grey of the same luminance (alpha kept)'),
    Spec('land_lighting.rim_color', lambda v, s: grey_same_luma(v), rim_on,
         'grey of the same luminance (power in alpha kept)'),
    # Probes: every factory preset leaves these off; what would turning them on do?
    Spec('effects.sharpening_amount', const(0.5), ALWAYS, 'probe: 0.5', kind='probe', times=('afternoon',)),
    Spec('effects.enable_normal_maps', const(1), fragment, 'probe: on at the preset strength 0.45',
         kind='probe', times=('afternoon',)),
    Spec('effects.art_mottle_strength', const(1.0), ALWAYS, 'probe: 1.0', kind='probe', times=('afternoon',)),
    Spec('lighting.gloom_params', const(0.0), gloom_on,
         'probe: height fade off (0), so gloom covers every height', comp=1, kind='probe'),
]

# Settings measured through another one, or never read (static analysis).
NOT_MEASURED = {
    'effects.shading_mode': 'selector: picks the land shading model; it defines the render style',
    'effects.post_process_profile': 'selector: picks the post-process profile; it defines the render style',
    'effects.enable_water_animation': 'measured in the water launch (time-driven)',
    'effects.blur_strength': 'amount of enable_blur: 0 is the same as blur off',
    'effects.grunge_strength': 'amount of enable_grunge: 0 is the same as grunge off',
    'effects.kr_transition_grime_strength': 'amount of enable_kr_transition_grime',
    'effects.kr_color_wash_strength': 'amount of enable_kr_color_wash',
    'effects.art_projected_shadow_strength': 'amount of enable_art_projected_shadows',
    'effects.land_normal_map_strength': 'read only with enable_normal_maps, which every preset leaves off',
    'lighting.gloom_params.x': 'amount of enable_gloom: 0 is the same as gloom off',
    'lighting.grade_params.x': 'amount of enable_grading: 0 is the same as grading off',
    'lighting.fog_color.a': 'fog opacity cap: 0 is the same as fog off',
    'lighting.fog_params.z': 'cloud scale: acts only with fog_params.w',
}


def make_plan():
    factory = load_factory()
    configs = {}
    entries = []

    def add(config):
        configs[config['id']] = config

    for mode in MODES:
        for period in TIMES:
            base = deterministic(factory_slot(factory, mode, period))
            add({'id': f'{mode}.{period}.baseline', 'mode': mode, 'time': period, 'slot': base,
                 'baseline': f'{mode}.{period}.baseline', 'setting': None})
    for setting in SPECS:
        for mode in setting.modes:
            for period in setting.times:
                factory_values = factory_slot(factory, mode, period)
                base = deterministic(factory_values)
                entry = {'setting': setting.name, 'mode': mode, 'time': period, 'kind': setting.kind,
                         'how': setting.how}
                if setting.kind == 'reverse':
                    reached = setting.reach(factory_values) and setting.value(factory_values) > 0
                    preset_value = setting.value(factory_values)
                    neutral_value = setting.value(base)
                    variant = setting.with_value(base, preset_value)
                else:
                    reached = setting.reach(base) if setting.kind == 'neutral' else setting.reach(
                        setting.with_value(base, setting.neutral(setting.value(base), base)))
                    preset_value = setting.value(base)
                    neutral_value = setting.neutral(preset_value, base)
                    variant = setting.with_value(base, neutral_value)
                entry.update(preset=preset_value, neutral=neutral_value)
                if not reached:
                    entry['status'] = 'unreachable'
                elif json.dumps(variant, sort_keys=True) == json.dumps(base, sort_keys=True):
                    entry['status'] = 'noop'
                else:
                    entry['status'] = 'measured'
                    entry['config'] = f'{mode}.{period}.{setting.name}'
                    add({'id': entry['config'], 'mode': mode, 'time': period, 'slot': variant,
                         'baseline': f'{mode}.{period}.baseline', 'setting': setting.name})
                entries.append(entry)

    baselines = [f'{m}.{t}.baseline' for m in MODES for t in TIMES]
    launches = [
        {'id': 'base-a', 'configs': baselines},
        # A second pass over the first views after switching through every slot: the same-launch noise floor.
        {'id': 'base-b', 'configs': baselines, 'repeat_views': REPEAT_VIEWS},
        {'id': 'water', 'configs': baselines, 'water': 1},
        {'id': 'nostatics', 'configs': baselines, 'world_rendering': {'enable_statics': 'false'}},
        {'id': 'nolights', 'configs': baselines, 'world_rendering': {'enable_static_lights': 'false'}},
    ]
    variants = {mode: [c['id'] for c in configs.values() if c['setting'] and c['mode'] == mode] for mode in MODES}
    count = max(math.ceil(len(ids) / SLOTS_PER_MODE) for ids in variants.values())
    for index in range(count):
        ids = []
        for mode in MODES:
            ids += variants[mode][index * SLOTS_PER_MODE:(index + 1) * SLOTS_PER_MODE]
        launches.append({'id': f'v{index + 1:02d}', 'configs': ids})
    plan = {'configs': configs, 'entries': entries, 'launches': launches}
    PLAN.write_text(json.dumps(plan, indent=1))
    return plan


def load_plan():
    return json.loads(PLAN.read_text())


SCRATCH_ROOT = Path('/home/claudio/rust-target/uodynamapper-scratch')
PREVIEW_LOCK = SCRATCH_ROOT / 'preview.lock'
LOWPRIO = SCRATCH_ROOT / 'lowprio.sh'
LAUNCH_ATTEMPTS = 2


def run_all(launch_ids):
    """Run launches one at a time, each in its own process under the shared preview lock and at low priority.

    Taking the lock per launch lets other agents' GPU runs interleave. A failed launch is retried once; the log is
    SCRATCH/run_all.log and each launch's output SCRATCH/runs/ID.out."""
    plan = load_plan()
    known = [launch['id'] for launch in plan['launches']]
    ids = launch_ids or known
    unknown = [launch_id for launch_id in ids if launch_id not in known]
    if unknown:
        raise SystemExit(f'unknown launches: {unknown}')
    RUNS.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, DYNAMAPPER_REPO=str(REPO))
    with (SCRATCH / 'run_all.log').open('a') as log:
        for launch_id in ids:
            for attempt in range(1, LAUNCH_ATTEMPTS + 1):
                log.write(f'{time.strftime("%H:%M:%S")} start {launch_id} attempt {attempt}\n')
                log.flush()
                with (RUNS / f'{launch_id}.out').open('w') as out:
                    code = subprocess.call(['flock', str(PREVIEW_LOCK), str(LOWPRIO), sys.executable, str(SCRIPT),
                                            'launch', launch_id], stdout=out, stderr=subprocess.STDOUT, env=env)
                tail = (RUNS / f'{launch_id}.out').read_text().strip().splitlines()[-1:]
                log.write(f'{time.strftime("%H:%M:%S")} {"done" if code == 0 else "FAILED"} {launch_id} {tail}\n')
                log.flush()
                if code == 0:
                    break


# ----------------------------------------------------------------------------------------------------------------
# Launch
# ----------------------------------------------------------------------------------------------------------------

def _sub_toml_value(text, key, value):
    pattern = re.compile(rf'^{re.escape(key)}\s*=.*$', re.M)
    assert pattern.search(text), key
    return pattern.sub(f'{key} = {value}', text, count=1)


def wait_for_terrain(endpoint, timeout, position=None):
    """control.wait_for_scene without the art conditions, for a launch with statics turned off."""
    deadline = time.monotonic() + timeout
    stable = 0
    previous = None
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('terrain did not settle')
        status = control.request(endpoint, {'command': 'status'}, remaining)
        if position is None:
            position = status.get('position')
        terrain = status.get('terrain')
        if (position is not None and status.get('position') == list(position)
                and status.get('rendered_position') == list(position) and terrain
                and not terrain['metadata_pending']
                and terrain['metadata_revision'] == terrain['submitted_metadata_revision']):
            fingerprint = (tuple(position), terrain['metadata_revision'])
            stable = stable + 1 if fingerprint == previous else 1
            previous = fingerprint
            if stable >= SCENE_STABLE_SNAPSHOTS:
                return status
        else:
            stable = 0
            previous = None
        time.sleep(SCENE_POLL_SECONDS)


class Run:
    def __init__(self, launch, configs, factory):
        self.launch = launch
        self.dir = RUNS / launch['id']
        if self.dir.exists():
            shutil.rmtree(self.dir)
        self.dir.mkdir(parents=True)
        assets = self.dir / 'assets'
        shutil.copytree(ASSETS, assets)
        # Without the bundled land.toml, the factory presets are the defaults under the run's cache copy.
        (assets / BUNDLED_PRESETS).unlink()
        settings = assets / 'settings'
        runtime = settings / 'runtime_assets.toml'
        runtime.write_text(_sub_toml_value(runtime.read_text(), 'udd_path', json.dumps(str(PACKAGES) + '/')))
        prefs = settings / 'user_preferences.toml'
        prefs.write_text(_sub_toml_value(prefs.read_text(), 'show_overlay', 'false'))
        graphics = settings / 'graphics.toml'
        text = _sub_toml_value(graphics.read_text(), 'reduce_unfocused_fps', 'false')
        for key, value in launch.get('graphics', {}).items():
            text = _sub_toml_value(text, key, value)
        graphics.write_text(text)
        if launch.get('lift_clamps'):
            for path, clamp, lifted in CLAMP_LIFTS:
                shader = assets / path
                source = shader.read_text()
                assert source.count(clamp) == 1, (path, clamp)
                shader.write_text(source.replace(clamp, lifted))
        world = settings / 'world_rendering.toml'
        text = world.read_text()
        for key, value in launch.get('world_rendering', {}).items():
            text = _sub_toml_value(text, key, value)
        world.write_text(text)

        # Slots: each mode's configs fill its time-of-day slots in order, then those of a group that renders them the
        # same (SLOT_GROUPS); the rest keep the baseline.
        self.slot_of = {}
        slots = {(m, t): deterministic(factory_slot(factory, m, t)) for m in MODES for t in TIMES}
        free = {m: list(TIMES) for m in MODES}
        for config_id in launch['configs']:
            config = configs[config_id]
            group = next((g for g in SLOT_GROUPS[config['mode']] if free[g]), None)
            assert group is not None, f'no free slot for {config_id}'
            period = free[group].pop(0)
            slot = dict(config['slot'])
            slot['effects.enable_water_animation'] = launch.get('water', 0)
            slots[(group, period)] = slot
            self.slot_of[config_id] = (group, period)
        first = configs[launch['configs'][0]]
        view = VIEWS[0]
        cache = self.dir / 'cache/settings'
        (cache / 'shaders').mkdir(parents=True)
        (cache / 'session_state.toml').write_text(
            f'[world.last_p]\nx = {view[1]}\ny = {view[2]}\nz = {view[3]}\nm = 0\n\n'
            f'[window]\nheight = {float(WINDOW[1])}\nwidth = {float(WINDOW[0])}\nzoom = {ZOOM}\n')
        start_mode, start_period = self.slot_of[first['id']]
        (cache / 'shaders/land.toml').write_text(presets_toml(slots, f'{start_mode}.{start_period}'))
        self.slots = slots
        self.socket = self.dir / 'control.sock'
        self.child = None
        self.caps = CAPS / launch['id']
        self.caps.mkdir(parents=True, exist_ok=True)

    def start(self):
        env = os.environ.copy()
        env['DYNAMAPPER_CONTROL_SOCKET'] = str(self.socket)
        tmp = SCRATCH / 'tmp'
        tmp.mkdir(exist_ok=True)
        env['TMPDIR'] = str(tmp)
        self.log = open(self.dir / 'preview.out', 'w')
        self.child = subprocess.Popen(
            [sys.executable, str(PREVIEW), '--desktop', DESKTOP, '--cwd', str(self.dir), '--display',
             os.environ['DISPLAY'], '--seconds', str(PREVIEW_SECONDS), '--placement-timeout',
             str(PLACEMENT_TIMEOUT_SECONDS), '--', str(BINARY)],
            env=env, stdout=self.log, stderr=subprocess.STDOUT)
        self.started = time.monotonic()
        deadline = self.started + SOCKET_TIMEOUT_SECONDS
        while not self.socket.exists():
            if self.child.poll() is not None:
                raise RuntimeError(f'preview exited early: {(self.dir / "preview.out").read_text()}')
            if time.monotonic() > deadline:
                raise TimeoutError('socket never appeared')
            time.sleep(SOCKET_POLL_SECONDS)
        return self

    def stop(self):
        if self.child and self.child.poll() is None:
            self.child.send_signal(signal.SIGTERM)
            try:
                self.child.wait(HELPER_EXIT_SECONDS)
            except subprocess.TimeoutExpired:
                print('preview helper did not exit', file=sys.stderr)
        self.log.close()
        return (self.dir / 'preview.out').read_text()

    def __enter__(self):
        return self.start()

    def __exit__(self, *_):
        out = self.stop()
        print(out[-LOG_TAIL_CHARS:])
        assert 'placement_verified' in out, out

    def req(self, command, timeout=REQUEST_TIMEOUT_SECONDS):
        return control.request(self.socket, command, timeout)

    def status(self):
        return self.req({'command': 'status'})

    def wait_in_game(self, position=None):
        # Without statics there is no art to wait for: control.wait_for_scene would never see visible art chunks.
        statics_off = self.launch.get('world_rendering', {}).get('enable_statics') == 'false'
        wait = wait_for_terrain if statics_off else control.wait_for_scene
        deadline = time.monotonic() + SCENE_TIMEOUT_SECONDS
        while True:
            try:
                return wait(self.socket, SCENE_WAIT_SECONDS, position)
            except (OSError, RuntimeError, TimeoutError) as error:
                if time.monotonic() > deadline:
                    raise TimeoutError(f'scene never ready: {error}')
                time.sleep(SCENE_RETRY_SECONDS)

    def teleport(self, view):
        _, x, y, z, _ = view
        self.req({'command': 'teleport', 'x': x, 'y': y, 'z': z, 'map': 0})
        status = self.wait_in_game(position=[x, y, z, 0])
        time.sleep(TELEPORT_SETTLE_SECONDS)
        return status

    def input(self, command):
        self.req(command)
        control.wait_for_input(self.socket, INPUT_TIMEOUT_SECONDS)

    def key(self, name):
        self.input({'command': 'key', 'key': name, 'state': 'tap'})

    def click(self, label):
        x, y = PANEL_BUTTONS[label]
        self.input({'command': 'click', 'x': float(x), 'y': float(y), 'button': 'left', 'state': 'tap'})
        time.sleep(CLICK_SETTLE_SECONDS)

    def select(self, config_id):
        """Open F3, pick the config's slot with Render Style then Time of Day, close F3, release the pointer."""
        mode, period = self.slot_of[config_id]
        self.key('F3')
        time.sleep(PANEL_OPEN_SETTLE_SECONDS)
        self.click(MODE_BUTTON[mode])
        self.click(TIME_BUTTON[period])
        self.key('F3')
        self.input({'command': 'reset_input'})
        time.sleep(PANEL_CLOSE_SETTLE_SECONDS)
        status = self.wait_in_game()
        self.verify(config_id, status)

    def verify(self, config_id, status):
        mode, period = self.slot_of[config_id]
        render = status['render']
        expected = self.slots[(mode, period)]
        problems = []
        if render['preset'] != f'{mode}.{period}':
            problems.append(f'preset {render["preset"]}')
        lit_by_scene = 'global_lighting' in expected
        if lit_by_scene and abs(render['global_lighting'] - expected['global_lighting']) > VERIFY_TOLERANCE:
            problems.append(f'global_lighting {render["global_lighting"]}')
        for key, value in render['effects'].items():
            want = expected.get(f'effects.{key}')
            if want is not None and abs(float(value) - float(want)) > VERIFY_TOLERANCE:
                problems.append(f'{key} {value} != {want}')
        if problems:
            raise RuntimeError(f'{config_id}: active state differs: {problems}')

    def capture(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            path.unlink()
        self.req({'command': 'capture', 'path': str(path)})
        deadline = time.monotonic() + CAPTURE_TIMEOUT_SECONDS
        while not control.png_complete(path):
            if time.monotonic() > deadline:
                raise TimeoutError(f'capture did not finish: {path}')
            time.sleep(CAPTURE_POLL_SECONDS)
        return path


def run_launch(launch_id):
    plan = load_plan()
    launch = next(item for item in plan['launches'] if item['id'] == launch_id)
    configs = plan['configs']
    run = Run(launch, configs, load_factory())
    timings = {}
    views = [view for view in VIEWS if view[0] in launch.get('views', [v[0] for v in VIEWS])]
    with run:
        first_view = VIEWS[0]
        run.wait_in_game(position=[first_view[1], first_view[2], first_view[3], 0])
        timings['ready'] = time.monotonic() - run.started
        # The first capture of a run may be black or stale.
        run.capture(run.caps / '0_discard.png')
        for view in views:
            run.teleport(view)
            for config_id in launch['configs']:
                run.select(config_id)
                run.capture(run.caps / config_id / f'{view[0]}.png')
            if view[0] in launch.get('repeat_views', ()):
                for config_id in launch['configs']:
                    run.select(config_id)
                    run.capture(run.caps / config_id / f'{view[0]}.repeat.png')
            timings[view[0]] = time.monotonic() - run.started
            if timings[view[0]] > MAX_LAUNCH_SECONDS:
                raise TimeoutError(f'launch too long for the preview limit: {timings}')
    (run.caps / 'timings.json').write_text(json.dumps(timings, indent=1))
    print(json.dumps(timings))


CALIBRATION_CANDIDATES = (
    ('britain', 1434, 1699), ('castle', 1323, 1624), ('castle_hall', 1323, 1590), ('bank', 1427, 1683),
    ('trinsic', 1880, 2790), ('moonglow', 4442, 1122), ('vesper', 2900, 690), ('minoc', 2500, 450),
    ('coast_britain', 1500, 1750), ('docks_britain', 1490, 1760), ('skara_shore', 640, 2180),
    ('vesper_canal', 2895, 660), ('yew', 620, 1020), ('covetous', 2460, 880), ('slopes', 5650, 2950),
)


def calibrate():
    factory = load_factory()
    configs = {}
    for mode, period in (('kr', 'night'), ('classic', 'night')):
        config_id = f'{mode}.{period}.baseline'
        configs[config_id] = {'id': config_id, 'mode': mode, 'time': period,
                              'slot': deterministic(factory_slot(factory, mode, period))}
    launch = {'id': 'calibrate', 'configs': list(configs)}
    run = Run(launch, configs, load_factory())
    report = {}
    with run:
        view = VIEWS[0]
        run.wait_in_game(position=[view[1], view[2], view[3], 0])
        report['ready_seconds'] = time.monotonic() - run.started
        run.capture(run.caps / '0_discard.png')
        run.capture(run.caps / 'start.png')
        run.key('F3')
        time.sleep(1.0)
        run.capture(run.caps / 'panel_open.png')
        run.key('F3')
        run.input({'command': 'reset_input'})
        time.sleep(1.0)
        run.capture(run.caps / 'panel_closed.png')
        status = run.status()
        report['window'] = status.get('window')
        for name, x, y in CALIBRATION_CANDIDATES:
            t0 = time.monotonic()
            run.req({'command': 'teleport', 'x': x, 'y': y, 'z': 0, 'map': 0})
            status = run.wait_in_game(position=[x, y, 0, 0])
            time.sleep(TELEPORT_SETTLE_SECONDS)
            status = run.status()
            run.capture(run.caps / f'cand_{name}.png')
            report[name] = {'static_lights': status.get('static_lights'), 'seconds': time.monotonic() - t0}
    print(json.dumps(report, indent=1))
    (run.caps / 'report.json').write_text(json.dumps(report, indent=1))


PROBE_CANDIDATES = (
    ('moonglow_lamps', 4473, 1168), ('nujelm_south', 3766, 1254), ('nujelm_north', 3765, 1190),
    ('serpents_hold', 3032, 3352), ('britain_south', 1416, 2106), ('fire_isle', 4621, 3561),
    ('britain_castle', 1414, 1605), ('trinsic_east', 1960, 2064),
)
# Night and cave slots with and without their static-light terms (land light pool and light decals).
PROBE_SLOTS = (('enhanced', 'night'), ('kr', 'cave'))


def probe(candidates):
    """Find the view where static lights change the most pixels at night."""
    import numpy as np
    factory = load_factory()
    configs = {}
    ids = []
    for mode, period in PROBE_SLOTS:
        base = deterministic(factory_slot(factory, mode, period))
        dark = dict(base)
        dark['effects.light_decal_intensity'] = 0.0
        dark['effects.static_light_decal_visibility'] = 0.0
        for suffix, slot in (('lit', base), ('unlit', dark)):
            config_id = f'{mode}.{period}.{suffix}'
            configs[config_id] = {'id': config_id, 'mode': mode, 'time': period, 'slot': slot}
            ids.append(config_id)
    launch = {'id': 'probe', 'configs': ids}
    run = Run(launch, configs, factory)
    report = {}
    with run:
        view = VIEWS[0]
        run.wait_in_game(position=[view[1], view[2], view[3], 0])
        run.capture(run.caps / '0_discard.png')
        for name, x, y in candidates:
            run.teleport((name, x, y, 0, ''))
            for config_id in ids:
                run.select(config_id)
                run.capture(run.caps / config_id / f'{name}.png')
            report[name] = {'lights': run.status().get('static_lights')}
        report['total'] = time.monotonic() - run.started
    for name, _, _ in candidates:
        for mode, period in PROBE_SLOTS:
            d = change(load_rgb(run.caps / f'{mode}.{period}.lit' / f'{name}.png'),
                       load_rgb(run.caps / f'{mode}.{period}.unlit' / f'{name}.png'))
            report[name][f'{mode}.{period}'] = stats(d)
    print(json.dumps(report, indent=1))


# ----------------------------------------------------------------------------------------------------------------
# Metrics
# ----------------------------------------------------------------------------------------------------------------

def load_rgb(path):
    import numpy as np
    from PIL import Image
    with Image.open(path) as image:
        return np.asarray(image.convert('RGB'), dtype=np.int16)


def change(a, b):
    import numpy as np
    return np.abs(a - b).max(axis=2)


def stats(d, mask=None):
    import numpy as np
    values = d if mask is None else d[mask]
    if values.size == 0:
        return {'mean': 0.0, 'p99': 0.0, 'share': 0.0, 'any': 0.0, 'max': 0}
    return {'mean': float(values.mean()), 'p99': float(np.percentile(values, PERCENTILE)),
            'share': float((values > CHANGE_LEVEL).mean()), 'any': float((values > 0).mean()),
            'max': int(values.max())}


def cap(launch, config_id, view, suffix=''):
    return CAPS / launch / config_id / f'{view}{suffix}.png'


def region_masks(base_id, view, base):
    masks = {}
    for region, launch, config_suffix in (('water', 'water', ''), ('lights', 'nolights', ''),
                                           ('statics', 'nostatics', '')):
        path = cap(launch, base_id, view)
        if path.exists():
            masks[region] = change(base, load_rgb(path)) > CHANGE_LEVEL
    return masks


def attribute(d, masks):
    import numpy as np
    changed = d > CHANGE_LEVEL
    total = int(changed.sum())
    if total == 0:
        return {}
    claimed = np.zeros_like(changed)
    shares = {}
    for region in REGION_ORDER:
        mask = masks.get(region) if region != 'land' else np.ones_like(changed)
        if mask is None:
            continue
        own = changed & mask & ~claimed
        shares[region] = float(own.sum()) / total
        claimed |= own
    shares['frame'] = float(changed.mean())
    shares['count'] = total
    return shares


def metrics():
    plan = load_plan()
    results = {'noise': {}, 'water': {}, 'variants': {}}
    views = [v[0] for v in VIEWS]
    for mode in MODES:
        for period in TIMES:
            base_id = f'{mode}.{period}.baseline'
            for view in views:
                base = load_rgb(cap('base-a', base_id, view))
                other = cap('base-b', base_id, view)
                repeat = cap('base-b', base_id, view, '.repeat')
                entry = {}
                if other.exists():
                    entry['cross_launch'] = stats(change(base, load_rgb(other)))
                if repeat.exists() and other.exists():
                    entry['same_launch'] = stats(change(load_rgb(other), load_rgb(repeat)))
                results['noise'][f'{base_id}/{view}'] = entry
                masks = region_masks(base_id, view, base)
                results.setdefault('mask_share', {})[f'{base_id}/{view}'] = {
                    region: float(mask.mean()) for region, mask in masks.items()}
                water = cap('water', base_id, view)
                if water.exists():
                    d = change(base, load_rgb(water))
                    results['water'][f'{mode}.{period}/{view}'] = dict(stats(d), where=attribute(d, masks))
    launch_of = {}
    for launch in plan['launches']:
        if launch['id'].startswith('v'):
            for config_id in launch['configs']:
                launch_of[config_id] = launch['id']
    mask_cache = {}
    for config_id, launch in launch_of.items():
        config = plan['configs'][config_id]
        per_view = {}
        for view in views:
            path = cap(launch, config_id, view)
            if not path.exists():
                continue
            base = load_rgb(cap('base-a', config['baseline'], view))
            key = (config['baseline'], view)
            if key not in mask_cache:
                mask_cache[key] = region_masks(config['baseline'], view, base)
            d = change(base, load_rgb(path))
            per_view[view] = dict(stats(d), where=attribute(d, mask_cache[key]))
        results['variants'][config_id] = per_view
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / 'metrics.json').write_text(json.dumps(results, indent=1))
    print(f'{len(results["variants"])} variants measured')


# ----------------------------------------------------------------------------------------------------------------
# Sheets and report
# ----------------------------------------------------------------------------------------------------------------

THUMB = (256, 192)
HEAT_GAIN = 8
LABEL_HEIGHT = 16


def heat(d):
    import numpy as np
    from PIL import Image
    v = np.clip(d.astype(np.float32) * HEAT_GAIN, 0, 255).astype(np.uint8)
    return Image.fromarray(np.stack([v, v // 2, 255 - v], axis=2)).resize(THUMB)


def sheets():
    from PIL import Image, ImageDraw
    plan = load_plan()
    results = json.loads((RESULTS / 'metrics.json').read_text())
    launch_of = {c: l['id'] for l in plan['launches'] if l['id'].startswith('v') for c in l['configs']}
    by_setting = {}
    for config_id in results['variants']:
        by_setting.setdefault(plan['configs'][config_id]['setting'], []).append(config_id)
    SHEETS.mkdir(parents=True, exist_ok=True)
    views = [v[0] for v in VIEWS]
    for setting, config_ids in by_setting.items():
        # Rows: one per (mode, time). Columns: per view, baseline | variant | change x8.
        width = len(views) * 3 * THUMB[0]
        height = len(config_ids) * (THUMB[1] + LABEL_HEIGHT)
        sheet = Image.new('RGB', (width, height), 'black')
        draw = ImageDraw.Draw(sheet)
        for row, config_id in enumerate(config_ids):
            config = plan['configs'][config_id]
            y = row * (THUMB[1] + LABEL_HEIGHT)
            draw.text((4, y + 2), f'{config["mode"]}.{config["time"]}', fill='white')
            for col, view in enumerate(views):
                path = cap(launch_of[config_id], config_id, view)
                if not path.exists():
                    continue
                base_path = cap('base-a', config['baseline'], view)
                x = col * 3 * THUMB[0]
                with Image.open(base_path) as base, Image.open(path) as variant:
                    sheet.paste(base.convert('RGB').resize(THUMB), (x, y + LABEL_HEIGHT))
                    sheet.paste(variant.convert('RGB').resize(THUMB), (x + THUMB[0], y + LABEL_HEIGHT))
                sheet.paste(heat(change(load_rgb(base_path), load_rgb(path))), (x + 2 * THUMB[0], y + LABEL_HEIGHT))
                m = results['variants'][config_id].get(view, {})
                draw.text((x + 4, y + 2 + LABEL_HEIGHT), f'{view} mean {m.get("mean", 0):.2f}', fill='yellow')
        sheet.save(SHEETS / f'{setting}.png')
    print(f'{len(by_setting)} sheets in {SHEETS}')


def fmt(value):
    return f'{value:.2f}'


def pct(value):
    return f'{value * 100:.2f}%'


def worst_view(per_view):
    return max(per_view.items(), key=lambda item: (item[1]['mean'], item[1]['share']))


def regions_of(counts, whole_frame):
    if whole_frame:
        return ['whole frame']
    total = sum(counts.values())
    if total == 0:
        return []
    return [r for r in REGION_ORDER if counts.get(r, 0) / total >= REGION_REPORT_SHARE]


def summarize(plan, results):
    """Per setting: every condition's status and worst view, the regions it changes, and a suggested verdict."""
    summary = {}
    for entry in plan['entries']:
        item = summary.setdefault(entry['setting'], {'how': entry['how'], 'kind': entry['kind'], 'conditions': [],
                                                      'regions': {}, 'whole_frame': False})
        cond = {'condition': f'{entry["mode"]}.{entry["time"]}', 'preset': entry['preset'],
                'neutral': entry['neutral'], 'status': entry['status']}
        per_view = results['variants'].get(entry.get('config'), {}) if entry['status'] == 'measured' else {}
        if entry['status'] == 'measured' and not per_view:
            cond['status'] = 'not captured'
        if per_view:
            view, m = worst_view(per_view)
            cond.update(view=view, mean=m['mean'], p99=m['p99'], share=m['share'], any=m.get('any', 0.0),
                        max=m['max'], views={v: {k: x[k] for k in ('mean', 'p99', 'share')} for v, x in per_view.items()})
            for x in per_view.values():
                where = x.get('where', {})
                for region in REGION_ORDER:
                    item['regions'][region] = item['regions'].get(region, 0) + where.get(region, 0) * where.get('count', 0)
                if where.get('frame', 0) >= WHOLE_FRAME_SHARE:
                    item['whole_frame'] = True
        item['conditions'].append(cond)
    for item in summary.values():
        measured = [c for c in item['conditions'] if 'mean' in c]
        item['where'] = regions_of(item['regions'], item['whole_frame'])
        if not measured:
            item['suggested'] = 'dead (never reached)' if all(
                c['status'] == 'unreachable' for c in item['conditions']) else 'no-op at every preset value'
            continue
        top = max(measured, key=lambda c: (c['mean'], c['share']))
        item['top'] = {k: top[k] for k in ('condition', 'view', 'mean', 'p99', 'share')}
        low = all(c['mean'] < LOW_MEAN and c['share'] < LOW_SHARE for c in measured)
        visible = [c['condition'] for c in measured if not (c['mean'] < LOW_MEAN and c['share'] < LOW_SHARE)]
        item['visible_in'] = visible
        if low:
            item['suggested'] = 'low impact'
        elif len(visible) < len(item['conditions']):
            item['suggested'] = 'conditional'
        else:
            item['suggested'] = 'useful'
    return summary


def report():
    plan = load_plan()
    results = json.loads((RESULTS / 'metrics.json').read_text())
    summary = summarize(plan, results)
    (RESULTS / 'summary.json').write_text(json.dumps(summary, indent=1))
    views = [v[0] for v in VIEWS]
    lines = ['# Shader settings audit: measurements', '']
    worst_noise = max((m for entry in results['noise'].values() for m in entry.values()),
                      key=lambda m: m['mean'], default=None)
    if worst_noise:
        lines += [f'Noise floor (same values in two launches, and twice in one launch), worst view: mean '
                  f'{worst_noise["mean"]:.4f}, p99 {fmt(worst_noise["p99"])}, > {CHANGE_LEVEL} levels '
                  f'{pct(worst_noise["share"])}, max {worst_noise["max"]}.', '']
    lines += ['## Water animation (frozen against animated, same frame time unknown)', '',
              '| condition | ' + ' | '.join(views) + ' |', '|---|' + '---|' * len(views)]
    for mode in MODES:
        for period in TIMES:
            row = []
            for view in views:
                m = results['water'].get(f'{mode}.{period}/{view}')
                row.append(f'{fmt(m["mean"])} / {pct(m["share"])}' if m else '')
            lines.append(f'| {mode}.{period} | ' + ' | '.join(row) + ' |')
    lines += ['', '## Region masks (share of the frame)', '',
              '| baseline/view | water | lights | statics |', '|---|---|---|---|']
    for key, masks in results.get('mask_share', {}).items():
        lines.append(f'| {key} | ' + ' | '.join(pct(masks.get(r, 0)) for r in ('water', 'lights', 'statics')) + ' |')
    lines += ['', '## Summary', '',
              '| setting | suggested | worst condition (view) | mean | p99 | > 4 levels | where | visible in |',
              '|---|---|---|---|---|---|---|---|']
    for setting, item in summary.items():
        top = item.get('top')
        if top:
            lines.append(f'| {setting} | {item["suggested"]} | {top["condition"]} ({top["view"]}) | {fmt(top["mean"])} | '
                         f'{fmt(top["p99"])} | {pct(top["share"])} | {", ".join(item["where"])} | '
                         f'{", ".join(item["visible_in"])} |')
        else:
            lines.append(f'| {setting} | {item["suggested"]} | | | | | | |')
    lines += ['', '## Per setting', '']
    for setting, item in summary.items():
        lines += [f'### {setting}', '', f'Neutral: {item["how"]}', '',
                  '| condition | preset | status | worst view | mean | p99 | > 4 levels | any change |',
                  '|---|---|---|---|---|---|---|---|']
        for c in item['conditions']:
            if 'mean' in c:
                lines.append(f'| {c["condition"]} | {c["preset"]} | measured | {c["view"]} | {fmt(c["mean"])} | '
                             f'{fmt(c["p99"])} | {pct(c["share"])} | {pct(c["any"])} |')
            else:
                lines.append(f'| {c["condition"]} | {c["preset"]} | {c["status"]} | | | | | |')
        lines.append('')
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / 'report.md').write_text('\n'.join(lines))
    print(RESULTS / 'report.md')


# ----------------------------------------------------------------------------------------------------------------
# Strong values: kept low-impact settings pushed past their presets
# ----------------------------------------------------------------------------------------------------------------
#
# The note's "Strong values" section. Each setting is rendered off, at the preset, stronger, at its control's maximum
# and beyond it, in UOD-Painterly and in every other style where it can act once its gate is on. Point
# SHADER_AUDIT_SCRATCH at a folder of its own: strong-plan writes that folder's plan.json, `run` executes it as
# usual, then strong-metrics, strong-report and strong-sheets read it.
#
# Beyond values exceed the F3 sliders, which clamp the values they draw (egui's default SliderClamping::Always).
# The rig binary is therefore built with the slider helpers of land_shader.rs set to SliderClamping::Edits, a
# rig-only change that draws the same frames. Every capture is verified against its slot, so a clamped value fails
# the launch.

# The shader clamps that stop two settings at their slider maximum, lifted in each launch's copy of the shaders
# (lift_clamps) so a beyond value reaches the screen. Inside the slider range the result is the same. The land shadow
# mottle reaches its strong look inside its slider (land/shading.wesl), so it has no beyond value any more.
BEYOND_CLAMP = 16.0
CLAMP_LIFTS = (
    ('shaders/world/art/shading.wesl', 'clamp(art_mottle_strength, 0.0, 1.5)',
     f'clamp(art_mottle_strength, 0.0, {BEYOND_CLAMP})'),
    ('shaders/postprocess/kr_color_wash.wesl', 'clamp(strength, 0.0, 1.5)', f'clamp(strength, 0.0, {BEYOND_CLAMP})'),
)
# The settings whose columns go past those clamps: a plan with any of them lifts the clamps in every launch.
LIFTED_SETTINGS = ('art_mottle_strength', 'kr_color_wash')

STRONG_LAND_VIEWS = ('slopes', 'covetous')
STRONG_ART_VIEWS = ('yew', 'britain')
STRONG_WASH_VIEWS = ('slopes', 'britain', 'yew')
STRONG_ALL_VIEWS = ('slopes', 'covetous', 'yew', 'britain')
# For a style that only lends a setting its gate, one day and one night row: the term itself does not change with
# the time of day, only the light it sits in.
DAY_NIGHT = ('afternoon', 'night')
CC_PERIOD = 'afternoon'
TEXTURE_SETS = {'ec': {}, 'cc': {'art_texture_source': '"cc"', 'land_texture_source': '"cc"'}}

# The strength sliders of art mottle, shadow mottle and colour wash end at 1.5, where their shader clamps them.
STRENGTH_SLIDER_MAX = 1.5
STRONG_GAIN = 2.0
# The audit's probe value, about the 0.9..1.15 the shipped land.toml once set in KR.
ART_MOTTLE_STRONG = 1.0
ART_MOTTLE_BEYOND = 3.0
WASH_BEYOND = 3.0
# The scale slider runs 8..160 world units; the shader floors it at 8.
WASH_SCALE_MIN = 8.0
WASH_SCALE_MAX = 160.0
WASH_SCALE_BEYOND = 480.0
# When this was measured, a colour button gave at most 1 per channel: the preset hue brought up to 1, then this many
# times past it.
FILL_BEYOND_GAIN = 3.0
# The Advanced window edits Rim Color as numbers up to 2 (HDR_COLOR_RANGE) and Rim Power with a 0.1..8 slider
# (RIM_POWER_RANGE in dialogs/land_shader.rs); the shader floors the power at 0.1, the widest rim.
RIM_COLOR_MAX = 2.0
RIM_POWER_MIN = 0.1
RIM_POWER_ONE = 1.0
RIM_POWER_MAX = 8.0
# Specular: the Highlights slider's maximum and the Advanced Specular slider's.
SPECULAR_HIGHLIGHTS_MAX = 0.25
SPECULAR_SLIDER_MAX = 0.4
# Enhanced reads a quarter of rim_strength; give it the Highlights slider's maximum.
ENHANCED_RIM_GATE = 0.45

ART_MOTTLE = 'effects.art_mottle_strength'
SHADOW_MOTTLE = 'effects.kr_land_shadow_mottle_strength'
FILL_GROUND = 'land_lighting.fill_ground_color'
FILL_STRENGTH = 'land_lighting.fill_strength'
RIM_COLOR = 'land_lighting.rim_color'
RIM_STRENGTH = 'land_lighting.rim_strength'
SPECULAR = 'land_lighting.specular_strength'
WASH_ON = 'effects.enable_kr_color_wash'
WASH_STRENGTH = 'effects.kr_color_wash_strength'
WASH_SCALE = 'effects.kr_color_wash_scale'
TERRAIN_MODEL = 'land_lighting.terrain_shading_model'
TERRAIN_SMOOTH_PER_PIXEL = 0


def _with(slot, key, value):
    out = dict(slot)
    out[key] = value
    return out


def _gain(rgb, gain):
    return [round(c * gain, 6) for c in rgb]


def _to_one(rgb):
    return _gain(rgb, 1.0 / max(rgb))


def _art_mottle(s):
    return [('off = preset 0', _with(s, ART_MOTTLE, 0.0)),
            (f'{ART_MOTTLE_STRONG} (old shipped KR)', _with(s, ART_MOTTLE, ART_MOTTLE_STRONG)),
            (f'{STRENGTH_SLIDER_MAX} slider max', _with(s, ART_MOTTLE, STRENGTH_SLIDER_MAX)),
            (f'{ART_MOTTLE_BEYOND} beyond', _with(s, ART_MOTTLE, ART_MOTTLE_BEYOND))], 0


def _shadow_mottle(s, preset):
    """Off, the preset, halfway from the preset to the slider's end, and the end."""
    label = f'preset {preset}' if s[SHADOW_MOTTLE] == preset else f'UOD-Painterly value {preset}'
    mid = round((preset + STRENGTH_SLIDER_MAX) / 2, 4)
    return [('off', _with(s, SHADOW_MOTTLE, 0.0)), (label, _with(s, SHADOW_MOTTLE, preset)),
            (f'mid {mid}', _with(s, SHADOW_MOTTLE, mid)),
            (f'{STRENGTH_SLIDER_MAX} slider max', _with(s, SHADOW_MOTTLE, STRENGTH_SLIDER_MAX))], 1


def _fill_ground(s):
    rgb, alpha = s[FILL_GROUND][:3], s[FILL_GROUND][3]
    strong = round(alpha * STRONG_GAIN, 4)
    return [('off (alpha 0)', _with(s, FILL_GROUND, rgb + [0.0])),
            (f'preset (alpha {alpha})', s),
            (f'2x (alpha {strong})', _with(s, FILL_GROUND, rgb + [strong])),
            ('max: hue at 1, alpha 1', _with(s, FILL_GROUND, _to_one(rgb) + [1.0])),
            (f'beyond: {FILL_BEYOND_GAIN:g}x max',
             _with(s, FILL_GROUND, _gain(_to_one(rgb), FILL_BEYOND_GAIN) + [1.0]))], 1


def _rim_color(s):
    """The rim colour at the preset power: black, the preset, twice the preset, and the preset hue at the Advanced
    window's maximum of 2."""
    rgb, power = s[RIM_COLOR][:3], s[RIM_COLOR][3]
    return [('off (black)', _with(s, RIM_COLOR, [0.0, 0.0, 0.0, power])),
            (f'preset (power {power})', s),
            ('2x colour', _with(s, RIM_COLOR, _gain(rgb, STRONG_GAIN) + [power])),
            (f'max: hue at {RIM_COLOR_MAX:g}', _with(s, RIM_COLOR, _gain(_to_one(rgb), RIM_COLOR_MAX) + [power]))], 1


def _rim_power(s):
    """The rim power at the preset rim strength and colour, against the rim off."""
    rgb, power = s[RIM_COLOR][:3], s[RIM_COLOR][3]
    return [('rim off (strength 0)', _with(s, RIM_STRENGTH, 0.0)), (f'preset power {power}', s),
            (f'power {RIM_POWER_MIN:g} (slider min)', _with(s, RIM_COLOR, rgb + [RIM_POWER_MIN])),
            (f'power {RIM_POWER_ONE:g}', _with(s, RIM_COLOR, rgb + [RIM_POWER_ONE])),
            (f'power {RIM_POWER_MAX:g} (slider max)', _with(s, RIM_COLOR, rgb + [RIM_POWER_MAX]))], 1


def _specular(s):
    strength = s[SPECULAR]
    return [('off', _with(s, SPECULAR, 0.0)), (f'preset {strength}', s),
            (f'{SPECULAR_HIGHLIGHTS_MAX:g} Highlights max', _with(s, SPECULAR, SPECULAR_HIGHLIGHTS_MAX)),
            (f'{SPECULAR_SLIDER_MAX:g} slider max', _with(s, SPECULAR, SPECULAR_SLIDER_MAX))], 1


def _enhanced_rim_power(s):
    """Enhanced reads only the rim power, and only with per-pixel land shading."""
    smooth = _with(s, TERRAIN_MODEL, TERRAIN_SMOOTH_PER_PIXEL)
    on = _with(smooth, RIM_STRENGTH, ENHANCED_RIM_GATE)
    rgb, power = s[RIM_COLOR][:3], s[RIM_COLOR][3]
    return [('per-pixel, rim off', smooth), (f'rim {ENHANCED_RIM_GATE}, preset power {power}', on),
            (f'power {RIM_POWER_MIN:g} (slider min)', _with(on, RIM_COLOR, rgb + [RIM_POWER_MIN])),
            (f'power {RIM_POWER_MAX:g} (slider max)', _with(on, RIM_COLOR, rgb + [RIM_POWER_MAX]))], 1


def _wash(s, strength):
    on = _with(s, WASH_ON, 1)
    if s[WASH_ON] == 1:
        strong = round(strength * STRONG_GAIN, 4)
        columns = [('off', _with(s, WASH_ON, 0)), (f'preset {strength}', s),
                   (f'2x {strong}', _with(s, WASH_STRENGTH, strong))]
        preset = 1
    else:
        columns = [('off = preset', s), (f'on at {strength}', _with(on, WASH_STRENGTH, strength))]
        preset = 0
    return columns + [(f'{STRENGTH_SLIDER_MAX} slider max', _with(on, WASH_STRENGTH, STRENGTH_SLIDER_MAX)),
                      (f'{WASH_BEYOND} beyond', _with(on, WASH_STRENGTH, WASH_BEYOND))], preset


def _wash_scale(s, strength):
    wash = _with(_with(s, WASH_ON, 1), WASH_STRENGTH, strength)
    scale = s[WASH_SCALE]
    return [('wash off', _with(s, WASH_ON, 0)),
            (f'scale {WASH_SCALE_MIN:g} (min)', _with(wash, WASH_SCALE, WASH_SCALE_MIN)),
            (f'preset scale {scale:g}', wash),
            (f'{WASH_SCALE_MAX:g} (max)', _with(wash, WASH_SCALE, WASH_SCALE_MAX)),
            (f'{WASH_SCALE_BEYOND:g} beyond', _with(wash, WASH_SCALE, WASH_SCALE_BEYOND))], 2


def strong_conditions(factory, settings=None):
    """Every measured condition: the setting, style, time of day and texture set, the views, the columns as
    (label, slot) and which column is the preset. Column 0 is the setting off. `settings` keeps only those."""
    out = []

    def base(group, period):
        return deterministic(factory_slot(factory, group, period))

    def add(setting, group, period, condition, views, built, textures='ec'):
        if settings and setting not in settings:
            return
        columns, preset = built
        out.append({'setting': setting, 'group': group, 'period': period, 'textures': textures,
                    'condition': condition, 'views': list(views), 'labels': [c[0] for c in columns],
                    'slots': [c[1] for c in columns], 'preset': preset})

    def for_textures(textures, periods):
        for period in periods:
            p = base('painterly', period)
            add('art_mottle_strength', 'painterly', period, 'UOD-Painterly', STRONG_ART_VIEWS, _art_mottle(p), textures)
            add('art_mottle_strength', 'classic', period, 'Classic', STRONG_ART_VIEWS,
                _art_mottle(base('classic', period)), textures)
            add('kr_land_shadow_mottle_strength', 'painterly', period, 'UOD-Painterly', STRONG_LAND_VIEWS,
                _shadow_mottle(p, p[SHADOW_MOTTLE]), textures)
            add('fill_ground_color', 'painterly', period, 'UOD-Painterly', STRONG_LAND_VIEWS, _fill_ground(p), textures)
            add('rim_color', 'painterly', period, 'UOD-Painterly', STRONG_LAND_VIEWS, _rim_color(p), textures)
            add('rim_power', 'painterly', period, 'UOD-Painterly', STRONG_LAND_VIEWS, _rim_power(p), textures)
            add('specular_strength', 'painterly', period, 'UOD-Painterly', STRONG_LAND_VIEWS, _specular(p), textures)
            add('kr_color_wash', 'painterly', period, 'UOD-Painterly', STRONG_WASH_VIEWS,
                _wash(p, p[WASH_STRENGTH]), textures)
            add('kr_color_wash_scale', 'painterly', period, 'UOD-Painterly, wash at 1.5', STRONG_WASH_VIEWS,
                _wash_scale(p, STRENGTH_SLIDER_MAX), textures)

    for_textures('ec', TIMES)
    for period in TIMES:
        p = base('painterly', period)
        add('art_mottle_strength', 'kr', period, 'KR-like', STRONG_ART_VIEWS, _art_mottle(base('kr', period)))
        add('kr_color_wash', 'kr', period, 'KR-like', STRONG_WASH_VIEWS, _wash(base('kr', period), p[WASH_STRENGTH]))
        add('kr_color_wash_scale', 'painterly', period, 'UOD-Painterly, preset wash', STRONG_WASH_VIEWS,
            _wash_scale(p, p[WASH_STRENGTH]))
    def kr_per_pixel_mottle(textures, periods):
        for period in periods:
            p = base('painterly', period)
            smooth = _with(base('kr', period), TERRAIN_MODEL, TERRAIN_SMOOTH_PER_PIXEL)
            add('kr_land_shadow_mottle_strength', 'kr', period, 'KR-like, Land Shading smooth per-pixel',
                STRONG_LAND_VIEWS, _shadow_mottle(smooth, p[SHADOW_MOTTLE]), textures)

    kr_per_pixel_mottle('ec', DAY_NIGHT)
    for period in DAY_NIGHT:
        p = base('painterly', period)
        enhanced = base('enhanced', period)
        filled = _with(_with(enhanced, TERRAIN_MODEL, TERRAIN_SMOOTH_PER_PIXEL), FILL_STRENGTH, p[FILL_STRENGTH])
        add('fill_ground_color', 'enhanced', period,
            'Enhanced, Land Shading smooth per-pixel, Soft Lighting at the UOD-Painterly fill',
            STRONG_LAND_VIEWS, _fill_ground(filled))
        add('rim_power', 'enhanced', period, 'Enhanced, Land Shading smooth per-pixel', STRONG_LAND_VIEWS,
            _enhanced_rim_power(enhanced))
    for_textures('cc', (CC_PERIOD,))
    kr_per_pixel_mottle('cc', (CC_PERIOD,))
    add('rim_power', 'enhanced', CC_PERIOD, 'Enhanced, Land Shading smooth per-pixel', STRONG_LAND_VIEWS,
        _enhanced_rim_power(base('enhanced', CC_PERIOD)), 'cc')
    return out


def make_strong_plan(settings=None):
    factory = load_factory()
    conditions = strong_conditions(factory, settings)
    lift = any(cond['setting'] in LIFTED_SETTINGS for cond in conditions)
    configs = {}
    by_slot = {}
    needs = {}

    def config_for(textures, group, period, name, slot):
        key = (textures, group, json.dumps(slot, sort_keys=True))
        if key not in by_slot:
            config_id = f'{textures}.{group}.{period}.{name}'
            assert config_id not in configs, config_id
            by_slot[key] = config_id
            configs[config_id] = {'id': config_id, 'mode': group, 'time': period, 'slot': slot,
                                  'textures': textures, 'setting': name}
        return by_slot[key]

    baselines = {textures: [config_for(textures, group, period, 'baseline',
                                       deterministic(factory_slot(factory, group, period)))
                            for group in MODES for period in TIMES]
                 for textures in TEXTURE_SETS}
    for cond in conditions:
        cond['configs'] = []
        for index, slot in enumerate(cond['slots']):
            config_id = config_for(cond['textures'], cond['group'], cond['period'],
                                   f'{cond["setting"]}.{cond["condition"]}.c{index}', slot)
            needs.setdefault(config_id, set()).update(cond['views'])
            cond['configs'].append(config_id)
        del cond['slots']

    # Two launches of the factory slots measure the noise between launches; the first also serves every
    # condition column equal to a factory slot. A third draws the factory slots with CC textures.
    launches = [{'id': launch_id, 'configs': baselines['ec'], 'views': list(STRONG_ALL_VIEWS), 'lift_clamps': lift}
                for launch_id in ('base-a', 'base-b')]
    launches.append({'id': 'base-cc', 'configs': baselines['cc'], 'views': list(STRONG_ALL_VIEWS),
                     'graphics': TEXTURE_SETS['cc'], 'lift_clamps': lift})
    launch_of = {config_id: 'base-a' for config_id in baselines['ec']}
    launch_of.update({config_id: 'base-cc' for config_id in baselines['cc']})
    packs = {}
    for config_id, views in needs.items():
        if config_id in launch_of:
            continue
        key = (configs[config_id]['textures'], tuple(v for v in STRONG_ALL_VIEWS if v in views))
        packs.setdefault(key, []).append(config_id)
    for (textures, views), pending in sorted(packs.items()):
        while pending:
            room = {group: SLOTS_PER_MODE for group in MODES}
            batch, rest = [], []
            for config_id in pending:
                group = next((g for g in SLOT_GROUPS[configs[config_id]['mode']] if room[g]), None)
                if group is None:
                    rest.append(config_id)
                    continue
                room[group] -= 1
                batch.append(config_id)
            launch_id = f'{textures}-{len(launches) - 2:02d}'
            launches.append({'id': launch_id, 'configs': batch, 'views': list(views),
                             'graphics': TEXTURE_SETS[textures], 'lift_clamps': lift})
            launch_of.update({config_id: launch_id for config_id in batch})
            pending = rest
    plan = {'configs': configs, 'launches': launches, 'strong': conditions, 'launch_of': launch_of}
    SCRATCH.mkdir(parents=True, exist_ok=True)
    PLAN.write_text(json.dumps(plan, indent=1))
    return plan


def strong_metrics():
    plan = load_plan()
    launch_of = plan['launch_of']
    results = {'noise': {}, 'conditions': []}
    for config_id in plan['launches'][0]['configs']:
        for view in STRONG_ALL_VIEWS:
            other = cap('base-b', config_id, view)
            if other.exists():
                results['noise'][f'{config_id}/{view}'] = stats(change(load_rgb(cap('base-a', config_id, view)),
                                                                        load_rgb(other)))
    for cond in plan['strong']:
        per_view = {}
        for view in cond['views']:
            images = [load_rgb(cap(launch_of[c], c, view)) for c in cond['configs']]
            per_view[view] = {'vs_off': [stats(change(images[0], image)) for image in images],
                              'vs_preset': [stats(change(images[cond['preset']], image)) for image in images]}
        results['conditions'].append(dict(cond, metrics=per_view))
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / 'strong_metrics.json').write_text(json.dumps(results, indent=1))
    worst = max(results['noise'].values(), key=lambda m: m['mean'], default=None)
    print(f'{len(results["conditions"])} conditions; worst cross-launch noise {worst}')


def _worst(metrics, column, against='vs_off'):
    return max(((view, m[against][column]) for view, m in metrics.items()),
               key=lambda item: (item[1]['mean'], item[1]['share']))


def _cell(numbers):
    return f'{numbers["mean"]:.2f} / {numbers["p99"]:.0f} / {numbers["share"] * 100:.1f}%'


def _strong_groups(conditions):
    groups = {}
    for cond in conditions:
        groups.setdefault((cond['setting'], cond['condition'], cond['textures']), []).append(cond)
    return groups


def strong_report():
    results = json.loads((RESULTS / 'strong_metrics.json').read_text())
    lines = ['# Strong values: measurements', '',
             'Each cell: mean / p99 / share over 4 levels of the change against the off column, in the view where '
             'it is largest (named in brackets). The last column compares the strongest value with the preset.', '']
    worst = max(results['noise'].values(), key=lambda m: m['mean'], default=None)
    if worst:
        lines += [f'Cross-launch noise (same values, two launches), worst view: mean {worst["mean"]:.4f}, p99 '
                  f'{worst["p99"]:.0f}, share {worst["share"] * 100:.2f}%.', '']
    for (setting, condition, textures), conds in _strong_groups(results['conditions']).items():
        labels = conds[0]['labels']
        lines += [f'## {setting}: {condition} ({textures.upper()} textures)', '',
                  '| time | ' + ' | '.join(labels[1:]) + ' | strongest vs preset |',
                  '|---|' + '---|' * len(labels)]
        for cond in conds:
            row = []
            for column in range(1, len(labels)):
                view, numbers = _worst(cond['metrics'], column)
                row.append(f'{_cell(numbers)} ({view})')
            row.append(_cell(_worst(cond['metrics'], len(labels) - 1, 'vs_preset')[1]))
            lines.append(f'| {cond["period"]} | ' + ' | '.join(row) + ' |')
        lines.append('')
    (RESULTS / 'strong_report.md').write_text('\n'.join(lines))
    print(RESULTS / 'strong_report.md')


STRONG_THUMB = (384, 288)
STRONG_CROP = (192, 144)
STRONG_LABEL = 18
STRONG_FONT = 13
STRONG_HEAT_GAIN = 8


def _best_window(d, size):
    """Top-left corner of the size window holding the most change."""
    import numpy as np
    w, h = size
    integral = np.pad(d.astype(np.int64), ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    sums = integral[h:, w:] - integral[:-h, w:] - integral[h:, :-w] + integral[:-h, :-w]
    y, x = np.unravel_index(np.argmax(sums), sums.shape)
    return int(x), int(y)


def _heat_image(d):
    import numpy as np
    from PIL import Image
    return Image.fromarray(np.clip(d.astype(np.int32) * STRONG_HEAT_GAIN, 0, 255).astype(np.uint8)).convert('RGB')


def strong_sheets():
    from PIL import Image, ImageDraw, ImageFont
    plan = load_plan()
    results = json.loads((RESULTS / 'strong_metrics.json').read_text())
    launch_of = plan['launch_of']
    font = ImageFont.load_default(size=STRONG_FONT)
    SHEETS.mkdir(parents=True, exist_ok=True)
    count = 0
    for (setting, condition, textures), conds in _strong_groups(results['conditions']).items():
        columns = len(conds[0]['labels']) + 1
        row_height = 2 * STRONG_LABEL + 2 * STRONG_THUMB[1]
        sheet = Image.new('RGB', (columns * STRONG_THUMB[0], STRONG_LABEL + len(conds) * row_height), 'black')
        draw = ImageDraw.Draw(sheet)
        draw.text((4, 2), f'{setting} - {condition} - {textures.upper()} textures. Top: full view; below: 2x crop of '
                          f'the yellow box. Numbers: change against the off column (mean / share over 4 levels).',
                  fill='white', font=font)
        for row, cond in enumerate(conds):
            last = len(cond['labels']) - 1
            view, _ = _worst(cond['metrics'], last)
            paths = [cap(launch_of[c], c, view) for c in cond['configs']]
            images = [load_rgb(path) for path in paths]
            d = change(images[0], images[last])
            height, width = d.shape
            scale = STRONG_THUMB[0] / width
            x0, y0 = _best_window(d, STRONG_CROP)
            top = STRONG_LABEL + row * row_height
            draw.text((4, top + 2), f'{cond["period"]} - {view} view', fill='yellow', font=font)
            tiles = list(zip(cond['labels'], paths)) + [(f'change, strongest vs off (x{STRONG_HEAT_GAIN})', None)]
            for col, (label, path) in enumerate(tiles):
                x = col * STRONG_THUMB[0]
                if path is None:
                    full = _heat_image(d).resize(STRONG_THUMB)
                    crop = _heat_image(d[y0:y0 + STRONG_CROP[1], x0:x0 + STRONG_CROP[0]]).resize(STRONG_THUMB,
                                                                                               Image.NEAREST)
                    text = label
                else:
                    with Image.open(path) as image:
                        rgb = image.convert('RGB')
                        full = rgb.resize(STRONG_THUMB, Image.LANCZOS)
                        crop = rgb.crop((x0, y0, x0 + STRONG_CROP[0], y0 + STRONG_CROP[1])).resize(STRONG_THUMB,
                                                                                                    Image.NEAREST)
                    m = cond['metrics'][view]['vs_off'][col]
                    text = label if col == 0 else f'{label}: {m["mean"]:.2f} / {m["share"] * 100:.1f}%'
                draw.text((x + 4, top + STRONG_LABEL + 2), text, fill='white', font=font)
                sheet.paste(full, (x, top + 2 * STRONG_LABEL))
                sheet.paste(crop, (x, top + 2 * STRONG_LABEL + STRONG_THUMB[1]))
                box = [x + x0 * scale, top + 2 * STRONG_LABEL + y0 * scale,
                       x + (x0 + STRONG_CROP[0]) * scale, top + 2 * STRONG_LABEL + (y0 + STRONG_CROP[1]) * scale]
                draw.rectangle(box, outline='yellow')
        name = f'{setting}--{condition}--{textures}'
        name = ''.join(c if c.isalnum() or c in '-_' else '_' for c in name)
        sheet.save(SHEETS / f'{name}.png', optimize=True)
        count += 1
    print(f'{count} sheets in {SHEETS}')


COMPARE_SHEETS = SHEETS / 'compare'


def strong_compare(before_scratch):
    """Every capture of this plan against the same capture in `before_scratch`, which ran the same plan with an
    earlier build. Writes the numbers per config (worst view) and one before/after sheet per style and texture set of
    the factory slots, one row per time of day in the view that changed most."""
    from PIL import Image, ImageDraw, ImageFont
    before_caps = Path(before_scratch) / 'caps'
    plan = load_plan()
    rows = {}
    for launch in plan['launches']:
        for config_id in launch['configs']:
            per_view = {}
            for view in launch['views']:
                after = cap(launch['id'], config_id, view)
                before = before_caps / launch['id'] / config_id / f'{view}.png'
                per_view[view] = stats(change(load_rgb(before), load_rgb(after)))
            rows.setdefault(config_id, {}).update(per_view)
    RESULTS.mkdir(parents=True, exist_ok=True)
    (RESULTS / 'compare.json').write_text(json.dumps(rows, indent=1))
    lines = ['# Before and after: every capture of the plan', '',
             f'Before: `{before_scratch}`. After: `{SCRATCH}`. Each cell: mean / p99 / share over 4 levels / '
             'largest change, in the view where the change is largest (named in brackets).', '',
             '| config | change |', '|---|---|']
    for config_id, per_view in sorted(rows.items()):
        view, m = max(per_view.items(), key=lambda item: (item[1]['mean'], item[1]['max']))
        lines.append(f'| {config_id} | {m["mean"]:.3f} / {m["p99"]:.0f} / {m["share"] * 100:.2f}% / {m["max"]} '
                     f'({view}) |')
    (RESULTS / 'compare.md').write_text('\n'.join(lines) + '\n')

    font = ImageFont.load_default(size=STRONG_FONT)
    COMPARE_SHEETS.mkdir(parents=True, exist_ok=True)
    launch_of = plan['launch_of']
    for textures in TEXTURE_SETS:
        for group in MODES:
            row_height = 2 * STRONG_LABEL + 2 * STRONG_THUMB[1]
            sheet = Image.new('RGB', (3 * STRONG_THUMB[0], STRONG_LABEL + len(TIMES) * row_height), 'black')
            draw = ImageDraw.Draw(sheet)
            draw.text((4, 2), f'{MODE_BUTTON[group]} factory preset, {textures.upper()} textures: before, after, '
                              f'change (x{STRONG_HEAT_GAIN}). Below: 2x crop of the yellow box.',
                      fill='white', font=font)
            for row, period in enumerate(TIMES):
                config_id = f'{textures}.{group}.{period}.baseline'
                launch_id = launch_of[config_id]
                view, m = max(rows[config_id].items(), key=lambda item: (item[1]['mean'], item[1]['max']))
                paths = [before_caps / launch_id / config_id / f'{view}.png', cap(launch_id, config_id, view)]
                images = [load_rgb(path) for path in paths]
                d = change(images[0], images[1])
                scale = STRONG_THUMB[0] / d.shape[1]
                x0, y0 = _best_window(d, STRONG_CROP)
                top = STRONG_LABEL + row * row_height
                draw.text((4, top + 2), f'{period} - {view} view: mean {m["mean"]:.3f}, p99 {m["p99"]:.0f}, '
                                        f'{m["share"] * 100:.2f}% over 4 levels, largest {m["max"]}',
                          fill='yellow', font=font)
                for col, (label, path) in enumerate(zip(('before', 'after', 'change'), paths + [None])):
                    x = col * STRONG_THUMB[0]
                    if path is None:
                        full = _heat_image(d).resize(STRONG_THUMB)
                        crop = _heat_image(d[y0:y0 + STRONG_CROP[1], x0:x0 + STRONG_CROP[0]]).resize(
                            STRONG_THUMB, Image.NEAREST)
                    else:
                        with Image.open(path) as image:
                            rgb = image.convert('RGB')
                            full = rgb.resize(STRONG_THUMB, Image.LANCZOS)
                            crop = rgb.crop((x0, y0, x0 + STRONG_CROP[0], y0 + STRONG_CROP[1])).resize(
                                STRONG_THUMB, Image.NEAREST)
                    draw.text((x + 4, top + STRONG_LABEL + 2), label, fill='white', font=font)
                    sheet.paste(full, (x, top + 2 * STRONG_LABEL))
                    sheet.paste(crop, (x, top + 2 * STRONG_LABEL + STRONG_THUMB[1]))
                    draw.rectangle([x + x0 * scale, top + 2 * STRONG_LABEL + y0 * scale,
                                    x + (x0 + STRONG_CROP[0]) * scale,
                                    top + 2 * STRONG_LABEL + (y0 + STRONG_CROP[1]) * scale], outline='yellow')
            sheet.save(COMPARE_SHEETS / f'{group}--{textures}.png', optimize=True)
    print(RESULTS / 'compare.md', COMPARE_SHEETS)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('plan')
    sub.add_parser('calibrate')
    sub.add_parser('probe')
    run = sub.add_parser('run')
    run.add_argument('ids', nargs='*')
    launch = sub.add_parser('launch')
    launch.add_argument('ids', nargs='+')
    sub.add_parser('metrics')
    sub.add_parser('sheets')
    sub.add_parser('report')
    strong = sub.add_parser('strong-plan')
    strong.add_argument('--settings', help='comma-separated settings to keep (default: all)')
    for name in ('strong-metrics', 'strong-report', 'strong-sheets'):
        sub.add_parser(name)
    compare = sub.add_parser('strong-compare')
    compare.add_argument('before', help='scratch folder of the same plan run with the earlier build')
    args = parser.parse_args()
    if args.command == 'plan':
        plan = make_plan()
        statuses = {}
        for entry in plan['entries']:
            statuses[entry['status']] = statuses.get(entry['status'], 0) + 1
        per_mode = {m: sum(1 for c in plan['configs'].values() if c['setting'] and c['mode'] == m) for m in MODES}
        print(json.dumps({'entries': statuses, 'variants_per_mode': per_mode,
                          'launches': [l['id'] for l in plan['launches']]}))
    elif args.command == 'calibrate':
        calibrate()
    elif args.command == 'probe':
        probe(PROBE_CANDIDATES)
    elif args.command == 'run':
        run_all(args.ids)
    elif args.command == 'launch':
        for launch_id in args.ids:
            run_launch(launch_id)
    elif args.command == 'metrics':
        metrics()
    elif args.command == 'sheets':
        sheets()
    elif args.command == 'report':
        report()
    elif args.command == 'strong-plan':
        plan = make_strong_plan(args.settings.split(',') if args.settings else None)
        print(json.dumps({'conditions': len(plan['strong']), 'configs': len(plan['configs']),
                          'launches': {l['id']: len(l['configs']) for l in plan['launches']}}))
    elif args.command == 'strong-metrics':
        strong_metrics()
    elif args.command == 'strong-report':
        strong_report()
    elif args.command == 'strong-sheets':
        strong_sheets()
    elif args.command == 'strong-compare':
        strong_compare(args.before)


if __name__ == '__main__':
    main()
