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
