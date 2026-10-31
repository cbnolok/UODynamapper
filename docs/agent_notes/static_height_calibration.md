# Static height and legacy pixel calibration

## Evidence and units

The original EC reference `docs/images/ec_reference/20261026_185018.png`
uses legacy object artwork, as confirmed by the user. Compare it with explicit
CC objects and EC terrain. EC world-art trees and roofs are different assets;
whole-scene registration cannot isolate an individual sprite's aspect.

Original ordinary-sprite construction at `UOSA.exe:0x459b00` uses equal
camera-plane source-pixel scales (`0.021427423 * descriptor_scale`). The legacy
window path gives `0.032141134` world units per original pixel. Placement at
`0x4598d0` uses `0.12856454` per raw Z, or four legacy pixels per Z unit.
See [the original-client findings](ec_original_water_code.md#ordinary-sprite-windows-and-legacy-artwork)
for window selection, scale branches and camera coordinates.

Two independent legacy oak interiors matched against the original town image
required native-to-reference vertical/horizontal scale ratios of **0.912268**
and **0.912495**. The first fit has 10 inliers spanning 105 by 173 pixels with
0.49-pixel RMS error; the second has 31 spanning 88 by 214 with 0.64-pixel RMS.
Their agreement independently supports the old projection's predicted inverse
aspect of **0.912287**. A small rock fit and a pine fit containing overlapping
foliage are insufficient controls and are not acceptance evidence.

The old upright world-Y/XZ pixel ratio was 0.75. With the isometric pitch and
1.79 vertical lens coefficient, it projected a source pixel to an aspect of
`0.75 * sqrt(2/3) * 1.79 = 1.096147`. The ground grid was also stretched by
`1.79 / sqrt(3)`, and its 62/44 size correction approximated sqrt(2).

The calibrated orthographic lens uses sqrt(3), and the tile correction uses
sqrt(2). At zoom 1, ground X and Y steps project to `(22,-22)` and `(-22,-22)`
screen pixels. Legacy upright pixels use sqrt(2)/44 horizontally in the XZ
plane and 1/44 vertically in world Y, giving square projected pixels. One raw
Z unit uses **4/44 world Y**, giving exactly four screen pixels. The perspective
lens uses the same coefficient and keeps its established orbit distance and
isometric pitch. Arbitrary rotated cameras remain views of baked 2D artwork.

## Coordinate contract

`util_lib/uo_coords.rs::UO_Z_TO_WORLD_SCALE` and
`assets/shaders/world/uo_coords.wesl` define the rendering conversion. Terrain
metadata remains the original signed byte biased by 128; decoding applies the
conversion. Static geometry, painter priority, light placement and hover
geometry use the same units. Reflection reconstruction first converts render
Y back to raw Z, then applies the recovered original camera's height scale.

The inverse `Vec3 -> UOVec3/UOVec4` conversion must divide by that factor and
round. The old implementation cast world Y directly to i8, returning scaled
render heights as logical Z. Tests cover every signed-byte height, multiple
map positions and both coordinate types.

Logical map/object XYZ, tile/object heights, IDs and movement flags are not
rewritten. Walking/collision must consume those preserved values; billboard
bounds, small depth biases and lens parameters remain rendering data. This
calibration does not establish every EC world-art source scale or anchor: the
former 64-pixel EC normalization was a separate assumption from the original
window/descriptor branches. The subsequent
[source-geometry repair](ec_sprite_descriptor_geometry.md) replaces it with
the selected owner's authored scale and restores positive ordinary window ends.

## Validation

The production Bevy projection test checks actual clip matrices for both ground
steps, square sprite pixels and four-pixel raw-Z steps at 1280 by 800. Sprite
bounds tests cover both source families. The full application suite passes,
as do explicit Vulkan liquid sampling/reflection and item hue-mask tests.
Native captures cover all three styles in orthographic and follow perspective:
Classic uses CC art/land, Enhanced uses CC legacy art with EC land, and KR uses
EC art/land. Each covers town `(1647,1682)`, bridge `(2834,987)`, lava
`(5744,2928)` and the road/roof fixture `(2518,561)` on map 0. All 24 calibrated
frames have settled terrain metadata, paused water, isolated settings and
verified Desktop-4 placement. Logs contain no shader/application errors.

The matched town's stronger post-calibration oak fit has 33 inliers spanning
121 by 245 pixels, 0.70-pixel RMS and aspect **0.995557**. The second has 8
inliers spanning 71 by 144 pixels, 0.56-pixel RMS and aspect **1.017099**; its
smaller sample is weaker evidence. These are sprite-interior measurements,
not claims of terrain/color parity. The original reference is smaller and
uses a different camera focus/zoom and lighting.

Continuous town/thatched roofs, orthographic bridge stair/stone-support ordering,
and existing lava-bank/crust composition survive the calibration. Perspective
bridge stairs overlap part of the deck. An unchanged-build CC-art control
reproduces the overlap; its visible footprint differs after calibration. The
older EC-art perspective control also contains it. Keep this as a remaining
geometry/priority issue, not a successful depth-parity check. Arbitrary-camera
2D artwork and every EC window/source scale remain outside this acceptance.

A subsequent [priority follow-up](static_depth_ordering.md#perspective-tie-bias)
isolates and repairs that perspective deck overlap: the fixed NDC tie bias grew
too large in perspective. A small projected view-space tie displacement fixes it
without changing this calibration or sprite geometry. The failed perspective
frames above remain historical before controls.

Temporary evidence is under `/tmp/uodynamapper-ec-followup/`:
`height-accepted-evidence.json`, `height-before-hybrid-*`,
`height-before-hybrid-persp-*`, `height-after-*`, the individual sprite fits,
and test/build logs. Native player overlays show raw Z = +20 and -20 after
app-only teleports, agreeing with the logical/rendered status. The actual
working-desktop focus and pointer are unchanged across that input test.
The pinned comparison binary isolates this phase from
concurrent unrelated builds; the shared checkout also passes its full suite.
