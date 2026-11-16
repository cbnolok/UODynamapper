#!/usr/bin/env python3
"""Screenshot regression harness: capture fixed views with a Dynamapper binary and compare them with a baseline.

The views are the fixtures of ../fixtures.toml (read strictly). Commands:

  list                      the selected fixtures and the captures they expand to
  record LABEL              capture the selection with --binary into LABEL
  compare BASELINE LABEL    capture LABEL with --binary unless it exists, then diff it against BASELINE
  sheet LABEL               contact sheets of a label's captures
  clean                     delete old labels (--keep N) or the labels named
  self-test                 offline checks of the fixture parsing, settings rewriting and diff math

Selection: --tier quick|full, --only NAME_OR_TAG (glob, repeatable), --style, --source ART/LAND, --time.

Each capture stores the collected statics' digest (status.art.digest), which does not depend on the loading order:
compare marks a capture changed when two labels of one binary (live and --fresh-launch) collected different statics.
--reload-land-check also drops the loaded map blocks and the cached statics after each capture (the control
command reload_land, the state 60 idle seconds reach) and requires the same digest after the rebuild.

Every GUI run follows the repository's preview rules itself: KDE "Desktop 4" through the exact-PID preview helper,
a private working directory with copies of the tree's assets/ and a fresh cache/, never the user's settings; the
shared preview lock held around each launch only; the control socket for positioning and capture; the first capture
of a launch and after every runtime switch discarded; water and lava animation and the KR fog's cloud drift frozen in
every preset; each capture repeated until two consecutive frames agree (else marked unstable).

By default one launch per map runs fixture after fixture, switching the style, time of day, art and land sources,
zoom and the whitelisted settings through the control socket as the UI does, for at most --chunk-seconds before it
closes and the next launch goes on (the preview lock is held per launch). A fixture with an override the socket
cannot switch, or a binary without the switch commands, gets the --fresh-launch behaviour: one launch per style,
source, time of day, map and settings set, up to a time budget.

Labels live outside the repository (derived from client files), under --root. Each holds manifest.json (tree commit,
binary hash, fixture definitions, per-capture status) and caps/<fixture>/<style>.<time>.<art>-<land>.<ext>.
Comparisons write compares/<baseline>--<label>/summary.json and JPEG sheets. Exit status: 0 no change beyond the
fixtures' tolerances, 1 unexpected changes, 2 errors (failed launches, missing or stale captures).
"""
import argparse
import concurrent.futures
import datetime
import fcntl
import fnmatch
import hashlib
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest
import zlib
from dataclasses import dataclass
from pathlib import Path

SCRIPT = Path(__file__).resolve()
SKILL = SCRIPT.parents[1]
REPO = SCRIPT.parents[3]
FIXTURES = SKILL / 'fixtures.toml'
CONTROL = REPO / 'scripts/dev-helpers/renderer/control.py'
PREVIEW = REPO / 'skills/kde-x11-preview/scripts/preview.py'
SCRATCH_ROOT = Path('/home/claudio/rust-target/uodynamapper-scratch')
DEFAULT_ROOT = SCRATCH_ROOT / 'visual-baselines'
PREVIEW_LOCK = SCRATCH_ROOT / 'preview.lock'
LOWPRIO = SCRATCH_ROOT / 'lowprio.sh'
DEFAULT_PACKAGES = Path('/home/claudio/test/uddp')
DESKTOP = 'Desktop 4'
HARNESS_VERSION = 1

STYLES = ('classic', 'enhanced', 'kr', 'painterly')
TIMES = ('morning', 'afternoon', 'night', 'cave')
ART_SOURCES = ('cc', 'ec')
LAND_SOURCES = ('cc', 'ec', 'kr')
TIERS = ('quick', 'full')
CAMERAS = ('orthographic', 'perspective')
MAX_MAP = 5
MAX_COORD = 7168
MIN_Z, MAX_Z = -128, 127
MIN_ZOOM, MAX_ZOOM = 0.1, 20.0
MAX_PIXEL_THRESHOLD = 255
MAX_CHANGED_PCT = 100.0
NAME_PATTERN = re.compile(r'^[a-z0-9]+(-[a-z0-9]+)*$')

# A preview window two thirds of the screen or larger comes up maximized: keep at or under this width.
MAX_WINDOW_WIDTH = 1900
MAX_WINDOW_HEIGHT = 1100
DEFAULT_WINDOW = (1280, 800)

# preview.py refuses --seconds above 300; captures stop being started past the budget and continue in a new launch.
PREVIEW_SECONDS = 300
LAUNCH_BUDGET_SECONDS = 220
# A switching launch holds the shared preview lock for about this long (plus its last capture and the shutdown)
# before it closes and lets other agents' previews in; the next launch goes on from there.
LIVE_CHUNK_SECONDS = 60
# The overrides the control socket switches at runtime (dynamapper's dev_control/settings.rs): a fixture whose
# overrides are all here shares the switching launches; any other override needs launches of its own.
LIVE_SETTINGS = {
    'world_rendering': ('show_only_player_floor', 'caves', 'cave_surroundings', 'enable_statics',
                        'enable_static_lights', 'show_hidden_multi_parts'),
    'graphics': ('cc_terrain_transitions', 'ec_terrain_transitions'),
    'style': ('light_cover', 'player_floor_rule', 'land_texture_variation'),
}
# A status field only binaries with the switch commands report; older binaries run in --fresh-launch mode.
SWITCH_MARKER = b'frames_since_switch'
MAX_CAPTURES_PER_LAUNCH = 16
PLACEMENT_TIMEOUT_SECONDS = 60
SOCKET_TIMEOUT_SECONDS = 180
SOCKET_POLL_SECONDS = 0.2
HELPER_EXIT_SECONDS = 30
REQUEST_TIMEOUT_SECONDS = 60
SCENE_TIMEOUT_SECONDS = 240
SCENE_WAIT_SECONDS = 60
SCENE_RETRY_SECONDS = 0.5
SCENE_STABLE_SNAPSHOTS = 3
SCENE_POLL_SECONDS = 0.1
# After the scene reports ready: light-cover rechecks and uploads that follow the land.
SETTLE_SECONDS = 1.0
# The zoom the app reports is an f32: compare it to the fixture's with this slack.
ZOOM_TOLERANCE = 1e-4
CAPTURE_TIMEOUT_SECONDS = 60
CAPTURE_POLL_SECONDS = 0.03
# Each capture is taken until two consecutive frames agree, at most this many times.
STABLE_ATTEMPTS = 4
STABLE_INTERVAL_SECONDS = 0.5
# A Unix socket path must fit sockaddr_un.sun_path (108 bytes with its terminator); control.py binds its reply
# socket in a 36-character temporary folder under tempfile's directory.
SUN_PATH_MAX = 107
CONTROL_REPLY_SUFFIX = len('/dynamapper-ctl-xxxxxxxx/reply.sock')
LOG_ERROR_WORDS = ('ERROR', 'panicked', 'Validation Error', 'wgpu error')
LOG_TAIL_LINES = 40
ENCODE_WORKERS = 4
ENCODE_NICE = 10

FORMATS = {'webp': '.webp', 'png': '.png', 'jpeg': '.jpg'}
LOSSLESS_FORMATS = ('webp', 'png')
WEBP_LOSSLESS_METHOD = 4
WEBP_LOSSLESS_QUALITY = 80
JPEG_QUALITY = 95

SHEET_SCALE = 2
SHEET_LABEL_HEIGHT = 34
SHEET_GAP = 4
SHEET_JPEG_QUALITY = 88
DIFF_GAIN = 4
SHEET_BACKGROUND = (24, 24, 24)
SHEET_TEXT = (235, 235, 235)
SHEET_WARN = (255, 140, 110)
FONT_PATH = '/usr/share/fonts/TTF/DejaVuSans.ttf'
FONT_SIZE = 13
VERDICTS = ('identical', 'within', 'changed')


class HarnessError(Exception):
    """A user-facing error: the message is printed and the harness exits with status 2."""


# ----------------------------------------------------------------------------------------------------------------
# Fixtures
# ----------------------------------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Tolerance:
    pixel_threshold: int
    changed_pct: float


@dataclass
class Fixture:
    name: str
    guards: str
    tier: str
    map: int
    x: int
    y: int
    z: int
    tags: tuple
    times: tuple
    styles: tuple
    sources: dict          # style -> tuple of (art, land); full tier
    quick_sources: dict    # style -> tuple of (art, land); quick tier
    zoom: float
    camera: str
    tolerance: Tolerance
    settings: dict
    reference: dict = None

    def view(self):
        """Everything a capture of this fixture depends on: a baseline taken with another view is stale."""
        return {'map': self.map, 'x': self.x, 'y': self.y, 'z': self.z, 'zoom': self.zoom, 'camera': self.camera,
                'settings': self.settings}

    def view_hash(self):
        return hashlib.sha256(json.dumps(self.view(), sort_keys=True).encode()).hexdigest()[:16]


FIXTURE_REQUIRED = ('name', 'guards', 'tier', 'map', 'x', 'y', 'z')
FIXTURE_OPTIONAL = ('tags', 'times', 'styles', 'sources', 'zoom', 'camera', 'tolerance', 'settings', 'reference')
DEFAULTS_REQUIRED = ('times', 'styles', 'zoom', 'camera', 'tolerance', 'sources', 'quick_sources')
TOLERANCE_KEYS = ('pixel_threshold', 'changed_pct')
SETTINGS_KEYS = ('world_rendering', 'graphics', 'style', 'preset')
REFERENCE_REQUIRED = ('path', 'note')
REFERENCE_OPTIONAL = ('crop',)


