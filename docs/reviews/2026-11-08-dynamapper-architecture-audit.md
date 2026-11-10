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
| `DrawLandChunkMeshPlugin` | Reviewed | Findings T1–T16; other texture-cache plugins and the independent shader/asset sweep remain pending. |
| `LandProfilingPlugin` | Reviewed | Findings GP1–GP4 below; feature-gated GPU execution was not run. |
| `RenderPlugin` | Reviewed | Composition reviewed; child plugins remain separately tracked. |
| `FogPostProcessPlugin` | Reviewed | Findings F1–F5 below; no visual validation. |
| `LandTextureCachePlugin` | Reviewed | Findings LC1–LC11 below; runtime fault and device tests remain follow-up work. |
| `TextureCachePlugin` | Reviewed | Composition reviewed; art child and direct installers remain separately tracked. |
| `ArtPageFormatCheckPlugin` | Reviewed | Finding AP1 below; art atlas implementation remains separately tracked. |
| `DrawStaticSpritesPlugin` | Reviewed | Findings A1–A16 below; independent shared-shader and asset sweep remains pending. |
| Overlay plugins | Reviewed | Composition and findings O1–O8 below. |
| Dialog plugins and paperdoll support | Reviewed | Findings D1–D12 below. |
| Remaining scene plugins | Pending | Includes world, terrain, statics, lights and effects. |
| Dialog and overlay plugins | Pending | Includes input and player interactions already noted below. |
| Settings and shader preset plugins | Pending | Includes configuration ownership and persistence. |
| `UOFilesPlugin` | Reviewed | Findings U1–U3 below; W1/W3 cover map catalog and missing-map behavior. |
| Texture caches and export | Pending | Trace remaining runtime owners and conditional paths. |
| Frame rate, frame limiter and diagnostics | Reviewed | Findings R1–R4 below; D8–D10 cover preferences and limiter policy. |
| Application wiring, non-plugin modules, build, shaders and runtime assets | Pending | Account for files not owned by a reviewed plugin. |

Plugin coverage is tracked individually so reviewing an orchestration plugin
does not imply its children have been reviewed:

- Reviewed: `ControlsPlugin`, `PlayerMovementPlugin`, `WindowControlsPlugin`,
  Unix `DevControlPlugin`, `PlayerPlugin`, `CameraPlugin`, `ScenePlugin`,
  `PlayerDynamicLightPlugin`, `WorldPlugin`, `DrawLandChunkMeshPlugin`,
  feature-gated `LandProfilingPlugin`, `RenderPlugin`, `FogPostProcessPlugin`,
  `LandTextureCachePlugin`, `TextureCachePlugin`, `ArtPageFormatCheckPlugin`,
  `OverlaysPlugin`, `PerformanceOverlayPlugin`, `PlayerPositionOverlayPlugin`,
  `CursorBehaviorOverlayPlugin`, `SystemMessagesPlugin`, `DialogsPlugin`,
  `KeybindingsHelpPlugin`, `TeleportPlugin`, `LandUiPlugin`, `GumpDialogPlugin`,
  `PreferencesDialogPlugin`, `UOFilesPlugin`, `FrameRatePlugin`,
  `FrameLimiterPlugin`, `LogGpuPreprocessingModePlugin`.
- Reviewed scene/rendering child: `DrawStaticSpritesPlugin`.
- Pending data/runtime: `ExternalDataPlugin`, `SettingsPlugin`,
  `ShaderPresetsPlugin`, `ExportJobPlugin`.

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

## DrawLandChunkMeshPlugin

Reviewed registration, geometry and background loading, material and uniform
ownership, the metadata atlas and its upload path, terrain picking, and the land
shader entry points and imports. The separate land texture-cache and profiling
plugins and the independent shader/asset sweep remain pending. Current source
transition controls are active, and dynamic blending remains unfinished; neither
is a candidate for deletion because this area has changed during development.

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

**Classification:** policy discrepancy. **Confidence:** Classic source routing
confirmed statically; actual rendered result not checked in this audit.

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

### T8 — Reused and partially initialized atlas pages can expose old or zero cells

**Classification:** metadata validity gap. **Confidence:** CPU page and
mapping behavior confirmed by tracing; effects on a rendered frame or cursor
have not been reproduced.

