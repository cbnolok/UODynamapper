# Dynamapper Architecture Reference

Technical reference for AI agents working in the `dynamapper/` crate. Read this alongside `ENGINEERING.md` for any non-trivial task in that crate.

---

## Application Source Structure (`dynamapper/src/`)

```
main.rs
core.rs / core/          — Bevy plugin registration, app setup
configs.rs / configs/    — TOML settings loading
prelude.rs               — Common re-exports
core/
  app_states.rs          — Bevy states
  system_sets.rs         — System ordering / sets
  controls/              — Input handling
  render/
    scene/world/land/    — Land/terrain rendering (mesh_material.rs, draw_mesh.rs)
    dialogs/             — egui dialog systems
    overlays/            — egui overlay systems
  texture_cache/         — GPU texture atlas management
  maps.rs                — Map metadata and block management
  statics.rs             — Static object management
```

Shaders: `dynamapper/assets/shaders/world/` — `.wesl` modules using WESL `import package::shaders::...;` (see `docs/agent_notes/bevy_0_20_port_notes.md`).

---

## Rendering Presets

Three modes: **Classic 2D (0)**, **Enhanced Classic (1)**, **KR-like (2)**. Always test visual changes across all three. Use the **F3 UI** for real-time shader/uniform testing without recompiling.

