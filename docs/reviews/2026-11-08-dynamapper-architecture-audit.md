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
| `CameraPlugin` | Reviewed | Findings V1–V7 below. |
| `ScenePlugin` | Reviewed | Findings S1–S7 below; V1 covers free-camera streaming. |
| `PlayerDynamicLightPlugin` | Reviewed | Findings L1–L2 below. |
| `WorldPlugin` | Reviewed | Findings W1–W3 below; child plugins remain pending. |
| `DrawLandChunkMeshPlugin` | In progress | Geometry and loading findings T1–T7; material, atlas and shader coverage remain incomplete. |
| Remaining scene plugins | Pending | Includes world, terrain, statics, lights and effects. |
| Dialog and overlay plugins | Pending | Includes input and player interactions already noted below. |
| Settings and shader preset plugins | Pending | Includes configuration ownership and persistence. |
| Texture caches, file loading and export | Pending | Trace runtime owners and conditional paths. |
| Frame rate, frame limiter and diagnostics | Pending | Include registration and configuration. |
| Application wiring, non-plugin modules, build, shaders and runtime assets | Pending | Account for files not owned by a reviewed plugin. |

Plugin coverage is tracked individually so reviewing an orchestration plugin
does not imply its children have been reviewed:

- Reviewed: `ControlsPlugin`, `PlayerMovementPlugin`, `WindowControlsPlugin`,
  Unix `DevControlPlugin`, `PlayerPlugin`, `CameraPlugin`, `ScenePlugin`,
  `PlayerDynamicLightPlugin`, `WorldPlugin`.
- Pending scene/rendering: `RenderPlugin`,
  `DrawLandChunkMeshPlugin`,
  `DrawStaticSpritesPlugin`, `FogPostProcessPlugin`, feature-gated
  `LandProfilingPlugin`.
- Pending overlays/dialogs: `OverlaysPlugin`, `PerformanceOverlayPlugin`,
  `PlayerPositionOverlayPlugin`, `CursorBehaviorOverlayPlugin`,
  `SystemMessagesPlugin`, `DialogsPlugin`, `PreferencesDialogPlugin`,
  `LandUiPlugin`, `KeybindingsHelpPlugin`, `GumpDialogPlugin`, `TeleportPlugin`.
- Pending data/runtime: `ExternalDataPlugin`, `SettingsPlugin`,
  `ShaderPresetsPlugin`, `TextureCachePlugin`, `ArtPageFormatCheckPlugin`,
  `LandTextureCachePlugin`, `UOFilesPlugin`, `ExportJobPlugin`.
- Pending frame/diagnostics: `FrameRatePlugin`, `FrameLimiterPlugin`,
  `LogGpuPreprocessingModePlugin`.

Also review direct system installers, conditional/platform-specific paths and
third-party integration; plugin definitions alone do not cover the application.

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

## CameraPlugin

Reviewed camera construction, projection, graphics settings, player following,
free-camera input, zoom, the perspective projection module and related scene
streaming and export callers.

### V1 — Chunk streaming does not follow the free camera's actual view

**Classification:** cross-plugin contract mismatch; likely coverage defect.
**Confidence:** inputs and data flow confirmed statically; visible effects not
reproduced in this audit.

