# Source-specific terrain transition controls

Graphics settings own `cc_terrain_transitions` and `ec_terrain_transitions`.
The bundled TOML explicitly selects CC off and EC on. Saved settings missing
either field inherit that field from the bundled TOML while retaining other
user edits. Missing values in both files fail rather than using a Rust default.
The two controls follow texture sources, independently of lighting style or
time-of-day presets. An EC static-ground patch follows the EC control even when
the map uses CC textures.

The shared effects uniform reuses its former padding word without changing
size, alignment or bindings. Bit 0 enables CC transitions; bit 1 disables EC
transitions. Zero therefore preserves the established CC-off/EC-on policy.
The word is derived from Graphics settings, omitted from shader-preset TOML,
and preserved when switching/resetting a preset. The existing dirty-uniform
update propagates it to terrain and static-art materials.

## Classic terrain

`land/cc_transitions.wesl` blends packaged CC texmaps between cell centers.
Smooth separable weights give equal contributions at shared edges and corners
while retaining the owner's color at its center. The four participating cells
are the owner and the horizontal, vertical and diagonal cells toward the nearest
corner. Invalid, missing, wet and foreign-source neighbors are excluded and the
remaining weights are normalized. Wet owners retain their existing sampling,
including flat-water art diamonds. This is an optional rendering improvement,
not a reconstruction of original Classic Client blending.

CC texmaps can contain authored transition details. A neighboring image must
be sampled at its adjacent clamped edge (`uv - neighbor_offset`), rather than
repeated over the owner's whole cell. Repetition duplicated thin road-edge
details in the native town pilot. A UV-coded GPU fixture detects this mistake;
constant-color interpolation tests alone cannot. Atlas-slot identity, not raw
land ID, identifies aliases; homogeneous regions retain the exact base color.
Gradients are captured before material branches, and the CC opt-in path uses
explicit-gradient sampling to avoid divergent derivative footprints.

## Enhanced terrain and ground patches

The EC control gates ordered map-material coverage, fallback neighbor blending
and the added eligible-static ground fade. It does not remove item artwork
masks, per-material auxiliary blends or the authored item overlap margin.
Turning smoothing off must also retain liquid-bank height/shore protection:
level lava cannot regain coverage on raised mountain quads. The unblended path
therefore shades the selected dry bank material using the existing bank weight.
No routing, texture substitution, geometry or missing-EC-texture fallback is
introduced by these controls.

## Validation and limits

Three settings tests cover explicit bundled values, saved-edit preservation,
all four control combinations and serialization. The existing preset-switch
test also checks that source controls survive applying another style/time slot.
The explicitly run Vulkan regression executes 100 production-shader cases,
including CC shared edges/corners, aliases, wet/missing/source exclusions and
UV-coded adjacent-edge sampling, alongside existing liquid/bank/static coverage.

Twenty native current-shader frames plus two pre-feature shader controls cover
Enhanced CC, Classic CC, KR CC perspective, KR EC on/off and a final rebuilt
Enhanced EC check. The inactive EC control leaves the CC town exact; the inactive
CC control leaves the EC road/lava within one channel byte. Open CC sea is exact
with smoothing on/off. The EC-off lava fixture retains bank/crust placement, and
the final Enhanced EC road is identical to the preceding accepted road capture.
All ten previews verify Desktop 4, metadata readiness and cleanup without shader
errors. Temporary structured evidence is `transitions-accepted-evidence.json`
under `/tmp/uodynamapper-ec-followup/`.

Do not claim full pixel identity against the preceding shader program. The CC
town is exact, but coast/bridge have sparse changed pixels on water triangle
diagonals (374/192 pixels over one byte). KR EC road/coast stay within one byte;
lava/bridge have differences in high-frequency liquid bands and sparse art
shading (maximum 55/29 bytes). Native pairs were inspected; their exact numerical
cause has not been isolated. Keep them in the remaining liquid/filtering parity
review rather than treating these captures as proof of original-client parity.

Direct Options interaction remains unverified: the hidden XWayland window
ignored synthetic keyboard events. Check actual dialog interaction/layout with
the shader-UI follow-up. Source callbacks and persisted values are present;
the configuration-driven native checks do not substitute for that UI check.