The preset also picks the gump package (`GumpMapRes::decode_gump` in `core/uo_files_loader.rs`, keyed by `UniformState.effects.shading_mode`). Classic shows `gumps_cc.uddp`, or the Classic client's gump files when that package is missing or unreadable, and never `gumps_ec.uddp`. Enhanced and KR-like show `gumps_ec.uddp` and fall back to the Classic source only for ids it lacks; a decode error there is logged, not covered by the fallback. Open gump and paperdoll windows redraw in place when the preset changes. Under Enhanced and KR-like a paperdoll is the Enhanced client's 3D one (`core/render/dialogs/paperdoll_ec.rs`): the frames of `paperdoll_ec.uddp`, found by `uocf::enhanced::paperdoll::plan` from the `tileart.uop` records in `tilemeta.uddp`, drawn on one canvas for every outfit (the client's 440x732 paperdoll window around the frames' anchor, widened to both sets' frame box: 512x732) and shown at the window's 0.70 scale, sampled linearly. The user setting `window.enhanced_paperdoll = "gumps"`, picked in the Open Gump dialog, draws the old-style gump paperdoll there instead, and so does a missing `paperdoll_ec.uddp` (logged at startup); Classic always draws gumps. A worn item without a 3D frame draws nothing and is logged once. Both paperdolls draw the worn layers in the `WearableRules.kdl` order of `uocf::enhanced::paperdoll::WearableRules`, which the uocf editor shares; the gump paperdoll takes an item's layer from tiledata under Classic and from its `tileart.uop` record first under Enhanced and KR-like. Both then take off what a robe covers, unlike the editor: the garments `paperdoll_ec.uddp` decides per dress-layer item and body (measured when packing, `udd_conv::paperdoll_ec_dress`), under the user's `PaperdollOverrides.kdl` (`udd_assets::paperdoll_overrides`, applied in `core/render/dialogs/paperdoll_overrides.rs`, `PaperdollOrderRes`); a taken-off item counts as not worn and the order is taken again. The user's copy is `cache/settings/PaperdollOverrides.kdl`, read in place of the bundled file when it exists; a right-click on a paperdoll window opens its menu (`core/render/dialogs/paperdoll_menu.rs`), which shows, hides or gives back to the measurement the garment picked beneath the robe on that body and writes the choice into that copy. The gump paperdoll takes the decision for the 3D body of its profile, and none without `paperdoll_ec.uddp` (docs/agent_notes/ec_paperdoll_robe_override.md). The Enhanced client's `gumpartLegacyMUL.uop` is not used: every id in it is also in `Interface.uop`, which `gumps_ec.uddp` is packed from. A `gumps_ec.uddp` packed with `--share-with` leaves out the Classic gumps `gumps_cc.uddp` draws the same, so the Classic source serves them under Enhanced and KR-like; `SharedGumpCheck` hashes each the first time it is served against the hash `gumps_ec.uddp` lists, and a mismatch draws the gump and warns once. A startup warning counts the shared gumps when no Classic source is loaded. Atlas gumps come trimmed to their drawn texels (`udd_assets::gumps::GumpImage`): the paperdoll compose (`compose_decoded_layers` in `core/render/dialogs/gump.rs`) draws each layer's stored texels at their offset in its canvas, and Open Gump pads a gump back to its whole canvas. `gumps_ec.uddp` carries the Enhanced paperdoll's drawn rule baked into its paperdoll gumps' alpha, so every preset composes by alpha blending; Enhanced and KR-like keep the client's 256x256 skip and its hue rule. Gump and paperdoll images sample nearest when magnified and linearly through mip levels when shrunk.

---

## Architecture Rules

### TOML Configuration

- **No defaults in Rust.** Never use `.unwrap_or()` or `.unwrap_or_default()` when loading settings. All settings must be explicit in TOML files; missing settings must fail fast with a clear error.
- Bundled `assets/settings/` files load strictly. The app never writes them: it saves to `cache/settings/`, whose files overlay the bundled file of the same name and load leniently (`load_user_settings`), so a copy from an older build still opens.
- All keybindings are defined in `assets/settings/keybindings.toml`, accessed via `Settings.keybindings`. Never hardcode `KeyCode` for primary actions.

### Bevy

- **egui systems must run in `EguiPrimaryContextPass`** (not `Update` or `PostUpdate`). Bevy 0.20 + bevy_egui 0.43 requirement.
- **Never call `get_mut()` on Materials in hot paths.** It triggers expensive re-extraction and re-binding every frame; use `get()` for read-checks.
- **All internal plugins must implement `TrackedPlugin`**: add `pub registered_by: &'static str`, call `log_plugin_build(self)` at the start of `build()`. Register third-party plugins manually in `core.rs`.

### Shaders (WESL)

- WGSL variable names **cannot end with a digit** (naga-oil constraint; Bevy 0.20 replaced naga-oil with WESL, whose mangler accepts such names, so this rule is a candidate for removal).
- `#[uniform(10X)]` in Rust must exactly match `@binding(10X)` in WGSL. See `mesh_material.rs` and `land_bindings.wesl`.
- Land bindings 113 and 114 are free since the table-driven two-base blend and ripple were removed (2026-11-06); the other bindings kept their numbers.
- **Adding a new uniform** (follow this exact order):
  1. Add field to struct in `mesh_material.rs`
  2. Populate value in `draw_mesh.rs` inside `create_land_chunk_material`
  3. Add corresponding field in `land_bindings.wesl`
  4. Use the field in the shader
  5. Verify binding indices match between Rust and WGSL

### EC Land Missing-Texture Policy

In Enhanced Classic mode, missing EC terrain textures must remain explicit (black output). Do not silently fall back to Classic Client texmaps. A CC fallback may only exist as an explicit compatibility/diagnostic mode, never as a default.

### General

- Preserve logical map/object X, Y, raw Z, tile heights and movement flags losslessly for the future playable client. Billboard offsets, visual relief, camera-plane displacement and painter priority belong to rendering; they must not replace the world data used for walking, collision or surface selection.
- **No backwards compatibility**: do not add code to support legacy `.udd` file versions. Always require the latest format.
- **Do not remove existing comments.** Code is thoroughly commented for readers not versed in graphics programming.
- **Logging**: use `log::trace!` / `debug!` / `info!` / `warn!` / `error!`. Do not use `println!` for diagnostics; `println!` is acceptable only for existing end-user CLI output following established convention.

---

## Common Debugging

| Symptom | Cause | Fix |
|---------|-------|-----|
| `Binding is missing from pipeline layout` | `#[uniform(10X)]` ≠ `@binding(10X)` | Verify indices match in Rust and WGSL |
| Colors washed out / grayish | Double gamma correction | Remove manual `pow(color, 1.0/2.2)` — Bevy handles gamma |
| High GPU usage idle | `get_mut()` in hot path | Use `get()` instead |
| Dialog/overlay not showing | Wrong schedule | Use `EguiPrimaryContextPass` |

**Texture upload alignment:**
- BC7: `(width + 3) / 4 * 16`
- Rg16u: `width * 4`

**Idle eviction**: MapBlocks and texture pixel data are evicted after 60 seconds of inactivity.