[Free-camera movement](../../dynamapper/src/core/render/scene/camera.rs#L536)
changes the camera transform and requests chunk recomputation. However,
[scene chunk selection](../../dynamapper/src/core/render/scene.rs#L470) passes
the player transform, window and zoom to a
[fixed isometric orthographic footprint](../../dynamapper/src/core/render/scene.rs#L162),
without the actual camera position, rotation or projection. Moving sufficiently
far from the player can therefore leave the free view outside the selected
coverage; arbitrary rotation and perspective are also approximated. The prior
[perspective note](../agent_notes/isometric_perspective_projection.md) does not
establish arbitrary free-camera or maximum-zoom coverage. Export intentionally
uses a separate explicit footprint.

**Follow-up:** review this jointly with `ScenePlugin` and define a current view
or conservative footprint contract that accounts for camera pose, projection,
viewport and elevation. Preserve export's explicit footprint and handle the
documented one-frame latency of Bevy's computed camera matrices. Test free
pan/rotation, follow and perspective views, zoom/resize and export separately.

### V2 — Projection and view calculations have several owners

**Classification:** scattered responsibility. **Confidence:** confirmed.

Orthographic dimensions are independently calculated in
[camera startup](../../dynamapper/src/core/render/scene/camera.rs#L128),
[camera updates](../../dynamapper/src/core/render/scene/camera.rs#L350),
[scene chunk selection](../../dynamapper/src/core/render/scene.rs#L186),
[export camera setup](../../dynamapper/src/core/export_job.rs#L523) and the
[position overlay](../../dynamapper/src/core/render/overlays/player_position.rs#L151).
The calculations share constants but changes still require coordinated edits.
The camera plugin itself also owns the world camera, UI camera and egui context,
AA configuration, fog attachment, input and player following.

**Follow-up:** extract a shared view/projection calculation where semantics are
actually identical, and give world view, UI camera setup and input/follow clear
owners. Preserve export's target-specific sizing and the active custom
[`perspective`](../../dynamapper/src/core/render/scene/camera/perspective.rs)
projection and its frustum/subview contract; do not mechanically unify
orthographic approximations with actual perspective coverage.

### V3 — Startup zoom application has no explicit ordering against camera setup

**Classification:** schedule-dependent defect candidate. **Confidence:**
missing ordering established; actual startup ordering and visible result have
not been reproduced.

[`SettingsPlugin`](../../dynamapper/src/configs/settings.rs#L685) schedules
`sys_apply` in `Startup`, where it writes saved window resolution and zoom.
[`CameraPlugin`](../../dynamapper/src/core/render/scene/camera.rs#L90) schedules
camera setup in a startup set and reads both, without an explicit edge from
`sys_apply`. Resource access forces serialization but does not express which
system runs first. If setup reads default zoom first in free perspective mode,
the follow system is disabled and later zoom dolly uses only the change from
the already updated zoom, potentially leaving the initial distance inconsistent.

**Follow-up:** test startup with a non-default saved zoom and free perspective,
then make the settings-applied-before-camera contract explicit if needed.
Check export's override separately. This is a hypothesis until that test runs.

### V4 — UI camera dependency survives where the helper does not use it

**Classification:** obsolete plumbing. **Confidence:** confirmed for helper
parameter; the resource itself still has live callers.

[`get_egui_context_ready_mut`](../../dynamapper/src/core/render/dialogs.rs#L75)
ignores its `UiCameraResource` argument and gets the primary context directly,
yet callers still acquire the resource to pass it through. Camera AA setup and
player input do use the resource, so it cannot simply be removed globally.

**Follow-up:** remove the unused helper argument and only those caller-side
resource dependencies that have no other use. Review broader UI context
ownership during the dialog/overlay audit.

### V5 — Camera source retains disabled experiments and ambiguous controls

**Classification:** inactive source clutter and control semantics.
**Confidence:** commented code and key paths confirmed; desired interaction
is undecided.

[Camera setup](../../dynamapper/src/core/render/scene/camera.rs#L170) contains
commented-out tonemapping, depth-prepass and UI-context experiments; a later
[directional-light block](../../dynamapper/src/core/render/scene/camera.rs#L330)
is also commented out. The preceding tonemapping comment reads as if the
disabled component were active. Separately,
[Shift+Up/Down](../../dynamapper/src/core/render/scene/camera.rs#L560) adds both
horizontal movement and altitude, although help labels the shortcut as
altitude. It may be intentional; its contract is not explicit.

**Follow-up:** remove obsolete commented code and correct misleading comments
after preserving any still-relevant rationale. Decide whether altitude keys also
pan, then test and document the chosen interaction. Do not remove the active
player marker or projection code as a consequence of removing comments.

### V6 — Free-camera projection switching does not reconcile framing

**Classification:** behavior candidate. **Confidence:** static calculation;
the transition has not been reproduced in this audit.

[Projection switching](../../dynamapper/src/core/render/scene/camera.rs#L373)
replaces the projection without changing the camera transform. Follow mode
subsequently computes an appropriate pose, but free mode retains its pose. At
the normal initial orthographic offset, the camera is much closer to its focus
than the [perspective orbit distance](../../dynamapper/src/core/render/scene/camera/perspective.rs#L23)
for a typical window and zoom. Switching a free view can therefore change its
framing substantially.

**Follow-up:** establish the intended free-camera switching behavior and test
both directions after pan and rotation. Any adjustment must respect the user's
free pose; simply resetting it to the follow camera is not equivalent.

### V7 — Camera graphics updates observe the entire settings resource

**Classification:** unnecessary coupling; performance impact unverified.
**Confidence:** broad change gate confirmed; update frequency is an inference.

[Camera AA updates](../../dynamapper/src/core/render/scene/camera.rs#L96) run
when any `Settings` field changes and issue component insert/remove commands.
The [settings synchronization system](../../dynamapper/src/configs/settings.rs#L705)
obtains mutable settings during fixed updates; whether that causes repeated
component updates in normal operation should be measured or checked with a
focused change-detection test before estimating a cost.

**Follow-up:** if repeated or unrelated updates are confirmed, narrow the
trigger to the effective AA mode and retain matching sample counts for both
cameras. Review the cause together with `SettingsPlugin`.

### Camera validation

`cargo test -p dynamapper core::render::scene::camera::perspective::tests`
passed: 2 tests for orbit scale and lens/frustum/subview consistency. They do
not cover the streaming mismatch, startup order, focus or free-camera controls.
No application behavior was changed and no visual comparison was performed.

## ScenePlugin

Reviewed scene registration, viewport-to-chunk calculations, chunk lifecycle,
scale handoffs, queues, diagnostics, and export's footprint and readiness
callers. Child plugins are tracked separately in the coverage list.

### S1 — Chunk selection omits partial chunks at facet edges

**Classification:** likely rendering and export defect. **Confidence:**
selection arithmetic confirmed; rendered impact not reproduced here.

Both [interactive selection](../../dynamapper/src/core/render/scene.rs#L181)
and [export selection](../../dynamapper/src/core/render/scene.rs#L262) floor
map dimensions to base chunks and only emit super-chunks that fit completely.
The base chunk side is [32 tiles](../../dynamapper/src/core/render/scene/world/land.rs#L23).
For stock [Tokuno's 1448 × 1448 tiles](../../lib/uocf/src/enhanced/maps/facet_sector.rs#L71),
the last eight tiles on each edge are therefore not selected even at scale 1.
Larger super-chunks can omit wider strips; for example, a 1600-tile dimension
at scale 4 leaves the final 64 tiles uncovered. Export readiness calls the
same selection helper, so an empty or incomplete required set can appear ready.

**Follow-up:** determine how the land mesh, metadata atlas and statics handle
partial edge chunks, then support all in-bounds tiles without reading past the
map. Test stock and custom dimensions at all supported scales, both interactive
and export paths, and make readiness independently detect missing edge coverage.

### S2 — An unread chunk-target resource is maintained at runtime

**Classification:** confirmed unused state in this binary and repository.
**Confidence:** repository-wide symbol search.

[`VisibleLandChunkTargets`](../../dynamapper/src/core/render/scene.rs#L28) is
initialized and [rebuilt](../../dynamapper/src/core/render/scene.rs#L786), but
no reader was found anywhere in the repository. The public item does not make
the binary crate a supported library API by itself.

**Follow-up:** recheck current references and remove the resource, its record
type and producer if no consumer has been added. Retain the distinct committed
and transition lists needed by the streaming state machine and diagnostics.

### S3 — A handoff retargeting branch is unreachable

**Classification:** confirmed dead branch. **Confidence:** control-flow proof.

Immediately before the [retarget branch](../../dynamapper/src/core/render/scene.rs#L586),
a changed scale sets the target to `desired_scale`; an unchanged scale clears
any existing target. Thus a present target cannot differ from `desired_scale`
at that branch. Retargeting itself currently happens in the preceding assignment,
which logs it as a handoff start.

**Follow-up:** remove or restructure the unreachable branch without losing
retarget behavior. If distinct start, cancel and retarget events are useful,
compare against the previous target before overwriting it. Test rapid scale
changes and reversal during an in-flight handoff.

### S4 — Pending retirements are not reconciled with later targets

**Classification:** conditional correctness risk. **Confidence:** static
queue tracing; triggering conditions and visual effect not reproduced.

The [retirement queue](../../dynamapper/src/core/render/scene.rs#L317) persists
across frames and is drained in bounded batches. On recomputation,
[currently needed chunks](../../dynamapper/src/core/render/scene.rs#L619) are
counted as present, but an entity already queued for retirement is not removed
from that queue if it becomes needed again. With a queue larger than one batch,
a rapid target or scale change can therefore retire a newly required chunk
after its presence prevented a replacement spawn.

**Follow-up:** exercise a multi-frame retirement queue with a changed or
reversed target and verify coverage settles without another input event. Cancel
obsolete retirements or otherwise reconcile queue membership with the latest
required set; preserve the bounded handoff and no-clear behavior.

### S5 — Scene registration and chunk-streaming policy share one large system

**Classification:** separation and testability. **Confidence:** confirmed.

[`sys_update_worldmap_chunks_to_render`](../../dynamapper/src/core/render/scene.rs#L373)
combines player/map tracking, footprint selection, scale transition state,
queue planning, entity mutation, telemetry and export override. `ChunkScale`
is defined in the land mesh module but updated here and consumed elsewhere.
The diagnostic target count is also derived from the last logged target, tying
telemetry to log-deduplication state. Resize is registered in `FixedUpdate`,
whereas ordering between the similarly named scene sets is configured for
`Update`; labels across schedules do not by themselves establish ordering.
The resize handler also updates saved window dimensions, which the settings
plugin separately [synchronizes from the window](../../dynamapper/src/configs/settings.rs#L705).

**Follow-up:** give chunk streaming and its transitions an explicit owner, with
testable footprint and queue-planning operations, and keep scene composition
and diagnostics at their own boundaries. Preserve mesh-ready scale handoff,
export footprint and the stronger export upload/readiness checks. Make the
fixed/main schedule relationship explicit rather than relying on set names.

### S6 — Chunk-size comments describe superseded geometry

**Classification:** stale implementation guidance. **Confidence:** confirmed.

[Scene comments](../../dynamapper/src/core/render/scene.rs#L228) and
[land mesh comments](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L47)
describe 8-tile base chunks, older mesh sizes and older zoom thresholds, while
the current base chunk is 32 tiles and
[`scale_from_zoom`](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L168)
uses different thresholds. This makes the handoff harder to reason about.

**Follow-up:** verify the actual mesh dimensions and update or remove the stale
comments during the land plugin audit; do not infer that the current geometry
itself should be changed from those comments.

### S7 — Recompute requests are consumed one at a time

**Classification:** redundant reconciliation candidate. **Confidence:**
confirmed iterator behavior; frequency and performance cost unmeasured.

[Chunk sync](../../dynamapper/src/core/render/scene.rs#L417) calls
`event.read().next().is_some()`. Bevy's message iterator advances its cursor as
items are read, so this consumes only one request from a burst. Multiple
resize, zoom or movement requests can trigger reconciliation in subsequent
frames even though the system calculates from the latest state each time.

**Follow-up:** if these messages remain pure invalidation signals, consume
and coalesce the full unread batch before calculating once. Add a focused test
for multiple requests and preserve the pending queue's independent progress.

No focused tests for the scene footprint, handoff and retirement helpers were
found in `scene.rs`. The existing perspective and controls tests do not establish
these streaming cases. No application code was changed or visually compared.

## PlayerDynamicLightPlugin

Reviewed registration, the spawned Bevy light and its component, and nearby
material and lighting consumers. Other lighting plugins and shader paths remain
to be reviewed separately.

### L1 — A directional light is presented as a player-following dynamic light

**Classification:** misleading ownership and naming; removal candidate
requiring receiver review. **Confidence:** startup-only behavior confirmed;
the light's full visual impact has not been measured.

[`PlayerDynamicLightPlugin`](../../dynamapper/src/core/render/scene/dynamic_light.rs#L16)
registers one startup system. It creates a Bevy `DirectionalLight` whose
orientation is determined from the initial player position; no subsequent
system follows the player, camera, time or lighting settings. A directional
light's direction comes from orientation, rather than following its translation.
The world terrain and art have their own custom lighting paths; for example,
the [land material](../../dynamapper/src/core/texture_cache/land.rs#L1426)
is unlit from Bevy's standard material perspective and receives explicit
shader lighting. The optional green player marker uses a lit `StandardMaterial`,
so the spawned light itself must not be declared dead solely from its name.

**Follow-up:** inventory all lit Bevy material receivers and decide whether to
retain this as an explicitly named scene or diagnostic light, scope it to a
marker, or remove it together with the need for it. Preserve active custom
terrain, sprite and static-light rendering. Validate appearance if changing
the actual light; this audit did not visually compare it.

### L2 — The attached light component has no reader

**Classification:** confirmed unused component state and inactive source
clutter. **Confidence:** repository-wide symbol search.

[`PlayerDynamicLight`](../../dynamapper/src/core/render/scene/dynamic_light.rs#L6)
stores a relative vector that is read locally to construct the initial light
transform, then attached to the entity. No active system queries the component
or its field afterwards. The file also contains a commented-out point-light
experiment and old query fragments.

**Follow-up:** if the directional light remains, keep any needed initial vector
local and remove the unused component and inactive fragments after rechecking
references. Do not remove the live light entity merely because its marker is
unused. No focused tests for this plugin were found.

## WorldPlugin

Reviewed the plugin's composition and the construction and use of its map
catalog. Its terrain and art children have not yet been audited as part of this
section.

### W1 — Shared map catalog is owned by the render scene and duplicates metadata

**Classification:** cross-subsystem ownership. **Confidence:** confirmed.

[`WorldGeoData`](../../dynamapper/src/core/render/scene/world.rs#L15) is a
startup snapshot of map identities and dimensions derived from
`MapPlanesRes`. Movement, teleportation, export and rendering all consume it,
so it is broader than a render-scene concern. `MapPlane` already exposes
[`size_cells`](../../dynamapper/src/core/maps.rs#L96); the catalog and other
callers separately multiply block dimensions by eight.

The [loader](../../dynamapper/src/core/uo_files_loader.rs#L544) currently opens
only the session's starting facet; the catalog consequently contains only that
loaded facet, and teleport rejects unloaded maps. This is an active limitation,
not evidence that the catalog is unused. Population happens once at startup
after loading; it is not a dynamic map-loading mechanism.

**Follow-up:** establish a world-data owner for the facet catalog and loading
lifecycle, with one authoritative definition of dimensions and clear sparse
facet identity. Preserve existing startup ordering and loaded-map validation.
Adding dynamic facet loading is a separate feature, not a prerequisite for
moving the current catalog.

### W2 — Map metadata repeats an unread identifier

**Classification:** unused field and redundant resource marker candidate.
**Confidence:** repository-wide search of current consumers.

[`MapPlaneMetadata.id`](../../dynamapper/src/core/maps.rs#L15) is populated in
the map catalog and one test, but no reader was found; the catalog key already
contains the same map identifier. The type derives `Resource`, but no system
was found that inserts or requests it as a standalone Bevy resource.

**Follow-up:** recheck references when acting, then remove the unused field and
standalone resource derive if they still have no consumer. Retain the catalog
itself and its dimensions.

### W3 — Missing startup map is treated as optional until scene use

**Classification:** startup error-path gap. **Confidence:** static path traced;
runtime ordering and error output have not been reproduced.

The [loader](../../dynamapper/src/core/uo_files_loader.rs#L592) logs a missing
selected map but still publishes the map-plane resource. World catalog setup
then accepts an empty catalog, while
[scene synchronization](../../dynamapper/src/core/render/scene.rs#L454) expects
metadata for the selected map and panics if it is absent. Other required asset
errors can occur earlier, so the exact observed error depends on the inputs.
The loader iterates only its supported facet range; a saved facet outside that
range selects no map and does not enter the missing-file branch.

**Follow-up:** define the selected startup map as a required input and report a
clear startup error at the loading boundary. Test missing and malformed selected
maps and an out-of-range saved facet separately from intentionally absent,
unloaded facets.

No focused tests for the world catalog setup were found. No application code
was changed or visually compared.

## DrawLandChunkMeshPlugin — partial review

The geometry and background-loading pass is recorded below. The plugin remains
**in progress**: its complete material, atlas, picking and shader dependencies
still need review, as do the separate land texture-cache and profiling plugins.
Current source-transition controls are active, and dynamic blending remains
unfinished; neither is a candidate for deletion just because this area has
changed during development.

### T1 — Chunk construction sorts camera and chunks in different units

**Classification:** incorrect priority calculation. **Confidence:** units
confirmed statically; effect on observed load order not measured.

[Camera position](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L441)
is divided by eight before it is passed to `sort_construction_targets`, but the
target coordinates are `LCMesh.gx/gy` in the
[32-tile base chunk grid](../../dynamapper/src/core/render/scene/world/land.rs#L23).
[The sort](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L1191)
compares those values directly. Away from the origin, the claimed nearest-camera
ordering is centered at a different location.

**Follow-up:** use an explicit shared conversion to the logical chunk grid and
test priority at representative positive and negative camera positions and chunk
scales. Measure load order if assessing a performance impact.

### T2 — Background texture warming does not warm textures

**Classification:** confirmed inert work and misleading diagnostics.
**Confidence:** worker and request traced.

The [loader request](../../dynamapper/src/core/render/scene/world/land/chunk_loader.rs#L36)
carries a Classic land texture package and describes warming its cache. The
worker instead [only counts distinct tile IDs](../../dynamapper/src/core/render/scene/world/land/chunk_loader.rs#L159);
it never reads that package. Its performance log nevertheless calls the count
textures warmed. This scan does not establish texture residency or loading.

**Follow-up:** remove the unused request field, counting scan and false warming
claim if current profiling confirms the count has no diagnostic purpose. Keep
the active asynchronous map-block loader and the separate live texture cache
work on the draw path.

### T3 — Land draw retains unused prototype state

**Classification:** confirmed unused or inert code. **Confidence:** symbol and
control-flow searches.

[`_Arrayable`](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L275)
has no call site. `DrawMeshLocals.pending_blocks` is never used, and
[`last_scale`](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L298)
is only compared and assigned to itself; it does not perform the pin-clearing
work its comment describes. The file has a broad `allow(unused)` that hides
such findings. `PendingTextureBake` is active and must be retained.

**Follow-up:** remove the proven inert items and obsolete comments, then narrow
the unused-code allowance after reviewing remaining warnings. Preserve the
separate live pin-reset system and rebake signaling.

### T4 — Loader failure and completion have no explicit error contract

**Classification:** stalled-streaming risk. **Confidence:** conditional static
trace; fault cases not reproduced.

The [worker](../../dynamapper/src/core/render/scene/world/land/chunk_loader.rs#L149)
prints a map read error and returns an empty sub-batch as though completed. The
[draw system](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L498)
may then retain a matching loaded-version checkpoint and skip rechecking core
blocks until another successful insertion changes that version. Separately,
`send_request` discards a channel send failure while the caller unconditionally
marks the request pending. A failed worker can consequently leave chunks waiting
without a structured error or completion. No claim is made about frequency.

**Follow-up:** make worker submission and results fallible, establish bounded
retry or clear failure behavior, and cover failed reads and disconnected
workers with targeted tests. Associate results with a request and facet before
supporting asynchronous facet changes; today only the starting facet loads.

### T5 — Low-detail LOD may be unreachable under normal scale scheduling

**Classification:** dead-path candidate. **Confidence:** threshold and
scheduling analysis; startup and transition cases require a focused test.

[`lod_from_zoom`](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L133)
selects `Low` at zoom 10 or greater, while `scale_from_zoom` selects a scale
greater than 1 from zoom 5. The LOD update returns when the current chunk scale
is not 1. Under normal synchronized scene updates this makes `Low` appear
unreachable, although the handle is generated and the picking test exercises its
topology. Startup, rapid zoom and handoff ordering could affect that conclusion.

**Follow-up:** test actual system scheduling at thresholds and during handoffs
before removing `Low`, its mesh or its picker entry. Retain the shared mesh-grid
contract for every mesh that remains live.

### T6 — LOD update and new mesh attachment lack an explicit ordering

**Classification:** schedule-dependent defect candidate. **Confidence:**
systems and data flow traced; Bevy scheduling and visual effect not reproduced.

[The plugin](../../dynamapper/src/core/render/scene/world/land.rs#L573)
registers the LOD updater and chunk draw in the same set without an ordering
between them. Drawing reads the LOD resource and queues new mesh attachments;
the updater changes the resource and updates existing mesh components. If draw
observes the old LOD before the updater and its deferred attachment is not
visible to the update query, a newly attached mesh could retain the old LOD
after the updater considers the resource current.

**Follow-up:** reproduce with a focused ECS test covering a new chunk at a LOD
threshold and actual deferred-command application, then establish an ordering
or another explicit invariant if needed. Preserve delayed LOD-aware picking.

### T7 — Missing EC package reaches a Classic fallback path

**Classification:** policy discrepancy to verify. **Confidence:** static
routing traced; actual rendered result not checked in this audit.

The [land draw](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L755)
uses the legacy texture-array path when the selected EC land package is absent.
The [source-switch warning](../../dynamapper/src/core/texture_cache/land.rs#L1190)
also describes a fallback. This appears inconsistent with the current
missing-Enhanced-texture policy in `ENGINEERING.md`; it differs from the case
where an EC package is present but an individual slot is missing, which uses
the explicit missing mode.

**Follow-up:** trace startup and source switching in `LandTextureCachePlugin`,
test both a wholly absent package and an absent slot, and settle the intended
policy before removing any cache path. Do not silently replace missing EC
textures with Classic texmaps or remove active Classic and EC atlas paths.

`cargo test -p dynamapper core::render::scene::world::land::picking::tests`
passed all 8 tests, including generated mesh-grid consistency. These tests do
not cover T1–T7, worker failures, the scene edge issue S1 or transition
scheduling. No visual comparison was performed.