class FixtureFile:
    """Strict reader of the fixtures file: errors name the file, the line, the offending name and the allowed names."""

    def __init__(self, path, text):
        self.path = path
        self.lines = text.splitlines()

    def line_of(self, pattern):
        regex = re.compile(pattern)
        for number, line in enumerate(self.lines, 1):
            if regex.search(line):
                return number
        return 0

    def error(self, line, message):
        return HarnessError(f'{self.path}:{line}: {message}')

    def check_keys(self, table, required, optional, where, line):
        unknown = sorted(set(table) - set(required) - set(optional))
        if unknown:
            raise self.error(line, f'{where}: unknown key "{unknown[0]}"; allowed: {", ".join(required + optional)}')
        missing = [key for key in required if key not in table]
        if missing:
            raise self.error(line, f'{where}: missing required key "{missing[0]}"')

    def typed(self, value, kinds, where, line, key):
        # bool is an int in Python; a TOML true is never a number here.
        if isinstance(value, bool) and bool not in kinds or not isinstance(value, kinds):
            names = '/'.join(kind.__name__ for kind in kinds)
            raise self.error(line, f'{where}: "{key}" must be {names}, got {value!r}')
        return value

    def choice_list(self, value, allowed, where, line, key):
        self.typed(value, (list,), where, line, key)
        if not value:
            raise self.error(line, f'{where}: "{key}" must not be empty')
        for item in value:
            if item not in allowed:
                raise self.error(line, f'{where}: "{key}" has unknown value "{item}"; allowed: {", ".join(allowed)}')
        if len(set(value)) != len(value):
            raise self.error(line, f'{where}: "{key}" repeats a value')
        return tuple(value)

    def source_list(self, value, where, line, key):
        self.typed(value, (list,), where, line, key)
        if not value:
            raise self.error(line, f'{where}: "{key}" must not be empty')
        out = []
        for item in value:
            parts = item.split('/') if isinstance(item, str) else []
            if len(parts) != 2 or parts[0] not in ART_SOURCES or parts[1] not in LAND_SOURCES:
                raise self.error(line, f'{where}: "{key}" has "{item}"; a source is "art/land" with art '
                                       f'{"|".join(ART_SOURCES)} and land {"|".join(LAND_SOURCES)}')
            out.append(tuple(parts))
        if len(set(out)) != len(out):
            raise self.error(line, f'{where}: "{key}" repeats a source')
        return tuple(out)

    def tolerance(self, value, where, line):
        self.typed(value, (dict,), where, line, 'tolerance')
        self.check_keys(value, TOLERANCE_KEYS, (), f'{where}.tolerance', line)
        threshold = self.typed(value['pixel_threshold'], (int,), where, line, 'pixel_threshold')
        share = self.typed(value['changed_pct'], (int, float), where, line, 'changed_pct')
        if not 0 <= threshold < MAX_PIXEL_THRESHOLD or not 0 <= share <= MAX_CHANGED_PCT:
            raise self.error(line, f'{where}: tolerance out of range (pixel_threshold 0..{MAX_PIXEL_THRESHOLD - 1}, '
                                   f'changed_pct 0..{MAX_CHANGED_PCT})')
        return Tolerance(threshold, float(share))

    def per_style_sources(self, value, where, line, key):
        self.typed(value, (dict,), where, line, key)
        self.check_keys(value, STYLES, (), f'{where}.{key}', line)
        return {style: self.source_list(value[style], f'{where}.{key}', line, style) for style in STYLES}


def load_fixtures(path=FIXTURES, settings_files=None):
    """Read and check the fixtures file. settings_files (SettingsFiles) also checks the settings overrides."""
    text = Path(path).read_text()
    reader = FixtureFile(path, text)
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise HarnessError(f'{path}: {error}') from error
    reader.check_keys(data, ('defaults', 'fixture'), (), 'top level', 1)
    defaults_line = reader.line_of(r'^\[defaults\]')
    defaults = reader.typed(data['defaults'], (dict,), 'defaults', defaults_line, 'defaults')
    reader.check_keys(defaults, DEFAULTS_REQUIRED, (), 'defaults', defaults_line)
    base = {
        'times': reader.choice_list(defaults['times'], TIMES, 'defaults', defaults_line, 'times'),
        'styles': reader.choice_list(defaults['styles'], STYLES, 'defaults', defaults_line, 'styles'),
        'zoom': float(reader.typed(defaults['zoom'], (int, float), 'defaults', defaults_line, 'zoom')),
        'camera': defaults['camera'],
        'tolerance': reader.tolerance(defaults['tolerance'], 'defaults', defaults_line),
        'sources': reader.per_style_sources(defaults['sources'], 'defaults', defaults_line, 'sources'),
        'quick_sources': reader.per_style_sources(defaults['quick_sources'], 'defaults', defaults_line,
                                                  'quick_sources'),
    }
    if base['camera'] not in CAMERAS:
        raise reader.error(defaults_line, f'defaults: camera "{base["camera"]}"; allowed: {", ".join(CAMERAS)}')
    fixtures = []
    seen = set()
    for index, table in enumerate(reader.typed(data['fixture'], (list,), 'fixture', 1, 'fixture')):
        name = table.get('name') if isinstance(table, dict) else None
        line = reader.line_of(rf'^name\s*=\s*"{re.escape(name)}"') if isinstance(name, str) else 0
        where = f'fixture "{name}"' if isinstance(name, str) else f'fixture #{index + 1}'
        reader.typed(table, (dict,), where, line, 'fixture')
        reader.check_keys(table, FIXTURE_REQUIRED, FIXTURE_OPTIONAL, where, line)
        reader.typed(name, (str,), where, line, 'name')
        if not NAME_PATTERN.match(name):
            raise reader.error(line, f'{where}: a name is lower-case words joined by "-"')
        if name in seen:
            raise reader.error(line, f'{where}: duplicate name')
        seen.add(name)
        guards = reader.typed(table['guards'], (str,), where, line, 'guards').strip()
        if not guards:
            raise reader.error(line, f'{where}: "guards" must say what the view guards against')
        if table['tier'] not in TIERS:
            raise reader.error(line, f'{where}: tier "{table["tier"]}"; allowed: {", ".join(TIERS)}')
        position = [reader.typed(table[key], (int,), where, line, key) for key in ('map', 'x', 'y', 'z')]
        if not (0 <= position[0] <= MAX_MAP and 0 <= position[1] < MAX_COORD and 0 <= position[2] < MAX_COORD
                and MIN_Z <= position[3] <= MAX_Z):
            raise reader.error(line, f'{where}: position out of range (map 0..{MAX_MAP}, x and y 0..{MAX_COORD - 1}, '
                                     f'z {MIN_Z}..{MAX_Z})')
        tags = table.get('tags', [])
        reader.typed(tags, (list,), where, line, 'tags')
        for tag in tags:
            if not isinstance(tag, str) or not NAME_PATTERN.match(tag):
                raise reader.error(line, f'{where}: tag {tag!r} is not lower-case words joined by "-"')
        sources = base['sources']
        quick_sources = base['quick_sources']
        if 'sources' in table:
            own = reader.source_list(table['sources'], where, line, 'sources')
            sources = quick_sources = {style: own for style in STYLES}
        zoom = float(reader.typed(table.get('zoom', base['zoom']), (int, float), where, line, 'zoom'))
        if not MIN_ZOOM <= zoom <= MAX_ZOOM:
            raise reader.error(line, f'{where}: zoom out of range ({MIN_ZOOM}..{MAX_ZOOM})')
        camera = table.get('camera', base['camera'])
        if camera not in CAMERAS:
            raise reader.error(line, f'{where}: camera "{camera}"; allowed: {", ".join(CAMERAS)}')
        settings = reader.typed(table.get('settings', {}), (dict,), where, line, 'settings')
        reader.check_keys(settings, (), SETTINGS_KEYS, f'{where}.settings', line)
        for group, values in settings.items():
            reader.typed(values, (dict,), f'{where}.settings', line, group)
            for key, value in values.items():
                reader.typed(value, (bool, int, float, str), f'{where}.settings.{group}', line, key)
        reference = table.get('reference')
        if reference is not None:
            reader.typed(reference, (dict,), where, line, 'reference')
            reader.check_keys(reference, REFERENCE_REQUIRED, REFERENCE_OPTIONAL, f'{where}.reference', line)
            crop = reference.get('crop')
            if crop is not None and (not isinstance(crop, list) or len(crop) != 4
                                     or not all(isinstance(v, int) and not isinstance(v, bool) and v >= 0
                                                for v in crop) or crop[2] == 0 or crop[3] == 0):
                raise reader.error(line, f'{where}: reference crop is [x, y, width, height] in pixels')
        fixture = Fixture(
            name=name, guards=' '.join(guards.split()), tier=table['tier'], map=position[0], x=position[1],
            y=position[2], z=position[3], tags=tuple(tags),
            times=reader.choice_list(table['times'], TIMES, where, line, 'times') if 'times' in table
            else base['times'],
            styles=reader.choice_list(table['styles'], STYLES, where, line, 'styles') if 'styles' in table
            else base['styles'],
            sources=sources, quick_sources=quick_sources, zoom=zoom, camera=camera,
            tolerance=reader.tolerance(table['tolerance'], where, line) if 'tolerance' in table
            else base['tolerance'],
            settings={group: dict(sorted(values.items())) for group, values in sorted(settings.items())},
            reference=reference)
        if settings_files is not None:
            try:
                settings_files.check_overrides(fixture)
            except HarnessError as error:
                raise reader.error(line, f'{where}: {error}') from error
        fixtures.append(fixture)
    return fixtures


# ----------------------------------------------------------------------------------------------------------------
# Selection and planning
# ----------------------------------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Capture:
    fixture: Fixture
    style: str
    time: str
    art: str
    land: str

    @property
    def stem(self):
        return f'{self.style}.{self.time}.{self.art}-{self.land}'

    @property
    def key(self):
        return f'{self.fixture.name}/{self.stem}'

    @property
    def source(self):
        return f'{self.art}/{self.land}'

    def group(self):
        """Captures with equal groups share one launch: everything here is fixed when the app starts."""
        f = self.fixture
        return (self.style, self.time, self.art, self.land, f.map, f.zoom, f.camera,
                json.dumps(f.settings, sort_keys=True))

    def live_order(self):
        """The switching launches' order: the settings set outermost, then the package switches, then the preset, the
        views in fixture order within. Each settings set then visits its views in the order of a --fresh-launch launch:
        some light glows depend on the views visited before (a renderer bug), so a different order changes pixels."""
        return (self.fixture.camera, self.fixture.map, json.dumps(self.fixture.settings, sort_keys=True), self.art,
                self.land, self.style, self.time)


def live_capable(fixture):
    """Whether the control socket can switch every override of the fixture at runtime."""
    return all(key in LIVE_SETTINGS.get(group, ()) for group, values in fixture.settings.items() for key in values)


def fixture_selected(fixture, args):
    if args.tier == 'quick' and fixture.tier != 'quick':
        return False
    if args.only:
        names = (fixture.name,) + fixture.tags
        return any(fnmatch.fnmatchcase(name, pattern) for pattern in args.only for name in names)
    return True


def expand(fixtures, args):
    captures = []
    for fixture in fixtures:
        if not fixture_selected(fixture, args):
            continue
        sources = fixture.quick_sources if args.tier == 'quick' else fixture.sources
        for style in fixture.styles:
            if args.style and style not in args.style:
                continue
            for time_of_day in fixture.times:
                if args.time and time_of_day not in args.time:
                    continue
                for art, land in sources[style]:
                    if args.source and f'{art}/{land}' not in args.source:
                        continue
                    captures.append(Capture(fixture, style, time_of_day, art, land))
    return captures


def plan_launches(captures, live=False):
    """Group captures by what is fixed at startup, in fixture order, chunked to the per-launch limit.

    live: the captures the socket can switch to form one sequence per camera and map instead (the app loads only its
    startup map), chunked by time at run time."""
    groups = {}
    launches = []
    if live:
        switchable = sorted((c for c in captures if live_capable(c.fixture)), key=Capture.live_order)
        for capture in switchable:
            groups.setdefault(('live', capture.fixture.camera, capture.fixture.map), []).append(capture)
        launches = list(groups.values())
        groups = {}
        captures = [c for c in captures if not live_capable(c.fixture)]
    for capture in captures:
        groups.setdefault(capture.group(), []).append(capture)
    for group, members in groups.items():
        for start in range(0, len(members), MAX_CAPTURES_PER_LAUNCH):
            launches.append(members[start:start + MAX_CAPTURES_PER_LAUNCH])
    return launches


