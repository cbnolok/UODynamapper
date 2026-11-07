# Shader settings audit: what each preset setting changes on screen

This note finds the shader settings that are dead, duplicated or too faint to see, and drafts a tooltip for every
setting. It changes no rendering code and no preset value. Every setting of
`dynamapper/assets/defaults/shader_presets.toml` was traced to its Rust struct, its GUI control and the shaders
that read it. Each setting that can act was then rendered in six fixed views, in every style and time of day where
it is reachable. Each was rendered once at the factory value and once neutralised.

- **Build:** dev `33242b90`.
- **Settings:** factory presets, with EC land and art at zoom 1.0.
- **Script:** [`scripts/dev-helpers/renderer/shader_settings_audit.py`](../../scripts/dev-helpers/renderer/shader_settings_audit.py).
- **Contact sheets:** one PNG per setting in `/home/claudio/rust-target/uodynamapper-scratch/shader-settings-audit/sheets/`.
  - Each row is one style and time of day.
  - Each view has three columns: baseline, variant, and a change image. In the change image, blue is unchanged and
    the colour shades through purple to orange, which marks a change of 32 levels or more (gain 8).
  - Each view is labelled with its mean change.

## Summary

The audit has 76 rows: the 65 keys of the preset file, with five vector keys split into their parts because each
part does a different job (`kr_tonemap_params`, `grade_params`, `fog_color`, `fog_params`, `gloom_params`).

