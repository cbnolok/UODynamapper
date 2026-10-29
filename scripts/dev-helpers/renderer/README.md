# Renderer preview control

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
python3 scripts/dev-helpers/renderer/control.py --socket /tmp/my-preview/control.sock capture /tmp/my-preview/transition.png
```

Use a separate terminal/tool call for commands while the bounded preview runs.
Teleport validates loaded-map bounds and follows the same player/transform update
and visible-chunk recompute as the UI dialog. `--z` and `--map` are explicit;
currently unloaded maps are rejected. Status reports player/rendered positions,
elapsed time, and existing smoothed frame diagnostics when enabled. It does not
claim that all texture streaming has finished. Allow streaming to settle and
compare multiple captures for animated fixtures.

Capture requests enqueue native Bevy screenshots. The client waits for the PNG
signature and final IEND chunk before reporting completion. Destinations must
be new absolute PNG paths with an existing parent; existing files are refused.
No arbitrary code, shell command or settings mutation is exposed.

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
The diagnostics are **smoothed**, so their sampled extrema are not raw frame
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
