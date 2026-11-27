# Local renderer test control

On Unix, `DYNAMAPPER_CONTROL_SOCKET` opts a process into a private Unix datagram
control endpoint. Normal launches install no socket or polling system. This is
a development transport; Windows builds omit it. Commands are JSON datagrams,
limited to 4096 bytes and eight requests per frame, handled nonblockingly on the
main ECS thread in `MovementActions`.

Commands include `teleport`, `status`, `capture` and app-only keyboard/pointer
input (`key`, `text`, `pointer`, `click`, `scroll`, `reset_input`), and the typed
runtime switches `preset`, `sources` and `set`, the `reload_land` check and the
test mobiles `spawn_mobile` and `clear_mobiles` (see below). There is no shell/code
execution or generic settings setter. The owner-only socket refuses occupied
paths; normal resource cleanup only removes the bound inode. Forced termination
may leave a socket, so isolate each run in its own temporary directory.

The UI dialog and endpoint share `controls::teleport::teleport_player` for loaded
map/bounds validation and player/transform updates. Both request visible-chunk
recomputation after success, including same-map teleports. Invalid destinations
leave state untouched. This does not add loading for previously unloaded maps.

`status` exposes current and previously rendered player positions, elapsed time,
frames counted over the last 500 ms window as `fps` and `frame_ms`, and the running
`total_frames`. Dividing the change in `total_frames` by wall-clock time checks
the rate from outside the app. Rendered
position equality does not establish completion of texture streaming. The
`art` snapshot reports the collector's map/chunk/instance counts, atlas misses
and pending sprite/ground pages. Its `digest` hashes every collected static
instance in output order without its art page placement, so two runs that
collected the same statics report the same digest however the land and the art
loaded: compare it between a fresh launch and a live switch, or two teleport
orders, to catch a statics cache that kept a chunk built from stale land.
`land_invalidated_chunks` counts the cached statics chunks rebuilt so far
because tile atlas cells they read changed (the cost of land arriving after the
statics; a full flush shows as every cached chunk at once). A terrain-only wait captured an empty bridge
while its sprite pages were still loading. `control.py wait-ready` now requires
position agreement, submitted metadata and settled art over three stable
snapshots. It has a bounded timeout; it does not fence GPU completion or shader
compilation. Keep actual-frame inspection in the validation workflow.

`status.render` reports the actual active preset, global lighting and effects
uniform values (`light_cover_rule` the cover rule the style's `light_cover` gives,
`player_floor_rule` KR-like's and UOD-Painterly's z-view rule, null in Enhanced and Classic,
`mobile_sharpening` the style's multiplier of each mobile's sharpening from
`runtime_specs/mobiles/Sharpening.kdl`);
`status.static_lights.instances` reports collected lights,
`status.client_light_sprites.instances` the KR-like and Enhanced light sprites
left after the light cover (`digest` a hash of every sprite's key, texture, quad
and ceiling: two runs that collected the same sprites report the same one), and `status.z_view` the z-view limit (under the KR
rule `max_z` is the bottom of the layer above the player's, and `roof_layer`
the player's layer, against which each roof is counted from its own cell's
land; null otherwise), `status.cave_surroundings` the setting and
`status.player_cave` the player's cave (its floor z and its cell count; null
out of one, or with Show Only the Player's Floor off). These
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


## Terrain cursor snapshot

`status.terrain_cursor` reports the logical pointer and optional terrain hit:
`logical` is `[map_x, map_y, original_signed_z, facet]`, `graphic` is the source
map cell, and `world_position` is the continuous rendered triangle point. These
heights intentionally differ on slopes and eased banks. A null hit does not
fabricate a plane destination; inspect `status.terrain` for streaming readiness.
See [the terrain query contract](terrain_cursor_surface.md).


## Runtime switches

Three commands change what a fresh launch would otherwise fix in its settings
files, so one preview can visit several styles, times and sources
(`controls/dev_control/settings.rs`):

- `{"command":"preset","style":"kr","time":"night"}`: the Shading window's
  Render Style and the World Scene window's Time of Day buttons
  (`shading::switch_render_preset`, which stores the
  live slot in the in-memory presets and applies the target, as a click does).
- `{"command":"sources","art":"ec","land":"kr"}`: the preferences' art and land
  texture sources (`cc`/`ec`; `cc`/`ec`/`kr`). The land texture images and every
  terrain chunk's bake follow through `sys_apply_land_texture_source_changes`,
  the statics through `sys_sync_static_art_source`, as after a click.
