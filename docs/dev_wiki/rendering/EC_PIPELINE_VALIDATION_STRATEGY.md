# EC Pipeline Validation Strategy

**Phase 13 artifact.** Describes how the EC material pipeline and renderer are validated
across the four complementary layers: parser tests, packer/sidecar tests, inspection
output, and screenshot-family regression fixtures.  Future agents must use all four layers
rather than relying on any one style alone.

---

## Why four layers

| Validation style | What it catches | What it misses |
|------------------|-----------------|----------------|
| Parser unit tests | Classification bugs, path inference errors, field loss during parsing | Visual drift, runtime routing errors |
| Packer/sidecar tests | Binary format regressions, round-trip data loss, liquid-base exclusion bugs | Visual correctness, shader behaviour |
| Inspection output | Runtime routing decisions, role/readiness classification for live tiles | Systematic regressions across whole content families |
| Screenshot-family fixtures | Visual drift for whole content categories | Root cause of structural problems |

Content-family validation matches the real visual goals better than only tile-by-tile
micro-tests.  A unit test cannot detect that water looks wrong across an entire scene.
A screenshot check cannot pinpoint which parser or packer step caused the wrong texture
to be selected.  Both are required.

---

## Layer 1 — Parser and metadata classification tests

**Location:** `lib/uocf/tests/tileart_ref_preservation.rs`, `lib/uocf/tests/ec_material_axes.rs`

**What is checked:**
- Textures-style path (e.g. `build/terraintexture/`) produces `EcLogicalFamily::Textures`, not `Unknown` or `WorldArt`
- WorldArt paths continue to produce `EcLogicalFamily::WorldArt` (regression guard)
- Raw path and normalized path are both preserved and differ for mixed-case backslash inputs
- `texture_stretch` survives from raw binary item to `TextureItem` field
- `EcPhysicalPackage::infer_from_path` correctly classifies known package families

**How to run:**
```
cargo test -p uocf tileart_ref_preservation
cargo test -p uocf ec_material_axes
```

**When to update:** When a new path keyword or package family is added to the inference logic.

---

## Layer 2 — Packer and sidecar integrity tests

**Location:** `lib/udd-conv/tests/tilemeta.rs`, `lib/udd-conv/tests/tex_art_ec.rs`, inline tests in `lib/udd-conv/src/tex_art_ec.rs`

**What is checked:**
- `TileMetaItemTextureRef` records with populated `stable_role`, `speculative_role`,
  `physical_package`, `logical_family`, and `texture_stretch` survive a UDDP write/read
  round-trip without data loss (`tilemeta_texture_refs_roundtrip_preserves_role_fields`)
- `implementation_readiness()` is correctly derived from the loaded round-tripped roles
- Liquid tiles with a `Base` or `SecondaryBase` stable role are NOT excluded from art
  packing (`liquid_with_base_role_is_included`, `liquid_with_secondary_base_role_is_included`)
- Liquid tiles with no base image, `EffectOnlyMetadata` role, or `UnknownSupport`-only role
  are correctly excluded

**How to run:**
```
cargo test -p udd-conv tilemeta
cargo test -p udd-conv tex_art_ec
cargo test -p udd-assets tilemeta
```

**When to update:** When binary format fields change, when liquid inclusion rules change, or
when a new sidecar entry is added.

---

## Layer 3 — Inspection output

**Location:** `tools/udd-conv-cli/examples/inspect_ec_tile_surface_signals.rs`

**What is exposed per queried art ID:**
- Raw tileart parser fields: `ec_logical_family`, `ec_physical_package`, `texture_stretch`,
  `raw_path`, `normalized_path`
- Processed classification: `tile_type`, `flags`, `ec_texture`, `cc_texture`
- Liquid eligibility: `liquid_base_eligible`, `liquid_exclusion_reason`
- Tilemeta chooser: `main_ec_texture_id` (legacy reason), `role_aware_reason`, `low_confidence`
- Per-ref graph: one summary line per `TileMetaItemTextureRef` containing `id`, `family`,
  `package`, `stable_role`, `speculative_role`, `readiness`, `block`, `item`, `aux`,
  `primary`, `stretch`, raw unknown fields
