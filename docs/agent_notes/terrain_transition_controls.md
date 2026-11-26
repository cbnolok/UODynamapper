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

`land/cc_transitions.wesl` blends packaged CC texmaps (source 4) across a
cell's edges and corners. This is an optional improvement, not a
reconstruction of Classic Client blending: the Classic client draws hard tile
edges and relies on authored transition texmaps and placed art.

- **Which edges blend.** At load, `cc_texmap_edge_colors` (`assets/land.rs`)
  stores each texmap's mean colour along its four sides, as lookup role 7
  (`LAND_PAGE_LOOKUP_ROLE_CC_EDGE_COLORS`). Each side's strip is an eighth of
  the texmap deep. A neighbour blends only where its facing side and the
  owner's differ by more than `CC_TRANSITION_SEAM_MIN`, reaching full strength
  at `CC_TRANSITION_SEAM_FULL`. Diagonal neighbours compare corner colours, the
  mean of the two sides that meet there.
  - Tiledata names and flags do not identify the hand-drawn transition texmaps
    (grass/dirt gradients 0x79-0x8C, path edges 0x8E-0xA7, sand/grass edges
    0x1D-0x4B). Their sides are painted to match the neighbours they were
    placed against, so the seam test leaves them alone; so are a material's
    variants, whose sides match each other.
  - Before 2026-11-26 every different atlas slot blended. Cobblestone variants
    and authored transitions were smeared into streaks: the "dirtying".
- **The boundary.** Each axis' boundary lies on the shared edge, moved up to
  `CC_TRANSITION_NOISE_AMPLITUDE` of a cell either way by world-space value
  noise and softened over `CC_TRANSITION_SOFTNESS`. Weights depend on the
  world position alone, so both cells of an edge agree. The inner tenth of a
  cell around its centre is always its own texture.
- **Sampling the neighbour.** The neighbour's texmap is mirrored across the
  shared edge (`1 - uv` on the offset axes) and sampled from its own atlas
  slot. Its texels next to the seam therefore face the owner, as in the map.
  - Clamping to the edge, the earlier approach, smeared the edge texel row
    across the band.
  - Repeating the texmap brings its far side to the seam, the wrong material
    for a non-tiling transition texmap.
  - A UV-coded GPU fixture checks the mirror inside the ramp.
- **What takes part.** Wet and reviewed-liquid cells, black land, Classic land
  art (source 5) and missing texmaps keep hard edges.
  - So do built surfaces: texmaps drawn for a land id whose tiledata name is in
    `BUILT_LAND_NAMES` (`land/cache.rs`: brick, flagstone, marble, planks, sand
    stone, stone, stone_paver, tile, wooden floor, worn "decimated_" variants
    too). Their patterns end exactly at the cell's edge. Blending them smeared
    Britain's brick borders into the flagstones, and the Enhanced Client keeps
    its floors hard too. The flag rides in the alpha byte of the first edge
    colour (`CC_TEXMAP_BLENDS`), so zeroed lookup data never blends.
  - Cobblestone roads are not on the list: they have no pattern to break, and
    they fade into the grass and dirt beside them.
- **Exactness.** The blend is added as deltas from the base colour. A cell with
  no seam returns the exact base and evaluates no noise. A single sampling call
  site in a loop keeps the shader's compile size down.
- **Gradients.** Gradients are captured before material branches. The CC
  opt-in path uses explicit-gradient sampling to avoid divergent derivative
  footprints.

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
Its CC cases fix the boundary noise at a half (a mocked `noise.wesl`) and cover
the seam test (matching edges, a built surface) and the mirrored neighbour
inside the ramp. Unit tests cover the edge colours and the built-surface names;
the regression fixtures `britain-cc-terrain-transitions-*` capture the feature
on in all four styles.

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

The subsequent app-only control channel closes the direct Options check. Native
Desktop-4 input opens Options with F2, clicks both controls through all four
combinations, checks the debounced cached TOML and closes/reopens the dialog.
The resulting CC-enabled town frame is byte-identical to the accepted
configuration-driven CC frame. Both controls are visible at 1280x800. The broader
shader-dialog organization remains separate work. Allow dialog fade-in to settle
before judging a capture: input acknowledgement is not visual settling.
