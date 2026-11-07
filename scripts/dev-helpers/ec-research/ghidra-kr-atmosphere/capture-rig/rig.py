"""Isolated Dynamapper preview rig for the KR atmosphere captures.

Adapted from the paperdoll-ec rig. One Run owns a private working directory with its own copy of assets/ and cache/,
reads the installed packages in PACKAGES, and starts under the KR-like preset whose [kr.afternoon] block may be
overridden in the run's own cache copy of the shader settings. The user's saved settings are never read or written.

The preview helper (~/.codex/skills/kde-x11-preview) starts the binary on Desktop 4, verifies the exact PID and the
desktop, and cleans up; its artifact folder goes under SCRATCH_RIG/tmp through TMPDIR, never /tmp. Run every
capture under the shared preview lock:

    flock /home/claudio/rust-target/uodynamapper-scratch/preview.lock \
        /home/claudio/rust-target/uodynamapper-scratch/lowprio.sh python3 views.py RUN VARIANT VIEW_SET

BINARY is a copy of a debug build of this repository, placed in SCRATCH_RIG/bin by hand.
"""
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[5]
SCRATCH = Path('/home/claudio/rust-target/uodynamapper-scratch/kr-atmosphere')
SCRATCH_RIG = SCRATCH / 'rig'
RUNS = SCRATCH_RIG / 'runs'
CAPS = SCRATCH / 'caps'
PREVIEW = Path.home() / '.codex/skills/kde-x11-preview/scripts/preview.py'
DESKTOP = 'Desktop 4'
BINARY = SCRATCH_RIG / 'bin/dynamapper'
PACKAGES = Path('/home/claudio/test/uddp')
PRESET_SECTION = 'kr.afternoon'
NEXT_SECTION = 'kr.night'

# The client's frustum is 26 world units wide at any resolution, about 18.4 tiles across (a tile's screen width is
# its diagonal, sqrt 2 units). Dynamapper shows window / 44 tiles at zoom 1, so a 1024 px window needs zoom 0.79.
# Keep windows well under two thirds of the 2880 px screen, or the preview comes up maximized.
DEFAULT_WINDOW = (1024, 768)
DEFAULT_ZOOM = 0.79
DEFAULT_POSITION = (2518, 561, 0, 0)
# preview.py refuses --seconds above 300.
PREVIEW_SECONDS = 300
PLACEMENT_TIMEOUT_SECONDS = 60
SOCKET_TIMEOUT_SECONDS = 180
SOCKET_POLL_SECONDS = 0.2
HELPER_EXIT_SECONDS = 30
LOG_TAIL_CHARS = 2000
REQUEST_TIMEOUT_SECONDS = 60
SCENE_TIMEOUT_SECONDS = 240
SCENE_WAIT_SECONDS = 60
SCENE_RETRY_SECONDS = 0.5
TELEPORT_SETTLE_SECONDS = 2.0
CAPTURE_TIMEOUT_SECONDS = 60
CAPTURE_POLL_SECONDS = 0.03

spec = importlib.util.spec_from_file_location('control', REPO / 'scripts/dev-helpers/renderer/control.py')
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)

# Every term the Kingdom Reborn client lacks, switched off inside [kr.afternoon] and its subtables.
# The land diffuse becomes the client's half-Lambert: wrap 0.5 gives (N.L + 1) / 2, unshaped, white, no ambient.
CLIENT_TERMS_OFF = {
    'global_lighting': '1.0',
    'normal_mode': '0',
    'enable_blur': '0',
    'art_shadow_strength': '0.0',
    'enable_grunge': '0',
    'light_decal_intensity': '0.0',
    'art_highlight_strength': '0.0',
    'art_depth_tint_strength': '0.0',
    'enable_normal_maps': '0',
    'enable_art_fake_normals': '0',
    'art_contact_shadow_strength': '0.0',
    'art_mottle_strength': '0.0',
    'kr_land_temperature_strength': '0.0',
    'kr_land_relief_shadow_strength': '0.0',
    'kr_land_shadow_mottle_strength': '0.0',
    'kr_art_temperature_strength': '0.0',
    'enable_art_projected_shadows': '0',
    'kr_land_material_contrast_strength': '0.0',
    'static_light_decal_visibility': '0.0',
    'enable_kr_transition_grime': '0',
    'enable_kr_color_wash': '0',
    'enable_fog': '0',
    'enable_tonemap': '0',
    'enable_grading': '0',
    'enable_gloom': '0',
    'light_color': '[1.0, 1.0, 1.0]',
    'exposure': '1.0',
    'ambient_strength': '0.0',
    'grade_params': '[1.0, 0.15, 0.4, 0.0]',
    'enable_bent': '0',
    'diffuse_strength': '1.0',
    'specular_strength': '0.0',
    'rim_strength': '0.0',
    'fill_strength': '0.0',
    'sharpness_mix': '0.0',
    'diffuse_wrap': '0.5',
}

