# Static hover geometry

`world/art/static_geometry.rs::StaticPickingResources` resolves visual bounds
through the collector's effective art source and
`resolve_static_visual_kind_cached`. It shares the collector's canonical EC
surface cache, CC owner/fallback slot precedence, source pixel scales, anchor
helpers and flat-diamond eligibility. It does not substitute rendered coordinates
for the source tile's logical X/Y/raw Z, height, flags or identity.

The old cursor implementation independently added `(0.5,1.5)` to every source.
CC upright art actually uses `(1,1)`. EC material surfaces stay at the owning
cell origin, while EC upright art retains its authored offsets and asymmetric
anchor. The old picker also reconstructed CC water/dry diamonds as upright
cards and did not implement the EC bit-34 authored camera quad.

## Shape and projection contracts

- CC upright and ordinary EC art use their actual slot's draw offset and logical
  dimensions. EC art uses the version-7 package's actual world/legacy source and
  per-owner descriptor scale, shared with collection; see
  [the source geometry contracts](ec_sprite_descriptor_geometry.md).
- CC wet sprites lie horizontally on the diagonal basis. Dry 44-by-44 diamonds
  require the collector's metadata gate and all four resident map corners at the
  object's raw Z. Missing, raised or sloping corners do not admit dry flat art.
  The shared `cc_static_uses_flat_geometry` helper enforces this in both paths.
- Terrain-owned EC material surfaces cover the unshifted owning cell. Tileart
  surface slots use the collector's EC surface/camera-quad classification.
- `StaticArtGeometry::corners` mirrors the non-shadow vertex transformations in
  `art/main.wesl`. The authored bit-34 quad uses `ec_item_quad.wesl`'s recovered
  screen constants only in the supported isometric orthographic view. It falls
  back to the horizontal cell under perspective or a rotated view, exactly as
  the drawing shader does. These constants must stay synchronized.
- Hit containment tests the two projected mesh triangles, in either winding.
  The old screen-axis bounding box admitted empty corners of tilted/diamond
  quads. Degenerate/nonfinite triangles do not hit.

A flat CC quad still has a rectangular projected image window in the standard
isometric view: its diamond is baked into the texture alpha. A rectangle alone
is therefore not evidence of the wrong geometry. Anchor placement and a
perspective control distinguish the old upright reconstruction from flat art.

The inspector's `live` route comes from `StaticArtRoute` on the resolved
geometry, including the selected CC fallback ID. The former label independently
preferred any resolvable EC land route; it could claim `ec->land` for a drawn
tileart camera quad or a bridge plank. Both hovered and listed raw statics now
use the shared decision. This is source routing, not proof of atlas residency or
GPU completion.

The inspector/highlight run after static collection. Inspection invalidates on
full camera transform/lens, zoom, settings, source packages and terrain metadata,
so a stationary pointer is refreshed when the view or a dry-diamond dependency
changes. Disabling static rendering suppresses object hits. Textured picking is
not applied to the separate radar/dot representation.

## Admitted candidates and ordinary sprite alpha

Picking now iterates logical tiles retained by the current static chunk
collector, after actual sprite/ground admission. Cached, non-visible chunks do
not contribute candidates. Expanded multi parts use their collected graphic,
position, collected signed Z and inherited hue. A sprite's owning cell no longer has
to fall inside nine blocks around the cursor's ground-plane intersection.
Neither the instance layout nor the rendered chunk ordering changes.

Ordinary CC/EC sprites and flat CC art test the selected atlas page's alpha.
Perspective-correct interpolation follows the rendered two triangles and
retains source UV orientation. The CPU sampler follows the sprite shader's
nearest-interior/bilinear-seam filter; its finite-difference UV footprint is an
approximation to GPU derivatives at rasterization boundaries. Wet art uses the
same wrapped clock and UV breathing formula. Coverage below 0.0001 passes
through to another candidate, matching the two sprite passes' shared discard.
Fractional but visible alpha remains selectable.