When an LRU layer is [assigned to a different logical page](../../dynamapper/src/core/render/scene/world/land/tile_atlas.rs#L263),
its existing CPU mirror and untouched GPU texels are not cleared or marked
invalid. The draw caller ignores the returned evicted page. An unwritten cell
on the new page can consequently retain the old page's data. Similarly,
[a new partially written mirror](../../dynamapper/src/core/render/scene/world/land/tile_atlas.rs#L353)
contains zeroes for all cells not yet written, and
[`read_cell`](../../dynamapper/src/core/render/scene/world/land/tile_atlas.rs#L225)
does not distinguish those cells from valid metadata. Terrain picking and
static-ground decisions read this mirror.

**Follow-up:** test first partial writes, LRU reuse and outstanding staged GPU
uploads for the same physical layer, including neighboring cells and resize.
Introduce a coherent CPU/GPU validity or clearing policy that preserves upload
ordering and revision/readiness semantics; do not assume clearing only the CPU
mirror fixes the rendered atlas.

### T9 — Terrain view-dependent shading receives a fixed camera position

**Classification:** cross-subsystem data flow; likely shading defect.
**Confidence:** writes and shader reads traced; visible effect and desired
orthographic behavior are unverified.

The land `SceneUniform.camera_position` is
[initialized](../../dynamapper/src/core/texture_cache/land.rs#L1439) to a fixed
player-camera offset. No subsequent land-material write to that field was found;
art-material writes target different materials. The land shader
[subtracts each world position from it](../../dynamapper/assets/shaders/world/land/main.wesl#L658)
to form a view vector used by applicable shading paths.

**Follow-up:** determine the correct view vector for both perspective and
orthographic projections and provide current camera/view data at an appropriate
rendering boundary. Avoid per-frame material re-extraction solely for camera
movement. Test movement, translation and projection changes before changing
shading; the presence of a view-vector calculation alone does not establish a
visible difference for every preset.

### T10 — Terrain uniforms are owned partly by a dialog

**Classification:** misplaced rendering responsibility and ordering risk.
**Confidence:** ownership confirmed; the ordering-dependent behavior requires
a focused test.

Land material creation lives in the [texture cache](../../dynamapper/src/core/texture_cache/land.rs#L1426),
some updates and simplification overrides in the
[land plugin](../../dynamapper/src/core/render/scene/world/land.rs#L209), and
the always-scheduled [dirty-uniform push](../../dynamapper/src/core/render/dialogs/land_shader.rs#L903)
in the shader dialog. The latter and the simplification override write some of
the same fields without an explicit ordering between them. The dialog push also
requests a material-handle query it does not read.

**Follow-up:** put effective material-uniform application under a rendering
owner, with an explicit contract for preset updates and diagnostic overrides.
Test their simultaneous changes, preserve transition flags and export behavior,
and retain the guarded material mutation that avoids needless GPU re-extraction.
Remove unused system parameters only after checking their scheduling effects.

### T11 — Cache pinning uses logical chunk coordinates as native block coordinates

**Classification:** incorrect unit conversion and cache-lifetime risk.
**Confidence:** static code trace; user-visible impact not reproduced.

The [active-texture pinning system](../../dynamapper/src/core/texture_cache/land.rs#L1021)
uses `LCMesh.gx/gy` directly as `MapBlockRelPos`, and iterates only `scale`
blocks per dimension. Those coordinates and scale describe 32-tile chunks,
while map blocks are 8 tiles; the [land draw](../../dynamapper/src/core/render/scene/world/land/draw_mesh.rs#L488)
correctly multiplies the origin and span by `CHUNK_STORAGE_BLOCKS_DIM`.
The same pinning pass touches map blocks to refresh their idle time, while the
cache later evicts idle blocks. Consequently the actual visible blocks may not
be refreshed, and texture IDs can be pinned from unrelated positions.
Terrain picking obtains source data through `block_no_update`, so its behavior
after eviction also merits testing.

**Follow-up:** share one conversion and range calculation for chunk-to-native
blocks, including super-chunks and borders. Test pinned IDs and block lifetime
at nonzero origins and multiple scales, plus terrain inspection after idle
eviction. Review this again as part of `LandTextureCachePlugin`.

### T12 — Atlas shrink measures historical mappings rather than live demand

**Classification:** ineffective resource-lifecycle policy. **Confidence:**
mapping lifecycle traced; memory and performance impact unmeasured.

The [shrink heuristic](../../dynamapper/src/core/texture_cache/land.rs#L1145)
uses the number of mapped pages divided by capacity. Atlas mappings grow on
first use, are replaced one-for-one by LRU eviction at capacity, and otherwise
clear on resize; they are not removed merely because the visible region shrinks.
Thus a drop in view demand does not itself lower a previously high ratio.

**Follow-up:** decide whether shrink should use current/recent page demand or
whether the heuristic should be removed. Test a high-demand view followed by a
small one, preserving resize generation and upload correctness.

### T13 — Custom facet dimensions can alias the atlas page table

**Classification:** size-contract gap. **Confidence:** indexing confirmed;
supported custom-map limits and visual consequences need verification.

The atlas uses [16 page columns of 2048 tiles](../../dynamapper/src/core/texture_cache/land/texture_array.rs#L43).
Both [CPU lookup](../../dynamapper/src/core/render/scene/world/land/tile_atlas.rs#L225)
and [shader lookup](../../dynamapper/assets/shaders/world/land/atlas.wesl#L23)
flatten page coordinates without checking that `page_x` is below the stride.
For example page `(16, 0)` has the same flattened index as `(0, 1)`; flattened
indices beyond the 256-entry table are unavailable. The map reader's
[dimension validation](../../dynamapper/src/core/maps.rs#L288) checks block
alignment rather than that atlas limit. Stock facets fit the stated page grid.

**Follow-up:** define and enforce supported custom facet dimensions or expand
the addressing scheme. Test both axes at the boundaries with matching CPU and
shader behavior. Do not assert full support for larger custom facets based only
on successful map-package loading.

### T14 — Material contracts and unused declarations are mixed with terrain

**Classification:** shared ownership and confirmed unused declarations.
**Confidence:** repository-wide symbol checks; uniform-layout changes require
coordinated validation.

[`mesh_material.rs`](../../dynamapper/src/core/render/scene/world/land/mesh_material.rs#L133)
defines uniform structures also consumed by art materials and shader presets.
The Rust field `GlobalLightingUniforms._pad2_.x` is populated with a live art
water-diffuse value; it is not spare padding despite its name. Conversely,
`LandShaderMode` and the Rust `DEFAULT_KR_TONEMAP_GAIN` have no active readers;
presets supply the current gain. `AtlasParams.tiles_per_page` has no field read
found in Rust or WESL, but is present in the live uniform layout on both sides.

**Follow-up:** put shared material contracts under a shared rendering owner and
name active lanes for their role. Remove truly unused declarations after a fresh
search. Treat field removals and renames as Rust/WESL ABI changes, including
serialization and binding tests; never delete or repurpose `_pad2_.x` as if it
were unused. Comments claiming normal maps or light decals are only future
fields should be corrected because those controls have active consumers.

### T15 — Land shader keeps old grid assumptions and unused declarations

**Classification:** obsolete shader constants and a live unit mismatch.
**Confidence:** references confirmed; visual effect of the live mismatch is
not established.

[`DATA_GRID_BORDER`, `DATA_GRID_SIDE` and `MESH_GRID_SIDE`](../../dynamapper/assets/shaders/world/land/land_bindings.wesl#L104)
have no readers found. The neighboring live constant and
[`chunk_edge_blend_factor`](../../dynamapper/assets/shaders/world/land/atlas.wesl#L138)
still use an eight-tile period, whereas the current logical chunk is 32 tiles
and wider meshes have different spans. The helper affects normal blending, so
the mismatch is not evidence that it can simply be deleted or changed.

**Follow-up:** remove only confirmed unused declarations; investigate what
the edge blend is meant to protect and test it across current mesh scales
before changing its period. Land `main.wesl` also contains substantial EC
composition and shading despite describing itself as entry points only; consider
extracting cohesive logic while preserving derivative and shader-import rules.
Both Classic and EC transition modules are active.

### T16 — The GPU FSR shader has no discovered caller

**Classification:** unused asset candidate. **Confidence:** repository-wide
name and import search; external loose-asset consumers not ruled out.

[`fsr_easu.wesl`](../../dynamapper/assets/shaders/postprocess/upscaling/fsr_easu.wesl)
is not imported or loaded by any discovered runtime or test path. Its own
comment describes a call from land sampling that no longer exists. The separate
CPU FSR implementation in `uot-image-postprocess` has active consumers and is
not part of this candidate.

**Follow-up:** verify that no supported external asset workflow loads this
loose shader, then remove it or explicitly document its intended future use.
Review remaining standalone shader assets in the independent asset sweep.

Additional checks: the focused atlas staged-revision/image-generation test,
`land_sampling_shader_composes`, and
`world_entry_points_discover_shader_dependencies` each passed. They do not
exercise page reuse, the live pinning calculation, camera-view data or visual
output. No visual comparison was performed.

## LandProfilingPlugin

Reviewed the `gpu-profiling` feature, registration, main/render-world request
flow and Core3d instrumentation. This is an active opt-in diagnostics feature,
not unused terrain code.

### GP1 — One export request continually re-arms itself

**Classification:** feature-gated behavior defect. **Confidence:** static
cross-world trace; no GPU export was run.

[F10 sets the main-world request](../../dynamapper/src/core/render/scene/world/land/profiling.rs#L112),
and no writer clears it. Every extraction therefore sets the separate
render-world request again; [finalization](../../dynamapper/src/core/render/scene/world/land/profiling.rs#L174)
clears only the latter. Completed profiler frames can keep producing trace
files after a single press. Names have one-second resolution and the profiler's
writer creates/truncates its target, so repeated writes in the same second can
replace the previous trace.

**Follow-up:** implement a one-shot request or acknowledged request sequence
across the two worlds, preserving requests until GPU results are ready. Test one
and multiple presses, delayed results, write failure and filename collisions.

### GP2 — Profiling scope is wider than land and narrower than a whole frame

**Classification:** misleading ownership and measurement name. **Confidence:**
registration and schedule placement confirmed; multi-view behavior not exercised.

The plugin is under the terrain module but is registered by
[`RenderPlugin`](../../dynamapper/src/core/render.rs#L15) and brackets Bevy's
[`Core3dSystems::MainPass`](../../dynamapper/src/core/render/scene/world/land/profiling.rs#L63),
not a land-only pass or the entire frame. The query is labeled `Total Frame`.
Core3d runs per camera, and the begin/end systems do not filter to the player
camera; comments claiming only the first view is timed are not supported by an
explicit per-frame selection contract.

**Follow-up:** place the tool under renderer diagnostics, name the measured
span accurately and define whether to capture one view or all views. Add an
explicit view identity/selection and multi-view test if multiple cameras are
supported. Preserve the distinction from Bevy's separate hardware diagnostics.

### GP3 — Unsupported-device behavior does not match the fallback message

**Classification:** startup contract mismatch. **Confidence:** configuration
and renderer initialization traced; no unsupported adapter was tested.

With the feature enabled, [device settings](../../dynamapper/src/core.rs#L163)
request both timestamp-query features as required features before the plugin's
[`finish` check](../../dynamapper/src/core/render/scene/world/land/profiling.rs#L78)
can warn and disable profiling. An adapter that cannot satisfy the device
request may fail before that warning. The module also retains outdated API and
render-graph terminology, and its F10 binding is outside the central input
policy reviewed in C4. Its query writes on a command encoder, but initialization
neither explicitly requests nor checks `TIMESTAMP_QUERY_INSIDE_ENCODERS`;
whether Bevy enables it implicitly depends on adapter features and settings
priority. The profiler can omit encoder timing if it is unavailable.

**Follow-up:** choose whether a profiling build requires capable hardware or
negotiates optional support, and align initialization and diagnostics with that
choice. Correct outdated comments and include the shortcut in the shared input
policy. Exercise supported, unsupported and differing device-priority paths
where feasible, including whether the resulting trace contains GPU timestamps.

### GP4 — Query begin and end use separate system-local render contexts

**Classification:** profiler API contract risk. **Confidence:** API and Bevy
system parameters inspected; runtime consequence not reproduced.

The [begin and end systems](../../dynamapper/src/core/render/scene/world/land/profiling.rs#L122)
each receive their own `RenderContext` system parameter. In the installed Bevy
version that parameter owns system-local deferred encoder state. The installed
`wgpu-profiler` API documents that a query must be ended using the same command
encoder or pass on which it began. Splitting the calls between systems therefore
appears to violate that contract; this audit did not establish what timestamps
or validation errors the current arrangement produces. Finalization also
discards the `end_frame` result, which could hide profiler errors.

**Follow-up:** use a profiler-supported way to bracket the intended pass and
surface query/finalization errors. Test a real capture, including multiple views,
before treating the resulting durations as trustworthy.

`cargo check -p dynamapper --features gpu-profiling` passed. It proves the
feature compiles, not that GPU timestamps or trace exports work. No focused
profiler tests or runtime GPU export were found or run.

## RenderPlugin

[`RenderPlugin`](../../dynamapper/src/core/render.rs#L12) composes the scene,
overlays, dialogs and fullscreen fog plugins, and registers profiling only with
its feature enabled. Its registrations are active and occur once through the
application bootstrap. No dead wrapper code or demonstrated registration-order
defect was found. Profiling and fullscreen fog apply beyond land/world despite
their current paths; GP2 and the pending fog review cover their ownership.
The children are tracked individually rather than considered reviewed by this
wrapper pass. The feature-enabled check recorded above covers compilation, not
runtime initialization or rendered output.

## FogPostProcessPlugin

Reviewed the fullscreen pass, its camera component and uniform sync, the fog
shader and the surrounding land/art fog references. The active fog is a
screen-space approximation applied after composition, not the old world-space
helper; this audit makes no claim of original-client fidelity.

### F1 — Disabled fog still schedules a fullscreen pass

**Classification:** unnecessary work candidate. **Confidence:** Bevy pass and
shader paths traced; cost not measured.

The [camera always has the fog component](../../dynamapper/src/core/render/scene/camera.rs#L188),
and the plugin installs a [fullscreen material](../../dynamapper/src/core/render/scene/world/effects/fog_pass.rs#L38).
When fog is disabled, the [fragment shader](../../dynamapper/assets/shaders/world/effects/fog_pass.wesl#L124)
returns the sampled input, but Bevy still performs the postprocess write and
fullscreen draw once the pipeline is available. Export also disables fog in
settings; it does not remove this pass.

**Follow-up:** measure the cost, then consider a per-view run condition or
component lifetime that skips the pass when disabled. Test rapid toggles,
extraction, render-target ping-pong and export, while retaining pass order.
The active fog effect must not be removed merely because some presets disable it.

### F2 — Old fog shader and an exposed height-bias control are disconnected

**Classification:** superseded asset and no-op UI control. **Confidence:**
current shader references and UI binding traced.

[`legacy/world/fog.wesl`](../../dynamapper/assets/shaders/legacy/world/fog.wesl)
is explicitly unimported by active shaders and implements the older world-space
fog path. The [shader dialog](../../dynamapper/src/core/render/dialogs/land_shader.rs#L755)
still offers a Fog Height Bias that writes `gloom_params.w`; the legacy shader
reads that lane, but the active fullscreen shader does not. The current effect
uses screen-space radial and vertical terms instead. The existing
[shader-settings audit](../agent_notes/shader_settings_audit.md) also records
the unused control and the dependency of cloud noise on nonzero fog density.

**Follow-up:** remove the superseded shader after checking asset-discovery
workflows, and update live references. Decide whether to remove the no-op
control or implement a meaningful replacement; do not imply it currently changes
fog. Preserve existing preset and uniform layout semantics when changing fields.

### F3 — Diagnostic land override does not control the fullscreen fog pass

**Classification:** split ownership and unclear diagnostic contract.
**Confidence:** fields and consumers traced; intended override is undecided.

The [land simplification override](../../dynamapper/src/core/render/scene/world/land.rs#L244)
sets the land material's copied `enable_fog` field to zero. Actual fullscreen
fog [reads `UniformState.lighting` directly](../../dynamapper/src/core/render/scene/world/effects/fog_pass.rs#L90),
so that material assignment cannot disable the fullscreen pass. The assignment
may be a leftover from per-material fog or may reflect an incomplete diagnostic
intent; it does not establish which behavior was intended.

**Follow-up:** decide whether land-only simplification should leave global fog
on or suppress it, and express that policy in the effective pass settings.
Remove obsolete material writes only after tracing other users of the shared ABI.

### F4 — Fog ownership is global but located under world geometry

**Classification:** module location. **Confidence:** registration and scope
confirmed.

The [render root](../../dynamapper/src/core/render.rs#L19) registers fog; the
fullscreen pass covers the world camera's composed land and art and runs after
tonemapping. Its source lives under `scene/world/effects` and its component is
attached by the camera module. A render postprocess owner would better express
the scope while keeping the camera/view dependency explicit.

**Follow-up:** review this placement together with other render effects, not as
a mechanical rename. Preserve the camera component, preset synchronization,
post-tonemap order and export's disabled-fog policy.

### F5 — Fog controls and view-size assumptions need explicit boundaries

**Classification:** UI contract and conditional view limitation. **Confidence:**
shader and system inputs traced; alternate-target impact not reproduced.

The [fog shader](../../dynamapper/assets/shaders/world/effects/fog_pass.wesl#L91)
returns zero if both distance and height density are near zero, before applying
cloud noise. Noise strength alone therefore does not produce a visible veil,
despite a comment describing it as independent of distance. The effect uses
the [first window's dimensions](../../dynamapper/src/core/render/scene/world/effects/fog_pass.rs#L105)
for noise aspect, rather than the actual camera viewport or render target. The
existing export disables fog, so this is not a demonstrated export defect.

**Follow-up:** make the cloud-density dependency clear in the controls or
deliberately revise the effect, and derive view dimensions from the target if
fog is supported on cropped viewports or non-window targets. Test those cases
before changing the current fullscreen appearance.

No focused fog-system or fullscreen-pass tests were found. Prior shader and
feature checks do not establish fog runtime behavior; no visual comparison was
performed. Static fog does not need a screen-size refresh for its current math:
the shader uses that size only in the dynamic-noise branch, which the sync system
already refreshes each frame.

## LandTextureCachePlugin

Reviewed setup, fixed-array cache and upload, source switching, EC coverage and
auxiliary ownership, and compressed-format selection. Findings T7, T8 and
T10–T14 also concern this plugin. Fault-injection, GPU capability and visual
behavior remain follow-up work, not conclusions from this static review.

### LC1 — Full-collection residency is unreachable in current app wiring

[`sys_setup_terrain_cache`](../../dynamapper/src/core/texture_cache/land.rs#L1269)
always selects `LruCache`. Repository search found no caller constructing the
other strategy through `from_preload_enabled`; full-collection planning and
upload branches cannot run through current application setup. The dormant
[prewarm loop](../../dynamapper/src/core/texture_cache/land/cache.rs#L313)
repeatedly reads page zero while ignoring each texture ID, so it would not warm
the whole package if re-enabled. This is current-wiring reachability, not a
language-level impossibility.

**Follow-up:** remove the unused strategy and related scaffolding if full
preloading is no longer intended, or connect and test a real policy and prewarm
implementation. Preserve active LRU and page-atlas paths.

### LC2 — Speculative land-art loader and cache dependencies are unused

The [Classic art loader](../../dynamapper/src/core/texture_cache/land/cache.rs#L142)
explicitly has no caller and contains unvalidated geometry assumptions. The live
fixed-array loader ignores its preferred-source, Classic-art and EC-package
arguments and reads Classic texmaps. Several cache fields and arguments merely
pass these ignored values. The independently active Classic wet-land-art atlas
is a different path. T7 records the missing-EC source-policy discrepancy.

**Follow-up:** recheck callers, remove the speculative helper and unused cache
plumbing, and resolve T7 before simplifying the remaining fallback path. Do not
remove authored static transitions or the live wet-land-art path.

### LC3 — Fixed-array uploads can be lost or read the wrong staging offset

The [render uploader](../../dynamapper/src/core/texture_cache/land/cache.rs#L1039)
clears queued uploads if neither GPU image is ready, and drains an upload without
retry if its specific image is unavailable. It packs every upload into staging,
but advances the copy offset only for uploads whose image exists. If a skipped
upload precedes a ready one, the latter can copy the skipped upload's bytes.
These are statically traced conditional paths; their runtime reachability and
impact in the legacy/fallback array path are not established.

**Follow-up:** use image-specific pending queues and correctly associate each
copy with its packed offset. Test delayed preparation of both and either image,
mixed upload order and retries; coordinate with T7 before prioritizing changes.

### LC4 — Upload generation does not protect layer or image reuse

[`upload_generation`](../../dynamapper/src/core/texture_cache/land/cache.rs#L100)
is initialized to zero and compared when draining tasks, but no increment was
found. Uploads carry a generation, size and layer, without texture or image
identity. Layers can be reassigned and images replaced while tasks are pending,
so delayed results can target a different assignment or image. This race is a
static risk in the fixed-array path, not a reproduced corruption.

**Follow-up:** test delayed tasks across eviction and resizing; establish image
and per-slot identity/generations through both main and render worlds and reject
stale work. Retain the independent metadata-atlas generation safeguards.

### LC5 — Fixed-array eviction is not true least-recently-used ordering

Cache hits [update `last_touch`](../../dynamapper/src/core/texture_cache/land/cache.rs#L430)
but do not move the ID in the eviction queue. Pressure eviction removes from
the front, so it follows insertion order among unpinned entries. Idle eviction
leaves queue entries behind; an old entry for a subsequently reinserted ID can
refer to its new live allocation. Runtime impact was not measured.

**Follow-up:** choose and name the intended policy, and test cache hits, pinned
entries, idle eviction and reinsertion. T11 separately covers incorrect pinning.

### LC6 — Terrain owns shared item assets and unrelated map lifetime

[Terrain cache setup](../../dynamapper/src/core/texture_cache/land.rs#L1352)
also constructs the EC auxiliary atlas for item liquids, surfaces and hue masks.
The same plugin's periodic pass combines texture eviction, map-block eviction
and metadata-atlas shrink. These are active responsibilities, but their owners
cross terrain, item rendering and world data. EC ownership-stack preparation is
also split among the draw system, coverage cache and material updates.

**Follow-up:** establish shared item-aux and world-map lifetime owners and an
explicit EC ownership-cache publication contract. Preserve auxiliary deduplication,
source flags, full-halo checks, cache fallback, staged revisions and the
sentinel-image rebind described in [the cache note](../agent_notes/ec_terrain_stack_cache.md).
CPU and shader stack-layout constants are mirrored; add contract checks before
changing them. A conditional invalidation risk when the camera center ray misses
the ground needs a focused test before being treated as a witnessed defect.

### LC7 — Fallback assumptions and temporary diagnostics remain

The fixed-array [missing-texture fallback](../../dynamapper/src/core/texture_cache/land/texture_array.rs#L168)
assumes Classic texture `0x4c` exists and unwraps it; custom packages without it
can panic if this path is reached. Source switching still emits a block marked
[`TEMPORARY EC package diagnostic`](../../dynamapper/src/core/texture_cache/land.rs#L1232),
and unused system parameters and commented-out eviction experiments remain.
BC7 page selection also needs a device-capability review; DXT1/BC1 exists in
other project paths, and future mobile formats must not be treated as BC7-only.

**Follow-up:** test missing fallback slots and device capabilities, provide a
defined missing-source result, and remove or convert temporary diagnostics to
intentional logging. Do not silently substitute Classic for missing EC data.

`cargo test -p dynamapper core::texture_cache::land::tests` passed 5 tests with
one installed-package test ignored. Eight EC coverage CPU tests and two aux
layer tests also passed. These checks do not cover LC3–LC5 or establish GPU
parity; the explicit Vulkan parity test was not run. No visual comparison was
performed.

### LC8 — Fixed-array cache entries become visible before upload completion

The [cache miss path](../../dynamapper/src/core/texture_cache/land/cache.rs#L451)
extracts a Classic package subrectangle synchronously before spawning the upload
task; a cold page can be read and decoded on the Bevy thread. It then records
the nonzero layer as resident before the task or GPU upload completes. The draw
path considers only a zero fallback layer pending, so a failed or dropped upload
can leave a nonzero layer treated as ready without a retry. This applies to the
conditional fixed-array path, not necessarily the normal page-atlas route; no
timing or fault-injection result was obtained.

**Follow-up:** if retaining this path, separate allocation, loading, submitted
and ready states and keep decoding off the draw thread. Test read errors, delayed
and dropped uploads, retry and slot reuse, including LC3–LC4. Otherwise remove it
only after the missing-source contract in T7 is resolved.

### LC9 — Device format and auxiliary-array limits are not checked here

The [EC land atlas](../../dynamapper/src/core/texture_cache/land.rs#L793) and
[auxiliary array](../../dynamapper/src/core/texture_cache/land.rs#L206) choose
their GPU format from package data; no device block-compression feature check
was found in these construction paths. The auxiliary array also allocates one
layer per distinct source or crop without checking the adapter's layer limit.
Unsupported devices or unusually large packages may therefore lack a usable
fallback or actionable error. Actual device failures and stock-package limits
were not tested.

**Follow-up:** define capability negotiation, fallback decoding and clear errors
for supported formats and array limits, then test unsupported devices and large
inputs. BC7 is one current format; DXT1/BC1 is used elsewhere and future mobile
codecs require their own capability checks. Preserve per-source color space and
item-mask semantics.

### LC10 — Ownership-cache invalidation can be skipped when the view ray misses

[`sys_prepare`](../../dynamapper/src/core/texture_cache/land/coverage/cache/runtime.rs#L121)
returns for a missing viewport, near-horizontal ray or intersection behind the
camera before checking the atlas revision and unpublishing nonresident slots.
The shader's chunk/map/raw-owner guards do not independently validate every
neighbor in the baked halo. A previously published slot can consequently remain
published through those early returns while neighboring metadata changes; no
visible defect has been reproduced.

**Follow-up:** test a published cache entry with a center-ray miss and changed
neighbor payload or liquid flags. Separate mandatory invalidation/publication
from optional camera-centered baking if the test confirms the risk, retaining
the documented halo, submitted-revision and uncached-fallback contracts.

### LC11 — Source switching combines synchronous preparation and publication

The [source-switch system](../../dynamapper/src/core/texture_cache/land.rs#L1174)
selects source policy, rebuilds all page images and the lookup, updates the shared
material and requests chunk rebakes in one Update system. The image builder reads
or decodes every source page and rebuilds its lookup synchronously. Its observed
latency has not been measured. Repository searches also found no callers of
`residency_strategy`, `invalidate_all_cached_textures`, or
`reupload_all_resident_textures` in the fixed-array cache; resize uses a separate
active reupload method.

**Follow-up:** give source preparation and publication explicit boundaries and
measure switching before optimizing. Preserve atomic atlas/lookup pairing and
rebake invalidation. Recheck and remove the unused methods without removing the
active resize path.

## TextureCachePlugin

[`TextureCachePlugin`](../../dynamapper/src/core/texture_cache.rs#L21) is an
active composition wrapper: it registers the art format-check and land-cache
plugins once. No duplicate registration or demonstrated build-order defect was
found in the wrapper. Its comment says it allocates texture and tile caches,
but the art child only checks formats; actual art atlas setup and upload systems
are installed by [DrawStaticSpritesPlugin](../../dynamapper/src/core/render/scene/world/art.rs#L42).
This is a real ownership split, not evidence that either path is dead.

The re-exported residency layer is described as shared, but current repository
callers are terrain-specific and its preload path is unreachable in current
setup (LC1). Review whether the abstraction earns its complexity after the art
cache audit. Any reorganization must preserve atlas initialization, extraction
and upload scheduling. Art and item submodules remain separate audit items;
no runtime or visual validation was performed for this wrapper.

## ArtPageFormatCheckPlugin

This [startup plugin](../../dynamapper/src/core/texture_cache/art.rs#L695)
checks the loaded Classic and Enhanced art package page formats and warns when
they differ. It is active; no dead plugin registration or demonstrated format
selection defect was found in this review.

### AP1 — Art atlas format selection and its warning are separate policies

The [warning](../../dynamapper/src/core/texture_cache/art.rs#L483) compares both
packages at startup, while [sprite atlas selection](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L362)
independently chooses a common format based on the active source. This duplicates
the policy and can warn about an EC/KR mixture even in a Classic-only session.
Both paths currently treat an empty page list as RGBA. Current art package
manifests use a single package-wide page format, so selecting the first page is
not by itself evidence of mixed formats within a package. These package versions
support RGBA and BC7; this does not describe all block formats used by the project.

**Follow-up:** share or explicitly cross-test the actual atlas decision and its
diagnostic, including missing/empty packages and runtime source changes. Decide
when a warning is useful. Keep device-capability checks distinct from format
agreement and preserve active mixed-source conversion and source identity.

`cargo test -p dynamapper art_page_format_mismatch_names_both_packages_and_the_repack`
passed one test of the warning helper. It does not exercise plugin startup, GPU
support or actual atlas uploads. See A7–A8 for the subsequent atlas review.

## DrawStaticSpritesPlugin

Reviewed plugin composition, collection, atlas lifecycle, static lights, material
updates, art shaders and the GPU picking boundary. Shared shaders and assets are
also subject to the separate whole-application sweep.

### A1 — Art water can retain stale land diffuse strength

The [art material change gate](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L1573)
compares effects, global lighting, sun and global intensity but omits the
snapshot's `land_lighting`. The update copies
[`land_lighting.diffuse_strength`](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L1691)
into the shared uniform lane consumed by art-water shading. Changing only that
value therefore does not cause the art material to update until another tracked
input changes. The data flow is confirmed statically; no UI or visual reproduction
was performed.

**Follow-up:** include the effective diffuse input in the material change
signature and add a focused update test, preserving the shared Rust/shader ABI.

### A2 — Art source selection is duplicated

[Atlas setup](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L298)
and [static collection](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L1609)
contain separate copies of the effective source and availability policy. They
currently agree but can drift, causing allocation and collection to select
different sources.

**Follow-up:** share a pure source-selection contract and test package
combinations and source switches. Preserve per-owner fallback and source-qualified
atlas keys; these are active compatibility behavior, not dead legacy code.

### A3 — Resident chunk selection and static-art ownership are scattered

[Collection](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L1870)
and [static lights](../../dynamapper/src/core/render/scene/world/art/static_lights.rs#L239)
independently derive and sort the same map-filtered `LCMesh` keys. The 4,244-line
collector also combines depth/geometry, source routing, cached chunk state,
lighting and instance emission. The plugin additionally installs art atlas
uploads, material systems and picking (see the texture-cache wrapper review).
The collector's TODO saying it needs a way to obtain terrain chunks is stale:
it already queries them; whether the current resident set is the desired
visibility contract should still be assessed.

**Follow-up:** give the resident render-chunk set and static-art policies clear
owners and split cohesive responsibilities where useful. Do not substitute the
scene's desired target set for actual resident chunks without examining scale
handoffs, picking and lighting. Preserve current scheduling and sorting contracts.

### A4 — Picking is active but its inputs and ownership span several modules

The GPU picker is [installed by the sprite plugin](../../dynamapper/src/core/render/scene/world/art.rs#L36),
uses production art shaders and collector owner snapshots, and is consumed by
overlay and highlight geometry. It is not the retired CPU alpha picker. Its
request contract spans those modules and asynchronous render-world readback.
When water animation is enabled, the
[query signature](../../dynamapper/src/core/render/scene/world/art/static_geometry.rs#L61)
changes with time even for a stationary non-water hit; cost is unmeasured. The
[render query](../../dynamapper/src/core/render/scene/world/art/gpu_picking.rs#L235)
does not select a camera by identity, a conditional limitation if multiple 3D
views are introduced rather than a demonstrated single-camera defect.

**Follow-up:** establish an explicit shared picking boundary, retain production
coverage, stale-reply rejection and separate terrain ownership, and measure
animated-query cost before changing its cadence. Test multi-view behavior if it
becomes supported. The focused picker test run passed 4 tests, with the Vulkan
pixel/depth/MSAA test ignored; no visual comparison was performed.

### A5 — A test-only visual resolver has diverged from production

[`resolve_static_visual_kind`](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L792)
is compiled only for tests and has one Classic-only caller. Its unused EC
branch differs from the production cached resolver, including the latter's
Classic fallback when EC art is missing. Keeping a parallel implementation
could make a future test exercise behavior the application does not use.

**Follow-up:** migrate the Classic test to the production resolver or its shared
pure policy, then remove the unused test-only branch and helper. Preserve the
production source fallback and cursor-routing behavior.

### A6 — Art writes a camera field with no discovered art-shader reader

The [art material update](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L1600)
queries the player transform to populate `scene_uniform.camera_position` on art
materials. No active art-shader read of that field was found; EC liquid
reflection uses its own view-based path. The shared field is nevertheless read
by land shading (T9), so it is not globally dead.

**Follow-up:** recheck imported shader variants and feature combinations, then
remove the art-only query and writes if still unused. Retain and correctly own
the shared land field and reflection inputs. Four focused `statics_draw` tests
passed; they do not cover A1, A2 or this candidate.

### A7 — Failed art-page preparation is recorded as residency

The [upload enqueue](../../dynamapper/src/core/texture_cache/art.rs#L147)
silently does nothing if reading or preparing a page fails. Nevertheless,
[`resolve_slot`](../../dynamapper/src/core/texture_cache/art.rs#L414) installs
the page-to-layer mapping. On a later request, no queued upload remains to block
resolution, so the layer can be treated as available despite containing zero or
previously used texels. The same enqueue path also serves atlas requeue after
resize. The failure path is established by code inspection; the prevalence and
visible result depend on package data and timing.

**Follow-up:** make preparation success or failure explicit and keep failed pages
out of the ready state; define diagnostics and retry behavior. Exercise a read
failure followed by recovery and a failed requeue after atlas replacement.

### A8 — Art uploads lack target generation and completion ownership

[Staging](../../dynamapper/src/core/texture_cache/art.rs#L733) clears the
main-world staging buffer on the next update, while the
[render uploader](../../dynamapper/src/core/texture_cache/art.rs#L789) retains
uploads when the GPU image is unavailable. `resolve_slot` checks only the
main-world queues, so it can return a layer before the deferred write completes.
The upload payload also has no target image or generation: a retained upload is
submitted to the currently extracted handle, including after
[source replacement](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L648)
or resize. Old uploads can consequently target a new image or layer; format and
size differences can make this more serious. These are conditional lifecycle
hazards, not a claim that a visible failure was reproduced.
The same main-world pending count is used by
[export readiness](../../dynamapper/src/core/export_job.rs#L610), so that check
does not itself prove that a deferred render-world upload has completed.

**Follow-up:** give uploads an image generation and a clear ready/acknowledgement
contract; invalidate stale work when images or layer assignments change. Test
delayed GPU preparation, source replacement, growth and layer reuse, including
the two sprite and ground atlas paths.

### A9 — Static records are interpreted as multis by numeric range alone

[Collection](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L244)
maps every graphic at or above `0x4000` to a potential multi ID and replaces the
source tile when a nonempty definition exists. The mapping has no separate
record-type discriminator. Classic tiledata can also contain ordinary static
IDs in that range, so a matching definition creates an ambiguity; actual input
prevalence and the intended source-format contract still need investigation.
Moreover, [`parts`](../../dynamapper/src/core/multis.rs#L93) returns `None` for
an empty visible definition, making an all-hidden definition fall back to the
original graphic.

**Follow-up:** establish which records can encode multi instances and how they
are distinguished from ordinary graphics before changing behavior. Add cases
for high-ID ordinary art, matching definitions and all-hidden multis. If the
source format does not distinguish them, keep static records and explicitly
typed multi objects separate rather than guessing from their graphic alone.

### A10 — Failed static-block reads become cached missing geometry

The [chunk builder](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L2050)
continues silently on `block_tiles` errors. The store can report package-read
errors and malformed chunk data; a skipped block contributes no atlas miss, so
the resulting chunk can pass the [completeness check](../../dynamapper/src/core/render/scene/world/art/statics_collect.rs#L1888)
and be reused without retry. The light collector independently skips the same
kind of errors. Missing data from a persistent corrupt package and a transient
read error both need an explicit policy.

**Follow-up:** distinguish unavailable blocks from empty blocks, surface a
bounded diagnostic and propagate incomplete/error state into chunk caching.
Define retries and test failure followed by recovery for geometry and lights.

### A11 — Static-light collection, response and rendering have mixed ownership

[Static lights](../../dynamapper/src/core/render/scene/world/art/static_lights.rs#L208)
combine resident-chunk scanning, color and hue response, material caching,
entity synchronization and style shaping. Their collected data also feeds land
and art lighting, so it is not merely a decal renderer or dead legacy code.
Collection scans resident blocks on each update and a fallback can
[read light bytes](../../dynamapper/src/core/render/scene/world/art/static_lights.rs#L623)
for individual lights each time; the cost is unmeasured. The sync signature
avoids some downstream work but not that scan.

**Follow-up:** give shared light data and the rendering-specific presentation
distinct owners, and profile before caching. Any cache must account for data and
settings changes. Package-identity invalidation is a future reload concern:
current loading appears startup-only, so no supported hot-reload defect is
claimed. A3 covers the shared resident-chunk selection.

### A12 — Static-light cache has an inert failure flag

`StaticLightMaterialCache::hue_texture_failed` is initialized to false and
[checked](../../dynamapper/src/core/render/scene/world/art/static_lights.rs#L792),
but no assignment to it was found. The guarded texture copy is an active hue
path; only this failure flag and condition appear dead. Several helpers and the
sync signature are also public despite only local uses found.

**Follow-up:** remove the inert flag and condition after confirming no external
contract, and narrow helper visibility where appropriate. Keep hue handling and
light response intact. The static-surface flags are also active: EC boundary
coverage is consumed by art shaders, and its module's height helper is reused
for Classic geometry. Make the Rust/shader flag contract explicit when those
responsibilities are reorganized.

### A13 — Ground shader retains an unused priority-depth path

The [ground vertex shader](../../dynamapper/assets/shaders/world/art/ground.wesl#L106)
calculates a logical priority depth with a per-instance tie ordinal, but its
fragment path never reads that varying: it uses horizontal-plane depth, or the
pick position in the pick variant. The helper chain and associated constants
are therefore unused by this ground output. CPU ground ordinals are assigned
per chunk, unlike globally reassigned sprite ordinals, but this shader path does
not establish a present visible ordering defect because the result is unread.
The sprite priority-depth path remains active.

The sprite bindings also contain an
[unused second `GroundTileInstance`](../../dynamapper/assets/shaders/world/art/art_bindings.wesl#L39)
that differs from the active ground binding definition. Keeping two definitions
invites ABI confusion.

**Follow-up:** remove the unused shader declaration and depth calculation after
checking shader variants, preserving horizontal-plane and picking depth. Trace
the CPU fields and their other consumers independently before changing the
storage ABI or sorting policy; test cross-chunk ties if priority ordering is
intended to return.

### A14 — Art atlas suppresses dead-code diagnostics for the whole module

The [art atlas module](../../dynamapper/src/core/texture_cache/art.rs#L1) has a
blanket `allow(dead_code)`. For example, `ResolvedArtSprite.pixel_width` and
`pixel_height` are populated but no reads were found in the application, while
its logical dimensions and offsets are used for geometry. This blanket also
makes later obsolete helpers less visible.

**Follow-up:** check non-default configurations, remove the unread fields and
other confirmed unused API, and narrow or remove the module-wide suppression.
Keep the active physical-size inputs used to calculate UVs and upload layouts.

### A15 — Batch draw setup retains unused state and duplicates reconciliation

The [setup quad](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L942)
is stored in both render-asset resources but no reads of those mesh fields were
found. Live drawing instead constructs per-batch meshes. Two transparent entity
count fields are similarly only compared and assigned, with no diagnostic or
other consumer. Five batch synchronization systems independently reconcile
similar keys and build equivalent meshes for their respective sprite and ground
passes, increasing the surface for drift.

**Follow-up:** remove the confirmed unread setup mesh and count bookkeeping.
Consider sharing batch reconciliation and equivalent mesh ownership while
preserving all opaque, transparent and shadow passes and verifying Bevy asset
lifetimes. The current signature shortcut assumes exclusive ownership of its
entities; external despawn or mutation is a conditional limitation, not a
demonstrated current use case.

### A16 — Unchanged art parameters still mutably access every material

On every in-game update with instances, the
[sprite](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L1411)
and [ground](../../dynamapper/src/core/render/scene/world/art/statics_draw.rs#L1485)
upload systems mutate all five material assets to write render parameters even
when their values have not changed. The instance-buffer comparison occurs
afterward. Material mutation can cause render-asset extraction and rebinding;
the actual performance cost has not been measured.

**Follow-up:** independently gate parameter changes and instance uploads, and
test zoom, map changes and transitions from empty to nonempty instances. The
[transparent phase sort](../../dynamapper/src/core/render/scene/world/art/transparent_order.rs#L21)
is active; at equal distance its optional art key also orders non-art items
before art items. Assess mixed transparent ties before modifying this policy,
and preserve agreement with picking. No visible ordering defect is asserted.

## OverlaysPlugin and PerformanceOverlayPlugin

The [parent plugin](../../dynamapper/src/core/render/overlays.rs#L18) composes
four active overlay children; reviewing it does not imply its children are all
covered. The performance child owns both native process telemetry and the UI
that presents it.

### O1 — Native VRAM telemetry is labeled as application-tracked memory

The [refresh system](../../dynamapper/src/core/render/overlays/performance.rs#L337)
calls OS-specific queries, but names and comments describe application-owned
tracked allocations. It also maps unavailable telemetry to zero, making it
indistinguishable from a genuine zero. The Linux `/proc` implementation is
compiled for several BSD systems and macOS as well, where that Linux interface
may be unavailable; unsupported platforms return no value.

**Follow-up:** preserve availability and source in the metrics contract and
display them accurately. Limit platform-specific probes to supported platforms
and test absent counters without representing them as measured zero. Review
driver and adapter semantics before claiming a complete process allocation total.

### O2 — Texture estimates do not represent all textures

The [estimate](../../dynamapper/src/core/render/overlays/performance.rs#L321)
counts fixed small and big land arrays plus a separately reported tile metadata
atlas. The [label](../../dynamapper/src/core/render/overlays/performance.rs#L396)
does not make that limited scope clear and hardcodes `RGBA8`; it excludes art,
other page arrays and other GPU allocations. This does not establish that all
project texture formats are RGBA8 or that BC7 is the only compression format.

**Follow-up:** label the specific estimated allocations, derive their formats
from their actual owners, and keep the estimate distinct from OS measurements.

### O3 — Hidden performance overlay still polls process metrics

[Metrics polling](../../dynamapper/src/core/render/overlays/performance.rs#L215)
runs four times per second in game even when the overlay is hidden; visibility
only gates text updates. No other `ProcessMetrics` consumer was found. Native
file polling and sysinfo refresh therefore continue for an unused display,
although their cost was not measured.

**Follow-up:** gate collection by visibility or establish a shared telemetry
owner if another consumer needs it. Preserve CPU sampling warmup and refresh
promptly when the display is enabled.

### O4 — Non-Windows DXGI stub has no caller

The [non-Windows stub](../../dynamapper/src/core/render/overlays/performance.rs#L172)
is suppressed as dead code, and its only call is itself Windows-gated. The
Windows implementation is active and must remain. A commented timer import is
also obsolete.

**Follow-up:** remove the unreachable stub and stale import; retain the active
platform-specific query and cover platform configuration in the later build
sweep. No platform runtime measurements were performed.

## PlayerPositionOverlayPlugin and SystemMessagesPlugin

### O5 — Viewport label assumes the fixed orthographic view

The [position panel](../../dynamapper/src/core/render/overlays/player_position.rs#L151)
derives a tile span from window size, zoom and fixed isometric constants, then
labels it `Viewport`. The camera can use perspective or move and rotate freely;
the label does not use its projection, transform or actual viewport, nor does
its rebuild signature observe them. The displayed estimate is therefore not a
general measurement of the active camera.

**Follow-up:** derive the intended measure from the active camera and document
its geometric meaning, or restrict and label the existing estimate to the view
it describes. Test camera mode, orientation and viewport changes.

### O6 — Message cache uses queue length as its change signature

The [logger](../../dynamapper/src/ingame_sysmessage_logger.rs#L62) caps its
queue at 100, dropping an old entry when a new one arrives. The
[overlay](../../dynamapper/src/core/render/overlays/sysmessages.rs#L43) reloads
on a length change or periodic cleanup, so once full a new message can wait
until the next cleanup interval to appear. Length and snapshot are also read
under separate locks. The queue is bounded; this is a freshness and ownership
issue, not an unbounded retention claim.

**Follow-up:** expose an atomic snapshot and mutation revision, or another
explicit notification, and test replacement at capacity. The global logger is
active and has multiple producers; if moved into Bevy ownership, preserve their
delivery and the wall-clock expiration behavior.

### O7 — Position and cursor panel scale updates omit layout properties

Position-panel [padding](../../dynamapper/src/core/render/overlays/player_position.rs#L50)
and cursor-panel [maximum width](../../dynamapper/src/core/render/overlays/cursor_behavior.rs#L146)
are scaled at creation but not updated alongside their live font-scale changes.
Changing either preference during a session can leave padding or wrapping width
at its startup scale.

**Follow-up:** update all scale-dependent layout properties together and cover
live setting changes in a focused UI-system test. Both position and cursor
panels spawn on entering the game without a discovered exit cleanup; duplicate
panels would matter if re-entry becomes supported, but the current app path
only showed initial entry, so this is a lifecycle constraint rather than a
reproduced defect.

## CursorBehaviorOverlayPlugin

### O8 — Cursor interaction, inspection and presentation share an owner

The [cursor plugin](../../dynamapper/src/core/render/overlays/cursor_behavior.rs#L103)
combines mode controls, click teleport, terrain and static inspection,
source-data formatting, and UI/highlight rendering. These are active: the
terrain query supplies logical teleport destinations, while the GPU picker
identifies rendered statics. Raw cell contents and the hovered rendered owner
are distinct useful observations.

**Follow-up:** separate interaction and inspection data from presentation around
the shared terrain/static picking contracts. Preserve logical map coordinates,
pointer and map guards, both inspection views and existing input bindings.
Current tests cover formatting rather than system-level interaction or scaling;
no visual comparison was performed.

## Dialog composition, keybinding help and teleport

### D1 — Shared egui context helper retains an unused camera dependency

The [shared helper](../../dynamapper/src/core/render/dialogs.rs#L75) accepts a
`UiCameraResource` but queries only the primary egui context. Dialog and overlay
callers still acquire the camera just to pass it to that helper. Its earlier
immutable variant remains commented out. The dialog parent also owns global
egui scale synchronization even though overlays use that same context.

**Follow-up:** remove the unused parameter and retired commented helper, and
consider moving shared context/scale ownership to a common UI integration
boundary. Keep real camera consumers and the current context selection. V4
records the same dependency from the camera perspective.

### D2 — Keybinding help names the wrong file

The [help panel](../../dynamapper/src/core/render/dialogs/keybindings_help.rs#L165)
points to `assets/keybindings.toml`; the loader actually uses
`settings/keybindings.toml` under the resolved assets directory. It also implies
all displayed bindings are configurable, although several are hardcoded. The
same stale path appears in `docs/keybindings.md`.

**Follow-up:** correct both references and distinguish configurable actions
from fixed shortcuts; C4 covers the larger scattered binding ownership.

### D3 — Teleport parsing failures receive no feedback

The [teleport button](../../dynamapper/src/core/render/dialogs/teleport.rs#L95)
does nothing when any coordinate fails integer parsing, including the initially
empty inputs. Parsed coordinates rejected by the shared teleport validator do
produce a message. The shared mutation helper and recompute on success are
active and should remain.

**Follow-up:** show actionable field-level parse errors and test malformed,
out-of-range and valid input. Consider an explicit close control: global key
dispatch is suppressed while egui captures keyboard input.

## LandUiPlugin

### D4 — Undo restores one global snapshot into whichever slot is active

The [Undo button](../../dynamapper/src/core/render/dialogs/land_shader.rs#L382)
restores `UniformState.saved_snapshot`, which is initialized from the startup
slot and updated only when the panel closes. Switching style or time saves the
slot being left and changes `active_preset`, but does not change that snapshot.
Undo after a switch therefore loads the previous snapshot's values, including
its shading mode, while retaining the new slot's key; closing can save those
values into that new slot. Separately, the close handler updates the snapshot
even when [`save_shader_settings`](../../dynamapper/src/configs/shader_presets.rs#L194)
logged a write error, so `Last Saved` may not actually reflect disk.

**Follow-up:** define undo against the last successfully saved value of the
active style/time slot, preserve other slots, and report persistence errors to
the caller. Test switching slots, undo, close/reopen and failed writes. Keep the
existing deliberate preservation of the slot being left and scoped factory reset.

### D5 — Shader panel owns renderer synchronization and retains old scaffolding

The [panel plugin](../../dynamapper/src/core/render/dialogs/land_shader.rs#L43)
also registers the runtime system that applies terrain material uniforms and
transition settings even when the panel is closed. T10 covers the renderer
ownership issue. That system's material-handle query is explicitly unused, and
the [save handler](../../dynamapper/src/core/render/dialogs/land_shader.rs#L879)
contains a commented-out old change check. Opening comments still name Bevy
0.16 and WGSL-era behavior.

**Follow-up:** move runtime synchronization to a render/settings owner, remove
the unused query and obsolete commented block, and update comments against the
current implementation. Preserve active preset, dirty-update and persistence
semantics, including D4's correction.

## GumpDialogPlugin

### D6 — Multiple open gumps can share an egui window identity

[`open_gump_window`](../../dynamapper/src/core/render/dialogs/gump.rs#L1250)
always appends an open window, but the [egui window](../../dynamapper/src/core/render/dialogs/gump.rs#L924)
uses only its title as identity. Opening the same gump or paperdoll profile
more than once can therefore collide in egui's window state. This follows from
the construction; independent interaction has not been reproduced in a GUI.

**Follow-up:** assign each open instance a stable unique ID and test duplicate
titles, independent close and texture cleanup. Source changes and the active
paperdoll/image paths must remain supported.

### D7 — Gump Enter shortcut is not scoped to its input

While Open Gump is displayed, [its submit condition](../../dynamapper/src/core/render/dialogs/gump.rs#L1297)
checks Enter across the egui context without checking the gump-ID field's focus.
Enter in another field or dialog can submit the unrelated gump ID. The same
file combines open-window state, source selection, paperdoll profile policy,
compositing and drawing, while several paperdoll responsibilities already live
in sibling modules.

**Follow-up:** scope Enter submission to the intended input and retain the Open
button; test another field and dialog taking focus. Separate window lifecycle,
content rendering and profile policy at clear boundaries, preserving resource
and texture cleanup. No GUI or visual comparisons were run.

## PreferencesDialogPlugin

### D8 — Selecting an FPS preset may not persist the change

The [FPS combo](../../dynamapper/src/core/render/dialogs/preferences.rs#L340)
changes its selected index in individual item responses but starts the settings
debounce only when the outer combo response reports a change. In the installed
egui implementation that response is the combo button response; the item
responses are ignored here. The live limiter is set separately while the Graphics
tab is rendered, whereas the `Settings` value is written by the debounce. An
FPS-only selection can thus affect the live limiter without being saved, and a
later settings-driven update can restore the old value. This is based on code
and dependency inspection, not a GUI reproduction.

**Follow-up:** detect the actual selected value change and apply it through a
single settings contract. Test an FPS-only change, closing and reopening the
dialog, and persistence without requiring another preference edit.

### D9 — Custom FPS values fall back to 15 in the preferences draft

[Draft initialization](../../dynamapper/src/core/render/dialogs/preferences.rs#L134)
maps a target absent from the preset list to index zero, which is 15 FPS. The
configuration value is an unrestricted `u32`. Opening the Graphics tab can
apply that fallback to the live limiter, and any unrelated debounced edit can
write it into settings. VSync or a disabled limiter can mask the immediate
effect, but the unintended settings write remains possible.

**Follow-up:** preserve custom targets in the draft or update the setting only
after an explicit FPS edit. Test a custom value such as 90 FPS with an unrelated
preference change, and with the limiter enabled and VSync disabled.

### D10 — Preferences mixes draft, live application and broad writeback

The [preferences state](../../dynamapper/src/core/render/dialogs/preferences.rs#L18)
mirrors many settings, applies some values directly, writes many fields together
after a timer, and independently writes frame-limiter state. Synchronization
from settings is suspended while the dialog is open, so a later broad writeback
can overwrite concurrent settings changes; whether this happens depends on
other writers. Persistence itself is owned by `SettingsPlugin`, not the dialog.
There is also an obsolete comment about a removed immediate-sync block.

**Follow-up:** define one owner for the effective values and apply only fields
actually edited, retaining intentional debounce and persistence behavior. Test
external changes while a draft is open and remove the stale comment when acting
on this review. No runtime or visual validation was performed.

## Paperdoll and image-background support

### D11 — Cover rules can act from an item removed in the same pass

[`taken_off`](../../dynamapper/src/core/render/dialogs/paperdoll_overrides.rs#L57)
collects all removals from the current draw list before replanning. For three
items ordered A, B, C, if B removes A and C removes B, the pass removes both A
and B even though B no longer survives to cover A. This conflicts with the
module's stated rule that a taken-off item is not worn. The rule format permits
user-defined overlapping covers; whether bundled rules encounter this case is
not established. Both the 3D planner and the
[gump planner](../../dynamapper/src/core/render/dialogs/gump.rs#L1609) use this
policy in separately implemented remove/replan loops.

**Follow-up:** define precedence for overlapping covers, then share the policy
and add a three-layer regression for both representations. Preserve their
different frame sources, stored decisions and user override semantics.

### D12 — Shared image-background policy is copied across UI frameworks

Dynamapper's [background control](../../dynamapper/src/core/render/dialogs/image_background.rs#L1)
closely duplicates the active `uot-gui` implementation. The copy is deliberate:
`uot-gui` brings in eframe whereas Dynamapper uses bevy_egui. Persisted
`ImageBackground` and `EnhancedPaperdoll` types are also imported by settings
from dialog-owned modules. The EC paperdoll adapter, alpha picking, override
menus and test-only measurement code all have active consumers or explicit
test roles and should not be classified as dead.

**Follow-up:** consider shared framework-neutral policy or an egui-only control
without adding an unnecessary eframe dependency, and move persisted types to a
domain/settings boundary if useful. Preserve each application's existing
serialization, frame planning and pick behavior.

## Frame rate, frame limiter and diagnostics

The frame-rate counter, limiter and GPU-capability log are all registered and
active. The counter samples the main-world `Last` schedule over a wall-clock
window; its rate is not an independently measured GPU presentation rate. The
limiter runs in a schedule after `Last`, resets its deadline on changes, and is
covered by existing conversion tests and historical wall-clock measurements.
D8–D10 describe its split settings ownership and preference defects.

### R1 — GPU preprocessing capability is reported as the active mode

The [one-shot logger](../../dynamapper/src/core/diagnostics.rs#L483) reads
`max_supported_mode` but labels it `GpuPreprocessingMode`, and its documentation
says it reports the active mode. Hardware support alone does not prove which
mode a view or pipeline actually uses. Successful capability reporting also
uses warning severity.

**Follow-up:** label this as the maximum supported capability, or separately
instrument actual use if needed; use appropriate severity and retain the
unavailable-resource diagnostic.

### R2 — Frame residual combines incompatible time samples

The [diagnostic residual](../../dynamapper/src/core/diagnostics.rs#L642)
subtracts the last samples of two individual systems from the mean frame time
of the last closed frame-rate window and clamps the result to zero. These
samples need not represent the same frame or interval; system and pipelined
work may also overlap. It is not a measurement of untracked frame time.

**Follow-up:** remove or explicitly qualify the estimate, or collect compatible
wall-clock intervals and report their overlap and measurement window.

### R3 — System-timing instrumentation runs when its output is disabled

[Render schedule timers](../../dynamapper/src/core/diagnostics.rs#L379) are
installed only when configured, but scoped main-world and cross-app timers in
chunk and atlas systems run regardless. Cross-app samples take a shared mutex.
Their displayed output is gated, and no performance impact was measured.
The independent upload telemetry has other consumers and is not the same thing.

**Follow-up:** consider a coherent low-overhead enablement policy for timers,
keeping independently consumed telemetry available. The console dump system is
installed based on startup configuration despite checking the setting live;
no supported runtime toggle was found, so that is an interface asymmetry rather
than a demonstrated toggle failure.

### R4 — GPU elapsed totals have no stated nonoverlap contract

The [console](../../dynamapper/src/core/diagnostics.rs#L508) sums matching
render diagnostic spans and labels the result total GPU time. The performance
overlay has a separate aggregation path. If reported spans overlap or nest,
their sum can double-count; current span overlap has not been demonstrated.
The two paths also need not use the same smoothing semantics.

**Follow-up:** define a top-level measurement or label the sum of reported
spans, and share aggregation semantics if the displays intend the same metric.
Do not treat GPU timing as equivalent to the main-world FPS window.

## UOFilesPlugin

### U1 — Runtime gump provider is embedded in the startup loader

The [loader module](../../dynamapper/src/core/uo_files_loader.rs#L73) owns
gump source routing, image decoding and shared-hash policy used after startup,
while its large startup system also discovers and opens maps, statics, textures,
metadata, gumps and other packages. This makes runtime content ownership hard
to find and couples it to loading orchestration.

**Follow-up:** separate typed runtime providers from package discovery and
resource publication while retaining real Classic fallback and shared-hash
behavior. W1/W3 already cover map metadata and missing startup maps.

### U2 — Loader suppresses unused diagnostics and retains unused state

The [module-wide suppression](../../dynamapper/src/core/uo_files_loader.rs#L1)
hides an apparently unread `UoFilesSettingsRes` created at startup, an unused
`SourceContainerKind::Uop` variant, and an unused import. Actual UOP multi
loading is a separate active path; its existence does not make the logging enum
variant live.

**Follow-up:** remove confirmed unread state and enum/import remnants after
checking configurations, then narrow or remove the blanket suppression. Keep
the configured source path in settings and active file fallbacks.

### U3 — Package error handling loses causes and varies by source

Several package initializers discard the underlying error when a present file
fails, for example [statics](../../dynamapper/src/core/uo_files_loader.rs#L577)
and [tile metadata](../../dynamapper/src/core/uo_files_loader.rs#L608). Optional
packages have differing absent/corrupt behavior: some corrupt files panic while
others log and continue. This is an inconsistent contract, not a conclusion that
every optional package should be fatal or recoverable.

**Follow-up:** define required, absent and corrupt behavior per package; retain
the file path and error chain and test each policy. Do not silently substitute
or treat a required startup map as optional.
