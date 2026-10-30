# Static light masks and visibility

`sys_collect_visible_static_lights` requires a loaded `WorldLightsPackageRes`.
Without `world_lights.uddp` in the selected UDDP folders it returns before
collecting anything, even with `enable_static_lights = true`. This also leaves
the approximate land/art local-light inputs empty. The loader reports the
missing package in the startup log. A checked option and populated tile metadata
do not establish that masks are loaded.

The 2026-10-30 investigation found the mask package absent from both the isolated
renderer workspace and `/home/claudio/test/uddp`, the configured app data folder.
The existing packer rebuilt it directly from the installed Classic client's
`lightidx.mul` and `light.mul`: 55 populated masks in a 480832-byte package. The
verified package was installed as a new file in that configured folder. Package
SHA-256: `93712c9fc20da0e1ec20c8d3e3872b5734fe5e92d43d68471565d5ce3443e362`.
No source client file or existing package was overwritten. The normal conversion
scripts already invoke `pack-lights`; there is no missing converter feature.

Rebuild with `udd-cli pack pack-lights --ccdir <classic-client> --output <data-dir>/world_lights.uddp`.
The reusable packed-data diagnostic is `cargo run -p udd-conv-cli --example
inspect_static_lights -- --packages <data-dir> --x 2507 --y 560`.
It reads actual placed statics, tile metadata and mask-slot availability.

## Separate controls and receivers

- `enable_static_lights` enables collection. Turning it off clears the collector
  and removes mask entities and approximate local-light inputs.
- Tile flag `0x00800000` marks a light source; `quality` selects the mask slot.
  A flag alone is insufficient: absent/zero-size slots are skipped. Several
  windows near `(2518,561)` have mask ID 255 and no corresponding slot; this
  observation does not establish a missing-slot bug or justify an invented mask.
- `static_light_decal_visibility` scales the existing additive mask-quads'
  alpha. Style and `global_lighting` also affect alpha. Classic/KR masks vanish
  at full daylight by design; Enhanced retains a small daylight contribution.
- `light_decal_intensity` controls approximate land/art light response; it does
  not scale the additive mask quad. Its art helper also scales the ordinary
  highlight term when fake normals are off, so changing pixels across a whole
  scene does not by itself establish correct local-light receiving.
- `enable_static_art_local_lights` separately opts art into collected local
  illumination. It defaults off. Original liquid/flat-water ownership exclusions
  remain intentional controls against terrain/static water seams.

## Native validation

The real lantern at map 0 `(2507,560,12)`, graphic 2594, uses mask 2 (150x150).
The fixture centered at `(2507,560,0)` collects 115 lights across the visible
chunks. Paired night captures with CC objects and EC terrain isolate mask
visibility 0 versus 2 while approximate interaction is zero: 10838 changed
pixels in Enhanced, 11011 in Classic and 10003 in KR, concentrated at the lantern.
Disabling collection gives zero lights and exactly matches the zero-visibility,
zero-interaction frame. Runtime status verifies each active night preset,
actual global lighting, effect values and collector count. KWin placement and
cleanup are verified; the app logs contain no shader/pipeline errors.

Direct Options interaction at the same fixture also verifies the actual
`Render Static Light Decals` checkbox: collected counts change 115 to 0 and
back to 115, with 10154 changed pixels between the closed-dialog off/on frames.
X11 focus and pointer state are unchanged. This uses the ordinary debounced
Settings path; no runtime settings override or temporary shader hook is involved.

Temporary evidence is `static-lights-accepted-evidence.json`, the package source
hash manifest, packed-item inventories and `static-lights-*` native frames.
This closes the missing-package cause of invisible masks. The separate roadmap
for projected receiver-aware decals, wider content/day/cave coverage and final
original-client lighting/color parity remains open. Other reported inactive
effects require their own source/strength/receiver checks.