Decoded pages retain alpha only, load on a geometric hit and share a bounded
LRU cache (32 MiB target, 128 records; one oversized page can occupy the cache
alone). Package change ticks invalidate the pages. Failed reads do not become
opaque rectangles. Multi-image EC material surfaces retain their existing
geometry hit policy: testing only their base image would misrepresent authored
coverage masks and static-ground fades.

## Limits

This repairs visual quad construction, not a complete gameplay picking system.
There is no GPU object-ID/depth readback or exact visible fragment selection.
The existing CPU priority chooses between alpha-qualified candidates and is
not a full raster-depth query. Terrain occlusion, authored material composition
and exact screen-edge sampling remain open. Collector-backed candidates cover
admitted large art and expanded parts; this does not prove every source multi,
uncached/offscreen owner or all facets. Walking/collision must use the original
logical surfaces and movement metadata, not the highlight polygon or its small
display lift.

The opt-in Unix controller exposes the displayed inspector text as
`status.cursor_inspect`. Enable the overlay, toggle it with `KeyI`, send an
app-only pointer and allow an update before reading. Hidden-panel text may be
stale. See [the controller workflow](../../scripts/dev-helpers/renderer/README.md).

## Validation

The alpha follow-up passes 97 art tests, with two adapter-dependent cases
ignored, and a normal application build. Eight paired Desktop-4 previews retain
56 frames across all three styles and KR perspective. At the crane fixture
`(500,768)`, pointer `(638,330)` changes from the overlapping piece 18911 to the
visible beam 18910. Pointer `(590,355)` passes through that same piece's empty
window to pier 942 at Z=-5; the solid body still selects 18911. Open water has
no object hit, and the centered `(2518,561)` road still selects graphic 1304.
OS focus and pointer remain unchanged. Unhighlighted world controls are exact
in Classic/Enhanced and within one color byte in KR. This does not establish
full fragment-depth or material-mask selection.

The paired builds use the same supported terrain package and effective presets.
The first old temporary package failed the current source-metadata schema; it
is not acceptance evidence. The baseline snapshot restores the prior picker
while preserving the current dependency/configuration schema. Evidence is
`/tmp/uodynamapper-ec-followup/alpha-picking-accepted-evidence.json`.

The initial geometry phase passed 146 application tests, with five adapter-dependent tests
ignored. New regressions cover both projected triangle windings and degenerate
quads, horizontal CC water/ground in both projections, the authored EC quad's
view gate and all four resident level corners for dry CC diamonds.

Native Desktop-4 validation retains 23 complete frames across six owned runs:
seven original-build controls and sixteen final frames. The final matrix uses
Classic CC art/land, Enhanced legacy CC art with EC land, and KR EC art/land in
orthographic view, plus Enhanced perspective. Each checks road/roof at map 0
`(2518,561)`, bridge `(2834,987)` and shoreline `(1445,1790)`. The perspective
shoreline pointer is adjusted to visible water rather than reusing an
orthographic screen position that lands on grass.

The centered orthographic road resolves raw tile `(2518,561)`, graphic 1304,
Z=0 in all three styles. CC routes to art slot 17688; KR correctly reports
**tileart 1304**, rather than the old label's unrelated terrain route 16433.
Shoreline probes retain raw Z=-5 and horizontal geometry; raised roofs and
bridge planks keep their sprite paths. These checks validate resolved source
and quad construction, not alpha-accurate visible-fragment selection.

Paired Enhanced road/roof frames have zero differing world pixels outside
explicit inspector, message and highlight masks. No rendering shader, uniform,
instance layout or logical object metadata changed in this phase. Captures,
settled terrain revisions, effective presets, native inspector replies, pinned
binary hashes and copied application logs are recorded in
`/tmp/uodynamapper-ec-followup/hover-accepted-evidence.json`.