- Per-ref detail lines: `physical_package`, `logical_family`, `stable_role`,
  `speculative_role`, `implementation_readiness`, raw/normalized path TODO markers
- Package routing: `tex_art_ec` slot presence, `tex_land_ec` slot presence and routing outcome

**How to run (requires packed data files):**
```
cargo run --example inspect_ec_tile_surface_signals -- \
    --ecdir <path-to-ec-data> \
    --tilemeta <tilemeta.uddp> \
    --ec-art <tex_art_ec.uddp> \
    --ec-land <tex_land_ec.uddp> \
    4651 11070 1080
```

**When to update:** When a new metadata axis is added to `TileMetaItemTextureRef`, or when
a new routing decision point is added.  The inspection output must remain sufficient to
explain any visual anomaly without re-reading the full codebase.

---

## Layer 4 — Screenshot-family regression fixtures

**Location:** `dynamapper/assets/regression_fixtures/ec_content_families.toml`

**What is defined:**

| Family | Land IDs | Art IDs | Primary risk |
|--------|----------|---------|--------------|
| `marsh_and_swamp_water` | 5, 8, 70 | 4651 | Liquid routing, ripple UV |
| `lava_fields` | 10, 49, 168–169, 193–194 | 10980–10987, 11070–11080 | LavaBubbleSupport extra-pass, NormalLike |
| `snow_and_waterfall` | 8, 61–67 | 10092, 13550–13576 | WaterfallSupport ExtraPassOnly |
| `marble_and_polished_floors` | — | 1080–1089 | BaseSafe routing, SurfaceLike |
| `cave_floors_and_stone` | — | 1035–1054, 1169–1172 | SurfaceLike classification, routing |
| `blood_stains` | — | 3616, 4651, 11702, 16503, 22306–22344 | Liquid base retention |
| `roads_and_plazas` | — | 1169–1180 | Stretch/repetition, world-space UV |
| `grass_to_sand_transitions` | TBD | TBD | AlphaMask blend path (Phase 10D) |

**How to use:**
1. Run `inspect_ec_tile_surface_signals` over the family's art_ids and confirm the
   expected role/readiness classification for each representative.
2. Render the map at a known coordinate that contains the family and take a screenshot.
3. Compare the screenshot against the visual targets documented in the plan
   (`plan_ec_textures.md` "Visual Target Families" section).
4. File a regression note in `docs/agent_notes/` if a visual family drifts materially.

**When to update:** When new representative IDs are confirmed by visual cross-check, when
the terrain-blend audit produces grass-to-sand land IDs, or when a new content family
becomes a test target.

---

## What is still manually reviewed

The following areas require human visual judgment and are not yet automated:

- Correct world-space UV repetition scale for any specific material (requires screenshot
  comparison against EC screenshots; see `EC_WORLD_SPACE_UV_EVIDENCE_MAP.md`)
- Alpha-mask blend quality for grass-to-sand transitions (Phase 10D)
- Liquid ripple animation timing and amplitude
- Extra-pass effects (Phase 10G) — not yet implemented

---

## Validation checklist for pipeline changes

Before committing a change that touches parser, packer, shader, or metadata code:

1. Run `cargo test -p uocf tileart_ref_preservation` — parser classification
2. Run `cargo test -p udd-conv` — packer and sidecar integrity
3. Run `cargo test -p udd-assets tilemeta` — chooser and readiness derivation
4. Run `cargo check --workspace` — cross-crate interface integrity
5. For renderer changes: run `inspect_ec_tile_surface_signals` on at least one
   representative from each visually-affected family
6. For shader changes: take screenshots for affected families and compare against
   known-good references

This checklist is not exhaustive — content-family visual checks are always the final
arbiter of correctness for visual pipeline work.
