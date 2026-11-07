# Renderer preview control

`compare_frames.py` checks repeat frames or an overlap at a known pixel offset.
It accepts PNGs or raw RGBA with `--size WIDTH HEIGHT`, supports `--guard`, and
fails on differences or insufficient nonblack content. Repeat frames must have
equal dimensions; nonzero offsets or `--overlap` explicitly select a partial
comparison. For example:

```sh
python3 scripts/dev-helpers/renderer/compare_frames.py a.rgba b.rgba \
  --size 4160 4160 --offset-x 3584 --guard 32
```

`seam_stats.py TILE_LEVEL --frame-tiles 8` compares discontinuities at exported
frame boundaries and ordinary tile boundaries. It reports statistics, not a
pass/fail verdict; scene edges can also create outliers. Both helpers require
Pillow and NumPy, and perform no capture. Verify readiness and shader logs in
the capture driver separately. Rust package/GPU/codec diagnostics live in
[udd-devtools-cli](../../../tools/udd-devtools-cli/README.md).

On Unix, set `DYNAMAPPER_CONTROL_SOCKET` to a fresh absolute socket path when
launching Dynamapper. The opt-in socket is owner-only, nonblocking, and processes
at most eight JSON datagrams per frame. The normal app creates no endpoint or
polling system. Windows builds omit this Unix development transport.

Keep preview settings/caches isolated and use the KDE exact-PID preview helper
on Desktop 4. Pass the environment variable to that helper; it is inherited by
the child. The socket remains responsive on an inactive desktop. Do not activate
the preview or move a terminal to reach it.

```sh
DYNAMAPPER_CONTROL_SOCKET=/tmp/my-preview/control.sock \
  python3 /path/to/kde-x11-preview/scripts/preview.py \
  --desktop 'Desktop 4' --display :1 --cwd /tmp/my-preview --seconds 120 \
  -- /path/to/dynamapper

python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock status
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock teleport 2518 561
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock wait-ready
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock capture /tmp/my-preview/transition.png
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock key F2
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock click 250 350
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock reset-input
```

Use a separate terminal/tool call for commands while the bounded preview runs.
Teleport validates loaded-map bounds and follows the same player/transform update
and visible-chunk recompute as the UI dialog. `--z` and `--map` are explicit;
currently unloaded maps are rejected. Status reports player/rendered positions,
elapsed time, and the frame rate counted over 500 ms windows. Its `terrain`
object reports CPU/submitted metadata revisions, pending metadata uploads and the
number of published terrain-cache chunks. Equal revisions with no pending writes
establish metadata submission. Its `art` object reports the last collected map,
visible chunk and emitted-instance counts, atlas misses, and queued sprite/ground
pages. Terrain can be ready while the bridge's sprites are still streaming.

Run `wait-ready` after teleporting and before capturing. It requires current and
rendered positions to agree, submitted terrain metadata, and art with no misses
or pending pages across three stable snapshots. The timeout is bounded by
`--timeout` (30 seconds by default). This guards known CPU collection/upload
queues; it is not a GPU fence or proof that shader compilation and every color
texture have finished. Inspect actual frames and compare multiple captures for
animated fixtures.

`status.render` exposes the effective preset name, global-light value and effect
uniforms. `status.static_lights.instances` exposes the collector count. Use these
to verify the source state for an effect comparison, then inspect actual pixels;
the count does not establish visible light decals or GPU completion.

`status.picking` exposes the physical query pixel, request serial, render-seen
serial, encoded serial, and ready/pending state. Encoded commands are not a GPU
completion fence. After an app-only pointer command, use `wait-pick` before
reading/capturing its inspector result:

```sh
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock pointer 638 330
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock wait-pick
```

The command accepts a ready empty hit, rejects a stale pixel and waits for the
following inspector update. Enable inspection/highlighting first; hidden
inspection does not request queries. Animated scenes can have both a ready
matching-view reply and a newer pending query.

`status.terrain_cursor` reports the input pointer and an optional terrain hit.
Its `logical` array preserves `[map_x, map_y, original_signed_z, facet]`;
`world_position` is the displayed triangle point, and `graphic` is the source
map cell. Bank easing and interpolation can change the displayed height without
changing logical Z. Wait for scene readiness before interpreting a null hit as
no terrain. Click teleporting uses the original logical cell height.

`status.cursor_inspect` exposes the current cursor-inspection label for native
picking probes. Enable the cursor overlay in the isolated settings, tap `KeyI`,
then send an app-only `pointer` command. Allow an update before reading status.
The text is the displayed diagnostic label, including logical object coordinates;
it is not a collision query and can be stale while the panel is hidden.

Capture requests enqueue native Bevy screenshots. The client waits for the PNG
signature and final IEND chunk before reporting completion. Destinations must
be new absolute PNG paths with an existing parent; existing files are refused.
No arbitrary code, shell command or generic settings setter is exposed.

## App-only keyboard and pointer input

Input commands feed Bevy's normal typed messages and ordered window-event stream.
They target the primary app window, including egui, on an inactive desktop. They
use no OS input/focus APIs. Pointer state is held separately from `Window` so winit
does not interpret a test click as a request to move the real desktop cursor.
World cursor inspection, click teleporting and mouse movement use that app-only
position as well.

- `key F2`: tap a physical Bevy `KeyCode`. Use names such as `Escape`, `Enter`,
  `ArrowDown`, `KeyG` and `ControlLeft`. `--state press|release` holds/releases a key;
  the default `tap` uses separate press/release frames. `--text '+'` supplies
  logical character input alongside a named key when needed.
