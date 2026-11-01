# Local renderer test control

On Unix, `DYNAMAPPER_CONTROL_SOCKET` opts a process into a private Unix datagram
control endpoint. Normal launches install no socket or polling system. This is
a development transport; Windows builds omit it. Commands are JSON datagrams,
limited to 4096 bytes and eight requests per frame, handled nonblockingly on the
main ECS thread in `MovementActions`.

Commands include `teleport`, `status`, `capture` and app-only keyboard/pointer
input (`key`, `text`, `pointer`, `click`, `scroll`, `reset_input`). There is no shell/code
execution or generic settings setter. The owner-only socket refuses occupied
paths; normal resource cleanup only removes the bound inode. Forced termination
may leave a socket, so isolate each run in its own temporary directory.

The UI dialog and endpoint share `controls::teleport::teleport_player` for loaded
map/bounds validation and player/transform updates. Both request visible-chunk
recomputation after success, including same-map teleports. Invalid destinations
leave state untouched. This does not add loading for previously unloaded maps.

`status` exposes current and previously rendered player positions, elapsed time,
and the existing smoothed FPS/frame-time diagnostics when available. Rendered
position equality does not establish completion of texture streaming. The
`art` snapshot reports the collector's map/chunk/instance counts, atlas misses
and pending sprite/ground pages. A terrain-only wait captured an empty bridge
while its sprite pages were still loading. `control.py wait-ready` now requires
position agreement, submitted metadata and settled art over three stable
snapshots. It has a bounded timeout; it does not fence GPU completion or shader
compilation. Keep actual-frame inspection in the validation workflow.

`status.render` reports the actual active preset, global lighting and effects
uniform values; `status.static_lights.instances` reports collected lights. These
read-only snapshots help detect a stale cached preset or an empty collector
without temporarily instrumenting shaders. A positive instance count does not
by itself prove visible mask composition or completion of GPU uploads.

`capture` uses Bevy's native screenshot path, so hidden/off-desktop windows do
not need activation. Existing files/symlinks, relative/non-PNG paths and missing
parent directories are rejected. The reply acknowledges queuing; the Python
client waits for the PNG signature and final IEND before declaring completion.

## Ordered app input

Input requests enqueue at most 256 frames. A PreUpdate system drains one frame
before Bevy's `InputSystems`, publishing both typed input messages and the ordered
`WindowEvent` stream. egui 0.43.1 reads pointer movement/button ordering from the
latter; emitting only `MouseButtonInput` does not reproduce a click correctly.
Taps use separate press/release frames, and modifier presses can remain held
across commands. The client waits for `status.input.queued_frames == 0` before
returning. Screenshot requests reject pending input. `reset_input` cancels queued
actions and releases the controller's already emitted keys/buttons.

The controller must not write `Window::set_cursor_position`: bevy_winit compares
that field with its cached window and calls the OS cursor-warp API. Instead,
`InjectedPointer` and `effective_cursor_position` supply an app-only position to
world movement, cursor inspection/highlighting and click teleporting; egui uses
the ordered cursor message. Neither the primary Window cursor nor its focused
flag is modified. Input coordinates are logical window pixels, validated against
the primary window bounds. Status reports size/scale for screenshot conversion.

Hidden-window UI input also needs egui focus. After input is emitted, the
controller overrides only `EguiInput.focused`, after normal input conversion and
before `BeginPass`. This is input preparation, not UI rendering; dialog systems
remain in `EguiPrimaryContextPass`. The override lasts until `reset_input`, when
egui returns to the actual window focus on its next normal input update.

Named physical keys are layout-independent. Logical text is explicit through
`text` or `key --text`; egui's existing physical-key fallback handles named
navigation/function keys. No external keyboard layout or desktop input is used.

Validation: six focused controller tests pass, including the real egui pointer
conversion, separate tap frames, held-input reset, queue limits and unchanged
Window focus/cursor fields. A native Desktop-4 run exercises the documented CLI:
all four transition-checkbox combinations are saved, Ctrl+G opens GoTo, explicit
field focus plus text/Tab fills `(2518,561,0,0)`, and clicking Teleport applies it.
Scroll and logical `+` input change framing. The UI-driven CC-enabled town frame
exactly matches its previously accepted capture. Actual X11 focus and pointer
readings are identical before/after, and KWin placement/cleanup are verified.
Temporary evidence is `control-input-accepted-evidence.json`, the live command
log and `control-auto-final-*` captures under `/tmp/uodynamapper-ec-followup/`.
The first frame of an opened dialog can be almost transparent during fade-in;
input completion alone does not establish a visually settled UI.

Reusable commands and transport documentation live in
`scripts/dev-helpers/renderer/`. They can drive one preview through multiple
locations without changing startup session state or rebuilding capture hooks.


## Pixel-query readiness

`status.picking` exposes the active physical pixel, request serial, render-seen
and encoded serials, and ready/pending state. Readback is asynchronous; encoded
commands alone do not establish a hit. `control.py wait-pick` checks the injected
pointer's physical pixel and allows the inspector's following update to finish.
A successful empty hit is ready, while map/readback failure stays unready.
Animation can keep another frame pending while a recent matching-view reply is
ready. Changed pointer, camera or content cannot reuse the old view's reply.
