# Dynamapper architecture and code audit

**Status: in progress.** This report is an audit and follow-up plan, not an
implementation plan that has already been approved. The review starts at commit
`38c4bb34`; later sections should record their own checkpoint if the code changes.
Paths and line numbers are evidence at the checkpoint and must be rechecked
before acting on a finding.

## Scope and method

Review the entire `dynamapper/` application one Bevy plugin at a time, then
cover application wiring, non-plugin modules, build code, shaders and runtime
assets not covered by a plugin. Trace registration, callers, data ownership and
cross-module dependencies. Distinguish code proven unused from conditional,
development, test-only or unfinished behavior. A past implementation is not a
compatibility requirement by itself; the current application behavior and
current project goals are the relevant constraints.

Findings are static observations unless a test or runtime check is named.
Proposed boundaries are directions for later design, not a prescribed folder
tree. The project is still changing, so revalidate findings before implementing
them. Do not treat this report as proof that unreviewed code is clean.

## Coverage

| Area | Status | Notes |
| --- | --- | --- |
| `ControlsPlugin` and its movement, window and development-control children | Reviewed | Findings C1–C7 below. |
| `PlayerPlugin` | Reviewed | Findings P1–P3 below; overlaps C1 and C6. |
| Remaining scene and camera plugins | Pending | Includes world, terrain, statics, lights and effects. |
| Dialog and overlay plugins | Pending | Includes input and player interactions already noted below. |
| Settings and shader preset plugins | Pending | Includes configuration ownership and persistence. |
| Texture caches, file loading and export | Pending | Trace runtime owners and conditional paths. |
| Frame rate, frame limiter and diagnostics | Pending | Include registration and configuration. |
| Application wiring, non-plugin modules, build, shaders and runtime assets | Pending | Account for files not owned by a reviewed plugin. |

## ControlsPlugin

Reviewed the dispatcher and action types, `PlayerMovementPlugin`,
`WindowControlsPlugin`, the shared teleport operation and Unix-only
`DevControlPlugin`, including its injected input. Related consumers in the
camera, scene, dialogs, overlays, settings and export were traced where needed.

### C1 — Player position and teleport responsibilities cross subsystem boundaries

**Classification:** architectural coupling. **Confidence:** confirmed from
current callers.