- `text '2518'`: insert text at the current UI text cursor. Use named keys for
  Enter/Tab and other control characters.
- `pointer X Y`: move only the app pointer. Coordinates are logical window pixels;
  `status.window` reports logical size and the physical/logical scale factor.
- `click X Y --button left`: click at that app position. Left is the default;
  right, middle, back and forward are supported. `--state press|release` permits
  dragging with intervening pointer commands.
- `scroll X Y`: send horizontal/vertical line deltas at the current app pointer.
- `reset-input`: discard pending input, release controller-held keys/buttons,
  clear the app pointer override and restore egui's normal focus input.

For a shortcut, press the modifier, tap the key, then release the modifier:

```sh
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock key ControlLeft --state press
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock key KeyG
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock key ControlLeft --state release
```

The server acknowledges queuing; the client waits until `status.input.queued_frames`
is zero. At most 256 input frames may be queued, drained one per app
update. Capture rejects pending input. egui receives an internal focus override
after the first input command, while `Window.focused` and desktop focus stay
unchanged. Run `reset-input` when finished, before using the app with real input.
Settings saves may be debounced. An empty input queue does not establish that
settings have reached disk, material updates have reached the GPU, or a dialog's
fade-in animation has settled. Allow settling before evaluating a screenshot.

Raw datagram examples (all input states/buttons are explicit):

```json
{"command":"key","key":"F2","state":"tap"}
{"command":"click","x":250,"y":350,"button":"left","state":"tap"}
{"command":"reset_input"}
```

The server refuses an occupied socket path and only unlinks its own inode during
normal shutdown. A force-terminated process can leave a stale socket: keep each
run in its own temporary directory and clean up only that run's owned artifacts.

`benchmark.py` samples several locations in one running preview and writes a new
JSON report with the raw status replies and per-fixture median/min/max sampled
FPS. For example:

```sh
python3 scripts/dev-helpers/renderer/benchmark.py \
  --socket /tmp/my-preview/control.sock --output /tmp/renderer-benchmark.json \
  --label 'release; KR morning; EC art/map; 1280x800 zoom0.8; orthographic' \
  --fixture transition:2518:561 --fixture bridge:2834:987 --fixture lava:5744:2928
```

Defaults allow eight seconds to settle and sample twenty seconds per fixture at
four requests per second. Allocate enough time in the bounded preview helper.
Each sample is a 500 ms window, so the sampled extrema are not raw frame
percentiles. Position agreement does not prove that uploads have settled; check
captures and upload diagnostics too. Use isolated settings to disable VSync,
the frame limiter and unfocused throttling for uncapped comparisons. Record the
build, camera, preset, package source, viewport and zoom in the label, keep other
GPU work comparable, and benchmark after compilation finishes.

For hardware pass measurements, enable `enable_render_diagnostics`,
`dump_to_console` and `log_render_breakdown` in the isolated
`world_rendering.toml` **before launch**. Set `dump_interval_sec = 1.0` and keep
the owned preview's `app.log`. The benchmark records Unix timestamps for its
measurement windows and replies, allowing the same-host console log to be
correlated with each fixture:

```sh
python3 scripts/dev-helpers/renderer/pass_timings.py \
  --benchmark /tmp/renderer-benchmark.json --app-log /tmp/my-preview/app.log \
  --output /tmp/renderer-pass-timings.json
```

This requires the current benchmark report and available hardware GPU diagnostics
(supported Vulkan/DX12 adapter). It rejects windows with insufficient GPU samples.
Console times have one-second resolution, so correlation excludes both boundary
seconds. Results retain smoothed, rounded pass durations. `reported_gpu_ms` sums
the renderer's timed spans; it is not complete frame wall time and overlapping
spans can count the same work. Use the named opaque-pass timing for comparisons.
When available, `main_opaque_fragments` retains the console's rounded fragment
invocation count. Compare stable counts within matched fixture windows; a cold
shader can leave terrain undrawn after metadata submission has completed.
Metadata readiness alone must not turn such a frame into a benchmark result.

On Mesa RADV, launching the owned preview with `RADV_DEBUG=shaderstats` also
records compiler statistics in `app.log`. Extract them with:

```sh
python3 scripts/dev-helpers/renderer/shader_stats.py \
  --app-log /tmp/my-preview/app.log --output /tmp/renderer-shader-stats.json \
  --label 'release; KR morning; terrain variant'
```

This preserves all reported stages and pipeline hashes and prints the five
largest modules. Identify the terrain pipeline by a controlled shader change;
the largest module alone is not a source-file identifier. Register allocation,
scratch allocation and static instruction counts help explain compilation
changes; they do not replace timed passes or count instructions actually executed
per fragment. This is an optional driver diagnostic, independent of the portable
runtime rendering path. Reused pipeline cache entries can omit compiler output;
keep a timing result usable even when its optional compiler statistics are absent.

## Temporary artifacts

After each completed task, remove superseded captures, obsolete test fixtures and
logs from the isolated temporary workspace. Keep current baselines, compact
benchmark/validation reports, active settings/packages, reusable helpers and
user-provided reference images. Archive useful small historical reports together.
Scope preview-directory cleanup to owned paths whose own metadata confirms
successful child/window cleanup. Record a retention/deletion manifest and verify
that the active fixture and helper entry points still work afterward.
