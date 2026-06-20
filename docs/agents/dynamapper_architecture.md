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

Shaders: `dynamapper/assets/shaders/worldmap/land/` — WGSL modules using naga-oil `#import`.

---

## Rendering Presets

Three modes: **Classic 2D (0)**, **Enhanced Classic (1)**, **KR-like (2)**. Always test visual changes across all three. Use the **F3 UI** for real-time shader/uniform testing without recompiling.

---

## Architecture Rules

### TOML Configuration

- **No defaults in Rust.** Never use `.unwrap_or()` or `.unwrap_or_default()` when loading settings. All settings must be explicit in TOML files; missing settings must fail fast with a clear error.
- All keybindings are defined in `assets/settings/keybindings.toml`, accessed via `Settings.keybindings`. Never hardcode `KeyCode` for primary actions.

### Bevy

- **egui systems must run in `EguiPrimaryContextPass`** (not `Update` or `PostUpdate`). Bevy 0.18 + bevy_egui 0.39 requirement.
- **Never call `get_mut()` on Materials in hot paths.** It triggers expensive re-extraction and re-binding every frame; use `get()` for read-checks.
- **All internal plugins must implement `TrackedPlugin`**: add `pub registered_by: &'static str`, call `log_plugin_build(self)` at the start of `build()`. Register third-party plugins manually in `core.rs`.

### Shaders (WGSL / naga-oil)

- WGSL variable names **cannot end with a digit** (naga-oil constraint).
- `#[uniform(10X)]` in Rust must exactly match `@binding(10X)` in WGSL. See `mesh_material.rs` and `bindings.wgsl`.
- **Adding a new uniform** (follow this exact order):
  1. Add field to struct in `mesh_material.rs`
  2. Populate value in `draw_mesh.rs` inside `create_land_chunk_material`
  3. Add corresponding field in `bindings.wgsl`
  4. Use the field in the shader
  5. Verify binding indices match between Rust and WGSL

### EC Land Missing-Texture Policy

In Enhanced Classic mode, missing EC terrain textures must remain explicit (black output). Do not silently fall back to Classic Client texmaps. A CC fallback may only exist as an explicit compatibility/diagnostic mode, never as a default.

### General

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