# ----------------------------------------------------------------------------------------------------------------
# Settings files of a run
# ----------------------------------------------------------------------------------------------------------------

HEADER = re.compile(r'^\s*\[([A-Za-z0-9_.]+)\]\s*(#.*)?$')
ASSIGNMENT = re.compile(r'^(\s*)([A-Za-z0-9_]+)\s*=\s*(.*)$')


def toml_literal(value):
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return '[' + ', '.join(toml_literal(item) for item in value) + ']'
    raise TypeError(f'no TOML literal for {value!r}')


def toml_set(text, section, key, value, required=True):
    """Replace `key = ...` in [section] ('' is the top level). Returns (text, replaced)."""
    lines = text.split('\n')
    current = ''
    for index, line in enumerate(lines):
        header = HEADER.match(line)
        if header:
            current = header.group(1)
            continue
        assignment = ASSIGNMENT.match(line)
        if current == section and assignment and assignment.group(2) == key:
            lines[index] = f'{assignment.group(1)}{key} = {toml_literal(value)}'
            return '\n'.join(lines), True
    if required:
        raise HarnessError(f'no "{key}" in [{section or "top level"}]')
    return text, False


def sections_of(text):
    return [match.group(1) for match in map(HEADER.match, text.split('\n')) if match]


def freeze_presets(text):
    """Freeze the time-driven terms of every preset slot: water and lava animation and the KR fog's cloud drift."""
    data = tomllib.loads(text)
    for style in STYLES:
        for period in TIMES:
            slot = data.get(style, {}).get(period)
            if slot is None:
                continue
            if 'enable_water_animation' in slot.get('effects', {}):
                text, _ = toml_set(text, f'{style}.{period}.effects', 'enable_water_animation', 0)
            fog = slot.get('lighting', {}).get('fog_params')
            if fog is not None:
                frozen = [float(v) for v in fog[:3]] + [0.0]
                text, _ = toml_set(text, f'{style}.{period}.lighting', 'fog_params', frozen)
    return text


def check_frozen(text):
    data = tomllib.loads(text)
    for style in STYLES:
        for period in TIMES:
            slot = data.get(style, {}).get(period, {})
            if slot.get('effects', {}).get('enable_water_animation', 0) != 0:
                raise HarnessError(f'{style}.{period}: water animation is not frozen')
            fog = slot.get('lighting', {}).get('fog_params')
            if fog is not None and fog[3] != 0:
                raise HarnessError(f'{style}.{period}: the fog cloud drift is not frozen')


class SettingsFiles:
    """The tree's bundled settings: the source of each run's copy and of the allowed override names."""

    def __init__(self, assets):
        self.assets = Path(assets)
        settings = self.assets / 'settings'
        self.paths = {'world_rendering': settings / 'world_rendering.toml', 'graphics': settings / 'graphics.toml',
                      'land': settings / 'shaders/land.toml'}
        for path in self.paths.values():
            if not path.is_file():
                raise HarnessError(f'{path}: missing; --tree must be a Dynamapper checkout')
        self.data = {name: tomllib.loads(path.read_text()) for name, path in self.paths.items()}

    @staticmethod
    def lookup(table, dotted):
        for part in dotted.split('.'):
            if not isinstance(table, dict) or part not in table:
                return None
            table = table[part]
        return table

    @staticmethod
    def same_type(old, new):
        if isinstance(old, bool) or isinstance(new, bool):
            return isinstance(old, bool) and isinstance(new, bool)
        if isinstance(old, (int, float)):
            return isinstance(new, (int, float))
        return isinstance(new, type(old))

    def check_overrides(self, fixture):
        land = self.data['land']
        for group, values in fixture.settings.items():
            for key, value in values.items():
                if group == 'world_rendering':
                    old = self.lookup(self.data['world_rendering'], key)
                    allowed = [k for k, v in self.data['world_rendering'].items() if not isinstance(v, dict)]
                elif group == 'graphics':
                    old = self.lookup(self.data['graphics'], f'graphics.{key}')
                    allowed = list(self.data['graphics']['graphics'])
                elif group == 'style':
                    olds = [land[s][key] for s in STYLES
                            if key in land.get(s, {}) and not isinstance(land[s][key], dict)]
                    old = olds[0] if olds else None
                    allowed = sorted({k for s in STYLES for k, v in land.get(s, {}).items()
                                      if not isinstance(v, dict) and k not in TIMES})
                else:
                    olds = [self.lookup(land, f'{s}.{t}.{key}') for s in STYLES for t in TIMES]
                    olds = [v for v in olds if v is not None and not isinstance(v, dict)]
                    old = olds[0] if olds else None
                    allowed = ['<table>.<key> of a preset slot, such as effects.light_decal_intensity']
                if old is None or isinstance(old, dict):
                    raise HarnessError(f'settings.{group}: unknown key "{key}"; allowed: {", ".join(allowed)}')
                if not self.same_type(old, value):
                    raise HarnessError(f'settings.{group}.{key}: {value!r} has not the type of the file\'s {old!r}')


def live_state(files, capture):
    """Everything a switching launch sets for a capture, the tree's values where the fixture overrides nothing."""
    f = capture.fixture
    world = {key: files.data['world_rendering'][key] for key in LIVE_SETTINGS['world_rendering']}
    graphics = {key: files.data['graphics']['graphics'][key] for key in LIVE_SETTINGS['graphics']}
    own = files.data['land'].get(capture.style, {})
    # A style without the key (player_floor_rule outside KR-like and UOD-Painterly) skips it, as a fresh launch does.
    style = {key: f.settings.get('style', {}).get(key, own[key]) for key in LIVE_SETTINGS['style'] if key in own}
    return {'preset': (capture.style, capture.time), 'sources': (capture.art, capture.land),
            'settings': world | f.settings.get('world_rendering', {}) | graphics | f.settings.get('graphics', {}),
            'style': style, 'zoom': f.zoom, 'position': [f.x, f.y, f.z, f.map]}


def supports_switching(binary):
    """Whether the binary has the control socket's switch commands (its status reports frames_since_switch)."""
    overlap = len(SWITCH_MARKER)
    tail = b''
    with open(binary, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 22), b''):
            if SWITCH_MARKER in tail + block:
                return True
            tail = block[-overlap:]
    return False


def write_run_settings(files, run_dir, capture, window, packages):
    """Make the run's private assets/ and cache/ for one launch group."""
    fixture = capture.fixture
    assets = run_dir / 'assets'
    shutil.copytree(files.assets, assets)
    settings = assets / 'settings'
    runtime = settings / 'runtime_assets.toml'
    runtime.write_text(toml_set(runtime.read_text(), '', 'udd_path', str(packages).rstrip('/') + '/')[0])
    graphics = settings / 'graphics.toml'
    text = graphics.read_text()
    for key, value in (('art_texture_source', capture.art), ('land_texture_source', capture.land),
                       ('reduce_unfocused_fps', False)):
        text = toml_set(text, 'graphics', key, value)[0]
    for key, value in fixture.settings.get('graphics', {}).items():
        text = toml_set(text, 'graphics', key, value)[0]
    graphics.write_text(text)
    prefs = settings / 'user_preferences.toml'
    text = prefs.read_text()
    for key in ('show_overlay', 'show_perf_stats_overlay', 'show_player_position_overlay',
                'show_cursor_inspect_overlay'):
        text = toml_set(text, 'performance', key, False)[0]
    text = toml_set(text, 'window', 'perspective_camera', fixture.camera == 'perspective')[0]
    prefs.write_text(text)
    world = settings / 'world_rendering.toml'
    text = world.read_text()
    for key, value in fixture.settings.get('world_rendering', {}).items():
        section, _, name = key.rpartition('.')
        text = toml_set(text, section, name, value)[0]
    world.write_text(text)

    cache = run_dir / 'cache/settings'
    (cache / 'shaders').mkdir(parents=True)
    (cache / 'session_state.toml').write_text(
        f'[world.last_p]\nx = {fixture.x}\ny = {fixture.y}\nz = {fixture.z}\nm = {fixture.map}\n\n'
        f'[window]\nheight = {float(window[1])}\nwidth = {float(window[0])}\nzoom = {fixture.zoom}\n')
    text = freeze_presets(files.paths['land'].read_text())
    text = toml_set(text, '', 'default_preset', f'{capture.style}.{capture.time}')[0]
    applied = {}
    for key, value in fixture.settings.get('style', {}).items():
        text, done = toml_set(text, capture.style, key, value, required=False)
        applied[f'style.{key}'] = done
    for key, value in fixture.settings.get('preset', {}).items():
        table, _, name = key.rpartition('.')
        section = f'{capture.style}.{capture.time}' + (f'.{table}' if table else '')
        text, done = toml_set(text, section, name, value, required=False)
        applied[f'preset.{key}'] = done
    check_frozen(text)
    (cache / 'shaders/land.toml').write_text(text)
    return applied


# ----------------------------------------------------------------------------------------------------------------
# Launch
# ----------------------------------------------------------------------------------------------------------------

