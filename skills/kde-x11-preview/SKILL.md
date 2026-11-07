---
name: kde-x11-preview
description: Run an isolated GUI preview through X11 or XWayland on a named KDE virtual desktop, verifying exact process ownership and cleaning temporary KWin rules. Use when a GUI test must avoid the user's working desktop.
---

# KDE X11 preview

Use the caller's requested desktop name, application command, and isolated working directory. A particular desktop number is not a default. This workflow targets KWin's desktop-object scripting API, tested on KDE Plasma 6 with XWayland.

## Launch and verify

Use [scripts/preview.py](scripts/preview.py) for a bounded preview. It requires Python `dbus` and `gi` modules, a live session bus with `org.kde.KWin`, and an X11 display. It accepts a foreground executable that creates its own window under the same PID; launchers that fork or reuse an existing application instance need an application-specific foreground/new-instance option.

Run from the repository root (the helper resolves its own resources):

```sh
python3 skills/kde-x11-preview/scripts/preview.py \
  --desktop 'Desktop 4' \
  --cwd /absolute/path/to/isolated-preview \
  --display "$DISPLAY" \
  --seconds 5 \
  -- /absolute/path/to/application application-arguments
```

The script creates a unique temporary artifact directory containing `app.log` and its KWin script. It prints the verified PID, desktop assignment, and artifact path. It does not switch the user's active desktop or modify saved KWin settings.

The application inherits the caller's environment with `WAYLAND_DISPLAY` removed and `DISPLAY` selected explicitly. Supply any application-specific X11 backend options as needed, for example `QT_QPA_PLATFORM=xcb` or `GDK_BACKEND=x11`; removing the Wayland display variable alone does not override every toolkit's backend preference.

For a renderer that already writes a PNG when given an environment variable:

```sh
python3 skills/kde-x11-preview/scripts/preview.py \
  --desktop 'Desktop 4' \
  --cwd /absolute/path/to/isolated-preview \
  --display "$DISPLAY" \
  --capture-env APP_CAPTURE_PATH \
  --capture-timeout 30 \
  -- /absolute/path/to/renderer
```

Replace `APP_CAPTURE_PATH` with the application's actual capture variable. This option waits for an app-produced PNG; it does not add capture support to the application. The path is a fresh `capture.png` inside the unique artifact directory. Without `--capture-env`, the process runs for `--seconds` after placement verification.

For Dynamapper, use its existing `DYNAMAPPER_CONTROL_SOCKET` and native `capture` command rather than assuming an environment-variable screenshot hook. See the sibling [renderer reference validation skill](../renderer-reference-validation/SKILL.md) and the repository's [renderer control commands](../../scripts/dev-helpers/renderer/README.md). Give the bounded preview enough time for the control commands to run from another tool call.

## Non-obvious constraints

- Obtain the child PID before it creates a window: a stopped shell is resumed into `exec`, preserving its PID. Install the exact-PID `windowAdded` handler while it is stopped.
- Start only this run's `/Scripting/Script<id>` through `org.kde.kwin.Script.run`; the global `Scripting.start` can start other pending scripts.
- Verify the named desktop exists before resuming. Verify the resulting window assignment through a private DBus callback. Unverified placement is a failed run.
- Match only that exact process. Window titles can contain application names inside terminals, editors, and multiplexer panes. Never broaden a failed match to title or class rules that could move those windows.
- XWayland's desktop count/index reported through `xdotool` can disagree with KWin's virtual desktops. Use KWin desktop objects and their names.
- Keep settings, caches, and capture outputs in the isolated run directory. The helper does not isolate application state by itself; prepare the app's actual settings mechanism first.
- Cleanup belongs in `finally`: terminate/reap only the owned child, resume it if still stopped, and unload only this run's uniquely named KWin script. Do not kill processes by name or remove another run's script.
- If the child window PID differs, the desktop API is unsupported, or verification times out, inspect the log and adapt the launch method. Do not repeat launches blindly or compensate by moving unrelated windows.

Use renderer-native capture for hidden/off-desktop windows when available. X11 `XGetImage` and ffmpeg window capture may fail with `BadMatch` under XWayland when the window is not visible on the active desktop. A PNG's existence alone is not completion: wait for its signature and final IEND chunk, then inspect it with the image-viewing tool.

When changing the helper, run `python3 skills/kde-x11-preview/scripts/test_preview.py` from the repository root for offline ownership, quoting, cleanup, and PNG-completion checks. A live KWin smoke test is still needed for DBus/API compatibility. Coordinate such tests with other ongoing GUI work; each run has unique service and plugin names. The helper's artifacts metadata includes its exact plugin name so a failed cleanup can be investigated without touching another run's rules.