- `{"command":"set","key":K,"value":V}` for an allowlist with typed values:
  `hide_player`, `player_body`, `player_mount_body` (core, the player's drawn
  body and mount; 0 for none); `show_only_player_floor`, `caves`, `cave_surroundings`, `enable_statics`,
  `enable_static_lights`, `show_hidden_multi_parts`, `mobile_in_between_frames` (`off`/`crossfade`/`rife_x2`/`rife_x4`), `mobile_upscale` (`off`/`balanced`/`always`, `span_x2` read as `balanced`; switching remakes the mobiles' frame atlases), `mobile_outline_width` (`one_texel`/`follows_upscale`: an outlined body's line on its 2x frames), `mobile_stop` (`half_step_later`/`at_once`), `mobile_walk_pace` (`client_timing`/`match_the_ground`)
  (world_rendering);
  `cc_terrain_transitions`, `ec_terrain_transitions` (graphics);
  `light_cover`, `player_floor_rule`, `land_texture_variation`,
  `mobile_sharpening` (0 to 2, the multiplier of Sharpening.kdl's strengths)
  (the active style's settings, refused where the
  style has none); and `zoom`
  (clamped range, refused with the free perspective camera, which dollies
  instead). An unknown key, a wrong type or an out-of-range value is an error
  naming the allowed keys or the expected type.

- `{"command":"reload_land"}`: drops every cached map block, the state 60
  idle seconds reach (`sys_evict_idle_land_cache`), and every cached statics
  chunk, so the statics are collected again from the land loaded now. It changes
  nothing a correct cache shows: `status.art.digest` after the scene settles
  again must equal the one before (the harness's `--reload-land-check`). Before
  2026-11-16 the KR client shadows read their ground from those blocks, so a
  rebuild after idling lost the land under them (the river-bank wall of the
  Britain house at 1409,1596).

Each reply has `changed` (false when the value was already in force; no switch
is counted then; always true for `reload_land`), `applied` and `switch_serial`. Freezes in the startup land.toml
(water animation, fog drift) stay in force: every preset slot is loaded frozen
and a preset switch keeps the current water-animation flag.

Persistence is the UI's. Preset switches stay in memory (the Shading window
writes land.toml only when it closes, and World Scene when it closes after
changing the time of day or light level). A `sources` or `set` change of `Settings`
reaches `cache/settings/` of the process's working directory through the app's
debounced save, as a preference picked in the window does. In a harness or
preview run that directory is the run's own isolated copy; never point a
controlled preview at the user's working directory.

`status.control` reports the applied values: `settings` (the allowlisted world
and graphics keys, both sources, the camera mode), `zoom`, `applied_sources`
(the land source the texture cache uses and the art source the statics were
collected with), `waiting_pipelines` (render pipelines still compiling, counted
in the render world), `switch_serial` and `frames_since_switch`.
`status.terrain.pending_bakes` and `unbuilt_chunks` count land chunks still
waiting for a texture bake or a mesh, `status.render.uniforms_dirty` style
values not yet pushed to the materials, and `status.render.fog_clouds` the
fog's cloud settings (its fourth value is the drift, which the harness freezes).

`control.py wait-ready` adds a switch's readiness to the scene's: at least
`MIN_FRAMES_AFTER_SWITCH` frames since the last switch, no waiting pipelines, no
pending bakes, no dirty uniforms and applied sources equal to the requested
ones (art only while statics are drawn). The stable fingerprint also covers the
static-light and light-sprite counts, the z-view and the switch serial. This is
still not a GPU fence: discard the first capture after a switch and require two
identical frames, as after a launch.

## Test mobiles

The player is the one mobile the app draws, and the regression harness hides it.
For draw-order checks of other bodies at chosen cells (`input/dev_control.rs`,
2026-11-26):

- `{"command":"spawn_mobile","x":1409,"y":1596,"z":30,"body":12,"mount":0,"facing":3}`:
  a standing mobile of `body` on the mount body `mount` (0 for none), facing 0
  (north, UO y - 1) clockwise to 7 (north-west), drawn with the player's art
  source, sharpening and upscale settings. The reply gives its entity bits. It
  carries no map: it shows on whatever map the player is on, so spawn it after
  the teleport. A body of 0 or a facing past 7 is refused.
  Optional `"clip_secs": 0.35` holds it that many seconds into its standing clip
  (0 when left out; negative or infinite is refused). A spawned mobile is held
  (`MobileHeldClip`): its clock never runs, so every capture shows one frame, and
  each layer shows its 2x frames as soon as they are ready, where a playing body
  switches only as its sequence starts over. A mobile kept across a source or
  preset switch keeps the cycle and 2x choices it made before: clear and spawn
  again for a capture that must match a fresh launch.
- `{"command":"clear_mobiles"}`: removes every spawned mobile.

At most `MAX_MOBILE_INSTANCES` frames (64; a mount and its rider are two, an
upscaled frame wider than a page one per part) are drawn at once, the player's
first when it is drawn; frames past the cap are left out.

`status.mobiles` (`MobileDrawProgress`, 2026-11-27) reports the last draw:
`mobiles` (those with an appearance, the player's when shown), `instances` (the
frames drawn) and `waiting_layers` (layers whose clip still decodes, whose
in-between sequence is still to come, or that want 2x frames not shown yet).
`wait-ready` waits for `waiting_layers` 0 and keeps `mobiles` in its stable
fingerprint. The status can be a frame old: after a spawn, wait for
`total_frames` to move on and for `mobiles` to count the spawned ones, then
discard the first capture, as the regression harness does (`place_mobiles`).
