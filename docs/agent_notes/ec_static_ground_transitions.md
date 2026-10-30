# Static ground patches and map terrain

Map 0 at (2518,561) contains map land 3 at Z=0 with cobblestone statics
1301–1304 and grass statics 6018–6021, also at Z=0. These are item-owned
material images, not missing terrain redirects. The installed original item
metadata selects WorldArt 2000090, repetition 8, for cobblestone and 2000010,
repetition 10, for grass. Both use world coordinates (`uv_set=1`). Cobblestone
names `UOSpriteShader`; grass names `UOStaticTerrainShader`. The routing
inspector prints the retained material definitions alongside source references.

The TileArt high-word bit 2 (combined bit 34, `Rotated`, formerly `Unused1`)
takes the authored camera-plane quad branch in UOSA.exe at `0x45a080`; its
callers include `0x42cbc0`, `0x449740` and `0x4592e0`. That constructor returns without building
neighbor coverage. This establishes the geometry branch, not the absence of
blending elsewhere in the original pipeline. See `ec_original_water_code.md`
for the quad's recovered coordinates and register-level evidence.

## Runtime policy

Enhanced and KR styling now fade eligible ground patches into the retained map
at exposed boundaries. This is an intentional rendering improvement; it does
not claim to reconstruct an original static-neighbor mask generator. The
source art, repetition, authored geometry, masks and depth remain in their
existing paths. Classic styling keeps its previous coverage.

Eligibility requires a packed item-owned material slot, zero tile height,
the authored terrain-item flag, a selected WorldArt Base with world UVs and a
finite positive repetition. Bridge volume, wet tiles, roofs, foliage, NoDraw
and local-UV art are excluded. The emitted base must equal the resident map's
raw height. Missing/unmapped map sources cannot qualify. Raised floors and
multi-only objects are therefore not made transparent by this policy.

`art/static_surface.rs` builds eight neighbor bits only when rebuilding a
cached static chunk. A one-cell halo reads raw statics across native blocks and
cached chunks; graphic aliases with the same source image, repetition and Z
share coverage. Bits use spare space in `SpriteInstance.is_wet_flags`, keeping
its 128-byte stride. The terrain atlas CPU mirror supplies map height without
another package reader or GPU readback.

`art/surface_blend.wesl` fades exposed borders over 0.82 local cell, matching
the existing terrain edge-transition reach. When both sides of an axis are
exposed, each ramp finishes at the half-cell so a narrow path or isolated tile
retains an opaque center. Missing-diagonal corner falloff also ends at the
half-cell. Occupied interiors and shared filled edges remain opaque, avoiding a visible
tile grid. Existing opaque/transparent passes preserve fractional straight
alpha. There is no new draw, texture, binding or per-frame material mutation.
Different static-material patches each reveal their underlying map at their
boundary; this is not direct cross-sampling of neighboring item artwork.

## Wider visible transition

The native KR checkpoint at (2518,561), `stack-final-kr-upload-one-transition.png`,
showed a hard-looking static-cobblestone/grass boundary despite the original
0.3-cell fade. Native eligibility and grayscale-coverage probes establish that
the road instances carry the blend flag and correct neighborhood coverage.
Disabling the fade changes 37,693 pixels by more than one byte in the matched
fixture, so the existing path was not inert. A ground-plane geometry control
did not resolve the narrow transition; no geometry/depth change was retained.

The wider fade makes the blend visible at ordinary zoom in both orthographic
and perspective views. Its acceptance comparison is the soft terrain blend in
the town fixture, not merely a successful alpha test. The half-cell limits are
essential: multiplying two 0.82-cell ramps would make isolated tiles translucent
even at their centers. The shader regression checks the broad edge midpoint,
isolated center, missing-diagonal center and matching shared edges independently.
This remains an intentional styling improvement, not newly recovered UOSA mask
generation. Original artwork, eligibility, repetition, ordering and geometry
remain in their established paths.

## Validation

The wider-transition follow-up executes 81 production GPU cases, including the
new shared-edge and opaque-center checks. Thirty native captures cover the
six established fixtures in KR orthographic/perspective, Enhanced EC, Classic EC
and Enhanced CC. The target transition changes in Enhanced/KR; the five other
EC control fixtures stay within one channel byte, and all CC controls are exact.
Current-slot Classic before/after controls are separate from the older
factory-Classic reference and are byte-identical at the road and bridge.
Previews use the same frozen executable and isolated
settings, with actual metadata readiness and Desktop-4 placement/cleanup checked.
Temporary evidence uses `static-boundary-` and `static-boundary-final-matrix.json`
under `/tmp/uodynamapper-ec-followup/`. No Rust runtime or binding change is needed.

Earlier implementation validation:

The renderer suite passes 123 tests, including the explicitly run Vulkan
regression with 78 actual production-shader samples. CPU checks cover source
eligibility, height/material distinction, chunk-boundary neighbors and resident
map height. GPU cases exercise filled interiors, exposed edges, fractional
coverage and diagonal corners.

Native captures on KDE Desktop 4 cover (2518,561), bridge (2834,987), lava
(5744,2928), coast (1445,1790), swamp (1872,2416) and town (1647,1682) in
Classic, Enhanced and KR, plus CC art/map controls. Each run uses the local
control socket for teleports and captures, with isolated settings. Temporary
evidence is in `/tmp/uodynamapper-ec-followup/` under `surface-first-`,
`surface-final-` and `static-map-transition-comparison.png`. This verifies the
boundary improvement and scene regressions, not complete original-client parity.