def load_control():
    spec = importlib.util.spec_from_file_location('dynamapper_control', CONTROL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PreviewLock:
    """The shared preview lock (the same flock(2) lock as `flock preview.lock`), held around one launch only."""

    def __enter__(self):
        PREVIEW_LOCK.parent.mkdir(parents=True, exist_ok=True)
        self.stream = open(PREVIEW_LOCK, 'a')
        began = time.monotonic()
        try:
            fcntl.flock(self.stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log('  waiting for the preview lock (another agent is capturing)')
            fcntl.flock(self.stream, fcntl.LOCK_EX)
        self.waited = round(time.monotonic() - began, 2)
        return self

    def __exit__(self, *_):
        fcntl.flock(self.stream, fcntl.LOCK_UN)
        self.stream.close()


def log(message):
    print(message, flush=True)


def png_rgb(path):
    import numpy as np
    from PIL import Image
    with Image.open(path) as image:
        return np.asarray(image.convert('RGB'))


class Launch:
    def __init__(self, ctx, launch_id, captures, live=False):
        self.ctx = ctx
        self.id = launch_id
        self.captures = captures
        self.live = live
        self.dir = ctx.work / launch_id
        if self.dir.exists():
            shutil.rmtree(self.dir)
        self.dir.mkdir(parents=True)
        self.staging = self.dir / 'staging'
        self.staging.mkdir()
        self.applied = write_run_settings(ctx.files, self.dir, captures[0], ctx.window, ctx.packages)
        self.socket = ctx.sockets / f'{zlib.crc32(f"{ctx.label}/{launch_id}/{os.getpid()}".encode()):08x}.sock'
        if self.socket.exists():
            self.socket.unlink()
        self.child = None
        self.control = ctx.control
        self.record = {'id': launch_id, 'mode': 'live' if live else 'fresh', 'captures': [c.key for c in captures],
                       'overrides_applied': self.applied}

    # -- process -------------------------------------------------------------------------------------------------
    def start(self):
        env = os.environ.copy()
        env['DYNAMAPPER_CONTROL_SOCKET'] = str(self.socket)
        env['TMPDIR'] = str(self.dir)
        command = [sys.executable, '-I', '-B', str(PREVIEW), '--desktop', DESKTOP, '--cwd', str(self.dir),
                   '--display', env['DISPLAY'], '--seconds', str(PREVIEW_SECONDS),
                   '--placement-timeout', str(PLACEMENT_TIMEOUT_SECONDS), '--', str(self.ctx.binary)]
        if LOWPRIO.is_file():
            command = [str(LOWPRIO)] + command
        self.out = open(self.dir / 'preview.out', 'w')
        self.child = subprocess.Popen(command, env=env, stdout=self.out, stderr=subprocess.STDOUT)
        self.started = time.monotonic()
        deadline = self.started + SOCKET_TIMEOUT_SECONDS
        while not self.socket.exists():
            if self.child.poll() is not None:
                raise HarnessError(f'{self.id}: preview exited early:\n{self.preview_text()[-2000:]}')
            if time.monotonic() > deadline:
                raise HarnessError(f'{self.id}: the control socket never appeared')
            time.sleep(SOCKET_POLL_SECONDS)

    def stop(self):
        if self.child and self.child.poll() is None:
            self.child.send_signal(signal.SIGTERM)
            try:
                self.child.wait(HELPER_EXIT_SECONDS)
            except subprocess.TimeoutExpired:
                log(f'  {self.id}: the preview helper did not exit; its own cleanup owns the app')
        if self.child:
            self.out.close()
        # The app leaves its socket file behind.
        self.socket.unlink(missing_ok=True)

    def preview_text(self):
        path = self.dir / 'preview.out'
        return path.read_text(errors='replace') if path.exists() else ''

    def app_log(self):
        for line in self.preview_text().splitlines():
            try:
                artifacts = json.loads(line).get('artifacts')
            except (json.JSONDecodeError, AttributeError):
                continue
            if artifacts:
                return Path(artifacts) / 'app.log'
        return None

    def log_errors(self):
        path = self.app_log()
        if not (path and path.exists()):
            return ['no app log']
        return [line[:300] for line in path.read_text(errors='replace').splitlines()
                if any(word in line for word in LOG_ERROR_WORDS)]

    # -- control -------------------------------------------------------------------------------------------------
    def request(self, command):
        return self.control.request(self.socket, command, REQUEST_TIMEOUT_SECONDS)

    def wait_terrain(self, position):
        """control.wait_for_scene without the art conditions, for a run with statics turned off."""
        deadline = time.monotonic() + SCENE_WAIT_SECONDS
        stable, previous = 0, None
        while True:
            if time.monotonic() > deadline:
                raise TimeoutError('terrain did not settle')
            status = self.request({'command': 'status'})
            terrain = status.get('terrain')
            if (status.get('position') == position and status.get('rendered_position') == position and terrain
                    and not terrain['metadata_pending']
                    and terrain['metadata_revision'] == terrain['submitted_metadata_revision']
                    and self.control.switch_settled(status)):
                fingerprint = self.control.scene_fingerprint(status)
                stable = stable + 1 if fingerprint == previous else 1
                previous = fingerprint
                if stable >= SCENE_STABLE_SNAPSHOTS:
                    return status
            else:
                stable, previous = 0, None
            time.sleep(SCENE_POLL_SECONDS)

    def wait_ready(self, capture):
        f = capture.fixture
        position = [f.x, f.y, f.z, f.map]
        statics_off = f.settings.get('world_rendering', {}).get('enable_statics') is False
        deadline = time.monotonic() + SCENE_TIMEOUT_SECONDS
        while True:
            try:
                if statics_off:
                    return self.wait_terrain(position)
                return self.control.wait_for_scene(self.socket, SCENE_WAIT_SECONDS, position)
            except (OSError, RuntimeError, TimeoutError) as error:
                if time.monotonic() > deadline:
                    raise HarnessError(f'{capture.key}: the scene never settled: {error}') from error
                time.sleep(SCENE_RETRY_SECONDS)

    def capture_png(self, path):
        if path.exists():
            path.unlink()
        self.request({'command': 'capture', 'path': str(path)})
        deadline = time.monotonic() + CAPTURE_TIMEOUT_SECONDS
        while not self.control.png_complete(path):
            if time.monotonic() > deadline:
                raise HarnessError(f'capture did not finish: {path}')
            time.sleep(CAPTURE_POLL_SECONDS)
        return path

    def capture_stable(self, capture):
        """Capture until two consecutive frames are equal; returns (path, attempts, stable, last difference).

        Files stay where they were written until the launch ends: the app clears a pending capture only once it sees
        the file, so moving one right away would block the next capture."""
        import numpy as np
        base = self.staging / capture.fixture.name
        base.mkdir(exist_ok=True)
        previous = self.capture_png(base / f'{capture.stem}.1.png')
        previous_pixels = png_rgb(previous)
        difference = None
        for attempt in range(2, STABLE_ATTEMPTS + 1):
            time.sleep(STABLE_INTERVAL_SECONDS)
            current = self.capture_png(base / f'{capture.stem}.{attempt}.png')
            pixels = png_rgb(current)
            if pixels.shape == previous_pixels.shape and np.array_equal(pixels, previous_pixels):
                return current, attempt, True, None
            difference = diff_stats(previous_pixels, pixels, 0) if pixels.shape == previous_pixels.shape else None
            previous, previous_pixels = current, pixels
        return previous, STABLE_ATTEMPTS, False, difference

    def verify(self, capture, status):
        problems = []
        render = status.get('render') or {}
        want = f'{capture.style}.{capture.time}'
        if render.get('preset') != want:
            problems.append(f'preset {render.get("preset")} instead of {want}')
        if (render.get('effects') or {}).get('enable_water_animation') not in (0, None):
            problems.append('water animation is running')
        if (render.get('fog_params') or [0] * 4)[3] != 0:
            problems.append('the fog cloud drift is running')
        if self.live:
            problems += self.verify_live(capture, status)
        size = (status.get('window') or {}).get('logical_size')
        if size and [round(v) for v in size] != list(self.ctx.window):
            problems.append(f'window {size} instead of {list(self.ctx.window)}')
        f = capture.fixture
        if status.get('position') != [f.x, f.y, f.z, f.map]:
            problems.append(f'position {status.get("position")}')
        return problems

    def verify_live(self, capture, status):
        """The switched state as the app reports it: the settings, the sources and the active style's settings."""
        want = live_state(self.ctx.files, capture)
        control = status.get('control') or {}
        settings = control.get('settings') or {}
        problems = [f'{key} {settings.get(key)!r} instead of {value!r}'
                    for key, value in want['settings'].items() if settings.get(key) != value]
        sources = (settings.get('art_texture_source'), settings.get('land_texture_source'))
        if sources != want['sources']:
            problems.append(f'sources {sources} instead of {want["sources"]}')
        render = status.get('render') or {}
        problems += [f'{key} {render.get(key)!r} instead of {value!r}'
                     for key, value in want['style'].items() if render.get(key) != value]
        if control.get('zoom') is None or abs(control['zoom'] - want['zoom']) > ZOOM_TOLERANCE:
            problems.append(f'zoom {control.get("zoom")} instead of {want["zoom"]}')
        return problems

    def switch(self, capture, state):
        """Bring the app from `state` to the capture's: sources, preset, settings, the style's settings, zoom and
        position, each through its control command. Returns whether anything changed."""
        want = live_state(self.ctx.files, capture)
        changed = False
        if want['sources'] != state['sources']:
            art, land = want['sources']
            changed |= self.request({'command': 'sources', 'art': art, 'land': land})['changed']
        if want['preset'] != state['preset']:
            style, time_of_day = want['preset']
            changed |= self.request({'command': 'preset', 'style': style, 'time': time_of_day})['changed']
        for key, value in want['settings'].items():
            if state['settings'].get(key) != value:
                changed |= self.request({'command': 'set', 'key': key, 'value': value})['changed']
        # A preset switch reloads the style's settings from the in-memory presets, where an earlier switch away
        # stored the then live values: set them every time.
        for key, value in want['style'].items():
            changed |= self.request({'command': 'set', 'key': key, 'value': value})['changed']
        if want['zoom'] != state['zoom']:
            changed |= self.request({'command': 'set', 'key': 'zoom', 'value': want['zoom']})['changed']
        if want['position'] != state['position']:
            x, y, z, map_id = want['position']
            self.request({'command': 'teleport', 'x': x, 'y': y, 'z': z, 'map': map_id})
            changed = True
        state.update(want)
        return changed

    # -- run -----------------------------------------------------------------------------------------------------
    def reload_land_check(self, capture, status):
        """Problems when the statics rebuilt from scratch after reload_land differ from the cached ones."""
        before = (status.get('art') or {}).get('digest')
        reply = self.request({'command': 'reload_land'})
        if not reply.get('ok'):
            return [f'reload_land: {reply.get("error")}'], None
        after = (self.wait_ready(capture).get('art') or {}).get('digest')
        check = {'before': before, 'after': after}
        if before != after:
            return [f'the statics changed after reload_land ({before} -> {after}): the cache kept land it no longer '
                    f'has'], check
        return [], check

    def run(self):
        """Capture as many of the launch's captures as fit the budget; returns (results, the ones left over)."""
        results = {}
        left = []
        timings = {}
        budget = self.ctx.chunk_seconds if self.live else LAUNCH_BUDGET_SECONDS
        with PreviewLock() as lock:
            timings['lock_wait'] = lock.waited
            self.start()
            try:
                first = self.captures[0]
                self.wait_ready(first)
                timings['ready'] = round(time.monotonic() - self.started, 2)
                if self.live and 'control' not in self.request({'command': 'status'}):
                    raise HarnessError(f'{self.id}: the binary has no switch commands; use --fresh-launch')
                # The first capture of a run may be black or stale.
                self.capture_png(self.staging / '0_discard.png')
                position = None
                # The startup settings are the first capture's (write_run_settings).
                state = live_state(self.ctx.files, first)
                switches = 0
                for index, capture in enumerate(self.captures):
                    if time.monotonic() - self.started > budget:
                        left = self.captures[index:]
                        break
                    began = time.monotonic()
                    f = capture.fixture
                    target = [f.x, f.y, f.z, f.map]
                    switched = False
                    if self.live:
                        switched = self.switch(capture, state)
                    elif position is not None and position != target:
                        self.request({'command': 'teleport', 'x': f.x, 'y': f.y, 'z': f.z, 'map': f.map})
                    position = target
                    self.wait_ready(capture)
                    time.sleep(SETTLE_SECONDS)
                    status = self.wait_ready(capture)
                    if switched:
                        # As after a launch: the first frame after a switch may be stale.
                        switches += 1
                        self.capture_png(self.staging / f'0_discard.{switches}.png')
                    path, attempts, stable, difference = self.capture_stable(capture)
                    problems = self.verify(capture, self.request({'command': 'status'}))
                    reload_check = None
                    if self.ctx.reload_land_check:
                        reload_problems, reload_check = self.reload_land_check(capture, status)
                        problems += reload_problems
                    results[capture.key] = {
                        'png': path, 'attempts': attempts, 'stable': stable, 'unstable_difference': difference,
                        'problems': problems, 'seconds': round(time.monotonic() - began, 2),
                        'reload_land': reload_check,
                        'status': {key: status.get(key) for key in ('position', 'art', 'z_view', 'static_lights',
                                                                     'client_light_sprites')}
                        | {'render': {key: (status.get('render') or {}).get(key) for key in (
                            'preset', 'light_cover', 'light_cover_rule', 'player_floor_rule',
                            'land_texture_variation', 'local_lights')},
                           'window': status.get('window')}}
                    flag = '' if stable else f'  UNSTABLE after {attempts} frames'
                    flag += ''.join(f'  PROBLEM: {problem}' for problem in problems)
                    log(f'  {capture.key}  {results[capture.key]["seconds"]}s{flag}')
            finally:
                self.stop()
        timings['total'] = round(time.monotonic() - self.started, 2)
        timings['captures'] = len(results)
        errors = self.log_errors()
        self.record.update(timings=timings, log_errors=errors[:LOG_TAIL_LINES],
                           placement_verified='placement_verified' in self.preview_text())
        if not self.record['placement_verified']:
            raise HarnessError(f'{self.id}: the preview helper did not verify the window placement:\n'
                               f'{self.preview_text()[-2000:]}')
        return results, left


# ----------------------------------------------------------------------------------------------------------------
# Storage
# ----------------------------------------------------------------------------------------------------------------

def encode(source, destination, image_format):
    """Store one capture in the label's format (lossless unless jpeg). Runs in a worker process."""
    from PIL import Image
    try:
        os.nice(ENCODE_NICE)
    except OSError:
        pass
    destination.parent.mkdir(parents=True, exist_ok=True)
    with Image.open(source) as image:
        rgb = image.convert('RGB')
        if image_format == 'webp':
            rgb.save(destination, 'WEBP', lossless=True, method=WEBP_LOSSLESS_METHOD, quality=WEBP_LOSSLESS_QUALITY)
        elif image_format == 'png':
            rgb.save(destination, 'PNG', optimize=True)
        else:
            rgb.save(destination, 'JPEG', quality=JPEG_QUALITY, subsampling=0)
    return destination


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def git_state(tree):
    def git(*args):
        result = subprocess.run(['git', '-C', str(tree), *args], capture_output=True, text=True)
        return result.stdout.strip() if result.returncode == 0 else None
    status = git('status', '--porcelain', '--', 'dynamapper/assets')
    return {'path': str(tree), 'commit': git('rev-parse', 'HEAD'), 'describe': git('log', '-1', '--format=%h %s'),
            'branch': git('rev-parse', '--abbrev-ref', 'HEAD'), 'assets_dirty': bool(status)}


def label_dir(root, label):
    if not NAME_PATTERN.match(label.replace('_', '-').replace('.', '-')):
        raise HarnessError(f'label "{label}": use letters, digits, "-", "_" and "."')
    return root / 'labels' / label


def load_manifest(root, label):
    path = label_dir(root, label) / 'manifest.json'
    if not path.is_file():
        raise HarnessError(f'no label "{label}" under {root / "labels"}')
    return json.loads(path.read_text())


class Context:
    def __init__(self, args, label):
        self.args = args
        self.label = label
        self.root = args.root
        self.window = args.window
        self.packages = args.packages
        self.binary = args.binary.resolve() if args.binary else None
        self.tree = args.tree.resolve()
        self.files = SettingsFiles(self.tree / 'dynamapper/assets')
        self.dir = label_dir(self.root, label)
        self.work = self.root / 'work' / label
        self.sockets = self.root / 'sk'
        self.control = load_control()
        self.chunk_seconds = getattr(args, 'chunk_seconds', LIVE_CHUNK_SECONDS)
        self.reload_land_check = getattr(args, 'reload_land_check', False)


def record(args, label, captures, fixtures_path):
    """Capture `captures` into a new label. Returns the manifest."""
    if not captures:
        raise HarnessError('the selection has no captures')
    if args.binary is None or not args.binary.is_file():
        raise HarnessError('--binary must name the dynamapper binary to capture with')
    if 'DISPLAY' not in os.environ:
        raise HarnessError('no DISPLAY: the preview needs the X11/XWayland display')
    ctx = Context(args, label)
    live = not args.fresh_launch and supports_switching(ctx.binary)
    if not args.fresh_launch and not live:
        log('note: the binary has no switch commands; one launch per startup settings set (--fresh-launch)')
    if ctx.dir.exists():
        if not args.replace:
            raise HarnessError(f'label "{label}" exists; pass --replace or pick another label')
        shutil.rmtree(ctx.dir)
    ctx.sockets.mkdir(parents=True, exist_ok=True)
    if len(str(ctx.sockets)) + CONTROL_REPLY_SUFFIX > SUN_PATH_MAX:
        raise HarnessError(f'--root is too long for Unix socket paths: {ctx.sockets}')
    # control.py binds its reply sockets under tempfile's folder: keep that path short and off the RAM disk.
    tempfile.tempdir = str(ctx.sockets)
    started = time.monotonic()
    manifest = {
        'label': label, 'harness_version': HARNESS_VERSION,
        'created': datetime.datetime.now().astimezone().isoformat(timespec='seconds'),
        'binary': {'path': str(ctx.binary), 'sha256': file_sha256(ctx.binary), 'size': ctx.binary.stat().st_size,
                   'mtime': datetime.datetime.fromtimestamp(ctx.binary.stat().st_mtime).isoformat(timespec='seconds')},
        'tree': git_state(ctx.tree), 'packages': str(ctx.packages), 'note': args.note,
        'fixtures_file': {'path': str(fixtures_path), 'sha256': file_sha256(fixtures_path)},
        'window': list(ctx.window), 'format': args.format, 'mode': 'live' if live else 'fresh',
        'chunk_seconds': ctx.chunk_seconds if live else None, 'reload_land_check': ctx.reload_land_check,
        'selection': {'tier': args.tier, 'only': args.only, 'style': args.style, 'source': args.source,
                      'time': args.time},
        'frozen': ['enable_water_animation = 0 in every preset slot', 'fog_params.w = 0 in every preset slot'],
        'launches': [], 'captures': {}, 'errors': [],
    }
    ctx.dir.mkdir(parents=True)
    pending = [(group, live and live_capable(group[0].fixture)) for group in plan_launches(captures, live)]
    log(f'{label}: {len(captures)} captures, {"switching at runtime" if live else "one launch per settings set"}; '
        f'binary {ctx.binary}')
    counter = 0
    encoders = concurrent.futures.ProcessPoolExecutor(max_workers=ENCODE_WORKERS)
    futures = []
    try:
        while pending:
            group, switching = pending.pop(0)
            counter += 1
            first = group[0]
            launch_id = f'l{counter:03d}-{first.style}-{first.time}-{first.art}-{first.land}-m{first.fixture.map}'
            log(f'[{counter}] {launch_id}: {len(group)} captures' + (' (switching)' if switching else ''))
            try:
                launch = Launch(ctx, launch_id, group, switching)
                results, left = launch.run()
            except (HarnessError, OSError, RuntimeError, TimeoutError) as error:
                # One failed launch (a control error, a crashed app) loses its captures, not the whole label.
                log(f'  FAILED: {error}')
                manifest['errors'].append(f'{launch_id}: {error}')
                continue
            if left:
                pending.insert(0, (left, switching))
            manifest['launches'].append(launch.record)
            if launch.record['log_errors']:
                log(f'  {len(launch.record["log_errors"])} error lines in the app log, first: '
                    f'{launch.record["log_errors"][0]}')
            for capture in group:
                result = results.get(capture.key)
                if result is None:
                    continue
                destination = ctx.dir / 'caps' / capture.fixture.name / f'{capture.stem}{FORMATS[args.format]}'
                futures.append((encoders.submit(encode, result['png'], destination, args.format), launch))
                manifest['captures'][capture.key] = {
                    'file': str(destination.relative_to(ctx.dir)), 'fixture': capture.fixture.name,
                    'view_hash': capture.fixture.view_hash(), 'style': capture.style, 'time': capture.time,
                    'source': capture.source, 'launch': launch_id,
                    **{key: value for key, value in result.items() if key != 'png'}}
            # A launch's staging folder goes once its captures are stored.
            for future, owner in [item for item in futures if item[1] is launch]:
                future.result()
            if not args.keep_work:
                shutil.rmtree(launch.dir, ignore_errors=True)
    finally:
        encoders.shutdown(wait=True)
        if not args.keep_work:
            shutil.rmtree(ctx.work, ignore_errors=True)
    missing = [c.key for c in captures if c.key not in manifest['captures']]
    if missing:
        manifest['errors'].append(f'{len(missing)} captures missing: {missing[:10]}')
    manifest['wall_seconds'] = round(time.monotonic() - started, 1)
    manifest['launch_count'] = len(manifest['launches'])
    manifest['bytes'] = sum(p.stat().st_size for p in (ctx.dir / 'caps').rglob('*') if p.is_file()) \
        if (ctx.dir / 'caps').exists() else 0
    (ctx.dir / 'manifest.json').write_text(json.dumps(manifest, indent=1, default=str))
    unstable = [key for key, value in manifest['captures'].items() if not value['stable']]
    for key, value in manifest['captures'].items():
        for problem in value.get('problems', []):
            log(f'PROBLEM: {key}: {problem}')
    log(f'{label}: {len(manifest["captures"])} captures in {manifest["launch_count"]} launches, '
        f'{manifest["bytes"] / 1e6:.1f} MB, {manifest["wall_seconds"]} s; {len(unstable)} unstable, '
        f'{len(manifest["errors"])} errors')
    return manifest


# ----------------------------------------------------------------------------------------------------------------
# Comparison
# ----------------------------------------------------------------------------------------------------------------

def diff_stats(a, b, pixel_threshold):
    """Largest channel difference per pixel, 0..255; a pixel changed when it is above pixel_threshold."""
    import numpy as np
    d = np.abs(a.astype(np.int16) - b.astype(np.int16)).max(axis=2)
    changed = d > pixel_threshold
    return {'changed_pct': float(changed.mean() * 100.0), 'changed_pixels': int(changed.sum()),
            'any_pct': float((d > 0).mean() * 100.0), 'max_delta': int(d.max()), 'mean_delta': float(d.mean())}


def verdict(stats, tolerance):
    if stats['max_delta'] == 0:
        return 'identical'
    return 'within' if stats['changed_pct'] <= tolerance.changed_pct else 'changed'


def compare(args, baseline, label, fixtures, fixtures_path):
    root = args.root
    base = load_manifest(root, baseline)
    by_name = {fixture.name: fixture for fixture in fixtures}
    selected = {key: entry for key, entry in base['captures'].items()
                if entry['fixture'] in by_name and capture_matches(by_name[entry['fixture']], entry, args)}
    gone = sorted({entry['fixture'] for entry in base['captures'].values() if entry['fixture'] not in by_name})
    current_dir = label_dir(root, label)
    if not (current_dir / 'manifest.json').is_file():
        wanted = []
        for key, entry in selected.items():
            style, time_of_day = entry['style'], entry['time']
            art, land = entry['source'].split('/')
            wanted.append(Capture(by_name[entry['fixture']], style, time_of_day, art, land))
        args.format = base['format']
        if tuple(base['window']) != tuple(args.window):
            log(f'using the baseline window {base["window"]}')
            args.window = tuple(base['window'])
        record(args, label, wanted, fixtures_path)
    current = load_manifest(root, label)
    if tuple(current['window']) != tuple(base['window']):
        raise HarnessError(f'window {current["window"]} differs from the baseline\'s {base["window"]}')
    out = root / 'compares' / f'{baseline}--{label}'
    if out.exists():
        shutil.rmtree(out)
    (out / 'sheets').mkdir(parents=True)
    rows = {}
    errors = list(current['errors'])
    if gone:
        errors.append(f'baseline fixtures no longer in the fixtures file: {gone}')
    if base['format'] not in LOSSLESS_FORMATS or current['format'] not in LOSSLESS_FORMATS:
        log('note: a jpeg label; equal frames stay equal, but a change also spreads over its 8x8 blocks')
    same_binary = base['binary']['sha256'] == current['binary']['sha256']
    for key, entry in sorted(selected.items()):
        fixture = by_name[entry['fixture']]
        other = current['captures'].get(key)
        row = {'fixture': fixture.name, 'style': entry['style'], 'time': entry['time'], 'source': entry['source'],
               'tolerance': vars(fixture.tolerance)}
        if other is None:
            row['verdict'] = 'missing'
            errors.append(f'{key}: missing from {label}')
        elif entry['view_hash'] != fixture.view_hash() or other['view_hash'] != fixture.view_hash():
            row['verdict'] = 'stale'
            errors.append(f'{key}: the fixture\'s view changed since one of the captures; record a new baseline')
        else:
            a = image_rgb(label_dir(root, baseline) / entry['file'])
            b = image_rgb(current_dir / other['file'])
            if a.shape != b.shape:
                row['verdict'] = 'changed'
                row['stats'] = {'shape': [list(a.shape), list(b.shape)]}
            else:
                row['stats'] = diff_stats(a, b, fixture.tolerance.pixel_threshold)
                row['verdict'] = verdict(row['stats'], fixture.tolerance)
            row['unstable'] = not (entry['stable'] and other['stable'])
            row['problems'] = other.get('problems', [])
            # One binary collects the same statics however it got to the view (live or --fresh-launch).
            digests = [((capture.get('status') or {}).get('art') or {}).get('digest') for capture in (entry, other)]
            if same_binary and None not in digests and digests[0] != digests[1]:
                row['verdict'] = 'changed'
                row['statics_digest'] = digests
            if any(problem.startswith('the statics changed after reload_land') for problem in row['problems']):
                row['verdict'] = 'changed'
            row['files'] = [str(label_dir(root, baseline) / entry['file']), str(current_dir / other['file'])]
        rows[key] = row
    counts = {name: sum(1 for row in rows.values() if row['verdict'] == name)
              for name in VERDICTS + ('missing', 'stale')}
    summary = {'baseline': baseline, 'label': label, 'created': datetime.datetime.now().isoformat(timespec='seconds'),
               'baseline_binary': base['binary'], 'label_binary': current['binary'],
               'baseline_tree': base['tree'], 'label_tree': current['tree'], 'counts': counts, 'errors': errors,
               'captures': rows}
    sheets = compare_sheets(rows, by_name, out / 'sheets', args.sheets)
    summary['sheets'] = [str(path) for path in sheets]
    (out / 'summary.json').write_text(json.dumps(summary, indent=1))
    print_compare(rows)
    log(f'{counts}; summary {out / "summary.json"}; {len(sheets)} sheets in {out / "sheets"}')
    for error in errors:
        log(f'ERROR: {error}')
    if errors:
        return 2
    return 1 if counts['changed'] else 0


def capture_matches(fixture, entry, args):
    """Whether a recorded capture is in the selection. Without --tier, compare and sheet take every recorded one."""
    if args.tier == 'quick' and fixture.tier != 'quick':
        return False
    if args.only and not any(fnmatch.fnmatchcase(name, pattern) for pattern in args.only
                             for name in (fixture.name,) + fixture.tags):
        return False
    return ((not args.style or entry['style'] in args.style) and (not args.time or entry['time'] in args.time)
            and (not args.source or entry['source'] in args.source))


def image_rgb(path):
    import numpy as np
    from PIL import Image
    with Image.open(path) as image:
        return np.asarray(image.convert('RGB'))


def print_compare(rows):
    width = max((len(key) for key in rows), default=10)
    for key, row in rows.items():
        stats = row.get('stats', {})
        detail = (f'{stats.get("changed_pct", 0):7.3f}% > {row["tolerance"]["pixel_threshold"]:<3d} '
                  f'max {stats.get("max_delta", 0):3d}  (allowed {row["tolerance"]["changed_pct"]}%)'
                  if 'changed_pct' in stats else '')
        mark = {'identical': ' ', 'within': '~', 'changed': '!'}.get(row['verdict'], 'E')
        statics = f'  statics {row["statics_digest"][0]} -> {row["statics_digest"][1]}' \
            if row.get('statics_digest') else ''
        problems = ''.join(f'  PROBLEM: {problem}' for problem in row.get('problems', []))
        log(f'{mark} {key:<{width}}  {row["verdict"]:<9} {detail}{"  unstable" if row.get("unstable") else ""}'
            f'{statics}{problems}')


# ----------------------------------------------------------------------------------------------------------------
# Sheets
# ----------------------------------------------------------------------------------------------------------------

def font():
    from PIL import ImageFont
    try:
        return ImageFont.truetype(FONT_PATH, FONT_SIZE)
    except OSError:
        return ImageFont.load_default()


def panel(image, lines, size, warn=False):
    """A thumbnail of `size` with up to two label lines above it."""
    from PIL import Image, ImageDraw
    out = Image.new('RGB', (size[0], size[1] + SHEET_LABEL_HEIGHT), SHEET_BACKGROUND)
    if image is not None:
        thumb = image.copy()
        thumb.thumbnail(size, Image.LANCZOS)
        out.paste(thumb, ((size[0] - thumb.size[0]) // 2, SHEET_LABEL_HEIGHT + (size[1] - thumb.size[1]) // 2))
    draw = ImageDraw.Draw(out)
    face = font()
    for index, line in enumerate(lines[:2]):
        draw.text((4, 2 + index * (FONT_SIZE + 3)), line, fill=SHEET_WARN if warn else SHEET_TEXT, font=face)
    return out


def reference_image(fixture):
    from PIL import Image
    reference = fixture.reference
    if not reference or not Path(reference['path']).is_file():
        return None
    with Image.open(reference['path']) as image:
        rgb = image.convert('RGB')
    if reference.get('crop'):
        x, y, w, h = reference['crop']
        rgb = rgb.crop((x, y, x + w, y + h))
    return rgb


def grid(panels_by_row, path):
    from PIL import Image
    columns = max(len(row) for row in panels_by_row)
    cell = panels_by_row[0][0].size
    sheet = Image.new('RGB', (columns * (cell[0] + SHEET_GAP), len(panels_by_row) * (cell[1] + SHEET_GAP)),
                      SHEET_BACKGROUND)
    for r, row in enumerate(panels_by_row):
        for c, item in enumerate(row):
            if item is not None:
                sheet.paste(item, (c * (cell[0] + SHEET_GAP), r * (cell[1] + SHEET_GAP)))
    sheet.save(path, 'JPEG', quality=SHEET_JPEG_QUALITY)
    return path


def compare_sheets(rows, by_name, out, which):
    """One JPEG per fixture: a row per capture with baseline | current | |diff| x DIFF_GAIN, and the reference."""
    import numpy as np
    from PIL import Image
    by_fixture = {}
    for key, row in rows.items():
        if 'files' not in row or (which == 'changed' and row['verdict'] != 'changed'):
            continue
        by_fixture.setdefault(row['fixture'], []).append((key, row))
    written = []
    for name, items in by_fixture.items():
        fixture = by_name[name]
        reference = reference_image(fixture)
        panels = []
        for index, (key, row) in enumerate(items):
            a = Image.open(row['files'][0]).convert('RGB')
            b = Image.open(row['files'][1]).convert('RGB')
            size = (a.size[0] // SHEET_SCALE, a.size[1] // SHEET_SCALE)
            stats = row.get('stats', {})
            title = f'{row["style"]} {row["time"]} {row["source"]}'
            line = []
            line.append(panel(a, [f'{name}', f'baseline  {title}'], size))
            line.append(panel(b, [f'{row["verdict"]}', f'current  {title}'], size, row['verdict'] == 'changed'))
            if a.size == b.size:
                d = np.abs(np.asarray(a, np.int16) - np.asarray(b, np.int16)).max(axis=2)
                heat = Image.fromarray(np.clip(d * DIFF_GAIN, 0, 255).astype(np.uint8)).convert('RGB')
                line.append(panel(heat, [f'|diff| x{DIFF_GAIN}: {stats["changed_pct"]:.3f}% > '
                                         f'{row["tolerance"]["pixel_threshold"]}, max {stats["max_delta"]}',
                                         f'allowed {row["tolerance"]["changed_pct"]}%'], size,
                                  row['verdict'] == 'changed'))
            else:
                line.append(panel(None, ['size differs'], size, True))
            if reference is not None:
                line.append(panel(reference, ['client reference', fixture.reference['note']], size)
                            if index == 0 else None)
            panels.append(line)
        written.append(grid(panels, out / f'{name}.jpg'))
    return written


def source_rank(fixture, style, source):
    listed = [f'{art}/{land}' for art, land in fixture.sources.get(style, ())]
    return (listed.index(source) if source in listed else len(listed), source)


def label_sheets(args, label, fixtures):
    """One JPEG per fixture of a label: rows time x source, columns the styles, and the client reference."""
    from PIL import Image
    root = args.root
    manifest = load_manifest(root, label)
    by_name = {fixture.name: fixture for fixture in fixtures}
    out = root / 'sheets' / label
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    by_fixture = {}
    for key, entry in manifest['captures'].items():
        if entry['fixture'] in by_name and capture_matches(by_name[entry['fixture']], entry, args):
            by_fixture.setdefault(entry['fixture'], []).append(entry)
    written = []
    for name, entries in sorted(by_fixture.items()):
        fixture = by_name[name]
        styles = [s for s in STYLES if any(e['style'] == s for e in entries)]
        # A row is a time of day and the n-th source of each style: the styles run different sources by default.
        sources = {style: sorted({e['source'] for e in entries if e['style'] == style},
                                 key=lambda source: source_rank(fixture, style, source)) for style in styles}
        times = [t for t in TIMES if any(e['time'] == t for e in entries)]
        rows = [(t, n) for t in times for n in range(max(len(v) for v in sources.values()))]
        reference = reference_image(fixture)
        size = None
        found = []
        for time_of_day, n in rows:
            line = []
            for style in styles:
                source = sources[style][n] if n < len(sources[style]) else None
                entry = next((e for e in entries if (e['time'], e['source'], e['style']) == (time_of_day, source,
                                                                                          style)), None)
                line.append(entry)
            found.append(line)
        panels = []
        for r, line in enumerate(found):
            row = []
            for c, entry in enumerate(line):
                if entry is None:
                    row.append(None)
                    continue
                image = Image.open(label_dir(root, label) / entry['file']).convert('RGB')
                size = size or (image.size[0] // SHEET_SCALE, image.size[1] // SHEET_SCALE)
                title = name if r == 0 and c == 0 else ''
                row.append(panel(image, [title, f'{entry["style"]} {entry["time"]} {entry["source"]}'
                                         + ('' if entry['stable'] else '  UNSTABLE')], size, not entry['stable']))
            if reference is not None and size is not None:
                row.append(panel(reference, ['client reference', fixture.reference['note']], size) if r == 0
                           else None)
            panels.append(row)
        if size is not None:
            # grid() takes the cell size from the first cell.
            if panels[0][0] is None:
                panels[0][0] = panel(None, [name], size)
            written.append(grid(panels, out / f'{name}.jpg'))
    log(f'{len(written)} sheets in {out}')
    return written


# ----------------------------------------------------------------------------------------------------------------
# Commands
# ----------------------------------------------------------------------------------------------------------------

def command_list(args, fixtures):
    captures = expand(fixtures, args)
    counts = {}
    for capture in captures:
        counts[capture.fixture.name] = counts.get(capture.fixture.name, 0) + 1
    for fixture in fixtures:
        if not fixture_selected(fixture, args):
            continue
        log(f'{fixture.name}  [{fixture.tier}]  map {fixture.map} {fixture.x},{fixture.y},{fixture.z}  '
            f'zoom {fixture.zoom}  times {",".join(fixture.times)}  captures {counts.get(fixture.name, 0)}  '
            f'tolerance {fixture.tolerance.pixel_threshold}/{fixture.tolerance.changed_pct}%'
            + (f'  settings {json.dumps(fixture.settings)}' if fixture.settings else '')
            + (f'  tags {",".join(fixture.tags)}' if fixture.tags else ''))
        if args.verbose:
            log(f'    guards: {fixture.guards}')
            if fixture.reference:
                log(f'    reference: {fixture.reference["path"]} ({fixture.reference["note"]})')
    fresh = plan_launches(captures)
    switching = [group for group in plan_launches(captures, live=True) if live_capable(group[0].fixture)]
    log(f'{len(captures)} captures (tier {args.tier}): {len(fresh)} launches with --fresh-launch; switching at '
        f'runtime, {sum(map(len, switching))} captures in launches of up to {LIVE_CHUNK_SECONDS} s and '
        f'{len(plan_launches(captures, live=True)) - len(switching)} launches of their own')
    return 0


def command_clean(args):
    labels = sorted((path for path in (args.root / 'labels').glob('*') if (path / 'manifest.json').is_file()),
                    key=lambda path: json.loads((path / 'manifest.json').read_text())['created'])
    if args.labels:
        doomed = [args.root / 'labels' / name for name in args.labels]
        unknown = [path.name for path in doomed if path not in labels]
        if unknown:
            raise HarnessError(f'no such labels: {unknown}')
    elif args.keep is not None:
        doomed = labels[:max(len(labels) - args.keep, 0)]
    else:
        raise HarnessError('clean needs label names or --keep N')
    names = {path.name for path in doomed}
    compares = [path for path in (args.root / 'compares').glob('*--*')
                if set(path.name.split('--', 1)) & names]
    sheets = [args.root / 'sheets' / name for name in names if (args.root / 'sheets' / name).exists()]
    for path in doomed + compares + sheets:
        log(('would delete ' if args.dry_run else 'delete ') + str(path))
        if not args.dry_run:
            shutil.rmtree(path)
    return 0


def parse_window(text):
    match = re.fullmatch(r'(\d+)x(\d+)', text)
    if not match:
        raise argparse.ArgumentTypeError('WIDTHxHEIGHT')
    size = (int(match.group(1)), int(match.group(2)))
    if not (0 < size[0] <= MAX_WINDOW_WIDTH and 0 < size[1] <= MAX_WINDOW_HEIGHT):
        raise argparse.ArgumentTypeError(f'at most {MAX_WINDOW_WIDTH}x{MAX_WINDOW_HEIGHT}: a larger preview comes '
                                         'up maximized')
    return size


def parser():
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument('--fixtures', type=Path, default=FIXTURES, help='fixtures file (default: the skill\'s)')
    common.add_argument('--root', type=Path, default=DEFAULT_ROOT, help=f'labels and results (default {DEFAULT_ROOT})')
    common.add_argument('--tree', type=Path, default=REPO,
                        help='the checkout whose dynamapper/assets the runs copy (default: this repository)')
    common.add_argument('--tier', choices=TIERS,
                        help='quick or full; list and record default to quick, '
                             'compare and sheet to every recorded capture')
    common.add_argument('--only', action='append', metavar='NAME_OR_TAG', help='fixture name or tag glob')
    common.add_argument('--style', action='append', choices=STYLES)
    common.add_argument('--source', action='append', metavar='ART/LAND')
    common.add_argument('--time', action='append', choices=TIMES)
    capture = argparse.ArgumentParser(add_help=False)
    capture.add_argument('--binary', type=Path, help='the dynamapper binary to capture with')
    capture.add_argument('--packages', type=Path, default=DEFAULT_PACKAGES)
    capture.add_argument('--window', type=parse_window, default=DEFAULT_WINDOW, help='WIDTHxHEIGHT, default 1280x800')
    capture.add_argument('--format', choices=tuple(FORMATS), default='webp',
                         help='storage: webp (lossless, default), png (lossless) or jpeg (quality 95, lean)')
    capture.add_argument('--note', default='', help='free text stored in the manifest')
    capture.add_argument('--replace', action='store_true', help='overwrite an existing label')
    capture.add_argument('--keep-work', action='store_true', help='keep the run folders (settings, logs)')
    capture.add_argument('--fresh-launch', action='store_true',
                         help='one launch per startup settings set instead of switching them at runtime')
    capture.add_argument('--reload-land-check', action='store_true',
                         help='after each capture, drop the loaded land and the cached statics (reload_land) and '
                              'require the same statics digest after the rebuild')
    capture.add_argument('--chunk-seconds', type=float, default=LIVE_CHUNK_SECONDS,
                         help=f'how long a switching launch holds the preview lock before the next launch goes on '
                              f'(default {LIVE_CHUNK_SECONDS})')
    top = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = top.add_subparsers(dest='command', required=True)
    listing = commands.add_parser('list', parents=[common], help='fixtures and captures of the selection')
    listing.add_argument('-v', '--verbose', action='store_true')
    rec = commands.add_parser('record', parents=[common, capture], help='capture a label')
    rec.add_argument('label')
    cmp_ = commands.add_parser('compare', parents=[common, capture], help='capture or reuse LABEL, diff it')
    cmp_.add_argument('baseline')
    cmp_.add_argument('label')
    cmp_.add_argument('--sheets', choices=('changed', 'all'), default='changed')
    sheet = commands.add_parser('sheet', parents=[common], help='contact sheets of a label')
    sheet.add_argument('label')
    clean = commands.add_parser('clean', parents=[common], help='delete labels with their compares and sheets')
    clean.add_argument('labels', nargs='*')
    clean.add_argument('--keep', type=int, help='keep the N most recent labels')
    clean.add_argument('--dry-run', action='store_true')
    commands.add_parser('self-test', parents=[common], help='offline checks')
    return top


def main(argv=None):
    args = parser().parse_args(argv)
    if args.command == 'self-test':
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SelfTest)
        SelfTest.args = args
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    for source in args.source or ():
        parts = source.split('/')
        if len(parts) != 2 or parts[0] not in ART_SOURCES or parts[1] not in LAND_SOURCES:
            raise HarnessError(f'--source {source}: art/land with art {"|".join(ART_SOURCES)} and land '
                               f'{"|".join(LAND_SOURCES)}')
    args.root = args.root.resolve()
    if args.command in ('list', 'record') and args.tier is None:
        args.tier = 'quick'
    if args.command == 'clean':
        return command_clean(args)
    files = SettingsFiles(args.tree / 'dynamapper/assets')
    fixtures = load_fixtures(args.fixtures, files)
    if args.command == 'list':
        return command_list(args, fixtures)
    if args.command == 'sheet':
        label_sheets(args, args.label, fixtures)
        return 0
    if args.command == 'record':
        manifest = record(args, args.label, expand(fixtures, args), args.fixtures.resolve())
        return 2 if manifest['errors'] else 0
    return compare(args, args.baseline, args.label, fixtures, args.fixtures.resolve())


# ----------------------------------------------------------------------------------------------------------------
# Self-test
# ----------------------------------------------------------------------------------------------------------------

SAMPLE = '''
[defaults]
times = ["afternoon"]
styles = ["classic", "enhanced", "kr", "painterly"]
zoom = 1.0
camera = "orthographic"
tolerance = { pixel_threshold = 4, changed_pct = 0.01 }
[defaults.sources]
classic = ["cc/cc", "ec/ec"]
enhanced = ["ec/ec"]
kr = ["ec/kr"]
painterly = ["ec/ec"]
[defaults.quick_sources]
classic = ["cc/cc"]
enhanced = ["ec/ec"]
kr = ["ec/kr"]
painterly = ["ec/ec"]

[[fixture]]
name = "one-view"
guards = "a test"
tier = "quick"
map = 1
x = 10
y = 20
z = 5
times = ["afternoon", "night"]
settings = { world_rendering = { show_only_player_floor = true } }

[[fixture]]
name = "two-view"
guards = "another"
tier = "full"
map = 0
x = 30
y = 40
z = 0
sources = ["ec/cc"]
'''


class SelfTest(unittest.TestCase):
    args = None

    def write(self, text):
        directory = Path(tempfile.mkdtemp(dir=self.scratch))
        path = directory / 'fixtures.toml'
        path.write_text(text)
        return path

    @classmethod
    def setUpClass(cls):
        root = cls.args.root if cls.args else DEFAULT_ROOT
        root.mkdir(parents=True, exist_ok=True)
        cls.scratch = Path(tempfile.mkdtemp(prefix='self-test-', dir=root))
        cls.files = SettingsFiles((cls.args.tree if cls.args else REPO) / 'dynamapper/assets')

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.scratch, ignore_errors=True)

    def selection(self, **kw):
        return argparse.Namespace(**({'tier': 'full', 'only': None, 'style': None, 'source': None, 'time': None}
                                     | kw))

    def test_repository_fixtures_read(self):
        fixtures = load_fixtures(FIXTURES, self.files)
        self.assertTrue(any(f.tier == 'quick' for f in fixtures))
        for fixture in fixtures:
            self.assertTrue(fixture.guards)
            if fixture.reference:
                self.assertTrue(Path(fixture.reference['path']).is_absolute())

    def test_sample_expands_and_groups(self):
        fixtures = load_fixtures(self.write(SAMPLE), self.files)
        quick = expand(fixtures, self.selection(tier='quick'))
        self.assertEqual(len(quick), 4 * 2)
        full = expand(fixtures, self.selection())
        # one-view: (2 + 1 + 1 + 1) sources x 2 times; two-view: 4 styles x its own source.
        self.assertEqual(len(full), 5 * 2 + 4)
        self.assertEqual({c.source for c in full if c.fixture.name == 'two-view'}, {'ec/cc'})
        launches = plan_launches(full)
        self.assertEqual(sum(len(group) for group in launches), len(full))
        for group in launches:
            self.assertEqual(len({c.group() for c in group}), 1)
        only = expand(fixtures, self.selection(only=['two-*'], style=['kr']))
        self.assertEqual([c.key for c in only], ['two-view/kr.afternoon.ec-cc'])

    def test_switching_launches(self):
        text = SAMPLE + """
[[fixture]]
name = "three-view"
guards = "a preset override"
tier = "full"
map = 1
x = 50
y = 60
z = 0
settings = { preset = { "effects.light_decal_intensity" = 0.5 } }
"""
        fixtures = load_fixtures(self.write(text), self.files)
        self.assertEqual([live_capable(f) for f in fixtures], [True, True, False])
        full = expand(fixtures, self.selection())
        launches = plan_launches(full, live=True)
        switching = [group for group in launches if live_capable(group[0].fixture)]
        # One sequence per map: the app loads only the map it starts on.
        self.assertEqual([{c.fixture.map for c in group} for group in switching], [{0}, {1}])
        self.assertEqual(sum(map(len, launches)), len(full))
        orders = [c.live_order() for group in switching for c in group]
        self.assertEqual(orders, sorted(orders))
        for group in launches[len(switching):]:
            self.assertEqual({c.fixture.name for c in group}, {'three-view'})
        kr = live_state(self.files, Capture(fixtures[0], 'kr', 'night', 'ec', 'kr'))
        self.assertTrue(kr['settings']['show_only_player_floor'])
        self.assertEqual(set(kr['style']), {'light_cover', 'player_floor_rule', 'land_texture_variation'})
        classic = live_state(self.files, Capture(fixtures[1], 'classic', 'afternoon', 'cc', 'cc'))
        self.assertFalse(classic['settings']['show_only_player_floor'])
        self.assertNotIn('player_floor_rule', classic['style'])
        self.assertEqual((classic['preset'], classic['sources'], classic['position']),
                         (('classic', 'afternoon'), ('cc', 'cc'), [30, 40, 0, 0]))
        binary = Path(tempfile.mkdtemp(dir=self.scratch)) / 'dynamapper'
        binary.write_bytes(b'x' * ((1 << 22) - 3) + SWITCH_MARKER)
        self.assertTrue(supports_switching(binary))
        binary.write_bytes(b'x' * 64)
        self.assertFalse(supports_switching(binary))

    def assert_rejected(self, text, *words):
        with self.assertRaises(HarnessError) as caught:
            load_fixtures(self.write(text), self.files)
        for word in words:
            self.assertIn(word, str(caught.exception))
        return str(caught.exception)

    def test_strict_errors_name_line_key_and_allowed(self):
        message = self.assert_rejected(SAMPLE.replace('tier = "full"', 'tier = "full"\ncolour = 1'),
                                       'unknown key "colour"', 'allowed:', 'two-view')
        self.assertRegex(message, r'fixtures\.toml:\d+:')
        self.assert_rejected(SAMPLE.replace('guards = "another"\n', ''), 'missing required key "guards"')
        self.assert_rejected(SAMPLE.replace('sources = ["ec/cc"]', 'sources = ["ec/xx"]'), 'art/land')
        self.assert_rejected(SAMPLE.replace('z = 0\n', 'z = 300\n'), 'out of range')
        self.assert_rejected(SAMPLE.replace('x = 30', 'x = true'), '"x" must be int')
        self.assert_rejected(SAMPLE.replace('name = "two-view"', 'name = "one-view"'), 'duplicate')
        self.assert_rejected(SAMPLE.replace('show_only_player_floor = true', 'show_only_floor = true'),
                             'unknown key "show_only_floor"', 'show_only_player_floor')
        self.assert_rejected(SAMPLE.replace('show_only_player_floor = true', 'show_only_player_floor = 1'),
                             'type')
        self.assert_rejected(SAMPLE.replace('[[fixture]]\nname = "two-view"',
                                            '[[fixture]]\nname = "two-view"\nsettings = { style = { foo = 1 } }'),
                             'settings.style', 'player_floor_rule')
        self.assert_rejected(SAMPLE.replace('x = 30', 'x = 30 30'), 'fixtures.toml')

    def test_toml_set_is_section_aware(self):
        text = 'a = 1\n[s]\na = 2\n[s.t]\na = 3\n'
        self.assertEqual(toml_set(text, 's', 'a', True)[0], 'a = 1\n[s]\na = true\n[s.t]\na = 3\n')
        self.assertEqual(toml_set(text, '', 'a', 'x')[0], 'a = "x"\n[s]\na = 2\n[s.t]\na = 3\n')
        self.assertEqual(toml_set(text, 's.t', 'b', 1, required=False), (text, False))
        with self.assertRaises(HarnessError):
            toml_set(text, 's.t', 'b', 1)

    def test_presets_freeze_every_slot(self):
        text = self.files.paths['land'].read_text()
        with self.assertRaises(HarnessError):
            check_frozen(text)
        frozen = freeze_presets(text)
        check_frozen(frozen)
        before, after = tomllib.loads(text), tomllib.loads(frozen)
        kr_fog = before['kr']['night']['lighting']['fog_params']
        self.assertEqual(after['kr']['night']['lighting']['fog_params'], kr_fog[:3] + [0.0])
        self.assertEqual(sections_of(text), sections_of(frozen))

    def test_run_settings(self):
        fixtures = load_fixtures(self.write(SAMPLE), self.files)
        capture = Capture(fixtures[0], 'kr', 'night', 'ec', 'kr')
        run = Path(tempfile.mkdtemp(dir=self.scratch))
        applied = write_run_settings(self.files, run, capture, DEFAULT_WINDOW, DEFAULT_PACKAGES)
        self.assertEqual(applied, {})
        graphics = tomllib.loads((run / 'assets/settings/graphics.toml').read_text())['graphics']
        self.assertEqual((graphics['art_texture_source'], graphics['land_texture_source']), ('ec', 'kr'))
        self.assertFalse(graphics['reduce_unfocused_fps'])
        world = tomllib.loads((run / 'assets/settings/world_rendering.toml').read_text())
        self.assertTrue(world['show_only_player_floor'])
        land = tomllib.loads((run / 'cache/settings/shaders/land.toml').read_text())
        self.assertEqual(land['default_preset'], 'kr.night')
        session = tomllib.loads((run / 'cache/settings/session_state.toml').read_text())
        self.assertEqual(session['world']['last_p'], {'x': 10, 'y': 20, 'z': 5, 'm': 1})
        self.assertEqual(session['window']['width'], float(DEFAULT_WINDOW[0]))
        prefs = tomllib.loads((run / 'assets/settings/user_preferences.toml').read_text())
        self.assertFalse(prefs['performance']['show_overlay'])
        runtime = tomllib.loads((run / 'assets/settings/runtime_assets.toml').read_text())
        self.assertEqual(runtime['udd_path'], str(DEFAULT_PACKAGES) + '/')

    def test_diff_math(self):
        import numpy as np
        a = np.zeros((10, 10, 3), np.uint8)
        b = a.copy()
        stats = diff_stats(a, b, 4)
        self.assertEqual((stats['max_delta'], stats['changed_pct']), (0, 0.0))
        self.assertEqual(verdict(stats, Tolerance(4, 0.0)), 'identical')
        b[0, 0] = (0, 3, 0)    # below the threshold
        b[1, 1] = (0, 0, 200)  # above it: largest channel counts
        stats = diff_stats(a, b, 4)
        self.assertEqual(stats['changed_pixels'], 1)
        self.assertAlmostEqual(stats['changed_pct'], 1.0)
        self.assertAlmostEqual(stats['any_pct'], 2.0)
        self.assertEqual(stats['max_delta'], 200)
        self.assertEqual(verdict(stats, Tolerance(4, 1.0)), 'within')
        self.assertEqual(verdict(stats, Tolerance(4, 0.5)), 'changed')
        c = a.copy()
        c[2, 2] = (255, 0, 0)
        self.assertEqual(diff_stats(c, a, 254)['changed_pixels'], 1)
        self.assertEqual(diff_stats(a, c, 255)['changed_pixels'], 0)

    def test_storage_round_trip(self):
        import numpy as np
        from PIL import Image
        rng = np.random.default_rng(1)
        pixels = rng.integers(0, 256, (64, 96, 3), dtype=np.uint8)
        source = Path(tempfile.mkdtemp(dir=self.scratch)) / 'capture.png'
        Image.fromarray(pixels).save(source)
        for image_format in LOSSLESS_FORMATS:
            stored = encode(source, source.with_name(f'stored{FORMATS[image_format]}'), image_format)
            self.assertTrue(np.array_equal(image_rgb(stored), pixels), image_format)
        first = encode(source, source.with_name('a.jpg'), 'jpeg')
        second = encode(source, source.with_name('b.jpg'), 'jpeg')
        self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_window_limit(self):
        self.assertEqual(parse_window('1900x1000'), (1900, 1000))
        with self.assertRaises(argparse.ArgumentTypeError):
            parse_window('1920x1080')


if __name__ == '__main__':
    try:
        sys.exit(main())
    except HarnessError as error:
        print(f'error: {error}', file=sys.stderr)
        sys.exit(2)