# Variant c: b plus the nearest stand-in for the client's projected sprite shadow (scene x 0.75, hard edge).
CLIENT_SHADOW_STANDIN = dict(CLIENT_TERMS_OFF, **{
    'enable_art_projected_shadows': '1',
    'art_projected_shadow_strength': '0.25',
    'art_projected_shadow_softness': '0.05',
})

VARIANTS = {'a': {}, 'b': CLIENT_TERMS_OFF, 'c': CLIENT_SHADOW_STANDIN}


def _sub_toml_value(text, key, value):
    pattern = re.compile(rf'^{re.escape(key)}\s*=.*$', re.M)
    assert pattern.search(text), key
    return pattern.sub(f'{key} = {value}', text, count=1)


def override_section(text, overrides):
    """Replace keys only inside [kr.afternoon] and its subtables; values may be multi-line arrays."""
    start = text.index(f'[{PRESET_SECTION}]\n')
    end = text.index(f'[{NEXT_SECTION}]\n')
    block = text[start:end]
    for key, value in overrides.items():
        pattern = re.compile(rf'^{re.escape(key)}\s*=\s*(\[[^\]]*\]|[^\n]*)$', re.M)
        assert len(pattern.findall(block)) == 1, key
        block = pattern.sub(f'{key} = {value}', block, count=1)
    return text[:start] + block + text[end:]


class Run:
    def __init__(self, name, variant, window=DEFAULT_WINDOW, position=DEFAULT_POSITION, zoom=DEFAULT_ZOOM):
        self.name = name
        self.dir = RUNS / name
        if self.dir.exists():
            shutil.rmtree(self.dir)
        self.dir.mkdir(parents=True)
        assets = self.dir / 'assets'
        shutil.copytree(REPO / 'dynamapper/assets', assets)
        cache = self.dir / 'cache/settings'
        (cache / 'shaders').mkdir(parents=True)
        runtime = assets / 'settings/runtime_assets.toml'
        runtime.write_text(_sub_toml_value(runtime.read_text(), 'udd_path', json.dumps(str(PACKAGES) + '/')))
        (cache / 'session_state.toml').write_text(
            f'[world.last_p]\nx = {position[0]}\ny = {position[1]}\nz = {position[2]}\nm = {position[3]}\n\n'
            f'[window]\nheight = {float(window[1])}\nwidth = {float(window[0])}\nzoom = {zoom}\n')
        shader_text = (assets / 'settings/shaders/land.toml').read_text()
        shader_text = _sub_toml_value(shader_text, 'default_preset', json.dumps(PRESET_SECTION))
        (cache / 'shaders/land.toml').write_text(override_section(shader_text, VARIANTS[variant]))
        self.socket = self.dir / 'control.sock'
        self.child = None
        self.caps = CAPS / name
        if self.caps.exists():
            shutil.rmtree(self.caps)
        self.caps.mkdir(parents=True)

    def start(self, seconds=PREVIEW_SECONDS, timeout=SOCKET_TIMEOUT_SECONDS):
        env = os.environ.copy()
        env['DYNAMAPPER_CONTROL_SOCKET'] = str(self.socket)
        env['TMPDIR'] = str(SCRATCH_RIG / 'tmp')
        self.log = open(self.dir / 'preview.out', 'w')
        self.child = subprocess.Popen(
            [sys.executable, str(PREVIEW), '--desktop', DESKTOP, '--cwd', str(self.dir), '--display',
             os.environ['DISPLAY'], '--seconds', str(seconds), '--placement-timeout', str(PLACEMENT_TIMEOUT_SECONDS),
             '--', str(BINARY)],
            env=env, stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + timeout
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

    def wait_in_game(self, timeout=SCENE_TIMEOUT_SECONDS, position=None):
        deadline = time.monotonic() + timeout
        while True:
            try:
                return control.wait_for_scene(self.socket, SCENE_WAIT_SECONDS, position)
            except (OSError, RuntimeError, TimeoutError) as error:
                if time.monotonic() > deadline:
                    raise TimeoutError(f'scene never ready: {error}')
                time.sleep(SCENE_RETRY_SECONDS)

    def teleport(self, x, y, z=0, map_id=0, settle=TELEPORT_SETTLE_SECONDS):
        self.req({'command': 'teleport', 'x': x, 'y': y, 'z': z, 'map': map_id})
        status = self.wait_in_game(position=[x, y, z, map_id])
        time.sleep(settle)
        return status

    def capture(self, label):
        path = self.caps / f'{label}.png'
        self.req({'command': 'capture', 'path': str(path)})
        deadline = time.monotonic() + CAPTURE_TIMEOUT_SECONDS
        while not control.png_complete(path):
            if time.monotonic() > deadline:
                raise TimeoutError(f'capture did not finish: {path}')
            time.sleep(CAPTURE_POLL_SECONDS)
        return path