| Verdict | Count | Settings |
|---|---|---|
| Useful | 23 | `global_lighting`, `enable_linear_filtering`, `enable_tonemap`, `exposure`, `enable_grading`, `grade_extra`, `grade_cool_color`, `enable_fog`, `fog_params.x`, `fog_params.w`, `enable_bent`, `enable_blur`, `enable_water_animation`, `kr_land_material_contrast_strength`, `ambient_strength`, `diffuse_strength`, `fill_strength`, `diffuse_wrap`, `gloom_params.y`, `art_contact_shadow_strength`, `enable_art_fake_normals`, `enable_art_projected_shadows`, `art_projected_shadow_length` |
| Conditional | 21 | see [Conditional](#conditional-visible-only-under-a-condition) |
| Low impact | 13 | below the line everywhere: `kr_land_shadow_mottle_strength`, `fog_color` (hue), `fog_params.y`, `fog_night_color`, `grade_warm_color`, `specular_strength`, `rim_strength`, `fill_ground_color`, `rim_color`. Faint: `enable_kr_color_wash`, `kr_color_wash_scale`, `grade_params.z`, `fill_sky_color` |
| Dead | 7 | `kr_tonemap_params.yzw`, `gloom_params.w`, `blur_radius`, `sharpening_amount`, `enable_normal_maps`, `land_normal_map_strength`, `art_mottle_strength` |
| Duplicate | 1 | `kr_tonemap_params.x` (the same as `exposure` in KR) |
| Amount of a switch | 8 | `grunge_strength`, `kr_color_wash_strength`, `grade_params.x`, `blur_strength`, `kr_transition_grime_strength`, `gloom_params.x`, `art_projected_shadow_strength`, `fog_color.a` |
| Selector | 2 | `shading_mode`, `post_process_profile` |
| Not measured | 1 | `fog_params.z` (cloud size; it acts only through `fog_params.w`) |

Main findings:

- **Never read:**
  - `kr_tonemap_params` y, z and w;
  - `gloom_params.w`, which the Advanced window labels "Fog Height Bias";
  - the alpha of `grade_warm_color` and `grade_cool_color`.
- **Never reached by a factory preset:**
  - `sharpening_amount`, `enable_normal_maps` with `land_normal_map_strength`, and `art_mottle_strength` are off in
    every slot;
  - `blur_radius` presets (0.0025, 0.005) sit below the shader's 0.5-pixel floor. The renderer always uses 0.5, and
    opening the Advanced Intensities section rewrites the value to 0.5.
- **Normal maps:** they change nothing in any of the six views even when switched on. Only EC land textures with a
  bump layer react, and only snow has one ([effect_visibility_validation.md](effect_visibility_validation.md)).
- **KR Tonemap Brightness duplicates Exposure.** In KR the tone curve multiplies the colour by both values
  (`postprocess/tonemapping.wesl`).
- **Largest effects:**
  - colour grading (Enhanced 19 to 49 mean levels, KR 7 to 27);
  - Sun Strength (25 to 63);
  - the Enhanced/KR headroom limit `grade_params.w` (14 to 30 by day);
  - global lighting at night and in caves (up to 22);
  - KR grunge (7 to 19).
- **The GUI hides settings that Classic uses.** Classic land reads Sun Strength (`diffuse_strength`, 49 to 54 mean
  when zeroed) and bent normals (2 to 2.3 mean). The panel grays Sun Strength out in Classic, and the Advanced
  window hides both controls there.
- **The Classic and Enhanced art presets are too light to see.** In those presets, Shadow Shape, Light Catch and
  Atmosphere Tint stay under the line (mean 0.24 to 0.42, at most 0.3% of pixels over 4 levels). In KR they are
  clearly visible.
- **The shipped `assets/settings/shaders/land.toml` replaces 66 factory values.** It is what a fresh install shows;
  see [The shipped land.toml](#the-shipped-landtoml-differs-from-the-factory-presets).

## Method

No rendering code was changed. Every value was set in the run's own cache copy of `settings/shaders/land.toml`.

- **Build.** Debug build of `33242b90` (dev), copied to the scratch rig.
- **Isolation.**
  - Each launch has a private working directory with copies of `assets/` and `cache/`. It uses the packages in
    `/home/claudio/test/uddp/` (EC land and art sources).
  - The bundled `assets/settings/shaders/land.toml` is deleted from the copy. Without it, the factory file is the
    default under the cache copy.
  - The overlays are off, and so is the unfocused-FPS cap.
  - Each launch runs on KDE Desktop 4 through the exact-PID preview helper, under the shared preview lock and at
    low priority.
- **Frame.** A 1024x768 window at zoom 1.0. That is below zoom 10, so none of the shader's zoom gates (blur,
  sharpening, bicubic and bent normals) switch anything off.
- **Determinism.**
  - Two terms change with time, so every baseline and variant freezes them:
    - water animation (off);
    - the KR fog cloud veil (`fog_params.w` = 0).

    Each was then measured on its own.
  - The same values were rendered in two launches, and again after switching through every slot in one launch.
    All 72 slot and view pairs gave identical frames; the worst difference was 1 level on a few pixels. Any change
    measured below therefore comes from the setting.
- **Batching.**
  - A launch writes up to four variants per render style into that style's four time-of-day slots. It switches
    between them with the F3 panel's Render Style and Time of Day buttons through the control socket, then closes
    the panel.
  - Before each capture, it checks that the app reports the expected preset key, `global_lighting` and every
    `effects` value.
  - The panel's sliders clamp a value when they draw it. Every value used is inside its slider's range, except the
    factory `blur_radius` (see below).
  - The audit took 63 launches of about 2 minutes each. The first capture of each launch was discarded.
- **Views** (map 0, z 0):

| View | Position | Why |
|---|---|---|
| slopes | 5650, 2950 | open terrain with slopes (task) |
| britain | 1434, 1699 | town: buildings, roads, roofs (task) |
| yew | 620, 1020 | forest: dense foliage statics (task) |
| covetous | 2460, 880 | mountain banks and cliffs (task) |
| water | 1500, 1750 | Britain coast: open sea, a shore and wooden docks. Among the candidates captured, this view had the most water. |
| lights | 1960, 2064 | A coastal building with wall lights. Light-source statics (tiledata flag 0x00800000) were counted in a 15x15-tile window over map 0, and the eight densest spots were probed. This one had the widest night glow: zeroing the light terms changes 5.6% of the frame by more than 4 levels at Enhanced night. The densest spots (Moonglow, Nujel'm, Britain castle) light mostly roofed interiors, which the roofs hide. |

- **Neutral value.** Each setting's row lists the value used.
  - Toggles and strengths: 0.
  - Multipliers (`global_lighting`, `exposure`, KR tonemap gain): 1.
  - Colour tints: white. For a tint whose alpha is a strength, a grey of the same luminance with the alpha kept,
    which measures the hue alone.
  - The grading extras: [0, 1, 1, 0].
  - Shapes take their identity: `sharpness_factor` 1, softness at its 0.05 minimum, `normal_mode` geometric.
  - `kr_color_wash_scale` has no neutral value, so it was doubled (72 to 144, 80 to 160).
  - The fog cloud veil was measured the other way round: preset strength against the frozen 0.
- **Probes.** Three settings are off in every factory preset, so each was switched on once in each style's
  afternoon:
  - `sharpening_amount` at 0.5;
  - normal maps at the preset strength 0.45;
  - `art_mottle_strength` at 1.0.

  The KR gloom height fade (`gloom_params.y`) was also set to 0, to see how much the fade hides.
- **Masks.**
  - Three extra launches gave a mask per view:
    - water: the pixels that move with water animation;
    - lights: the pixels that change with `world_rendering.enable_static_lights` off;
    - statics: the pixels that change with `world_rendering.enable_statics` off.
  - A changed pixel is attributed to the first mask that holds it, in that order, and otherwise to land.
  - "Whole frame" means more than half of the frame changed.
  - The map viewer draws no mobiles, so no setting was seen on mobiles.
- **Metrics.**
  - For each pixel, the change is the largest channel difference (0..255) between the variant and the baseline of
    the same slot and view.
  - Three numbers are reported: the mean, the 99th percentile, and the share of pixels that changed by more than
    4 levels.
  - A condition is one style and time of day. Its numbers come from the worst of the six views.

### Threshold

Each condition is put in one of three classes:

- **clear:** 1% of the pixels or more change by more than 4 levels;
- **faint:** under 1% over 4 levels, but a mean of 0.5 or more. This is a near-uniform shift of 1 to 3 levels. It
  shows side by side on a large flat area, but not in a busy scene;
- **below:** a mean under 0.5 and under 1% over 4 levels. This is the low-impact line: a difference image shows the
  change, but side by side it is very hard to see.

Each setting then gets one verdict:

- **useful:** clear in every condition where a preset uses it;
- **conditional:** clear only under some conditions, such as one style, day or night, or lights or slopes in view;
- **low impact:** never clear. The "below" ones are under the line in every reachable condition. The "faint" ones
  reach a mean of 0.5 somewhere, but never 1% over 4 levels;
- **dead:** never read by a shader, or never reached by a preset (the value is off, or outside the range that does
  anything);
- **duplicate:** does exactly what another setting does;
- **amount of a switch:** a strength whose 0 equals switching the effect off. It is measured through its switch;
- **selector:** picks a code path rather than an amount.

Eight settings were moved from conditional to useful by hand. Each falls short of "clear" only in the cave slot, or
in slots whose preset value sits next to the neutral one:

- `global_lighting`: faint in the mornings and the Classic afternoon. Those presets are 1.02 to 1.08, which shifts
  the whole frame by 1 to 3 levels. It is clear at night and in caves (up to 22 mean, the whole frame).
- `exposure`: the KR presets are 1.03 to 1.06, which shifts KR by 1.4 to 2.8 levels everywhere. It is clear in
  Classic and Enhanced (up to 6.1 mean, 79%).
- `enable_tonemap`: faint at the KR afternoon and night, where the KR curve is close to linear for these values.
  Elsewhere it changes 4.5 to 7.9 mean levels.
- `ambient_strength`: faint only in the Classic cave (2.70 mean, 0.77%).
- `fill_strength`: faint only in the KR afternoon (1.02 mean, 0.84%).
- `kr_land_material_contrast_strength`, `diffuse_wrap`, `fog_params.w` and `grade_cool_color`: faint only in the
  cave slot.

## Inventory

Every key of `[mode.time]`, `[mode.time.effects]`, `[mode.time.lighting]` and `[mode.time.land_lighting]` in
`assets/defaults/shader_presets.toml` (65 keys).

- **Rust structs.** The Rust side is one struct per table in
  `src/core/render/scene/world/land/mesh_material.rs`: `LandEffectsUniform`, `GlobalLightingUniforms` and
  `LandLightingUniforms`. `global_lighting` lives in the land scene uniform.
- **Loading.** `src/configs/shader_presets.rs` loads them (`LandMaterialUniformsPresets`).
- **Editing.** The F3 panel (`src/core/render/dialogs/land_shader.rs`) edits a live `UniformState` copy.
- **Columns.**
  - "Main" is the F3 window. "Adv" is the Advanced Raw Shader Knobs window and its collapsing section.
  - Factory values are given as classic / enhanced / kr, with one value when the four times of day agree, and
    otherwise as morning / afternoon / night / cave.
- **Shaders.** Shader paths are under `dynamapper/assets/shaders/`:
  - land is `world/land/` (`main.wesl`, `shading.wesl`, `lighting.wesl`, `normals.wesl`);
  - art is `world/art/`;
  - fog is `world/effects/fog_pass.wesl`;
  - tonemap and grading are in `postprocess/`.

### Whole scene

| Key | GUI control | Read by | Factory C / E / K | Static findings |
|---|---|---|---|---|
| `global_lighting` | Main Brightness; Adv Intensities | land, art and ground sprites (final multiply); `static_lights.rs` (night glow alpha) | 1.02/1.08/0.82/0.6; 1.08/1.14/0.72/0.5; 1.04/1.08/0.76/0.56 | It also decides whether the light glow sprites show: their alpha grows with 1 - value. |
| `effects.shading_mode` | Render Style buttons; Adv Shading Mode | land shading, static light style, gump package | 0 / 1 / 2 | Selector. The Render Style button replaces the slot value on every switch. |
| `effects.post_process_profile` | Adv Effects Visual Profile | land and art tonemap, KR grime, wash, material contrast, grunge texture, EC art surfaces | 0 / 1 / 2 | Selector. Like `shading_mode`, the Render Style button replaces it. |
| `effects.enable_linear_filtering` | Main Texture Smoothness; Adv Graphics | land sampling, EC art item surfaces, ground sprites | 0 / 0 / 1 | |
| `effects.enable_grunge` | Main Grunge (checkbox); Adv Effects | land, art | 0 / 1 / 1 | KR also uses it as a land shadow input (`land_surface_normal`). |
| `effects.grunge_strength` | Main Grunge (slider); Adv Effects | land, art | 0 / 0.18 / 0.65..0.58 | Amount of `enable_grunge`: 0 equals off. |
| `effects.enable_kr_color_wash` | Main KR Color Wash; Adv Graphics | land, art (profile 2 only) | 0 / 0 / 1 | |
| `effects.kr_color_wash_strength` | Main KR Color Wash (slider); Adv Graphics | land, art (profile 2) | 0 / 0 / 0.55..0.74 | Amount of the toggle. |
| `effects.kr_color_wash_scale` | Main KR Color Wash Scale; Adv Graphics | land, art (profile 2) | 72 / 72 / 72,72,80,64 | Size of the wash pattern; it has no neutral value. |
| `effects.light_decal_intensity` | Main Light/Static Interaction; Adv Graphics | land light pools (the 16 lights nearest the player, every mode except EC liquid, any time of day); art local lights | 1 / 0.75 / 1.35 | Art local lights need `world_rendering.enable_static_art_local_lights`, which is off by default. |
| `effects.static_light_decal_visibility` | Adv Graphics Static Light Decals | Rust only (`static_lights.rs`): alpha of the light glow sprites | 1 / 0.75 / 0.55 | Alpha = min(daylight + (1 - global_lighting)^gamma, daylight + max) x value. It is zero by day in Classic and KR. |
| `lighting.enable_tonemap` | Main Tonemap; Adv Toggles | land, art | 1 / 1 / 1 | |
| `lighting.exposure` | Adv Intensities | tonemap (land, art) | 1.05/1.1/1.2/0.95; 1.05/1.1/1/0.85; 1.03/1.06/1.06/0.92 | Needs Tonemap on. In KR it multiplies with `kr_tonemap_params.x`. |
| `lighting.kr_tonemap_params` | Main KR Tonemap Brightness (x) | tonemap, profile 2: x is the pre-gain | [0.9, 0, 0, 0] everywhere | x acts in KR only and does what `exposure` does (`c * exposure * pre_gain`). y, z and w are never read and have no control. |
| `lighting.enable_grading` | Main Color Grading; Adv Toggles (hidden in Classic) | land, art, every mode | 0 / 1 / 1 | Adv calls grading "fragment only" and hides its knobs in Classic, but the shaders grade in Classic too (`finish_land`). |
| `lighting.grade_params` | Adv Color Grading: x Grade Strength, y Headroom Reserve, z Fill Chroma Tint, w Enable Headroom Limit | x: grading; y, z, w: land shading modes 1 and 2 | [0.9..1.1, 0.15..0.2, 0.35..0.45, 1] in every style | x is the amount of `enable_grading`. y is read only in KR shading and only with w on; Enhanced uses a fixed 0.10. z is clamped to 0.20 in Enhanced. |
| `lighting.grade_extra` | Adv Color Grading: vibrance, saturation, contrast, split tone | grading | per style and time | Identity [0, 1, 1, 0]. With these values the grade is an exact identity, so `enable_grading` and `grade_extra` measure the same. |
| `lighting.grade_warm_color`, `grade_cool_color` | none (TOML only) | grading split toning | same in every slot | They act only through split tone (`grade_extra.w`): warm on bright pixels, cool on dark ones. The fourth component is never read. |
| `lighting.enable_fog` | Main Fog; Adv Effects | fullscreen fog pass | 0 / 0 / 1 | |
| `lighting.fog_color` | Adv Fog Params (rgb, alpha = max mix) | fog pass | light blue, a 0.5 / same / dark, a 0.28..0.36 | Alpha 0 equals fog off. |
| `lighting.fog_params` | Adv Fog Params: x Distance Density, y Height Density, z Noise Scale, w Noise Strength | fog pass | [0.02, 0, 0, 0] / same / [0.035..0.055, 0.01..0.026, 0.42..0.55, 0.14..0.22] | The fog is computed in screen space, not world space: x widens a radial vignette and y darkens the lower screen. w is a cloud veil that drifts with time, and z only matters with w. With x and y both under 0.001 the pass returns no fog at all, clouds included. |
| `lighting.fog_night_color` | Adv Night Fog Tint (rgb, blend) | fog pass | 0 / 0 / dark, a 0.25/0.22/0.55/0.6 | It blends the fog colour, so it needs fog. |

### Terrain

| Key | GUI control | Read by | Factory C / E / K | Static findings |
|---|---|---|---|---|
| `effects.normal_mode` | Main Land Relief (composite, grayed in Classic); Adv Normals | land vertex Lambert (Classic) and fragment normal (Enhanced, KR, KR grunge shadow) | 1 everywhere | Classic reads it in the vertex shader but shows no change (measured 0.00): at the grid corners where Classic lights vertices, the bicubic normal equals the geometric one. Opening the Advanced window in Classic sets it to 0 (`Geometric (fixed)`). Fragment use stops above zoom 10. |
| `land_lighting.enable_bent` | Main Land Relief (composite); Adv Toggles (hidden in Classic) | land vertex and fragment normal | 1 everywhere | It tilts the normal up where a neighbour tile is higher. Classic reads it but cannot edit it. Fragment use stops above zoom 20. |
| `effects.enable_blur` | Adv Toggles Blur (hidden in Classic) | land | 0 / 0 / 1 | Off above zoom 10. |
| `effects.blur_strength` | Adv Intensities | land | 0.15 / 0.15 / 0.28 | Amount of `enable_blur`. |
| `effects.blur_radius` | Adv Intensities (0.5..8 screen pixels) | land | 0.0025 / 0.0025 / 0.005 | Below both the shader's 0.5-pixel floor and the slider minimum. Every preset draws 0.5 px, and opening Adv Intensities rewrites the value to 0.5. Only 0 does anything: it turns blur off. |
| `effects.sharpening_amount` | Adv Graphics | land | 0 everywhere | Off in every preset. Not the same as `graphics.toml` `sharpening_strength`. |
| `effects.enable_water_animation` | Main Water Animation; Adv Effects | land, art, ground water | 1 everywhere | Every preset switch keeps the live value, so the slot value counts only for the startup slot. |
| `effects.enable_kr_transition_grime` | Main KR Transition Grime; Adv Graphics | land, profile 2 | 0 / 0 / 1 | |
| `effects.kr_transition_grime_strength` | Main (slider); Adv Graphics | land, profile 2 | 0 / 0 / 0.65..0.58 | Amount of the toggle. |
| `effects.enable_normal_maps` | Main Normal Maps (EC land only); Adv Graphics | land, modes 1 and 2, EC land textures with a normal layer, not EC liquid | 0 everywhere | Off in every preset. Only snow has a bump layer, and a probe changed 0 pixels in all six views. |
| `effects.land_normal_map_strength` | Main (slider); Adv Graphics | land, with normal maps | 0.45 | No preset reaches it. |
| `effects.kr_land_temperature_strength` | Main (grayed unless KR shading); Adv Graphics | land, mode 2 | 0 / 0 / 0.75..0.65 | |
| `effects.kr_land_relief_shadow_strength` | Main; Adv Graphics | land, mode 2 | 0 / 0 / 0.7..0.58 | |
| `effects.kr_land_shadow_mottle_strength` | Main; Adv Graphics | land, mode 2 | 0 / 0 / 0.6..0.48 | Its largest possible darkening is about 3%, even at the slider maximum 1.5. |
| `effects.kr_land_material_contrast_strength` | Main (grayed unless KR profile); Adv Graphics | land, profile 2, not liquid | 0 / 0 / 0.58..0.42 | A small saturation and contrast boost by material kind (foliage, stone, bright), with grain per tile. |
| `lighting.light_color` | Adv Lighting Colors | land modes 1 and 2 (sun colour); art Light Catch | warm by day, cool at night, the same in every style | Classic land ignores it. |
| `lighting.atmosphere_tint` | Adv Lighting Colors | art Atmosphere Tint; the gloom tint when `gloom_color` is black | dark blue, the same in every style | KR sets `gloom_color`, so only the art tint reads it. |
| `lighting.ambient_strength` | Main Terrain Ambient Light; Adv Intensities | land in every mode; flat water statics; EC item water | 0.18..0.06 / 0.18..0.04 / 0.25..0.07 | |
| `land_lighting.diffuse_strength` | Main Sun Strength (grayed in Classic); Adv Diffuse (hidden in Classic) | land in every mode; flat water statics; EC item water | 1/1/0.7/0.85; 1.05/1.05/0.45/0.5; 1.28/1.35/0.62/0.54 | Classic land reads it (`shade_mode0_classic_vertex`), but the GUI does not let Classic edit it. |
| `land_lighting.specular_strength` | Main Highlights (composite with rim); Adv Intensities | land modes 1 (x0.5) and 2 | 0 / 0.03 / 0.035..0.02 | The Highlights slider can raise it to 0.25. |
| `land_lighting.rim_strength` | Main Highlights; Adv "Rim (KR only)" | land modes 1 (x0.25, rim power only) and 2 | 0 / 0.06..0.22 / 0.11..0.16 | The Adv label says KR only, but Enhanced reads it too. |
| `land_lighting.fill_strength` | Main Soft Lighting (composite with wrap); Adv Intensities | land modes 1 and 2 | 0 / 0.28..0.12 / 0.34..0.18 | |
| `land_lighting.sharpness_factor` | Main Land Relief (composite); Adv Intensities | land modes 1 and 2, through `sharpness_mix` | 1 / 1.5 / 2 | No effect when `sharpness_mix` is 0. |
| `land_lighting.sharpness_mix` | Main Land Relief (composite); Adv Intensities | land modes 1 (clamped to 0.4) and 2 | 0 / 0.25 / 0.55 | |
| `land_lighting.diffuse_wrap` | Main Soft Lighting (composite); Adv "Diffuse Wrap (KR only)" | land mode 2 | 0 / 0 / 0.35..0.15 | |
| `land_lighting.fill_sky_color`, `fill_ground_color` | Adv Lighting Colors | land modes 1 and 2 (hemisphere fill, rgb x a) | the same in every style | Both need `fill_strength`. The fill mixes ground into sky by how much a surface faces up, and terrain faces up, so the ground colour is close to unused. |
| `land_lighting.rim_color` | Adv Lighting Colors (rgb, power) | land mode 2: rgb and power; mode 1: power only | the same in every style | Enhanced ignores its colour. |
| `lighting.enable_gloom` | Adv Toggles Gloom (sets `gloom_params.x` to 0 or 0.20) | land modes 1 and 2 | 0 / 0 / 1 | |
| `lighting.gloom_params` | Adv Gloom: x Amount, y Height Fade Height, z Shadow Bias, w "Fog Height Bias" | land modes 1 and 2 (`apply_gloom`) | set in every style, used in KR | x is the amount of the toggle. y fades gloom out above that height (0 = no fade). World height is Z x 4/44, so 0.005..0.01 keeps gloom only on land at Z <= 0, while the slider runs 0..200. No compiled shader reads w (only `legacy/world/fog.wesl` does). |
| `lighting.gloom_color` | Adv Gloom Tint (rgb, desaturation) | land gloom | 0 / 0 / blue-grey, a 0.1..0.28 | A black rgb falls back to `atmosphere_tint`. |

### Art (statics)

| Key | GUI control | Read by | Factory C / E / K | Static findings |
|---|---|---|---|---|
| `effects.art_shadow_strength` | Standard art Shadow Shape; Adv Graphics | art, ground | 0.12 / 0.12 / 0.24..0.34 | |
| `effects.art_highlight_strength` | Standard art Light Catch; Adv Graphics | art, ground | 0.08 / 0.08 / 0.42..0.24 | |
| `effects.art_depth_tint_strength` | Standard art Atmosphere Tint; Adv Graphics | art upright sprites (not floors, ground or background) | 0.1 / 0.1 / 0.2..0.34 | Amount of the `atmosphere_tint` colour. |
| `effects.art_contact_shadow_strength` | Standard art Contact Shadows; Adv Graphics | art, only inside the fake-normals block | 0 / 0 / 0.3..0.42 | Needs Fake Normals. |
| `effects.art_mottle_strength` | Standard art Mottle; Adv Graphics | art: shadow-shape noise and fake-normal dapple | 0 everywhere | Off in every factory preset. The shipped `land.toml` turns it on in KR. |
| `effects.enable_art_fake_normals` | KR-style art Fake Normals; Adv Graphics | art, ground | 0 / 0 / 1 | |
| `effects.kr_art_temperature_strength` | KR-style art Temperature; Adv Graphics | art, only inside the fake-normals block | 0 / 0 / 0.75..0.65 | Needs Fake Normals. |
| `effects.enable_art_projected_shadows` | KR-style art Projected Shadows (checkbox); Adv Graphics | art | 0 / 0 / 1 | |
| `effects.art_projected_shadow_strength` | KR-style art Projected Shadows (slider); Adv Graphics | art | 0 / 0 / 0.16..0.24 | Amount of the toggle. |
| `effects.art_projected_shadow_length` | Adv Graphics | art | 0.42 / 0.42 / 0.28..0.44 | Needs projected shadows. |
| `effects.art_projected_shadow_softness` | Adv Graphics | art (clamped to 0.05..1) | 0.62 / 0.62 / 0.62..0.74 | Needs projected shadows. |

## Measurements

Each cell reads mean / 99th percentile / share over 4 levels, as the range over the four times of day, in the worst
view of each condition:

- "off in preset": the preset leaves the setting or its switch off, or the style does not read it, so it cannot
  act;
- "= neutral": the preset value already equals the neutral value;
- "(NC only)" and similar: only those times of day (Morning, Afternoon, Night, Cave) are reachable or differ from
  neutral.

"Where" names the regions holding most of the changed pixels. The last four rows are the probes.

| Setting | Neutral value | Classic | Enhanced | KR | Where | Verdict |
|---|---|---|---|---|---|---|
| `global_lighting` | 1.0 (identity multiplier) | 0.97-17.01 / 39 / 0.0-100.0% | 3.40-21.99 / 34 / 0.6-100.0% | 1.79-16.57 / 29 / 0.0-99.2% | whole frame | useful |
| `effects.normal_mode` | 0 (geometric normals) | 0.00 / 0 / 0.0% | 0.93-1.61 / 19 / 8.0-10.9% | 0.90-1.83 / 17 / 5.1-13.2% | land | conditional |
| `effects.enable_blur` | 0 | off in preset | off in preset | 2.17-4.09 / 18 / 14.4-38.9% | land | useful |
| `effects.blur_radius` | 0.5 (the shader floor of half a screen pixel) | off in preset | off in preset | 0.00 / 0 / 0.0% | - | dead |
| `effects.enable_linear_filtering` | 0 (nearest texels) | = neutral | = neutral | 4.24-7.75 / 38 / 37.5-53.9% | whole frame | useful |
| `effects.enable_grunge` | 0 | off in preset | 0.81-1.01 / 2 / 0.0% | 7.37-18.71 / 29 / 51.3-99.2% | whole frame | conditional |
| `effects.enable_kr_transition_grime` | 0 | off in preset | off in preset | 1.33-3.23 / 11 / 0.0-21.6% | land | conditional |
| `effects.enable_kr_color_wash` | 0 | off in preset | off in preset | 0.71-1.14 / 3 / 0.0% | statics, land | low impact (faint) |
| `effects.kr_color_wash_scale` | twice the preset (no identity value; GUI maximum 160) | off in preset | off in preset | 0.96-1.19 / 2 / 0.0% | statics | low impact (faint) |
| `effects.light_decal_intensity` | 0 | 0.20-0.27 / 4 / 0.3-0.6% | 0.33-0.50 / 8 / 1.3-3.2% | 0.35-0.50 / 7 / 1.2-3.1% | lights | conditional |
| `effects.static_light_decal_visibility` | 0 | 0.60-1.21 / 53 / 2.2-2.7% (NC only) | 0.08-1.30 / 53 / 0.6-3.1% | 0.15-0.31 / 13 / 1.3-1.9% (NC only) | lights | conditional |
| `effects.enable_art_fake_normals` | 0 | = neutral | = neutral | 1.48-2.00 / 22 / 10.6-21.9% | statics | useful |
| `effects.art_contact_shadow_strength` | 0 | off in preset | off in preset | 0.34-0.40 / 5 / 1.1-2.3% | statics | useful |
| `effects.kr_land_temperature_strength` | 0 | off in preset | off in preset | 0.35-1.34 / 6 / 0.0-2.8% | land | conditional |
| `effects.kr_land_relief_shadow_strength` | 0 | off in preset | off in preset | 1.24-2.40 / 6 / 0.0-18.2% | land | conditional |
| `effects.kr_land_shadow_mottle_strength` | 0 | off in preset | off in preset | 0.00-0.01 / 1 / 0.0% | - | low impact |
| `effects.kr_land_material_contrast_strength` | 0 | off in preset | off in preset | 0.60-3.53 / 18 / 0.0-30.8% | land | useful |
| `effects.kr_art_temperature_strength` | 0 | off in preset | off in preset | 0.16-0.27 / 7 / 0.1-1.4% | statics | conditional |
| `effects.enable_art_projected_shadows` | 0 | off in preset | off in preset | 0.61-1.58 / 23 / 5.4-8.8% | statics | useful |
| `effects.art_projected_shadow_length` | 0 (shadow collapsed to the base) | off in preset | off in preset | 0.61-1.58 / 23 / 5.4-8.8% | statics | useful |
| `effects.art_projected_shadow_softness` | 0.05 (hardest edge, shader minimum) | off in preset | off in preset | 0.08-0.18 / 7 / 0.7-1.5% | statics, land | conditional |
| `effects.art_shadow_strength` | 0 | 0.24-0.28 / 2 / 0.0% | 0.25-0.32 / 2 / 0.0% | 1.32-1.48 / 10 / 12.0-21.3% | statics | conditional |
| `effects.art_highlight_strength` | 0 | 0.33-0.37 / 2 / 0.0% | 0.34-0.42 / 2 / 0.0% | 1.42-3.25 / 23 / 15.5-23.8% | statics | conditional |
| `effects.art_depth_tint_strength` | 0 | 0.25-0.28 / 3 / 0.0% | 0.27-0.33 / 4 / 0.0-0.3% | 1.05-1.49 / 10 / 2.2-20.4% | statics | conditional |
| `lighting.enable_fog` | 0 | off in preset | off in preset | 0.54-0.85 / 13 / 2.2-5.0% | statics, land | useful |
| `lighting.fog_color` | grey of the same luminance (alpha kept) | off in preset | off in preset | 0.14-0.20 / 3 / 0.0% | - | low impact |
| `lighting.fog_params.x` | 0 (distance density) | off in preset | off in preset | 0.26-0.56 / 13 / 1.8-4.5% | statics, land | useful |
| `lighting.fog_params.y` | 0 (height density) | off in preset | off in preset | 0.25-0.29 / 1 / 0.0% | - | low impact |
| `lighting.fog_params.w` | cloud veil on at the preset value against the frozen baseline (0) | off in preset | off in preset | 1.27-2.26 / 7 / 0.2-14.6% | statics, land | useful |
| `lighting.fog_night_color.w` | 0 (night blend: day fog colour only) | off in preset | off in preset | 0.11-0.36 / 4 / 0.0-0.2% | statics, land | low impact |
| `lighting.enable_tonemap` | 0 | 4.60-7.90 / 31 / 13.6-40.3% | 4.46-7.51 / 33 / 24.3-55.7% | 1.00-4.49 / 7 / 0.5-43.1% | whole frame | useful |
| `lighting.exposure` | 1.0 | 1.70-6.10 / 9 / 0.0-79.3% | 2.30-4.88 / 9 / 0.0-69.3% (MAC only) | 1.42-2.84 / 4 / 0.0-0.4% | whole frame | useful |
| `lighting.kr_tonemap_params.x` | 1.0 (identity gain) | off in preset | off in preset | 2.85-5.25 / 7 / 10.8-73.9% | whole frame | duplicate of exposure |
| `lighting.enable_grading` | 0 | off in preset | 19.18-49.39 / 116 / 97.7-99.9% | 7.29-27.45 / 76 / 65.1-98.4% | whole frame | useful |
| `lighting.grade_params.y` | 0 (KR headroom reserve) | off in preset | off in preset | 0.00-7.04 / 10 / 0.0-94.8% | whole frame | conditional |
| `lighting.grade_params.z` | 0 (fill chroma tint) | off in preset | 0.06-0.33 / 2 / 0.0% | 0.09-0.98 / 4 / 0.0-0.7% | land | low impact (faint) |
| `lighting.grade_params.w` | 0 (headroom limit off) | off in preset | 0.00-14.65 / 35 / 0.0-92.5% | 0.00-30.18 / 45 / 0.0-98.9% | whole frame | conditional |
| `lighting.grade_extra` | [0, 1, 1, 0] (no vibrance, saturation 1, contrast 1, no split toning) | off in preset | 19.18-49.39 / 116 / 97.7-99.9% | 7.29-27.45 / 76 / 65.1-98.4% | whole frame | useful |
| `lighting.grade_warm_color` | white | off in preset | 0.00-0.08 / 2 / 0.0% | 0.00-0.02 / 1 / 0.0% | statics | low impact |
| `lighting.grade_cool_color` | white | off in preset | 2.87-5.69 / 7 / 7.3-90.7% | 2.18-4.67 / 6 / 0.3-65.1% | whole frame | useful |
| `lighting.enable_gloom` | 0 | off in preset | off in preset | 1.41-1.90 / 5 / 0.1-2.0% | land | conditional |
| `lighting.gloom_params.z` | 0 (no shadow bias) | off in preset | off in preset | 1.33-2.14 / 5 / 0.1-3.0% | land | conditional |
| `lighting.gloom_color` | [0, 0, 0, 0] (tint falls back to atmosphere_tint, no desaturation) | off in preset | off in preset | 0.28-1.08 / 5 / 0.0-1.7% | land | conditional |
| `lighting.light_color` | white | 0.04-0.09 / 1 / 0.0% | 0.57-9.14 / 31 / 2.6-88.6% | 0.34-5.29 / 9 / 0.2-54.7% | whole frame | conditional |
| `lighting.atmosphere_tint` | white (identity of the art depth tint) | 0.25-0.28 / 3 / 0.0% | 0.27-0.33 / 4 / 0.0-0.3% | 1.05-1.49 / 10 / 2.2-20.4% | statics | conditional |
| `lighting.ambient_strength` | 0 | 2.70-8.24 / 14 / 0.8-96.8% | 2.69-9.46 / 19 / 1.6-92.4% | 3.65-10.76 / 22 / 32.1-79.1% | whole frame | useful |
| `land_lighting.enable_bent` | 0 | 1.97-2.32 / 23 / 11.8-13.5% | 1.90-2.45 / 29 / 11.9-15.7% | 1.51-2.21 / 19 / 10.0-14.5% | land | useful |
| `land_lighting.diffuse_strength` | 0 | 48.96-54.07 / 84 / 99.7-99.7% | 31.96-62.68 / 115 / 72.4-99.1% | 25.39-60.34 / 121 / 72.4-99.0% | whole frame | useful |
| `land_lighting.specular_strength` | 0 | off in preset | 0.00 / 0 / 0.0% | 0.00-0.01 / 0 / 0.0% | land | low impact |
| `land_lighting.rim_strength` | 0 | off in preset | 0.00-0.01 / 0 / 0.0% | 0.10-0.15 / 4 / 0.8-1.0% | land | low impact |
| `land_lighting.fill_strength` | 0 | off in preset | 2.55-5.99 / 16 / 2.0-74.1% | 0.52-4.97 / 9 / 0.8-46.4% | whole frame | useful |
| `land_lighting.sharpness_factor` | 1.0 (unshaped Lambert) | off in preset | 0.37-0.90 / 4 / 0.0-0.8% | 0.08-2.27 / 5 / 0.0-5.6% | land | conditional |
| `land_lighting.sharpness_mix` | 0 | off in preset | 0.37-0.90 / 4 / 0.0-0.8% | 0.08-2.27 / 5 / 0.0-5.6% | land | conditional |
| `land_lighting.diffuse_wrap` | 0 | off in preset | off in preset | 0.54-1.88 / 12 / 0.0-4.3% | land | useful |
| `land_lighting.fill_sky_color` | grey of the same luminance (alpha kept) | off in preset | 0.06-0.33 / 2 / 0.0% | 0.10-0.98 / 4 / 0.0-0.7% | land | low impact (faint) |
| `land_lighting.fill_ground_color` | grey of the same luminance (alpha kept) | off in preset | 0.00 / 0 / 0.0% | 0.00 / 0 / 0.0% | - | low impact |
| `land_lighting.rim_color` | grey of the same luminance (power in alpha kept) | off in preset | 0.00 / 0 / 0.0% | 0.02-0.04 / 1 / 0.0-0.2% | land | low impact |
| `effects.sharpening_amount` | probe: 0.5 | 15.59 / 79 / 62.5% | 19.85 / 91 / 64.1% | 8.95 / 57 / 56.0% | whole frame | dead (probe) |
| `effects.enable_normal_maps` | probe: on at the preset strength 0.45 | off in preset | 0.00 / 0 / 0.0% | 0.00 / 0 / 0.0% | - | dead (probe) |
| `effects.art_mottle_strength` | probe: 1.0 | 0.46 / 4 / 0.3% | 0.53 / 4 / 0.7% | 0.60 / 6 / 2.9% | statics | dead (probe) |
| `lighting.gloom_params.y` | probe: height fade off (0), so gloom covers every height | off in preset | off in preset | 2.26-2.89 / 8 / 0.3-11.9% | land | useful (probe) |

### Water animation

`enable_water_animation` was measured as frozen against animated. It is the only animated term apart from the fog
cloud veil. It changes only water: the water and lights views both show sea, and the four land views show no change
at all. Each cell is the mean change and the share of pixels over 4 levels.

| condition | slopes | britain | yew | covetous | water | lights |
|---|---|---|---|---|---|---|
| classic.morning | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 3.29 / 29.11% | 1.66 / 13.95% |
| classic.afternoon | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 3.46 / 29.15% | 1.38 / 10.72% |
| classic.night | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 2.20 / 18.21% | 0.68 / 4.72% |
| classic.cave | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 2.00 / 15.94% | 1.01 / 7.26% |
| enhanced.morning | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 3.69 / 32.28% | 1.94 / 16.60% |
| enhanced.afternoon | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 3.92 / 34.83% | 2.15 / 18.79% |
| enhanced.night | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 1.79 / 13.00% | 1.00 / 7.27% |
| enhanced.cave | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 1.40 / 7.44% | 0.79 / 4.21% |
| kr.morning | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 3.68 / 31.99% | 2.13 / 17.64% |
| kr.afternoon | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 4.03 / 34.84% | 2.34 / 19.20% |
| kr.night | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 1.56 / 10.28% | 0.93 / 6.18% |
| kr.cave | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 0.00 / 0.00% | 1.04 / 3.59% | 0.64 / 2.48% |

## Verdicts

Numbers are mean / share over 4 levels in the worst view of each condition, unless a p99 is given.

### Dead (7)

- `kr_tonemap_params` y, z, w: never read; only x reaches the tone curve.
- `gloom_params.w` (Adv Gloom "Fog Height Bias"):
  - no compiled shader reads it (only `legacy/world/fog.wesl`);
  - the label also places it under fog, though it sits in the gloom vector.
- `blur_radius`:
  - the factory values (0.0025 in Classic and Enhanced, 0.005 in KR) are below the shader's 0.5-pixel floor;
  - preset against 0.5: 0.00 mean, 0 pixels changed;
  - the slider minimum is 0.5, so opening Adv Intensities rewrites the value.
- `sharpening_amount`:
  - 0 in every preset;
  - probe at 0.5: Classic 15.6 / 62.5%, Enhanced 19.9 / 64.1%, KR 9.0 / 56.0%, across the whole frame. It would
    be a strong control if a preset used it.
- `enable_normal_maps` and `land_normal_map_strength`:
  - off in every preset;
  - probe (on at 0.45, Enhanced and KR afternoon): 0.00 in all six views;
  - only snow has a bump layer, and no view holds snow.
- `art_mottle_strength`:
  - 0 in every factory preset;
  - probe at 1.0, on statics: Classic 0.46 / 0.3%, Enhanced 0.53 / 0.7%, KR 0.60 / 2.9%;
  - the shipped `land.toml` sets 0.9 to 1.15 in KR. There it would be conditional: visible in KR only.
- Not counted as rows: the alpha of `grade_warm_color` and `grade_cool_color` is never read.

### Duplicate (1)

- `kr_tonemap_params.x` (Main "KR Tonemap Brightness") duplicates `lighting.exposure` in KR.
  - The KR curve starts from `c * exposure * pre_gain`, so the two multiply.
  - Preset 0.9 against 1.0: 2.9 to 5.3 mean, 11 to 74%, across the whole frame.
  - Outside KR it is never read, and the panel grays it out there.

### Low impact (13)

These are below the line in every reachable condition:

| Setting | Preset | Worst condition | Note |
|---|---|---|---|
| `kr_land_shadow_mottle_strength` | KR 0.6..0.48 | 0.01 / 0% (max 1 level) | The shader caps its darkening at about 3% even at 1.5. |
| `fog_color` (hue) | KR dark blue | 0.20 / 0% | Its alpha is the fog amount; see "Amount of a switch". |
| `fog_params.y` | KR 0.01..0.026 | 0.29 / 0% | |
| `fog_night_color` | KR blend 0.25..0.6 | 0.36 / 0.24% | Measured through its blend (alpha 0 = day fog colour only). |
| `grade_warm_color` | every slot [1.08, 0.98, 0.9] | 0.08 / 0.01% | No GUI control. |
| `specular_strength` | E 0.03, K 0.035..0.02 | 0.01 / 0% | Only at the preset value: the Highlights slider reaches 0.25, about 7 times more, which was not measured. |
| `rim_strength` | E 0.06..0.22, K 0.11..0.16 | KR 0.15 / 0.99%, Enhanced 0.01 / 0% | Just under the line in KR. Enhanced uses a quarter of it. |
| `fill_ground_color` | every slot | 0.00 / 0% | Terrain faces up, so the ground colour gets almost no weight. |
| `rim_color` | every slot | 0.04 / 0.2% | Enhanced reads only its power. |

These are faint: a mean of 0.5 or more somewhere, but never 1% over 4 levels:

| Setting | Preset | Worst condition | Note |
|---|---|---|---|
| `enable_kr_color_wash` | KR on, 0.55..0.74 | 1.14 / 0.03%, p99 3 | A near-uniform 1-level shift over land and statics. |
| `kr_color_wash_scale` (doubled) | KR 64..80 | 1.19 / 0% | |
| `grade_params.z` | 0.35..0.45 | KR afternoon 0.98 / 0.66% | Enhanced clamps it to 0.20 (0.33 / 0%). |
| `fill_sky_color` (hue) | every slot | KR afternoon 0.98 / 0.66% | Same numbers as `grade_params.z`, which scales the fill's colour. |

### Conditional (visible only under a condition)

| Setting | Clear under | Numbers there | Elsewhere |
|---|---|---|---|
| `enable_grunge` | KR, every time | 7.4..18.7 / 51..99%, whole frame | The Enhanced preset (0.18) is faint: 0.8..1.0 / 0%. Off in Classic. |
| `light_decal_intensity` | light sources in view, Enhanced and KR, any time of day | 0.33..0.50 / 1.2..3.2%, p99 5..8 | Classic (1.0) stays below: up to 0.27 / 0.65%. |
| `static_light_decal_visibility` | light sources in view, night and cave, every style | C 0.60..1.21 / 2.2..2.7%; E 0.79..1.30 / 2.7..3.1%; K 0.15..0.31 / 1.3..1.9%; p99 up to 53 | No glow by day in Classic and KR (alpha 0); Enhanced day 0.08 / 0.6%. |
| `grade_params.y` | KR, by day; needs `grade_params.w` | 6.8..7.0 / 84..95%, whole frame | 0 at night and in caves, where land never reaches the cap. Enhanced uses a fixed 0.10. |
| `grade_params.w` | Enhanced and KR, by day | E 13.9..14.7 / 90..93%; K 25.5..30.2 / 98..99% | 0 at night and in caves. |
| `normal_mode` | Enhanced and KR, slopes and cliffs | 0.9..1.8 / 5..13% | Classic: 0.00. |
| `enable_kr_transition_grime` | KR, by day | 2.5..3.2 / 1.9..21.6% | Night and cave faint (1.3..1.8 / 0%). |
| `kr_land_temperature_strength` | KR afternoon | 1.34 / 2.84% | Morning faint (0.82 / 0.12%); night and cave below. |
| `kr_land_relief_shadow_strength` | KR, by day | 2.3..2.4 / 15..18% | Night and cave faint. |
| `light_color` | Enhanced at every time; KR afternoon and night | E 0.57..9.14 / 2.6..89%; K 0.56 / 1.3% and 5.29 / 55% | Strongest at night (cool light). KR cave faint, KR morning below, Classic below (0.09 or less): Classic land ignores it. |
| `atmosphere_tint` | KR, statics | 1.05..1.49 / 2.2..20% | Classic and Enhanced below (0.33 / 0.3% or less). Same numbers as `art_depth_tint_strength`: the colour and its amount. |
| `sharpness_factor`, `sharpness_mix` | KR night and cave | 2.1..2.3 / 2.5..5.6% | KR by day below (0.08); Enhanced faint at night and cave, below by day. Same numbers: the factor acts only through the mix. |
| `enable_gloom` | KR night, land at sea level (Britain, Yew, lights) | 1.90 / 1.96% | Other times faint (1.4..1.7 / 0.1..0.8%). |
| `gloom_params.z` | KR night and cave | 1.9..2.1 / 1.6..3.0% | By day faint. |
| `gloom_color` | KR night | 1.08 / 1.68% (Yew) | Cave faint, day below. Black falls back to the atmosphere tint, so this measures the difference between the two colours. |
| `art_shadow_strength` | KR | 1.3..1.5 / 12..21% (statics) | The Classic and Enhanced presets (0.12) stay below: 0.24..0.32 / 0%. |
| `art_highlight_strength` | KR | 1.4..3.3 / 15..24% | The Classic and Enhanced presets (0.08) stay below: 0.33..0.42 / 0.02% or less. |
| `art_depth_tint_strength` | KR | 1.05..1.49 / 2.2..20% | The Classic and Enhanced presets (0.1) stay below: 0.25..0.33 / 0.3% or less. |
| `kr_art_temperature_strength` | KR by day, many lit statics in view | 0.24..0.27 / 1.4% | Night and cave below. Needs Fake Normals. |
| `art_projected_shadow_softness` | KR by day, Yew | 0.15..0.18 / 1.3..1.45% | Night and cave below. Needs Projected Shadows. |

### Useful (23)

The full numbers are in the measurement table. Under the threshold section above, eight of these were moved from
conditional by hand. `gloom_params.y` comes from a probe: setting the fade to 0 (gloom at every height) changes
2.3 to 2.9 mean and up to 12% at KR night. The preset fade (0.005 to 0.01) therefore keeps the gloom on land at
Z <= 0. `enable_water_animation` acts on water only.

### Amount of a switch (8)

For each of these, 0 equals the switch off, so the switch's numbers cover them:

| Amount | Its switch |
|---|---|
| `grunge_strength` | `enable_grunge` |
| `kr_color_wash_strength` | `enable_kr_color_wash` |
| `grade_params.x` | `enable_grading` |
| `blur_strength` | `enable_blur` |
| `kr_transition_grime_strength` | `enable_kr_transition_grime` |
| `gloom_params.x` | `enable_gloom`; the Adv toggle writes 0 or 0.20 to it |
| `art_projected_shadow_strength` | `enable_art_projected_shadows` |
| `fog_color.a` | `enable_fog`; it is the maximum fog mix |

### Selectors (2)

`shading_mode` and `post_process_profile` pick the code path. The Render Style buttons overwrite both on every
switch (`apply_render_preset`), so the value stored in a slot never matters once a style is picked.

## GUI findings

1. **Classic uses two settings the GUI won't let it edit.**
   - `diffuse_strength`: Main "Sun Strength" is grayed out in Classic, and Adv "Diffuse" is hidden there. Zeroed
     in Classic, it changes 49 to 54 mean.
   - `enable_bent`: Adv "Bent normals" is hidden in Classic. It changes 2.0 to 2.3 mean, 12 to 13.5%.
2. **Colour grading works in Classic, though the Advanced window says otherwise.** Adv labels it "fragment only" and
   hides its knobs in Classic, but `finish_land` grades in every mode. The main panel's Color Grading checkbox
   works in Classic, and its tuning is then out of reach.
3. **The Adv label "Rim (KR only)" is wrong:** Enhanced reads `rim_strength` too, at a quarter. "Diffuse Wrap (KR
   only)" is right.
4. **Adv Gloom "Fog Height Bias" edits `gloom_params.w`, which nothing reads.** Its caption describes valley fog
   that does not exist.
5. **`grade_warm_color` and `grade_cool_color` have no control.** The cool colour is clearly visible (2.2 to 5.7
   mean, up to 91%); the warm one is not (0.08 or less).
6. **`blur_radius`:** every preset value is below the slider minimum and the shader floor. Opening Adv Intensities
   rewrites it to 0.5, which looks the same.
7. **Main "KR Tonemap Brightness" and Adv "Exposure" are the same control in KR.**
8. **The Gloom "Height Fade Height" slider runs 0..200 world units.**
   - Land spans about -11.6..+11.5 (Z x 4/44).
   - The presets use 0.005 to 0.01, the bottom 0.005% of the slider.
   - A 0..12 range would cover every height.
9. **Adv "Normals" in Classic writes `normal_mode` 0** (`Geometric (fixed)`). This is harmless, since Classic
   shows no difference.
10. **The composite main sliders spread one value over several settings.**
    - Land Relief drives `normal_mode`, `enable_bent`, `sharpness_mix` and `sharpness_factor`.
    - Soft Lighting drives `fill_strength` and `diffuse_wrap`.
    - Highlights drives `rim_strength` (up to 0.45) and `specular_strength` (up to 0.25).

    Highlights at full is several times the presets; only the preset values were measured.
11. **Fog Noise Strength alone does nothing.** The fog pass returns no fog when Distance Density and Height Density
    are both under 0.001.
12. **`enable_water_animation` keeps its live value on every preset switch.** Its per-slot value matters only for
    the startup slot.

## The shipped land.toml differs from the factory presets

The bundled `dynamapper/assets/settings/shaders/land.toml` overrides the factory presets (see
[effect_visibility_validation.md](effect_visibility_validation.md)), so a fresh install shows its values. This audit
measured the factory file. The shipped file differs in 66 of 780 values. These differences change a verdict above:

- **Classic morning** turns on much of the KR set:
  - grunge 0.35;
  - KR transition grime, KR land temperature, relief shadow and mottle;
  - normal maps;
  - projected shadows (0.22);
  - art temperature 1.16;
  - fill 0.33, wrap 0.12, rim and specular;
  - stronger art shadow, light catch, depth tint and contact shadow (0.45, 0.58, 0.55, 0.59).

  It also sets `normal_mode` 0, `enable_bent` 0, water animation 0 and gloom amount 0.

  Classic shading has no fill, wrap, rim, specular or normal-map term. The KR land terms need KR shading or the KR
  profile, and contact shadows and art temperature need Fake Normals. All of those stay inert. Grunge, projected
  shadows and the three standard art strengths do act.
- **Every Classic and Enhanced slot** sets `kr_land_material_contrast_strength` 0.55 and
  `art_projected_shadow_strength` 0.22. These are inert there, except projected shadows in Classic morning.
- **Classic night** sets `global_lighting` 0.65 (factory 0.82) and `exposure` 1.0 (factory 1.2).
- **KR** turns projected shadows off in every slot; with the factory values they measure 0.61 to 1.58 mean and 5.4
  to 8.8%. It also sets `art_mottle_strength` to 0.9..1.15 (factory 0), and some projected-shadow and
  material-contrast values differ slightly.
- **`blur_radius`** is 0.5 in Classic morning, which looks the same as every other value.

## Tooltip drafts

There is one tooltip per setting. Wherever a setting has two controls (the main panel and the Advanced window), the
same text serves both. The amounts share their switch's text. Settings without a control have no tooltip.

### Main panel: whole scene

| Control | Setting | Tooltip |
|---|---|---|
| Tonemap | `enable_tonemap` | Compresses bright light so sunlit areas keep their detail instead of washing out. Turning it off brightens the whole frame, most in Classic and Enhanced. |
| Color Grading | `enable_grading` | Turns on the colour grading set under Advanced > Color Grading: vibrance, saturation, contrast and warm/cool tints. The biggest look change in Enhanced and KR; it also works in Classic. |
| Texture Smoothness | `enable_linear_filtering` | Blends neighbouring texture pixels on land and items instead of drawing hard square pixels. On in the KR presets. |
| Fog | `enable_fog` | Adds haze toward the screen edges and the bottom of the screen, with slowly drifting cloud patches. Only the KR presets turn it on; shape it under Advanced > Fog Params. |
| Brightness | `global_lighting` | Overall light level of the scene; the presets lower it for night and caves. The darker the scene, the more the light sources glow. |
| KR Tonemap Brightness | `kr_tonemap_params.x` | Brightens or darkens the KR image before its tone curve. KR style only; it does the same as Exposure, and the two multiply. |
| Light/Static Interaction | `light_decal_intensity` | How brightly lamps, torches and other light sources light the ground around them, at any time of day. Visible near light sources in Enhanced and KR; very faint in Classic. |
| Grunge | `enable_grunge`, `grunge_strength` | Lays a dirt and wear texture over land and items. Strong in the KR presets; the Enhanced preset amount is too light to see. |
| KR Color Wash | `enable_kr_color_wash`, `kr_color_wash_strength` | Adds large, soft patches of colour variation over land and items, like a painted wash. KR style only; at the preset strength it is very hard to see. |
| KR Color Wash Scale | `kr_color_wash_scale` | Size of the colour wash patches. KR style only, needs KR Color Wash on. |

### Main panel: terrain and water

| Control | Setting | Tooltip |
|---|---|---|
| Water Animation | `enable_water_animation` | Moves the water surfaces. Affects only water, and keeps its value when you switch presets. |
| Sun Strength | `diffuse_strength` | How strongly the sun lights land that faces it: the main light and shade on terrain. Classic uses it too, but it can be changed here only in Enhanced and KR. |
| Terrain Ambient Light | `ambient_strength` | Light that reaches every land tile, even in shade. Raise it to open up dark slopes, nights and caves; it also lights flat water. |
| Land Relief | `normal_mode`, `enable_bent`, `sharpness_mix`, `sharpness_factor` | Brings out the shape of hills and slopes with smoother normals and a crisper edge between sun and shade. Enhanced and KR only; visible on slopes, not on flat ground. |
| Soft Lighting | `fill_strength`, `diffuse_wrap` | Adds soft light from the sky and lets sunlight reach further around slopes, lifting the shade. Enhanced and KR only; strongest at night. |
| Highlights | `specular_strength`, `rim_strength` | Adds a shine where land reflects the sun and a thin light on slope edges. Enhanced and KR only; almost invisible at the preset values. |
| KR Land Temperature | `kr_land_temperature_strength` | Warms sunlit land and cools shaded land. KR shading only; visible by day. |
| KR Land Relief Shadow | `kr_land_relief_shadow_strength` | Darkens land at the foot of height steps and along cliff banks. KR shading only; visible by day. |
| KR Land Shadow Mottle | `kr_land_shadow_mottle_strength` | Breaks the shade on land into soft blotches. KR shading only; too small to see even at the highest setting. |
| KR Material Contrast | `kr_land_material_contrast_strength` | Makes grass, rock and other land textures slightly richer and more varied from tile to tile. KR style only, not on water; strongest by day. |
| KR Transition Grime | `enable_kr_transition_grime`, `kr_transition_grime_strength` | Dirties and darkens the edges where one kind of land meets another. KR style only; clear by day, faint at night. |
| Normal Maps (EC land only) | `enable_normal_maps`, `land_normal_map_strength` | Adds the small bumps stored with Enhanced Client land textures. Only textures with a bump layer change (snow); Enhanced and KR only, off in every preset. |

### Main panel: standard art and KR-style art

| Control | Setting | Tooltip |
|---|---|---|
| Shadow Shape | `art_shadow_strength` | Darkens the shaded side of items and buildings to give them volume. Clear in the KR presets; too light to see in the Classic and Enhanced presets. |
| Contact Shadows | `art_contact_shadow_strength` | Darkens items where they meet the ground. Needs Fake Normals on (KR presets). |
| Mottle | `art_mottle_strength` | Adds a dappled light and shade pattern to items. Off in every factory preset; clearest in KR. |
| Light Catch | `art_highlight_strength` | Brightens the sunlit side of items and buildings. Clear in KR; too light to see in the Classic and Enhanced presets. |
| Atmosphere Tint | `art_depth_tint_strength` | Fades upright items toward the Atmosphere Tint colour (Advanced > Lighting Colors) for a sense of depth. Clear in KR; too light to see in the other presets. |
| Temperature | `kr_art_temperature_strength` | Warms the sunlit side of items and cools their shaded side. Needs Fake Normals on; faint, mostly by day. |
| Projected Shadows | `enable_art_projected_shadows`, `art_projected_shadow_strength` | Casts a soft shadow from each item onto the ground. Only the KR presets turn it on. |
| Fake Normals | `enable_art_fake_normals` | Shades items as if they had depth, guessing their shape from the sprite. KR presets only; Contact Shadows and Temperature need it. |

### Advanced window

| Section: control | Setting | Tooltip |
|---|---|---|
| Effects: Visual Profile | `post_process_profile` | Which look's tone curve and style-only effects are used: Neutral, EC or KR. The Render Style buttons set it; the KR-only settings need KR here. |
| Effects: Shading Mode | `shading_mode` | How land is lit: per corner (Classic) or per pixel (Enhanced, KR). It decides which terrain settings have any effect; the Render Style buttons set it. |
| Effects: Normals | `normal_mode` | Bicubic uses the slope of the neighbouring tiles for rounder hills; Geometric shades each tile's own slope. Visible in Enhanced and KR; Classic shows no difference. |
| Toggles: Bent normals | `enable_bent` | Evens out the light on land that sits below higher ground, such as the foot of a slope or a dip. Visible on slopes in every style. |
| Toggles: Gloom | `enable_gloom`, `gloom_params.x` | Darkens and cools shaded land at sea level for a moodier look. KR presets only; clearest at night. |
| Toggles: Blur | `enable_blur`, `blur_strength` | Softens land textures slightly. On in the KR presets; land only. |
| Intensities: Exposure (Tonemap) | `exposure` | Brightens or darkens the whole frame before the tone curve. Needs Tonemap on; in KR it does the same as KR Tonemap Brightness. |
| Intensities: Specular | `specular_strength` | A shine where land reflects the sun toward you. Enhanced and KR; invisible at the preset values. |
| Intensities: Rim (KR only) | `rim_strength` | A thin light on land edges facing away from the sun. Enhanced (at a quarter) and KR; barely visible at the preset values. |
| Intensities: Fill (env) | `fill_strength` | Soft light from the sky added to all land, lifting the shade. Enhanced and KR; strongest at night. |
| Intensities: Diffuse Wrap | `diffuse_wrap` | Lets sunlight reach further around slopes, softening the edge of the shade. KR shading only. |
| Intensities: Sharpness Factor / Mix | `sharpness_factor`, `sharpness_mix` | Makes the line between sunlit and shaded land crisper: Factor sets how crisp, Mix how much of it is used. Visible in KR at night and in caves; faint in Enhanced. |
| Intensities: Blur Radius | `blur_radius` | How far the land blur reaches, in screen pixels. Needs Blur on; small values look the same as 0.5. |
| Lighting Colors: Light Color | `light_color` | Colour of the sunlight on land and on the lit side of items. Enhanced and KR, strongest at night; Classic land ignores it. |
| Lighting Colors: Atmosphere Tint | `atmosphere_tint` | Colour that upright items fade toward with the Atmosphere Tint slider. Visible in KR; also the gloom colour when Gloom Tint is black. |
| Lighting Colors: Fill Sky | `fill_sky_color` | Colour and strength (alpha) of the sky light from Soft Lighting. Enhanced and KR; its colour is hard to see. |
| Lighting Colors: Fill Ground | `fill_ground_color` | Colour of the soft light from below. Land faces up, so it has no visible effect. |
| Lighting Colors: Rim (rgb + power) | `rim_color` | Colour (KR only) and tightness (alpha) of the rim light. Hard to see at the preset values. |
| Color Grading: Grade Strength | `grade_params.x` | How much of the colour grading is applied; 0 is the same as Color Grading off. |
| Color Grading: Headroom Reserve | `grade_params.y` | How far below white the KR light on land is capped, so sunlit ground does not bleach. KR only, by day, with Enable Headroom Limit on. |
| Color Grading: Fill Chroma Tint | `grade_params.z` | How much the sky light's colour tints land in shade. Enhanced and KR; very faint. |
| Color Grading: Enable Headroom Limit | `grade_params.w` | Caps the light on land so sunlit ground keeps its colour instead of washing out. Enhanced and KR, by day; turning it off brightens daylit land a lot. |
| Color Grading: Vibrance | `grade_extra.x` | Boosts dull colours more than vivid ones. Needs Color Grading on. |
| Color Grading: Saturation | `grade_extra.y` | Strength of all colours; 1 leaves them unchanged. Needs Color Grading on. |
| Color Grading: Contrast | `grade_extra.z` | Pushes darks and lights apart; 1 leaves them unchanged. Needs Color Grading on. |
| Color Grading: Split Tone Strength | `grade_extra.w` | Tints bright areas warm and dark areas cool; 0 is off. Needs Color Grading on. |
| Gloom: Height Fade Height | `gloom_params.y` | Height above which the gloom fades out; the presets keep it to land at sea level. 0 lets it cover land at every height. |
| Gloom: Shadow Bias | `gloom_params.z` | Puts more of the gloom on land facing away from the sun. Visible in KR at night and in caves. |
| Gloom: Fog Height Bias | `gloom_params.w` | Not used by the renderer. |
| Gloom: Gloom Tint | `gloom_color` | Colour of the gloom, and in alpha how much colour it drains. Black uses the Atmosphere Tint; visible in KR at night. |
| Graphics: Static Light Decals | `static_light_decal_visibility` | Strength of the glow drawn around light sources. It shows at night and in caves (in Enhanced also faintly by day), and grows as Brightness drops. |
| Graphics: Sharpening Amount | `sharpening_amount` | Sharpens land textures. 0 in every preset; even 0.5 changes most of the frame. |
| Graphics: Art Projected Shadow Length | `art_projected_shadow_length` | How long the item shadows are. Needs Projected Shadows on. |
| Graphics: Art Projected Shadow Softness | `art_projected_shadow_softness` | How soft the edge of the item shadows is. Needs Projected Shadows on; the difference is small. |
| Fog Params: Fog Color | `fog_color` | Colour of the haze, and in alpha how thick it can get (0 turns fog off). Needs Fog on. |
| Fog Params: Distance Density | `fog_params.x` | How far the haze reaches in from the screen edges. Needs Fog on; with this and Height Density at 0 there is no fog at all. |
| Fog Params: Height Density | `fog_params.y` | Extra haze toward the bottom of the screen. Needs Fog on; too faint to see at the KR preset values. |
| Fog Params: Noise Scale | `fog_params.z` | Size of the drifting cloud patches. Only matters when Noise Strength is above 0. |
| Fog Params: Noise Strength | `fog_params.w` | How visible the slowly drifting cloud patches are. Needs Fog on and some Distance or Height Density. |
| Fog Params: Night Fog Tint | `fog_night_color` | A colour mixed into the haze, by the amount in alpha (the KR presets use more at night and in caves). Hard to see at the preset values. |

The other Advanced controls repeat a main-panel control and take its text. These are Tonemap, Color Grading,
Brightness (Global Lighting), Ambient, Diffuse, and the Graphics entries for texture smoothness, grunge, colour
wash, KR land effects, light decals, normal maps and the art strengths.

## Where the tooltips go

All the controls are in `dynamapper/src/core/render/dialogs/land_shader.rs`.

- **Main F3 window** (`egui::Window::new(title)`, line 213).
  - It builds the basic controls in four functions: `whole_scene_controls` (line 893), `terrain_controls` (930),
    `standard_art_controls` (965) and `stylized_art_controls` (975).
  - Each control is one call to a small helper:

    | Helper | Builds |
    |---|---|
    | `toggle_u32` | checkbox |
    | `toggle_tip` | checkbox with `.on_hover_text(tip)`; only Color Grading uses it |
    | `slider_s` | slider |
    | `slider_en` | slider grayed out when the style does not use it (`add_enabled`) |
    | `toggle_en` | grayed-out checkbox |
    | `toggle_slider` | strength slider plus its checkbox, the slider grayed out while off |

  - Land Relief, Soft Lighting and Highlights are `slider_en` calls whose value is spread over several settings
    (`apply_*_control`, lines 1087 to 1123). They have no tooltip yet.
  - The Render Style buttons already have tooltips (`RENDER_STYLE_PRESETS`, line 62).
- **Advanced window** ("Advanced Raw Shader Knobs", line 338).
  - It is a list of `ui.collapsing` sections: Effects (349), Toggles (429), Intensities (467), Lighting Colors
    (556), Color Grading (611), Gloom (679), Graphics Settings (721) and Fog Params (764).
  - They call the same `slider_s` and `toggle_u32`, plus `color3` and `color4`. A few `ui.checkbox` and
    `egui::Slider` calls are inline (Visual Profile, Shading Mode, Normals, Enable Headroom Limit, Fog Height
    Bias).

**Suggested shape.**

- **Helpers.** Give each helper a `tip: &str` argument and chain `.on_hover_text(tip).on_disabled_hover_text(tip)`
  on the widget's `Response`.
  - In egui 0.36, `on_hover_text` shows only on enabled widgets.
  - Many basic controls are grayed out by style, and that is exactly when a tip like "KR style only" matters.
- **Combined and colour controls.**
  - For `toggle_slider`, put the tip on both the slider and the checkbox.
  - For `color3` and `color4`, put it on the colour button and its label.
- **Text.** Keep the texts as named `const` strings next to `RENDER_STYLE_PRESETS`, one per setting, so both
  windows share them.

## Reproducing

The script batches every variant through the F3 panel. It follows the renderer reference-validation and KDE
preview rules: Desktop 4 only, an exact PID, isolated settings and cache copies, and the shared preview lock.

1. **Prepare the rig.**
   - Copy a debug build of the commit under test to
     `/home/claudio/rust-target/uodynamapper-scratch/shader-settings-audit/rig/bin/dynamapper`.
   - Set `SHADER_AUDIT_SCRATCH` to use another scratch folder.
2. **Run the stages:**

```sh
python3 scripts/dev-helpers/renderer/shader_settings_audit.py plan       # variant plan and launch list
python3 scripts/dev-helpers/renderer/shader_settings_audit.py run        # every launch, each under the preview lock
python3 scripts/dev-helpers/renderer/shader_settings_audit.py metrics    # diffs against the baselines
python3 scripts/dev-helpers/renderer/shader_settings_audit.py report     # results/report.md and summary.json
python3 scripts/dev-helpers/renderer/shader_settings_audit.py sheets     # one contact sheet per setting
```

- **`run`.** It takes launch ids to rerun a subset. Each launch runs under `flock` on the preview lock and
  `lowprio.sh`, and is retried once. The 63 launches take about two hours.
- **`calibrate`.** It captures the panel and the candidate views, for checking the button positions and the
  light-dense spots.
- **`report`.** It writes the per-setting suggestion that this note's verdicts start from. The manual overrides
  are listed in the threshold section.

## Related notes

- [effect_visibility_validation.md](effect_visibility_validation.md): normal-map inputs (only snow has a bump
  layer), the bundled `land.toml` overriding the factory presets, and earlier effect measurements.
- [static_light_visibility.md](static_light_visibility.md): how the light pools and glow sprites are gated.