[`Player`](../../dynamapper/src/core/render/scene/player.rs#L6) and its logical
and last-rendered positions live under the render scene;
[`player_movement`](../../dynamapper/src/core/controls/player_movement.rs#L219)
updates the logical position and transform, while
[`teleport_player`](../../dynamapper/src/core/controls/teleport.rs#L7) validates
and updates them from under controls. The latter is also called by the
[teleport dialog](../../dynamapper/src/core/render/dialogs/teleport.rs#L102),
[cursor overlay](../../dynamapper/src/core/render/overlays/cursor_behavior.rs#L929),
[export](../../dynamapper/src/core/export_job.rs#L553) and
[development controller](../../dynamapper/src/core/controls/dev_control.rs#L141).
The callers arrange chunk recomputation separately; for example, the dialog and
cursor paths both explicitly send it after a successful teleport.

**Follow-up:** establish a player/navigation owner for logical position changes
and their invalidation contract; let input, UI and export request operations
through that boundary. Keep rendered-position tracking, streaming and transform
smoothing distinct from logical coordinates. Trace export's sequencing before
changing its invalidation behavior. Review `PlayerPlugin` and scene streaming
before selecting the precise API and location.

### C2 — Unsmoothed movement can drift past logical bounds

**Classification:** likely behavior defect. **Confidence:** direct static
evidence; not reproduced in a running application.

When smooth movement is disabled,
[`sys_player_move`](../../dynamapper/src/core/controls/player_movement.rs#L234)
adds the requested horizontal delta to the transform before clamping the
logical position to the map. It does the same for height before clamping to the
`i8` range. Continuing to move at a limit can therefore move the transform even
when the corresponding logical coordinate no longer changes.

**Follow-up:** derive the unsmoothed transform from the accepted position or
accepted delta. Add focused tests for both horizontal edges and both height
limits, including diagonal movement, with smooth movement enabled and disabled.
Preserve the separation between logical Z and presentation offsets.

### C3 — UI keyboard and pointer capture are conflated for player movement

**Classification:** inconsistent input policy; possible behavior defect.
**Confidence:** confirmed code difference; user-visible cases not reproduced.

[`sys_player_input`](../../dynamapper/src/core/controls/player_movement.rs#L73)
stops both keyboard and pointer movement when egui wants pointer input, but
does not test keyboard capture before reading WASD and height keys. The
[central dispatcher](../../dynamapper/src/core/controls.rs#L48) and
[free camera](../../dynamapper/src/core/render/scene/camera.rs#L543) check
keyboard capture. A focused text field may allow movement when only keyboard
capture is requested, while pointer capture may suppress keyboard movement.

**Follow-up:** define shared capture policy per input source, then test text
focus, pointer-over-UI and simultaneous mouse/keyboard input for player and
camera controls. Retain the controller's app-only input behavior described in
[`renderer_control_channel.md`](../agent_notes/renderer_control_channel.md).

### C4 — Binding definitions, dispatch and help are scattered

**Classification:** ownership and maintainability. **Confidence:** confirmed.

[`SectKeybindings`](../../dynamapper/src/configs/settings.rs#L33) contains five
bindings. The [dispatcher](../../dynamapper/src/core/controls.rs#L70),
[movement](../../dynamapper/src/core/controls/player_movement.rs#L122) and
[free camera](../../dynamapper/src/core/render/scene/camera.rs#L560) contain
additional hardcoded controls, while the
[help dialog](../../dynamapper/src/core/render/dialogs/keybindings_help.rs#L71)
manually repeats bindings. The help omits cursor-mode shortcuts and points to
`assets/keybindings.toml`; the bundled file is
[`assets/settings/keybindings.toml`](../../dynamapper/assets/settings/keybindings.toml)
and an app-written cache can override its values. The linked
[keybindings guide](../keybindings.md#L25) also calls the implemented teleport
dialog planned and gives the old path.

**Follow-up:** identify primary actions and define their bindings once for
dispatch and help, with configuration and conflict handling appropriate to each
input mode. Update both help surfaces and explain bundled versus cached settings.
Do not assume all camera controls and player controls have identical semantics.

### C5 — Developer control mixes transport, command execution and observations

**Classification:** separation of concerns. **Confidence:** confirmed.

[`poll_control`](../../dynamapper/src/core/controls/dev_control.rs#L101) receives
and parses datagrams, executes teleports, queues screenshots and input, and
constructs status from terrain, art, picking, uniforms, window and UI resources.
Changes in those subsystems therefore touch a transport/dispatch system.

**Follow-up:** separate transport and protocol handling from command execution
and status collection, keeping the existing opt-in Unix endpoint, readiness
fields, scheduling and capture behavior. The existing
[`input.rs`](../../dynamapper/src/core/controls/dev_control/input.rs) separation
is useful. `InjectedPointer` is deliberately shared by movement, picking and
cursor interaction; it is active automation infrastructure, not dead code.

### C6 — Movement map-change checks are unreachable under the present invariant

**Classification:** dead-code candidate. **Confidence:** current production
initialization and writes traced; not impossible at the type level.

Both movement paths compare the old map with the new map and send a chunk
recompute if they differ, at
[`player_movement.rs`](../../dynamapper/src/core/controls/player_movement.rs#L252).
The horizontal and vertical helpers preserve `m`; production player creation
initializes `current_pos` to `Some`, and the located production writes assign
`Some`. Thus the checks only fire if a caller first makes the optional position
`None`, for which no production path was found.

**Follow-up:** decide whether an absent player position is a supported state.
If it is not, express that invariant in the type and remove the redundant
fallbacks and map-change checks. If it is supported, document and test the
transition instead. Do not remove teleport invalidation, including same-map
teleports.

### C7 — Close-dialog behavior is distributed and inconsistent

**Classification:** UI behavior and ownership. **Confidence:** confirmed
registration difference; desired behavior is undecided.

The [Escape dispatcher](../../dynamapper/src/core/controls.rs#L77) emits one
`ActionCloseActiveDialog`, but multiple dialog observers close their own state.
The [shader panel](../../dynamapper/src/core/render/dialogs/land_shader.rs#L43)
registers a toggle observer but no close observer. The action name implies one
active dialog, while the observers can close multiple dialogs.

**Follow-up:** establish whether Escape should close one or all dialogs and
how egui keyboard focus affects that action. Apply that policy consistently
and cover it when auditing the dialog plugins.

### Controls validation and exclusions

`cargo test -p dynamapper core::controls:: -- --nocapture` passed: 7 tests.
They exercise controller protocol and input injection, socket and capture path
handling, and teleport validation; they do not establish C2 or C3 behavior.
All nine declared action types have a current dispatcher and observer. The
fullscreen plugin and developer control channel are registered and live; no
entire controls module was established to be unused. No code was changed by
this audit and no visual comparison was performed.

## PlayerPlugin

Reviewed player spawning, logical and scene state, marker visibility and the
configuration and rendering consumers needed to establish their ownership.

### P1 — Player component also stores the scene's last processed position

**Classification:** cross-subsystem ownership and misleading name.
**Confidence:** confirmed by code tracing.

[`Player`](../../dynamapper/src/core/render/scene/player.rs#L6) stores both
`current_pos` and `prev_rendered_pos`. The latter is updated by
[scene chunk synchronization](../../dynamapper/src/core/render/scene.rs#L403)
before pending chunk spawns and other work finish. It is therefore a checkpoint
of the last position processed by that system, not proof that pixels have been
rendered. [Export readiness](../../dynamapper/src/core/export_job.rs#L569)
correctly performs further checks after comparing the positions, and the
development controller exposes the checkpoint as `rendered_position`.

**Follow-up:** as part of C1, consider placing the logical player state outside
the render scene and moving the processed-position checkpoint to the scene
streaming owner. Name the checkpoint for what it means, and update export and
controller consumers without weakening their other readiness checks. Separately
check whether the [player position overlay](../../dynamapper/src/core/render/overlays/player_position.rs#L83),
which reports a position derived from `Transform`, is intended to display the
smoothed position or the logical coordinate used for persistence and map data.

### P2 — Required startup position setting is not consumed

**Classification:** unused configuration. **Confidence:** confirmed by
repository source search; startup behavior not changed or retested.

[`SectWorld.start_p`](../../dynamapper/src/configs/settings.rs#L116) is a
required field supplied by the bundled
[`core.toml`](../../dynamapper/assets/settings/core.toml#L4), but no application
code reads that field. [Player startup](../../dynamapper/src/core/render/scene/player.rs#L49),
camera startup and the dynamic light use `session_state.world.last_p` instead.
Changing `core.world.start_p` consequently does not choose the startup position.

**Follow-up:** decide whether a separate first-run position is useful. Either
remove the unused field from the strict schema, bundled file and affected
fixtures/tests, or explicitly use it as the first-run fallback. Check the
bundled and cached settings precedence when making that decision; app-state
cache handling can warn about and drop an old key.

### P3 — Player identity and diagnostic marker are intertwined

**Classification:** ownership and duplication; active placeholder, not dead
code. **Confidence:** confirmed.

The player entity is needed by movement and world systems, while
[`PlayerPlugin`](../../dynamapper/src/core/render/scene/player.rs#L30) also
creates a green cube as a visual marker. `hide_player` is exposed in preferences
and export sets it, so the marker has current callers. Its mesh and material
are created at startup even when initially hidden, and are created again when
the marker is shown after being hidden. The startup and toggle paths duplicate
their construction. No claim about an asset leak has been established.

**Follow-up:** decide whether this cube is a supported diagnostic marker or a
temporary stand-in for future player visuals. If retained, give marker rendering
an explicit owner and reuse its assets or visibility state, without removing the
logical player entity. Add focused startup and hide/show tests if changed.

No tests specifically covering player spawn or marker visibility were found in
the plugin. The `current_pos` optional-state question remains C6; it should not
be resolved by deleting fallback code without reviewing the intended invariant.
