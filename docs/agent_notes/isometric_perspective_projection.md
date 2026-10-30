# Perspective lens and baked isometric art

The follow perspective camera previously multiplied its orbit Y by
`ORTHO_WIDTH_SCALE_FACTOR` (1.79), while the orthographic camera applies that
factor to the projection's vertical extent. These operations are not equivalent.
The former changes pitch from the art view's approximately 35.26 degrees to
approximately 51.69 degrees. Ground anchors then move farther apart on screen
relative to upright sprite pixels, exposing striped gaps between thatched roof
pieces at map 0 (2518,561).

A temporary perspective-only raster-depth diagnostic retained the stripes and
clipped foliage. Changing sprite painter priority therefore did not address
the primary geometry mismatch. That shader experiment was removed.

`camera/perspective.rs` now provides a custom Bevy projection with the existing
narrow perspective FOV, near/far limits, and vertical lens stretch. The follow
camera uses the isometric orbit direction while retaining the previous orbit
distance, preserving horizontal zoom framing. Terrain and art share the lens;
no sprite-only scale, synthetic depth replacement or coordinate-specific roof
rule is involved. That pitch repair retained the orthographic projection and
upright-art painter priority at the time.

The projection's frustum corners use the inverse vertical stretch. Off-center
subviews scale the lens coefficient without scaling the crop-center offset.
Bevy's viewport/world conversion and visibility therefore use the same matrix
as rendering. Projection switching uses `Projection::is_perspective()` so the
custom perspective can return to orthographic through the existing control.

The camera regression checks perspective classification, all eight projected
frustum corners, and off-center subview coordinates against full-view pixel
cropping. Native Desktop-4 captures show continuous thatched roofs with bridge,
town and foliage controls. Initial black captures during compilation of the new
pipeline are rejected; accepted captures use the `perspective-lens-settled-`
prefix in `/tmp/uodynamapper-ec-followup/`. This fixes the demonstrated pitch
mismatch, not arbitrary free-camera fidelity for art authored as 2D sprites.

## CC ground control and map arrival

The CC control exposed triangular road holes under perspective. Dry 44×44
background/surface diamonds previously retained upright, constant painter depth
against terrain's per-fragment depth. Zero-height, non-roof/non-foliage/non-Bridge
diamonds now use the existing diagonal flat-quad path only when all four resident
map corners equal the static's Z. Raised or sloped surfaces and larger sprites
retain their previous path; water keeps its separate existing rule.

This check initially did nothing: the static chunk was cached before its map
metadata arrived. `TileAtlas.content_revision` now invalidates the static cache
when map data changes or mappings clear. Identical block uploads leave the
revision unchanged. The dependency also serves EC static/map boundary coverage.
It prevents a launch/teleport race from permanently retaining an unresolved
ground classification. No per-frame material mutation or package read is added.

Native `perspective-ground-resident-enhanced-cc-` captures demonstrate continuous
roads, bridge and town controls. Temporary routing instrumentation was removed.

## Free perspective zoom

Free perspective previously accepted scroll and +/- input into `RenderZoom`,
but neither its fixed lens nor its free-movement system consumed that change.
The input zoom system now moves the free camera along its current backward
vector by `perspective_offset.length() * (new_zoom - previous_zoom)`. It uses
the actual clamped zoom, so further input at either limit cannot move the camera.
This matches the follow camera's distance change at its normal pose and also
respects a manually panned or rotated free camera. The lens and camera rotation
are preserved. Orthographic zoom and the follow camera retain their own paths.
Only the camera transform changes; object and terrain positions are untouched.

Native app-only scroll and text input checks cover Classic/CC, Enhanced with
legacy objects and EC terrain, and KR/EC in free perspective, plus follow
perspective and free orthographic controls. Enhanced also exercises rotation
and both zoom limits. Accepted frames use `free-zoom-final-` in the temporary
renderer workspace. Zooming out and back restores identical baseline pixels
in the Enhanced rotated and unrotated views. The existing lens/frustum/subview
test and the normal debug build pass. This does not establish arbitrary-angle
sprite fidelity or full-map streaming at maximum zoom.

## Exact legacy pixel calibration

The later [height calibration](static_height_calibration.md) replaces the
empirical 1.79 lens coefficient with sqrt(3) for both cameras. The isometric
pitch remains unchanged; the previous perspective orbit length is retained
explicitly with `(50,89.5,50)`, independently of the lens coefficient. Exact
sqrt(2) tile scaling, square upright sprite pixels and four pixels per raw Z
form one coherent rendering conversion. Original logical XYZ and tile/object
metadata remain unchanged.

The later [perspective tie-bias repair](static_depth_ordering.md#perspective-tie-bias)
closes the demonstrated bridge stair/deck overlap independently of lens, pitch,
sprite geometry or height calibration. Do not use that old artifact alone as
evidence that the follow camera needs another geometric correction.
